use std::collections::HashMap;

use lsp_types::{
    AnnotatedTextEdit, DocumentChangeOperation, DocumentChanges, OneOf,
    OptionalVersionedTextDocumentIdentifier, Position, Range, ResourceOp, TextDocumentEdit,
    TextEdit, WorkspaceEdit,
};

use super::*;

fn uri(path: &str) -> Uri {
    path_to_uri(Path::new(path)).unwrap()
}

fn edit(line: u32, text: &str) -> TextEdit {
    TextEdit {
        range: Range::new(Position::new(line, 0), Position::new(line, 1)),
        new_text: text.into(),
    }
}

#[cfg(unix)]
#[test]
fn file_uris_round_trip_including_spaces_and_non_ascii() {
    let path = Path::new("/tmp/a dir/ü.rs");
    let u = path_to_uri(path).unwrap();
    assert_eq!(u.as_str(), "file:///tmp/a%20dir/%C3%BC.rs");
    assert_eq!(uri_to_path(&u).as_deref(), Some(path));
    let web: Uri = "https://docs.rs/x".parse().unwrap();
    assert_eq!(uri_to_path(&web), None);
}

#[test]
fn a_workspace_edit_is_grouped_by_file_from_either_shape() {
    let mut changes = HashMap::new();
    changes.insert(uri("/b.rs"), vec![edit(1, "b")]);
    changes.insert(uri("/a.rs"), vec![edit(0, "a")]);
    let files = workspace_edit_files(WorkspaceEdit {
        changes: Some(changes),
        ..Default::default()
    });
    let names: Vec<&str> = files.iter().map(|(u, _)| u.as_str()).collect();
    assert_eq!(names, ["file:///a.rs", "file:///b.rs"]);

    let doc_edit = |path: &str, edits: Vec<OneOf<TextEdit, AnnotatedTextEdit>>| TextDocumentEdit {
        text_document: OptionalVersionedTextDocumentIdentifier {
            uri: uri(path),
            version: None,
        },
        edits,
    };
    let files = workspace_edit_files(WorkspaceEdit {
        document_changes: Some(DocumentChanges::Operations(vec![
            DocumentChangeOperation::Edit(doc_edit("/a.rs", vec![OneOf::Left(edit(0, "x"))])),
            DocumentChangeOperation::Op(ResourceOp::Create(lsp_types::CreateFile {
                uri: uri("/new.rs"),
                options: None,
                annotation_id: None,
            })),
            DocumentChangeOperation::Edit(doc_edit(
                "/a.rs",
                vec![OneOf::Right(AnnotatedTextEdit {
                    text_edit: edit(2, "y"),
                    annotation_id: "rename".into(),
                })],
            )),
        ])),
        ..Default::default()
    });
    assert_eq!(
        files.len(),
        1,
        "edits to one file are merged; resource ops skipped"
    );
    let texts: Vec<&str> = files[0].1.iter().map(|e| e.new_text.as_str()).collect();
    assert_eq!(texts, ["x", "y"]);
}

#[test]
fn a_rename_across_a_file_applies_every_occurrence() {
    // What a server sends for renaming `foo` to `renamed`, applied to a file
    // with no buffer open.
    let text = "fn foo() {}\nfn main() { foo(); /* 🎉 */ foo(); }\n";
    let at = |line, start, end| TextEdit {
        range: Range::new(Position::new(line, start), Position::new(line, end)),
        new_text: "renamed".into(),
    };
    let edits = vec![at(1, 12, 15), at(0, 3, 6), at(1, 28, 31)];
    assert_eq!(
        convert::apply_edits(text, &edits, Encoding::Utf16),
        "fn renamed() {}\nfn main() { renamed(); /* 🎉 */ renamed(); }\n"
    );
}

/// Drives a real language server through the whole client: spawn,
/// `initialize` behind the gate, `didOpen`, diagnostics arriving as an
/// event, a hover request, and a clean `shutdown`/`exit` with the process
/// reaped. clangd, because it ships with Xcode's command line tools and needs
/// no project to index; the file's type error is what it has to find.
///
/// `cargo test -p tty7 --bin tty7-app lsp_smoke -- --ignored`
#[test]
#[ignore = "needs clangd on PATH"]
fn lsp_smoke_a_real_server_publishes_diagnostics_and_shuts_down() {
    use std::time::Duration;

    let spec = servers::spec_for_language("c").unwrap();
    let Some((program, args)) =
        servers::resolve(spec, &std::env::var_os("PATH").unwrap_or_default())
    else {
        eprintln!("clangd not on PATH; skipping");
        return;
    };
    let dir = tempfile::TempDir::new().unwrap();
    let root = dir.path().canonicalize().unwrap();
    let file = root.join("main.c");
    let text = "int main(void) {\n    int x = \"not a number\";\n    return x;\n}\n";
    std::fs::write(&file, text).unwrap();

    let (client, events) = LspClient::spawn("clangd", &program, args, &root).unwrap();
    let within =
        |secs: u64, fut: std::pin::Pin<Box<dyn std::future::Future<Output = Option<Value>>>>| {
            smol::block_on(smol::future::or(fut, async move {
                smol::Timer::after(Duration::from_secs(secs)).await;
                None
            }))
        };

    let init = client.initialize(initialize_params(&root, spec));
    let result = within(30, Box::pin(async move { init.await.ok() })).expect("initialize answered");
    let init: lsp_types::InitializeResult = serde_json::from_value(result).unwrap();
    assert!(init.capabilities.hover_provider.is_some());
    client.notify_raw("initialized", json!({}));
    client.open_gate();

    let uri = path_to_uri(&file).unwrap();
    client.notify::<lsp_types::notification::DidOpenTextDocument>(
        lsp_types::DidOpenTextDocumentParams {
            text_document: lsp_types::TextDocumentItem::new(
                uri.clone(),
                "c".into(),
                0,
                text.into(),
            ),
        },
    );

    let events2 = events.clone();
    let published = within(
        60,
        Box::pin(async move {
            while let Ok(event) = events2.recv().await {
                match event {
                    ServerEvent::Notification { method, params }
                        if method == "textDocument/publishDiagnostics" =>
                    {
                        return Some(params);
                    }
                    _ => {}
                }
            }
            None
        }),
    )
    .expect("diagnostics published");
    let params: lsp_types::PublishDiagnosticsParams = serde_json::from_value(published).unwrap();
    assert_eq!(params.uri, uri);
    let (errors, _) = convert::count_problems(&params.diagnostics);
    assert!(errors >= 1, "{:?}", params.diagnostics);
    assert!(params.diagnostics.iter().any(|d| d.range.start.line == 1));

    let hover = client.request::<lsp_types::request::HoverRequest>(lsp_types::HoverParams {
        text_document_position_params: lsp_types::TextDocumentPositionParams::new(
            lsp_types::TextDocumentIdentifier::new(uri),
            Position::new(2, 11),
        ),
        work_done_progress_params: Default::default(),
    });
    let hovered = within(
        30,
        Box::pin(async move { hover.await.ok().map(|h| serde_json::to_value(h).unwrap()) }),
    );
    assert!(
        hovered.is_some_and(|h| !h.is_null()),
        "hover over `x` says something"
    );

    // The outline, the references to `x`, and signature help inside a call
    // all come back in shapes the editor reads.
    let doc_id = || lsp_types::TextDocumentIdentifier::new(path_to_uri(&file).unwrap());
    let symbols = client.request::<lsp_types::request::DocumentSymbolRequest>(
        lsp_types::DocumentSymbolParams {
            text_document: doc_id(),
            work_done_progress_params: Default::default(),
            partial_result_params: Default::default(),
        },
    );
    let outline = within(
        30,
        Box::pin(async move {
            symbols
                .await
                .ok()
                .flatten()
                .map(|s| serde_json::to_value(s).unwrap())
        }),
    )
    .map(|v| {
        symbols::outline(
            serde_json::from_value(v).unwrap(),
            &gpui_component::Rope::from(text),
            Encoding::Utf16,
        )
    })
    .expect("an outline");
    assert!(
        outline.symbols.iter().any(|s| s.name == "main"),
        "{outline:?}"
    );

    let references = client.request::<lsp_types::request::References>(lsp_types::ReferenceParams {
        text_document_position: lsp_types::TextDocumentPositionParams::new(
            doc_id(),
            Position::new(2, 11),
        ),
        work_done_progress_params: Default::default(),
        partial_result_params: Default::default(),
        context: lsp_types::ReferenceContext {
            include_declaration: true,
        },
    });
    let found = within(
        30,
        Box::pin(async move {
            references
                .await
                .ok()
                .flatten()
                .map(|r| serde_json::to_value(r).unwrap())
        }),
    )
    .expect("references");
    let found: Vec<lsp_types::Location> = serde_json::from_value(found).unwrap();
    assert!(
        found.len() >= 2,
        "the declaration and the use of x: {found:?}"
    );

    let shutdown = client.request_raw("shutdown", Value::Null);
    within(10, Box::pin(async move { shutdown.await.ok() }));
    client.notify_raw("exit", Value::Null);
    let mut exited = false;
    for _ in 0..50 {
        if client.reap_if_exited() {
            exited = true;
            break;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    client.kill();
    assert!(exited, "clangd exits on `exit`");
}

//! Claude Code sessions running in the background (`claude --bg`, or one sent
//! there from agent view). `claude --resume` refuses such a session and names
//! the short job id `claude attach` takes instead. Every live Claude process
//! keeps `<config>/sessions/<pid>.json`; a background one carries
//! `"kind": "bg"` and its `jobId`, so both directions (session → job for the
//! resume line, job → session for a pane running `claude attach`) are a read
//! of that directory, no `claude` spawned. [`resume_plan`] also checks
//! whether a session was ever saved, so a resume that would find nothing
//! starts fresh instead.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use serde::Deserialize;

use crate::core::agent_history::{ends, records, str_field};
use crate::core::cli_agent::{AgentSessionState, CLIAgent};
use crate::core::machine::{AgentFacts, PaneRecord};

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Entry {
    pid: i32,
    session_id: String,
    kind: Option<String>,
    job_id: Option<String>,
}

/// Claude's config root (`~/.claude`) on this machine.
pub fn local_claude_root() -> Option<PathBuf> {
    crate::core::agent_history::Roots::local().map(|roots| roots.claude)
}

/// `(session id, job id)` for each background session whose process is up.
/// A file whose process died (a crash leaves it behind) is not running.
fn live_jobs(dir: &Path) -> impl Iterator<Item = (String, String)> {
    std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| e.path().extension().is_some_and(|x| x == "json"))
        .filter_map(|e| serde_json::from_slice::<Entry>(&std::fs::read(e.path()).ok()?).ok())
        .filter(|e| e.kind.as_deref() == Some("bg") && pid_alive(e.pid))
        .filter_map(|e| Some((e.session_id, e.job_id.filter(|j| valid_job(j))?)))
}

/// The job id `session_id` is running in the background as, if it is.
pub fn job_for_session(dir: &Path, session_id: &str) -> Option<String> {
    live_jobs(dir).find_map(|(s, job)| (s == session_id).then_some(job))
}

/// The session background job `job` is running.
pub fn session_for_job(dir: &Path, job: &str) -> Option<String> {
    live_jobs(dir).find_map(|(s, j)| (j == job).then_some(s))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct JobState {
    session_id: Option<String>,
    resume_session_id: Option<String>,
}

/// The session `claude attach <job>` opens, with Claude's files under
/// `claude_root`. Without a live process (stopped, still respawning, killed by
/// a reboot) `jobs/<job>/state.json` still names it, so the pane records a
/// session to resume instead of caching a miss.
pub fn attached_session(claude_root: &Path, job: &str) -> Option<String> {
    session_for_job(&claude_root.join("sessions"), job).or_else(|| {
        let path = claude_root.join("jobs").join(job).join("state.json");
        let state: JobState = serde_json::from_slice(&std::fs::read(path).ok()?).ok()?;
        // A job resumed from another conversation keeps writing that one.
        let set = |s: Option<String>| s.filter(|s| !s.is_empty());
        set(state.resume_session_id).or(set(state.session_id))
    })
}

/// How to reopen an agent session, decided on the host that has its files.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ResumePlan {
    /// It is running in the background as this job: `claude --resume` would
    /// be refused, so attach instead.
    Attach(String),
    /// Its last turn ended: resume it with nothing to say.
    Resume,
    /// Its last turn was cut off mid-way (a reboot, a crash): resume it and
    /// tell it to carry on.
    Interrupted,
    /// Claude saved no transcript for it (it never took a turn), so a resume
    /// would stop at "No conversation found": start it fresh under its id.
    Fresh,
    /// The user ended it with `/exit` or `/quit`: a wake leaves it closed.
    Closed,
}

/// [`ResumePlan`] for `agent`'s `session_id`, with Claude's files under
/// `claude_root` (`~/.claude`). Every other agent just resumes.
pub fn resume_plan(claude_root: &Path, agent: CLIAgent, session_id: &str) -> ResumePlan {
    if agent != CLIAgent::Claude {
        return ResumePlan::Resume;
    }
    if let Some(job) = job_for_session(&claude_root.join("sessions"), session_id) {
        return ResumePlan::Attach(job);
    }
    // Fresh only on a clean read that finds nothing: an unreadable projects
    // dir (or a pane whose Claude kept its files elsewhere) says nothing about
    // whether the session was saved, and a resume that finds it loses nothing.
    let Ok(projects) = std::fs::read_dir(claude_root.join("projects")) else {
        return ResumePlan::Resume;
    };
    let file = format!("{session_id}.jsonl");
    for project in projects {
        let Ok(project) = project else {
            return ResumePlan::Resume;
        };
        let path = project.path().join(&file);
        if path.is_file() {
            if exited(&path) {
                return ResumePlan::Closed;
            }
            return match read_tail(&path).is_some_and(|t| interrupted(&t)) {
                true => ResumePlan::Interrupted,
                false => ResumePlan::Resume,
            };
        }
    }
    ResumePlan::Fresh
}

/// Whether the transcript at `path` ends with the user's own `/exit` or
/// `/quit`. A crash, kill or reboot writes no such turn.
fn exited(path: &Path) -> bool {
    let len = path.metadata().map_or(0, |m| m.len());
    let Ok((head, tail)) = ends(path, len) else {
        return false;
    };
    let text = if tail.is_empty() { head } else { tail };
    records(&text)
        .rev()
        .filter(|r| str_field(r, "type") == Some("user"))
        .filter_map(|r| r.pointer("/message/content")?.as_str().map(str::to_string))
        .find(|c| !c.starts_with("<local-command-"))
        .is_some_and(|c| c.contains("<command-name>/exit") || c.contains("<command-name>/quit"))
}

/// After `record.agent` takes an observation: a session the agent reported
/// becomes the pane's `last_session`. Nothing else replaces it, so an agent
/// with no session (`claude --version`) or one gone (a crash) keeps it;
/// `closed` ([`closed_on_leaving`]) lets it go.
pub fn note_session(record: &mut PaneRecord, closed: bool) {
    if closed {
        record.last_session = None;
    }
    if let Some(seen) = record.agent.as_ref().filter(|a| a.session_id.is_some()) {
        record.last_session = Some(AgentFacts {
            status: None,
            ..seen.clone()
        });
    }
}

/// Whether the agent that just left (`before`, now `after`) was a Claude
/// session the user `/exit`ed. Read by the daemon, on the host that has the
/// transcript, so the record says so before any wake looks.
pub fn closed_on_leaving(before: Option<&AgentFacts>, after: Option<&AgentFacts>) -> bool {
    closed_on_leaving_in(local_claude_root().as_deref(), before, after)
}

fn closed_on_leaving_in(
    claude_root: Option<&Path>,
    before: Option<&AgentFacts>,
    after: Option<&AgentFacts>,
) -> bool {
    let (Some(root), Some(left), None) = (claude_root, before, after) else {
        return false;
    };
    left.agent == CLIAgent::Claude
        && left.session_id.as_deref().is_some_and(|id| {
            let file = format!("{id}.jsonl");
            std::fs::read_dir(root.join("projects"))
                .into_iter()
                .flatten()
                .flatten()
                .map(|p| p.path().join(&file))
                .find(|p| p.is_file())
                .is_some_and(|p| exited(&p))
        })
}

/// `agent` with the `last` session filled in where a fresh agent of the same
/// kind has not reported one yet, or `last` when no agent is there.
pub fn with_last_session(
    agent: Option<AgentFacts>,
    last: Option<&AgentFacts>,
) -> Option<AgentFacts> {
    match (agent, last) {
        (None, last) => last.cloned(),
        (Some(a), Some(l)) if a.session_id.is_none() && a.agent == l.agent => Some(AgentFacts {
            session_id: l.session_id.clone(),
            launch_argv: a.launch_argv.or_else(|| l.launch_argv.clone()),
            ..a
        }),
        (a, _) => a,
    }
}

/// The shell started `command` with no agent in front: the user is using
/// the shell, so `last_session` is let go — unless the command is an
/// agent's own (`claude --version`, `codex --help`).
pub fn note_command(record: &mut PaneRecord, command: Option<&str>) {
    let argv: Vec<String> = command
        .unwrap_or_default()
        .split_whitespace()
        .map(str::to_string)
        .collect();
    if CLIAgent::detect_from_argv(&argv).is_none() {
        record.last_session = None;
    }
}

/// How much of a transcript's end [`interrupted`] reads: enough to hold a
/// long turn's background launches, not the whole of a long session.
const TAIL: u64 = 1024 * 1024;

/// The last [`TAIL`] bytes of `path`, from the first whole line in them.
fn read_tail(path: &Path) -> Option<String> {
    use std::io::{Read, Seek, SeekFrom};
    let mut file = std::fs::File::open(path).ok()?;
    let len = file.metadata().ok()?.len();
    let start = len.saturating_sub(TAIL);
    file.seek(SeekFrom::Start(start)).ok()?;
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes).ok()?;
    let text = String::from_utf8_lossy(&bytes).into_owned();
    Some(match start {
        0 => text,
        _ => text.split_once('\n').map(|(_, rest)| rest.to_string())?,
    })
}

/// Where a transcript's last turn stands.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Last {
    /// The user's message, or a tool result, with nothing after it.
    Unanswered,
    /// The model stopped mid-turn: a tool call, its token cap, a pause, or no
    /// stop reason at all (the process died while it streamed).
    Open,
    /// The turn ended, or Claude itself wrote the last word (an API error, the
    /// usage limit).
    Ended,
    /// The user stopped it (Esc).
    Stopped,
}

/// User entries that are not the user talking: slash commands and their
/// output, `!` shell lines, notices, hooks, other agents.
const WRAPPERS: &[&str] = &[
    "<command-",
    "<local-command",
    "<bash-",
    "<task-notification>",
    "<ide_",
    "<teammate-message",
    "<system-reminder>",
    "<user-prompt-submit-hook>",
];

/// Whether a Claude transcript's last turn was cut off, so a resume should
/// carry it on: the user's message or a tool result nothing answered, a model
/// reply that never ended its turn, or a turn that ended waiting on background
/// work (a background agent or shell) that never reported back. An ended
/// turn, the user's Esc, Claude's own error or limit message, and anything
/// unreadable are not.
pub fn interrupted(tail: &str) -> bool {
    let mut last = None;
    // This turn's background launches, by tool use id, until their result
    // names the task; then each one not yet reported back, as (tool use id,
    // task id).
    let mut launched: Vec<(String, bool)> = Vec::new();
    let mut pending: Vec<(String, Option<String>)> = Vec::new();
    for line in tail.lines() {
        let Ok(entry) = serde_json::from_str::<serde_json::Value>(line) else {
            continue;
        };
        if entry["isSidechain"] == true {
            continue;
        }
        let kind = entry["type"].as_str();
        // A task is reported back by whatever names it later: its notice, a
        // queued command or queue operation carrying it, a TaskOutput read.
        if matches!(kind, Some("user" | "attachment" | "queue-operation")) {
            pending.retain(|(id, task)| {
                !(line.contains(&format!("<tool-use-id>{id}"))
                    || task.as_deref().is_some_and(|t| line.contains(t)))
            });
        }
        if entry["isMeta"] == true {
            continue;
        }
        let message = &entry["message"];
        match kind {
            Some("assistant") => {
                let blocks = message["content"].as_array().into_iter().flatten();
                launched.extend(
                    blocks
                        .filter(|b| b["type"] == "tool_use")
                        .filter_map(|b| Some((b["id"].as_str()?.to_string(), launch(b)?))),
                );
                let synthetic =
                    message["model"] == "<synthetic>" || entry["isApiErrorMessage"] == true;
                last = Some(match message["stop_reason"].as_str() {
                    _ if synthetic => Last::Ended,
                    None | Some("tool_use" | "max_tokens" | "pause_turn") => Last::Open,
                    Some(_) => Last::Ended,
                });
            }
            Some("user") if entry["isCompactSummary"] != true => {
                let content = &message["content"];
                let results: Vec<&serde_json::Value> = content
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter(|b| b["type"] == "tool_result")
                    .collect();
                if !results.is_empty() {
                    for result in results {
                        let Some(id) = result["tool_use_id"].as_str() else {
                            continue;
                        };
                        if let Some(at) = launched.iter().position(|(l, _)| l == id) {
                            let (_, agent) = launched.remove(at);
                            let text = result["content"].to_string();
                            // An agent runs in the background only when its
                            // own result says it was launched there: one run
                            // in the foreground answers here, and a teammate
                            // is spawned and talks back on its own.
                            if !agent || text.contains("Async agent launched") {
                                pending.push((id.to_string(), task_id(&text)));
                            }
                        }
                    }
                    last = Some(Last::Unanswered);
                    continue;
                }
                let text = text_of(content);
                let trimmed = text.trim_start();
                if text.contains("[Request interrupted by user") {
                    last = Some(Last::Stopped);
                } else if !trimmed.is_empty() && !WRAPPERS.iter().any(|w| trimmed.starts_with(w)) {
                    last = Some(Last::Unanswered);
                    launched.clear();
                    pending.clear();
                }
            }
            _ => {}
        }
    }
    match last {
        Some(Last::Unanswered | Last::Open) => true,
        Some(Last::Ended) => !pending.is_empty(),
        Some(Last::Stopped) | None => false,
    }
}

/// Whether a tool call may start work in the background, and if so whether
/// it is an agent: a shell command asked to run there, or any agent, whose
/// result then says where it ran.
fn launch(block: &serde_json::Value) -> Option<bool> {
    let flag = &block["input"]["run_in_background"];
    match block["name"].as_str()? {
        "Bash" => (*flag == true || *flag == "true").then_some(false),
        "Agent" | "Task" => Some(true),
        _ => None,
    }
}

/// The task id a background launch's result gives it, which what reports it
/// back names.
fn task_id(text: &str) -> Option<String> {
    ["agentId: ", "ID: "].iter().find_map(|key| {
        let rest = &text[text.find(key)? + key.len()..];
        let id: String = rest
            .chars()
            .take_while(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_')
            .collect();
        (!id.is_empty()).then_some(id)
    })
}

/// A message's text: the string, or its text blocks joined.
fn text_of(content: &serde_json::Value) -> String {
    match content {
        serde_json::Value::String(s) => s.clone(),
        serde_json::Value::Array(blocks) => blocks
            .iter()
            .filter_map(|b| b["text"].as_str())
            .collect::<Vec<_>>()
            .join("\n"),
        _ => String::new(),
    }
}

/// The fork's per-pane daemon state, one field on upstream's `PaneState` so a
/// new piece of it touches no struct literal there.
#[derive(Default)]
pub(crate) struct PaneFork {
    /// See [`adopt_argv_session`].
    pub(crate) argv_session_checked: Option<(Vec<String>, Instant)>,
}

/// How long an argv's lookup stands before it is read again: the sessions
/// dir scan runs under the pane mutex, so never on every poll.
const RECHECK: Duration = Duration::from_secs(30);

/// Fill in the session id `argv` names for a pane no hook has spoken for
/// yet, which is what lets a reboot resume Claude without its hooks. `true`
/// when `session` changed. `checked` holds the argv last looked up and when:
/// a miss is retried, and a `claude attach` re-read (its job may move to
/// another conversation), once per [`RECHECK`].
pub fn adopt_argv_session(
    agent: Option<CLIAgent>,
    argv: Option<&[String]>,
    session: &mut Option<AgentSessionState>,
    checked: &mut Option<(Vec<String>, Instant)>,
) -> bool {
    let root = local_claude_root();
    adopt_argv_session_in(
        root.as_deref(),
        Instant::now(),
        agent,
        argv,
        session,
        checked,
    )
}

fn adopt_argv_session_in(
    claude_root: Option<&Path>,
    now: Instant,
    agent: Option<CLIAgent>,
    argv: Option<&[String]>,
    session: &mut Option<AgentSessionState>,
    checked: &mut Option<(Vec<String>, Instant)>,
) -> bool {
    let (Some(agent), Some(argv)) = (agent, argv) else {
        return false;
    };
    let job = attached_job(argv);
    let known = session.as_ref().and_then(|s| s.session_id.as_deref());
    // Only an attach is worth re-reading once known: its job is the one
    // source, and no hook reports for it.
    if known.is_some() && job.is_none() {
        return false;
    }
    if checked
        .as_ref()
        .is_some_and(|(a, at)| a == argv && now.duration_since(*at) < RECHECK)
    {
        return false;
    }
    *checked = Some((argv.to_vec(), now));
    // `claude attach <job>` names only its job; Claude's files map it back.
    let named = agent
        .session_id_in_argv(argv)
        .or_else(|| attached_session(claude_root?, job?));
    let Some(id) = named.filter(|id| Some(id.as_str()) != known) else {
        return false;
    };
    let sess = session.get_or_insert_with(Default::default);
    sess.session_id = Some(id);
    sess.launch_argv.get_or_insert_with(|| argv.to_vec());
    true
}

/// The job a `claude attach <job>` argv opens.
pub fn attached_job(argv: &[String]) -> Option<&str> {
    let at = argv.iter().position(|t| t == "attach")?;
    let names_claude = argv[..at]
        .iter()
        .any(|t| t.rsplit(['/', '\\']).next() == Some("claude"));
    argv.get(at + 1)
        .map(String::as_str)
        .filter(|j| names_claude && valid_job(j))
}

/// Job ids are short hex; anything else is not typed into a shell.
fn valid_job(job: &str) -> bool {
    !job.is_empty() && job.len() <= 64 && job.bytes().all(|b| b.is_ascii_alphanumeric())
}

#[cfg(unix)]
fn pid_alive(pid: i32) -> bool {
    // SAFETY: signal 0 only checks that `pid` exists and may be signalled.
    pid > 0
        && (unsafe { libc::kill(pid, 0) } == 0
            || std::io::Error::last_os_error().raw_os_error() == Some(libc::EPERM))
}

#[cfg(windows)]
fn pid_alive(pid: i32) -> bool {
    pid > 0 && !crate::daemon::winproc::wait_for_exit(pid as u32, std::time::Duration::ZERO)
}

#[cfg(test)]
mod tests {
    use super::*;

    const ID: &str = "2efe2a0b-933d-4efb-816b-b66a4c81cbd3";

    fn fixture(entries: &[(&str, String)]) -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        for (name, body) in entries {
            std::fs::write(dir.path().join(name), body).unwrap();
        }
        dir
    }

    fn entry(pid: u32, session: &str, kind: &str, job: Option<&str>) -> String {
        let job = job.map_or(String::new(), |j| format!(r#","jobId":"{j}""#));
        format!(r#"{{"pid":{pid},"sessionId":"{session}","kind":"{kind}","status":"busy"{job}}}"#)
    }

    #[test]
    fn a_live_background_session_maps_both_ways() {
        let me = std::process::id();
        let dir = fixture(&[
            ("1.json", entry(me, ID, "bg", Some("6011098d"))),
            ("2.json", entry(me, "other", "interactive", None)),
            ("3.key", "not json".into()),
        ]);
        assert_eq!(job_for_session(dir.path(), ID).as_deref(), Some("6011098d"));
        assert_eq!(session_for_job(dir.path(), "6011098d").as_deref(), Some(ID));
        assert_eq!(job_for_session(dir.path(), "other"), None);
    }

    #[test]
    fn a_dead_or_foreground_session_is_not_running_in_the_background() {
        let dir = fixture(&[
            ("1.json", entry(u32::MAX >> 1, ID, "bg", Some("6011098d"))),
            (
                "2.json",
                entry(std::process::id(), "fg", "interactive", Some("abcd")),
            ),
            ("3.json", "{broken".into()),
        ]);
        assert_eq!(job_for_session(dir.path(), ID), None);
        assert_eq!(job_for_session(dir.path(), "fg"), None);
        assert_eq!(job_for_session(Path::new("/nonexistent/tty7"), ID), None);
    }

    const USER_LAST: &str = r#"{"type":"assistant","message":{"stop_reason":"end_turn","content":[{"type":"text","text":"done"}]}}
{"type":"user","message":{"role":"user","content":"now fix the build"}}
"#;

    fn turn(lines: &[&str]) -> bool {
        interrupted(&lines.join("\n"))
    }

    const PROMPT: &str = r#"{"type":"user","message":{"role":"user","content":"fix the build"}}"#;
    const ENDED: &str = r#"{"type":"assistant","message":{"model":"claude-opus-5-5","stop_reason":"end_turn","content":[{"type":"text","text":"done"}]}}"#;

    #[test]
    fn a_turn_cut_off_is_interrupted() {
        assert!(turn(&[ENDED, PROMPT]), "the user's message, unanswered");
        let call = r#"{"type":"assistant","message":{"model":"claude-opus-5-5","stop_reason":"tool_use","content":[{"type":"tool_use","id":"t1","name":"Bash","input":{}}]}}"#;
        assert!(turn(&[PROMPT, call]), "a tool call nothing ran");
        let result = r#"{"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"t1","content":"ok"}]}}"#;
        assert!(
            turn(&[PROMPT, call, result]),
            "a tool result nothing answered"
        );
        for reason in ["null", r#""max_tokens""#, r#""pause_turn""#] {
            let reply = format!(
                r#"{{"type":"assistant","message":{{"model":"claude-opus-5-5","stop_reason":{reason},"content":[{{"type":"text","text":"Let me"}}]}}}}"#
            );
            assert!(turn(&[PROMPT, &reply]), "stopped on {reason}");
        }
        let thinking = r#"{"type":"assistant","message":{"model":"claude-opus-5-5","stop_reason":null,"content":[{"type":"thinking","thinking":"hm"}]}}"#;
        assert!(turn(&[PROMPT, thinking]), "only thinking so far");
    }

    #[test]
    fn an_ended_stopped_or_claude_written_turn_is_not() {
        assert!(!turn(&[PROMPT, ENDED]));
        let refusal = r#"{"type":"assistant","message":{"model":"claude-opus-5-5","stop_reason":"refusal","content":[]}}"#;
        assert!(!turn(&[PROMPT, refusal]), "a refusal ends the turn");
        let esc = r#"{"type":"user","message":{"content":[{"type":"text","text":"[Request interrupted by user]"}]}}"#;
        assert!(!turn(&[PROMPT, esc]), "Esc is the user's own stop");
        let limit = r#"{"type":"assistant","message":{"model":"<synthetic>","stop_reason":"stop_sequence","content":[{"type":"text","text":"You've hit your usage limit"}]}}"#;
        assert!(!turn(&[PROMPT, limit]), "Claude's own limit message");
        let api_error = r#"{"type":"assistant","isApiErrorMessage":true,"message":{"model":"<synthetic>","stop_reason":"stop_sequence","content":[{"type":"text","text":"API Error: 529"}]}}"#;
        assert!(!turn(&[PROMPT, api_error]), "an API error");
        let no_reply = r#"{"type":"assistant","message":{"model":"<synthetic>","stop_reason":"stop_sequence","content":[{"type":"text","text":"No response requested."}]}}"#;
        assert!(!turn(&[PROMPT, no_reply]));
        assert!(!turn(&[]), "nothing readable is not a reason to continue");
        assert!(!turn(&["{broken", "not json"]));
    }

    #[test]
    fn what_is_not_the_user_talking_does_not_reopen_a_turn() {
        for wrapper in [
            r#"{"type":"user","message":{"content":"<command-name>/exit</command-name>"}}"#,
            r#"{"type":"user","message":{"content":"<local-command-stdout>Bye!</local-command-stdout>"}}"#,
            r#"{"type":"user","message":{"content":"<bash-input>ls</bash-input>"}}"#,
            r#"{"type":"user","message":{"content":"<bash-stdout>a</bash-stdout><bash-stderr></bash-stderr>"}}"#,
            r#"{"type":"user","message":{"content":"<task-notification>\n<task-id>x</task-id>\n<status>completed</status>\n</task-notification>"}}"#,
            r#"{"type":"user","message":{"content":[{"type":"text","text":"<ide_opened_file>a.rs</ide_opened_file>"}]}}"#,
            r#"{"type":"user","message":{"content":"<teammate-message from=\"x\">hi</teammate-message>"}}"#,
            r#"{"type":"user","message":{"content":"<system-reminder>x</system-reminder>"}}"#,
            r#"{"type":"user","isMeta":true,"message":{"content":"Caveat: x"}}"#,
            r#"{"type":"user","isCompactSummary":true,"message":{"content":"This session is being continued"}}"#,
            r#"{"type":"user","isSidechain":true,"message":{"content":"a subagent's own prompt"}}"#,
            r#"{"type":"assistant","isSidechain":true,"message":{"model":"claude-opus-5-5","stop_reason":null,"content":[]}}"#,
            r#"{"type":"system","subtype":"turn_duration"}"#,
        ] {
            assert!(!turn(&[PROMPT, ENDED, wrapper]), "{wrapper}");
        }
    }

    #[test]
    fn a_truncated_last_line_is_read_past() {
        let cut = r#"{"type":"user","message":{"content":"half a li"#;
        assert!(!turn(&[PROMPT, ENDED, cut]));
        assert!(turn(&[ENDED, PROMPT, cut]));
    }

    /// The end of a real session (pixkidz, trimmed): a background build was
    /// started, the turn ended waiting on it, and the app went down before it
    /// reported back.
    const BACKGROUND_IN_FLIGHT: &[&str] = &[
        r#"{"type":"user","message":{"role":"user","content":"is the TestFlight link live?"}}"#,
        r#"{"type":"assistant","message":{"model":"claude-opus-5-5","stop_reason":"tool_use","content":[{"type":"tool_use","id":"toolu_01Bg","name":"Bash","input":{"command":"gh run watch","run_in_background":true}}]}}"#,
        r#"{"type":"user","message":{"content":[{"tool_use_id":"toolu_01Bg","type":"tool_result","content":"Command running in background with ID: bywb5zm89. Output is being written to: /private/tmp/claude-501/x/tasks/bywb5zm89.output. You will be notified when it completes."}]}}"#,
        r#"{"type":"assistant","message":{"model":"claude-opus-5-5","stop_reason":"end_turn","content":[{"type":"text","text":"The TestFlight link works now, so you can send the invite."}]}}"#,
    ];

    #[test]
    fn a_turn_that_ended_waiting_on_background_work_is_interrupted() {
        assert!(turn(BACKGROUND_IN_FLIGHT), "the build never reported back");

        let agent = [
            PROMPT,
            r#"{"type":"assistant","message":{"model":"claude-opus-5-5","stop_reason":"tool_use","content":[{"type":"tool_use","id":"toolu_02Ag","name":"Agent","input":{"prompt":"x","subagent_type":"general-purpose"}}]}}"#,
            r#"{"type":"user","message":{"content":[{"tool_use_id":"toolu_02Ag","type":"tool_result","content":[{"type":"text","text":"Async agent launched successfully.\nagentId: a0cc7ecfd6ae0327f (internal ID)"}]}]}}"#,
            ENDED,
        ];
        assert!(turn(&agent), "a background agent still out");

        let mut grep = agent;
        grep[1] = r#"{"type":"assistant","message":{"model":"claude-opus-5-5","stop_reason":"tool_use","content":[{"type":"tool_use","id":"toolu_02Ag","name":"Bash","input":{"command":"grep -r background"}}]}}"#;
        assert!(
            !turn(&grep),
            "a result that only mentions one launches nothing"
        );
        let mut foreground = agent;
        foreground[2] = r#"{"type":"user","message":{"content":[{"tool_use_id":"toolu_02Ag","type":"tool_result","content":[{"type":"text","text":"Done: 3 files fixed."}]}]}}"#;
        assert!(!turn(&foreground), "an agent that answered in place");
        let mut teammate = agent;
        teammate[2] = r#"{"type":"user","message":{"content":[{"tool_use_id":"toolu_02Ag","type":"tool_result","content":[{"type":"text","text":"Spawned successfully.\nagent_id: ci@session-1"}]}]}}"#;
        assert!(!turn(&teammate), "a teammate talks back on its own");

        for named in [
            r#"{"type":"attachment","attachment":{"type":"queued_command","prompt":"<task-notification><task-id>a0cc7ecfd6ae0327f</task-id>"}}"#,
            r#"{"type":"queue-operation","operation":"enqueue","content":"a0cc7ecfd6ae0327f done"}"#,
            r#"{"type":"user","message":{"content":[{"tool_use_id":"toolu_03","type":"tool_result","content":"task a0cc7ecfd6ae0327f: completed"}]}}"#,
        ] {
            assert!(!turn(&[&agent[..], &[named, ENDED]].concat()), "{named}");
        }
        let said = r#"{"type":"assistant","message":{"model":"claude-opus-5-5","stop_reason":"end_turn","content":[{"type":"text","text":"a0cc7ecfd6ae0327f is still running"}]}}"#;
        assert!(
            turn(&[&agent[..], &[said]].concat()),
            "the model naming it is no report"
        );

        let reported = r#"{"type":"user","message":{"content":"<task-notification>\n<task-id>a0cc7ecfd6ae0327f</task-id>\n<status>completed</status>\n</task-notification>"}}"#;
        assert!(
            !turn(&[&agent[..], &[reported, ENDED]].concat()),
            "it reported back"
        );
        let by_tool_use = r#"{"type":"user","message":{"content":"<task-notification>\n<task-id>bywb5zm89</task-id>\n<tool-use-id>toolu_01Bg</tool-use-id>\n<status>killed</status>\n</task-notification>"}}"#;
        assert!(!turn(
            &[BACKGROUND_IN_FLIGHT, &[by_tool_use, ENDED]].concat()
        ));

        let moved_on = [ENDED, PROMPT, ENDED];
        assert!(
            !turn(&[BACKGROUND_IN_FLIGHT, &moved_on].concat()),
            "the user moved on to another turn"
        );
        let esc = r#"{"type":"user","message":{"content":[{"type":"text","text":"[Request interrupted by user]"}]}}"#;
        assert!(
            !turn(&[BACKGROUND_IN_FLIGHT, &[esc]].concat()),
            "the user stopped it"
        );
    }

    #[test]
    fn a_tail_starts_at_a_whole_line() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("t.jsonl");
        let filler = format!("{{\"pad\":\"{}\"}}\n", "x".repeat(TAIL as usize));
        std::fs::write(&path, format!("{filler}{USER_LAST}")).unwrap();
        let tail = read_tail(&path).unwrap();
        assert!(tail.len() as u64 <= TAIL);
        assert!(tail.starts_with('{'), "the cut line is dropped");
        assert!(interrupted(&tail));
    }

    #[test]
    fn a_session_attaches_resumes_or_starts_fresh() {
        let root = tempfile::tempdir().unwrap();
        let plan = |id: &str| resume_plan(root.path(), CLIAgent::Claude, id);
        assert_eq!(
            plan(ID),
            ResumePlan::Resume,
            "no projects dir to read: can't say it was never saved"
        );

        let project = root.path().join("projects").join("-Users-me-src");
        std::fs::create_dir_all(&project).unwrap();
        assert_eq!(plan(ID), ResumePlan::Fresh, "read, and nothing saved");

        std::fs::write(project.join(format!("{ID}.jsonl")), "{}").unwrap();
        assert_eq!(plan(ID), ResumePlan::Resume);
        std::fs::write(project.join(format!("{ID}.jsonl")), USER_LAST).unwrap();
        assert_eq!(plan(ID), ResumePlan::Interrupted);

        let sessions = root.path().join("sessions");
        std::fs::create_dir_all(&sessions).unwrap();
        std::fs::write(
            sessions.join("1.json"),
            entry(std::process::id(), ID, "bg", Some("6011098d")),
        )
        .unwrap();
        assert_eq!(plan(ID), ResumePlan::Attach("6011098d".into()));
        assert_eq!(
            resume_plan(root.path(), CLIAgent::Codex, "nothing-saved"),
            ResumePlan::Resume,
            "only Claude is checked"
        );
    }

    fn argv(a: &[&str]) -> Vec<String> {
        a.iter().map(|s| s.to_string()).collect()
    }

    fn job_state(root: &Path, job: &str, body: &str) {
        let dir = root.join("jobs").join(job);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("state.json"), body).unwrap();
    }

    #[test]
    fn a_users_own_exit_is_closed() {
        let root = tempfile::tempdir().unwrap();
        let project = root.path().join("projects/-work");
        std::fs::create_dir_all(&project).unwrap();
        let user = |content: &str| {
            serde_json::json!({"type": "user", "message": {"role": "user", "content": content}})
                .to_string()
        };
        let write = |id: &str, lines: &[String]| {
            std::fs::write(project.join(format!("{id}.jsonl")), lines.join("\n") + "\n").unwrap();
        };
        let exit = "<command-name>/exit</command-name>\n  <command-message>exit</command-message>";
        write(
            "exited",
            &[
                user("fix the tests"),
                user(exit),
                user("<local-command-stdout>Goodbye!</local-command-stdout>"),
                r#"{"type":"last-prompt","leafUuid":"x"}"#.into(),
            ],
        );
        write("quit", &[user("<command-name>/quit</command-name>")]);
        write(
            "crashed",
            &[user("fix the tests"), r#"{"type":"assistant"}"#.into()],
        );
        write("exited-then-resumed", &[user(exit), user("keep going")]);
        let plan = |id: &str| resume_plan(root.path(), CLIAgent::Claude, id);
        assert_eq!(plan("exited"), ResumePlan::Closed);
        assert_eq!(plan("quit"), ResumePlan::Closed);
        assert_eq!(
            plan("crashed"),
            ResumePlan::Interrupted,
            "a crash or kill mid-reply resumes, as cut off"
        );
        assert_eq!(
            plan("exited-then-resumed"),
            ResumePlan::Interrupted,
            "a message after the exit, unanswered"
        );
    }

    #[test]
    fn an_agent_leaving_after_exit_closes_its_session_and_a_crash_keeps_it() {
        let root = tempfile::tempdir().unwrap();
        let project = root.path().join("projects/-work");
        std::fs::create_dir_all(&project).unwrap();
        let user = |c: &str| {
            serde_json::json!({"type": "user", "message": {"role": "user", "content": c}})
                .to_string()
        };
        std::fs::write(
            project.join("exited.jsonl"),
            [user("hi"), user("<command-name>/exit</command-name>")].join("\n"),
        )
        .unwrap();
        std::fs::write(project.join("crashed.jsonl"), user("hi")).unwrap();
        let claude = |id: &str| AgentFacts {
            agent: CLIAgent::Claude,
            session_id: Some(id.to_string()),
            launch_argv: None,
            status: None,
        };
        let left = |id: &str| closed_on_leaving_in(Some(root.path()), Some(&claude(id)), None);
        assert!(left("exited"));
        assert!(!left("crashed"));
        assert!(
            !closed_on_leaving_in(
                Some(root.path()),
                Some(&claude("exited")),
                Some(&claude("exited"))
            ),
            "still running: nothing left"
        );

        let mut p = PaneRecord::new(1);
        p.last_session = Some(claude("exited"));
        note_session(&mut p, left("crashed"));
        assert!(p.last_session.is_some(), "a crash keeps it");
        note_session(&mut p, left("exited"));
        assert!(p.last_session.is_none(), "an /exit lets it go");
    }

    #[test]
    fn a_fresh_agent_keeps_the_last_session_id_per_field() {
        let facts = |agent, id: Option<&str>| AgentFacts {
            agent,
            session_id: id.map(str::to_string),
            launch_argv: None,
            status: None,
        };
        let last = facts(CLIAgent::Claude, Some("old"));
        let id = |a| with_last_session(a, Some(&last)).and_then(|f| f.session_id);
        assert_eq!(id(None).as_deref(), Some("old"));
        assert_eq!(
            id(Some(facts(CLIAgent::Claude, None))).as_deref(),
            Some("old")
        );
        assert_eq!(
            id(Some(facts(CLIAgent::Claude, Some("new")))).as_deref(),
            Some("new")
        );
        assert_eq!(
            id(Some(facts(CLIAgent::Codex, None))),
            None,
            "not another agent's"
        );
    }

    #[test]
    fn only_a_reported_session_replaces_the_last_one_and_a_shell_command_clears_it() {
        let facts = |id: Option<&str>| AgentFacts {
            agent: CLIAgent::Claude,
            session_id: id.map(str::to_string),
            launch_argv: None,
            status: Some(crate::core::cli_agent::AgentStatus::Working),
        };
        let mut p = PaneRecord::new(1);
        p.agent = Some(facts(Some("a")));
        note_session(&mut p, false);
        assert_eq!(
            p.last_session,
            Some(AgentFacts {
                status: None,
                ..facts(Some("a"))
            })
        );
        p.agent = Some(facts(None));
        note_session(&mut p, false);
        p.agent = None;
        note_session(&mut p, false);
        assert_eq!(
            p.last_session.as_ref().unwrap().session_id.as_deref(),
            Some("a")
        );
        note_command(&mut p, Some("codex --help"));
        assert!(p.last_session.is_some());
        note_command(&mut p, None);
        assert!(
            p.last_session.is_none(),
            "a command it can't name is the shell's"
        );
    }

    #[test]
    fn an_argv_session_is_adopted_once() {
        let (mut session, mut checked) = (None, None);
        let named = argv(&["claude", "--session-id", ID]);
        let adopt = |s: &mut _, c: &mut _, now| {
            adopt_argv_session_in(None, now, Some(CLIAgent::Claude), Some(&named), s, c)
        };
        let t0 = Instant::now();
        assert!(adopt(&mut session, &mut checked, t0));
        let got = session.clone().unwrap();
        assert_eq!(got.session_id.as_deref(), Some(ID));
        assert_eq!(got.launch_argv.as_deref(), Some(&named[..]));
        assert!(!adopt(&mut session, &mut checked, t0 + RECHECK * 2));
    }

    #[test]
    fn an_attach_miss_is_retried_only_after_the_window_and_follows_its_job() {
        let root = tempfile::tempdir().unwrap();
        let attach = argv(&["claude", "attach", "6011098d"]);
        let (mut session, mut checked) = (None, None);
        let mut adopt = |s: &mut Option<AgentSessionState>, now| {
            adopt_argv_session_in(
                Some(root.path()),
                now,
                Some(CLIAgent::Claude),
                Some(&attach),
                s,
                &mut checked,
            )
        };
        let t0 = Instant::now();
        assert!(!adopt(&mut session, t0), "nothing on disk yet");
        job_state(
            root.path(),
            "6011098d",
            &format!(r#"{{"sessionId":"{ID}"}}"#),
        );
        assert!(!adopt(&mut session, t0 + RECHECK / 2), "within the window");
        assert!(adopt(&mut session, t0 + RECHECK));
        assert_eq!(session.as_ref().unwrap().session_id.as_deref(), Some(ID));

        job_state(
            root.path(),
            "6011098d",
            r#"{"sessionId":"x","resumeSessionId":"moved"}"#,
        );
        assert!(!adopt(&mut session, t0 + RECHECK * 3 / 2));
        assert!(adopt(&mut session, t0 + RECHECK * 2), "the job moved on");
        assert_eq!(
            session.as_ref().unwrap().session_id.as_deref(),
            Some("moved")
        );
        assert!(
            !adopt(&mut session, t0 + RECHECK * 3),
            "unchanged is no change"
        );
    }

    #[test]
    fn a_live_job_wins_over_its_state_file_and_blanks_are_skipped() {
        let root = tempfile::tempdir().unwrap();
        job_state(
            root.path(),
            "6011098d",
            r#"{"sessionId":"stale","resumeSessionId":""}"#,
        );
        assert_eq!(
            attached_session(root.path(), "6011098d").as_deref(),
            Some("stale")
        );
        job_state(
            root.path(),
            "44883d52",
            r#"{"sessionId":"","resumeSessionId":""}"#,
        );
        assert_eq!(attached_session(root.path(), "44883d52"), None);

        let sessions = root.path().join("sessions");
        std::fs::create_dir_all(&sessions).unwrap();
        std::fs::write(
            sessions.join("1.json"),
            entry(std::process::id(), ID, "bg", Some("6011098d")),
        )
        .unwrap();
        assert_eq!(
            attached_session(root.path(), "6011098d").as_deref(),
            Some(ID)
        );
    }

    #[test]
    fn a_job_with_no_process_still_names_the_session_it_runs() {
        // A stopped job, one `claude attach` is still respawning, or one a
        // reboot killed has no `sessions/<pid>.json`; its state file remains.
        // A resumed job runs `resumeSessionId`, not its own id.
        let root = tempfile::tempdir().unwrap();
        let job = root.path().join("jobs").join("6011098d");
        std::fs::create_dir_all(&job).unwrap();
        std::fs::write(
            job.join("state.json"),
            format!(
                r#"{{"state":"done","sessionId":"6011098d-13db-4e04-8991-8032b55894a8","resumeSessionId":"{ID}"}}"#
            ),
        )
        .unwrap();
        assert_eq!(
            attached_session(root.path(), "6011098d").as_deref(),
            Some(ID)
        );
        assert_eq!(attached_session(root.path(), "ffffffff"), None);

        let fresh = root.path().join("jobs").join("44883d52");
        std::fs::create_dir_all(&fresh).unwrap();
        std::fs::write(fresh.join("state.json"), r#"{"sessionId":"own"}"#).unwrap();
        assert_eq!(
            attached_session(root.path(), "44883d52").as_deref(),
            Some("own")
        );
    }

    #[test]
    fn an_attach_argv_names_its_job() {
        assert_eq!(
            attached_job(&argv(&["/usr/local/bin/claude", "attach", "6011098d"])),
            Some("6011098d")
        );
        assert_eq!(attached_job(&argv(&["claude", "--resume", ID])), None);
        assert_eq!(attached_job(&argv(&["tmux", "attach", "main"])), None);
    }
}

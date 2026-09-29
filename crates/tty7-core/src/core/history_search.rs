//! Full-text search over what was said in past agent sessions: the text of
//! the user's prompts and the agent's replies, never tool calls or their
//! output. Claude Code (and Qoder, which copied its layout) and Codex.
//!
//! Every file is read whole, line by line, newest first, and what it says is
//! kept in memory by path, size and modification time, so a second query
//! reads only the sessions that moved since.

use std::collections::HashMap;
use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::sync::{LazyLock, Mutex};
use std::time::{Duration, Instant, SystemTime};

use serde::Deserialize;
use serde::de::{Deserializer, IgnoredAny, SeqAccess, Visitor};

use crate::core::agent_history::{
    Found, Roots, claude_files, codex_files, codex_not_the_users, strip_injected, unix,
};
use crate::core::cli_agent::CLIAgent;

/// ponytail: a query stops after this long and shows what it found; the
/// next one starts where the cache is warm. A persistent index if a cold
/// history outgrows it.
const TIME_BUDGET: Duration = Duration::from_secs(3);

/// ponytail: stop at this many matching sessions, newest first, so an older
/// session with a better match can be missed.
const MAX_SESSIONS: usize = 100;

/// ponytail: the cache stops growing at this much text; files past it are
/// read again on every query. An on-disk index if histories get that big.
const MAX_CACHED_BYTES: usize = 256 * 1024 * 1024;

/// How much of a message a snippet shows around the match.
const BEFORE_CHARS: usize = 40;
const AFTER_CHARS: usize = 120;

/// One past session that said what was searched for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HistoryHit {
    pub agent: CLIAgent,
    pub id: String,
    /// Where it ran, and so where it resumes.
    pub cwd: Option<PathBuf>,
    /// Last written, in Unix seconds.
    pub updated: u64,
    /// The best matching message, cut down to one line around the match.
    pub snippet: String,
    /// How many of its messages match.
    pub hits: usize,
    /// How well the best message matched; see `score`.
    pub score: i32,
}

/// Sessions under `roots` whose conversation contains `query`, ignoring
/// case: best match first, then newest.
pub fn search(roots: &Roots, query: &str) -> Vec<HistoryHit> {
    let needle = query.trim().to_lowercase();
    if needle.is_empty() {
        return Vec::new();
    }
    let mut files = claude_files(&roots.claude.join("projects"), CLIAgent::Claude);
    files.extend(claude_files(
        &roots.qoder.join("projects"),
        CLIAgent::QoderCLI,
    ));
    files.extend(claude_files(
        &roots.qoder_cn.join("projects"),
        CLIAgent::QoderCLICn,
    ));
    files.extend(codex_files(&roots.codex));
    files.sort_by_key(|f| std::cmp::Reverse(f.modified));

    let started = Instant::now();
    let mut out = Vec::new();
    for file in files {
        if out.len() >= MAX_SESSIONS || started.elapsed() > TIME_BUDGET {
            break;
        }
        let hit = with_transcript(&file, |t| {
            t.search(&needle, file.agent, unix(file.modified))
        });
        out.extend(hit.flatten());
    }
    rank(&mut out);
    out
}

/// Best score first, then the most recently written.
fn rank(hits: &mut [HistoryHit]) {
    hits.sort_by_key(|h| (std::cmp::Reverse(h.score), std::cmp::Reverse(h.updated)));
}

/// What one transcript says, as far as a search cares.
#[derive(Default)]
struct Transcript {
    id: Option<String>,
    cwd: Option<PathBuf>,
    messages: Vec<Message>,
}

struct Message {
    from_user: bool,
    text: String,
    lower: String,
}

impl Transcript {
    fn push(&mut self, from_user: bool, text: &str) {
        let text = text.trim();
        if !text.is_empty() {
            self.messages.push(Message {
                from_user,
                lower: text.to_lowercase(),
                text: text.to_owned(),
            });
        }
    }

    fn bytes(&self) -> usize {
        self.messages
            .iter()
            .map(|m| m.text.len() + m.lower.len())
            .sum()
    }

    fn search(&self, needle: &str, agent: CLIAgent, updated: u64) -> Option<HistoryHit> {
        let mut best: Option<(i32, &Message, usize)> = None;
        let mut hits = 0;
        for m in &self.messages {
            let Some(at) = m.lower.find(needle) else {
                continue;
            };
            hits += 1;
            let score = score(m, at);
            if best.is_none_or(|(s, ..)| score > s) {
                best = Some((score, m, at));
            }
        }
        let (score, message, at) = best?;
        Some(HistoryHit {
            agent,
            id: self.id.clone()?,
            cwd: self.cwd.clone(),
            updated,
            snippet: snippet(message, at, needle.len()),
            hits,
            score,
        })
    }
}

/// What the user typed outranks what the agent answered, and a match at the
/// start of a word outranks one inside it.
fn score(m: &Message, at: usize) -> i32 {
    let word_start = m.lower[..at]
        .chars()
        .next_back()
        .is_none_or(|c| !c.is_alphanumeric());
    1 + 2 * i32::from(m.from_user) + i32::from(word_start)
}

/// One line of `m` around the match at byte `at` of its lowercase text.
fn snippet(m: &Message, at: usize, len: usize) -> String {
    // Lowercasing that changed a byte length (`İ`) would shift the offsets
    // into the original, so such a message is shown lowercase.
    let text = match m.text.len() == m.lower.len() {
        true => m.text.as_str(),
        false => m.lower.as_str(),
    };
    let start = text[..at]
        .char_indices()
        .rev()
        .nth(BEFORE_CHARS - 1)
        .map_or(0, |(i, _)| i);
    let end = text[at + len..]
        .char_indices()
        .nth(AFTER_CHARS)
        .map_or(text.len(), |(i, _)| at + len + i);
    let mut out = String::new();
    if start > 0 {
        out.push('…');
    }
    out.extend(text[start..end].chars().map(|c| match c.is_whitespace() {
        true => ' ',
        false => c,
    }));
    if end < text.len() {
        out.push('…');
    }
    out
}

struct Cached {
    len: u64,
    modified: SystemTime,
    transcript: Transcript,
}

static CACHE: LazyLock<Mutex<HashMap<PathBuf, Cached>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

/// `f` over `file`'s transcript, from the cache while the file is unchanged.
fn with_transcript<T>(file: &Found, f: impl FnOnce(&Transcript) -> T) -> Option<T> {
    let fresh = |c: &Cached| c.len == file.len && c.modified == file.modified;
    {
        let cache = CACHE.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(c) = cache.get(&file.path).filter(|c| fresh(c)) {
            return Some(f(&c.transcript));
        }
    }
    let transcript = read(&file.path, file.agent)?;
    let answer = f(&transcript);
    let mut cache = CACHE.lock().unwrap_or_else(|e| e.into_inner());
    let used: usize = cache.values().map(|c| c.transcript.bytes()).sum();
    if used + transcript.bytes() <= MAX_CACHED_BYTES {
        cache.insert(
            file.path.clone(),
            Cached {
                len: file.len,
                modified: file.modified,
                transcript,
            },
        );
    }
    Some(answer)
}

fn read(path: &Path, agent: CLIAgent) -> Option<Transcript> {
    let mut reader = BufReader::new(File::open(path).ok()?);
    let mut transcript = Transcript::default();
    if agent != CLIAgent::Codex {
        transcript.id = Some(path.file_stem()?.to_str()?.to_owned());
    }
    let mut line = Vec::new();
    while reader.read_until(b'\n', &mut line).ok()? > 0 {
        let keep = match agent {
            CLIAgent::Codex => codex_line(&line, &mut transcript),
            _ => {
                claude_line(&line, &mut transcript);
                true
            }
        };
        if !keep {
            // Kept, empty, so it is not read again until it changes.
            return Some(Transcript::default());
        }
        line.clear();
    }
    Some(transcript)
}

/// A Claude Code record, with everything but what is searched skipped
/// unparsed: tool output runs to megabytes a line.
#[derive(Deserialize)]
struct ClaudeRecord {
    #[serde(rename = "type")]
    kind: Option<String>,
    cwd: Option<String>,
    #[serde(rename = "isMeta", default)]
    is_meta: bool,
    #[serde(rename = "isSidechain", default)]
    is_sidechain: bool,
    #[serde(rename = "isCompactSummary", default)]
    is_compact_summary: bool,
    #[serde(rename = "isVisibleInTranscriptOnly", default)]
    transcript_only: bool,
    message: Option<ClaudeMessage>,
}

#[derive(Deserialize)]
struct ClaudeMessage {
    #[serde(default)]
    content: Texts,
}

/// The text parts of a message's `content`: the string itself, or the
/// `{"type": "text"}` parts of a list. Tool calls, their results and
/// thinking are skipped.
#[derive(Default)]
struct Texts(Vec<String>);

#[derive(Deserialize)]
struct Part {
    #[serde(rename = "type")]
    kind: Option<String>,
    text: Option<String>,
}

impl<'de> Deserialize<'de> for Texts {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        struct V;
        impl<'de> Visitor<'de> for V {
            type Value = Texts;
            fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
                f.write_str("a string or a list of parts")
            }
            fn visit_str<E>(self, s: &str) -> Result<Texts, E> {
                Ok(Texts(vec![s.to_owned()]))
            }
            fn visit_unit<E>(self) -> Result<Texts, E> {
                Ok(Texts::default())
            }
            fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Texts, A::Error> {
                let mut out = Vec::new();
                while let Some(part) = seq.next_element::<Part>()? {
                    if part.kind.as_deref() == Some("text") {
                        out.extend(part.text);
                    }
                }
                Ok(Texts(out))
            }
            fn visit_map<A: serde::de::MapAccess<'de>>(
                self,
                mut map: A,
            ) -> Result<Texts, A::Error> {
                while map.next_entry::<IgnoredAny, IgnoredAny>()?.is_some() {}
                Ok(Texts::default())
            }
        }
        d.deserialize_any(V)
    }
}

fn claude_line(line: &[u8], t: &mut Transcript) {
    let Ok(r) = serde_json::from_slice::<ClaudeRecord>(line) else {
        return;
    };
    if t.cwd.is_none() {
        t.cwd = r.cwd.filter(|c| !c.is_empty()).map(PathBuf::from);
    }
    let from_user = match r.kind.as_deref() {
        Some("user") => true,
        Some("assistant") => false,
        _ => return,
    };
    if r.is_meta || r.is_sidechain || r.is_compact_summary || r.transcript_only {
        return;
    }
    for text in r.message.map(|m| m.content.0).unwrap_or_default() {
        let text = match from_user {
            // The harness's own injections (`<command-name>`, caveats) are
            // not what anyone said.
            true => strip_injected(&text),
            false => text.as_str(),
        };
        if from_user && (text.starts_with('<') || text.starts_with("Caveat:")) {
            continue;
        }
        t.push(from_user, text);
    }
}

#[derive(Deserialize)]
struct CodexRecord {
    #[serde(rename = "type")]
    kind: Option<String>,
    payload: Option<CodexPayload>,
}

#[derive(Deserialize)]
struct CodexPayload {
    #[serde(rename = "type")]
    kind: Option<String>,
    message: Option<String>,
    id: Option<String>,
    cwd: Option<String>,
    source: Option<serde_json::Value>,
    thread_source: Option<String>,
}

/// `false` once the rollout turns out to be one Codex ran for itself.
fn codex_line(line: &[u8], t: &mut Transcript) -> bool {
    let Ok(r) = serde_json::from_slice::<CodexRecord>(line) else {
        return true;
    };
    let Some(p) = r.payload else { return true };
    match (r.kind.as_deref(), p.kind.as_deref()) {
        (Some("session_meta"), _) => {
            let meta = serde_json::json!({ "source": p.source, "thread_source": p.thread_source });
            if codex_not_the_users(&meta) {
                return false;
            }
            t.id = t.id.take().or(p.id);
            t.cwd = t.cwd.take().or(p.cwd.map(PathBuf::from));
        }
        (Some("turn_context"), _) if t.cwd.is_none() => t.cwd = p.cwd.map(PathBuf::from),
        // The event stream's copy of a turn: what was typed, without the
        // context Codex wraps around it in the `response_item`.
        (Some("event_msg"), Some(kind @ ("user_message" | "agent_message"))) => {
            if let Some(text) = p.message {
                t.push(kind == "user_message", &text);
            }
        }
        _ => {}
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    fn roots(home: &Path) -> Roots {
        Roots::under(home)
    }

    fn write(path: &Path, lines: &[serde_json::Value]) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let text: String = lines.iter().map(|l| format!("{l}\n")).collect();
        std::fs::write(path, text).unwrap();
    }

    fn claude_fixture(home: &Path) {
        use serde_json::json;
        write(
            &home.join(".claude/projects/-work-app/abc.jsonl"),
            &[
                json!({"type": "user", "cwd": "/work/app", "message": {"content": "<system-reminder>x</system-reminder>fix the flaky widget test"}}),
                json!({"type": "assistant", "message": {"content": [
                    {"type": "thinking", "thinking": "widget musing"},
                    {"type": "text", "text": "The widget test races the clock."},
                    {"type": "tool_use", "name": "Bash", "input": {"command": "grep widget"}}
                ]}}),
                json!({"type": "user", "message": {"content": [
                    {"type": "tool_result", "tool_use_id": "t", "content": "widget.rs:1"}
                ]}}),
                json!({"type": "user", "isMeta": true, "message": {"content": "meta widget"}}),
                json!({"type": "user", "message": {"content": "<command-name>/widget</command-name>"}}),
                json!({"type": "progress", "data": {"widget": 1}}),
            ],
        );
    }

    #[test]
    fn claude_transcripts_keep_only_what_was_said() {
        let home = tempfile::tempdir().unwrap();
        claude_fixture(home.path());
        let t = read(
            &home.path().join(".claude/projects/-work-app/abc.jsonl"),
            CLIAgent::Claude,
        )
        .unwrap();
        assert_eq!(t.id.as_deref(), Some("abc"));
        assert_eq!(t.cwd.as_deref(), Some(Path::new("/work/app")));
        let said: Vec<_> = t
            .messages
            .iter()
            .map(|m| (m.from_user, m.text.as_str()))
            .collect();
        assert_eq!(
            said,
            vec![
                (true, "fix the flaky widget test"),
                (false, "The widget test races the clock."),
            ]
        );
    }

    #[test]
    fn codex_rollouts_keep_the_event_streams_messages() {
        use serde_json::json;
        let home = tempfile::tempdir().unwrap();
        let path = home
            .path()
            .join(".codex/sessions/2026/01/02/rollout-x.jsonl");
        write(
            &path,
            &[
                json!({"type": "session_meta", "payload": {"id": "c1", "cwd": "/work/api", "source": "cli"}}),
                json!({"type": "response_item", "payload": {"type": "message", "content": [{"text": "<environment_context>"}]}}),
                json!({"type": "event_msg", "payload": {"type": "user_message", "message": "rename the gizmo"}}),
                json!({"type": "event_msg", "payload": {"type": "agent_message", "message": "Renamed the gizmo."}}),
                json!({"type": "event_msg", "payload": {"type": "token_count", "info": {}}}),
            ],
        );
        let t = read(&path, CLIAgent::Codex).unwrap();
        assert_eq!(t.id.as_deref(), Some("c1"));
        assert_eq!(t.cwd.as_deref(), Some(Path::new("/work/api")));
        assert_eq!(t.messages.len(), 2);
        assert!(t.messages[0].from_user && !t.messages[1].from_user);

        let sub = home
            .path()
            .join(".codex/sessions/2026/01/02/rollout-y.jsonl");
        write(
            &sub,
            &[
                json!({"type": "session_meta", "payload": {"id": "c2", "source": {"subagent": "review"}}}),
                json!({"type": "event_msg", "payload": {"type": "user_message", "message": "gizmo"}}),
            ],
        );
        let own = read(&sub, CLIAgent::Codex).unwrap();
        assert!(
            own.id.is_none() && own.messages.is_empty(),
            "Codex's own runs stay out"
        );
    }

    #[test]
    fn a_search_finds_one_row_per_session_with_its_best_hit() {
        let home = tempfile::tempdir().unwrap();
        claude_fixture(home.path());
        let found = search(&roots(home.path()), "WIDGET");
        assert_eq!(found.len(), 1);
        let hit = &found[0];
        assert_eq!((hit.agent, hit.id.as_str()), (CLIAgent::Claude, "abc"));
        assert_eq!(hit.cwd.as_deref(), Some(Path::new("/work/app")));
        assert_eq!(hit.hits, 2, "the tool call and its result do not count");
        assert_eq!(
            hit.snippet, "fix the flaky widget test",
            "the user's words win"
        );
        assert!(search(&roots(home.path()), "races").len() == 1);
        assert!(search(&roots(home.path()), "grep widget").is_empty());
        assert!(search(&roots(home.path()), "  ").is_empty());
    }

    #[test]
    fn a_changed_file_is_read_again() {
        use serde_json::json;
        let home = tempfile::tempdir().unwrap();
        let path = home.path().join(".claude/projects/-p/s.jsonl");
        write(
            &path,
            &[json!({"type": "user", "message": {"content": "alpha"}})],
        );
        assert_eq!(search(&roots(home.path()), "alpha").len(), 1);
        write(
            &path,
            &[
                json!({"type": "user", "message": {"content": "alpha"}}),
                json!({"type": "assistant", "message": {"content": "bravo"}}),
            ],
        );
        assert_eq!(search(&roots(home.path()), "bravo").len(), 1);
    }

    fn message(from_user: bool, text: &str) -> Message {
        Message {
            from_user,
            text: text.into(),
            lower: text.to_lowercase(),
        }
    }

    #[test]
    fn a_snippet_is_one_line_around_the_match() {
        let long = format!("{}\nthe Needle here\n{}", "a".repeat(100), "b".repeat(300));
        let m = message(true, &long);
        let at = m.lower.find("needle").unwrap();
        let s = snippet(&m, at, "needle".len());
        assert!(s.starts_with('…') && s.ends_with('…'), "{s}");
        assert!(!s.contains('\n'));
        assert!(s.contains("the Needle here"), "original case kept: {s}");
        assert_eq!(
            s.chars().count(),
            2 + BEFORE_CHARS + "needle".len() + AFTER_CHARS
        );

        let short = message(true, "a needle");
        assert_eq!(snippet(&short, 2, 6), "a needle");
    }

    #[test]
    fn prompts_and_word_starts_rank_first_then_recency() {
        assert!(score(&message(true, "x needle"), 2) > score(&message(false, "x needle"), 2));
        assert!(score(&message(false, "needle"), 0) > score(&message(false, "aneedle"), 1));
        let hit = |id: &str, score, updated| HistoryHit {
            agent: CLIAgent::Claude,
            id: id.into(),
            cwd: None,
            updated,
            snippet: String::new(),
            hits: 1,
            score,
        };
        let mut hits = vec![
            hit("old-good", 4, 1),
            hit("new-weak", 1, 9),
            hit("new-good", 4, 5),
        ];
        rank(&mut hits);
        let ids: Vec<_> = hits.iter().map(|h| h.id.as_str()).collect();
        assert_eq!(ids, ["new-good", "old-good", "new-weak"]);
    }

    /// One cold and one warm query over this machine's real history. Prints
    /// timings and counts only, never what the sessions say:
    /// `cargo test -p tty7-core history_search::tests::real_history -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn real_history() {
        let roots = Roots::local().unwrap();
        for query in ["worktree", "worktree", "zzqxj-no-such"] {
            let t = Instant::now();
            let found = search(&roots, query);
            let cached: usize = CACHE.lock().unwrap().len();
            println!(
                "query#{} {:?}: {} sessions, {} cached files",
                query.len(),
                t.elapsed(),
                found.len(),
                cached
            );
        }
    }
}

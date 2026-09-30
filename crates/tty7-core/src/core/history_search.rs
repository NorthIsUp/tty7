//! Full-text search over what was said in past agent sessions: the text of
//! the user's prompts and the agent's replies, never tool calls or their
//! output. Claude Code (and Qoder, which copied its layout) and Codex.
//!
//! [`session_mentions`] reads one session the same way, tool output
//! included, for the pull requests it names.
//!
//! Every file is read whole, line by line, newest first, and what it says is
//! kept in memory by path, size and modification time, so a second query
//! reads only the sessions that moved since.

use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::sync::LazyLock;
use std::time::{Duration, Instant};

use regex::Regex;
use serde::Deserialize;
use serde::de::{Deserializer, IgnoredAny, SeqAccess, Visitor};

use crate::core::agent_history::{
    Found, Roots, claude_files, codex_files, codex_not_the_users, strip_injected, unix,
};
use crate::core::cli_agent::CLIAgent;
use crate::core::git::git_output;
use crate::core::github::RepoSlug;
use crate::core::github::remote::github_remotes;
use crate::core::history_cache::{forget_all_but, with_transcript};

/// ponytail: a query stops after this long and shows what it found; the
/// next one starts where the cache is warm. A persistent index if a cold
/// history outgrows it.
const TIME_BUDGET: Duration = Duration::from_secs(3);

/// ponytail: stop at this many matching sessions, newest first, so an older
/// session with a better match can be missed.
const MAX_SESSIONS: usize = 100;

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
    let mut files: Vec<Found> = SEARCHED.iter().flat_map(|&a| files_for(roots, a)).collect();
    forget_all_but(&files);
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

/// ponytail: the newest this many; older mentions drop off.
const MAX_MENTIONS: usize = 200;

/// The issue and pull request numbers of one repository a session names.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Mentions {
    /// `#N`, `owner/repo#N` and `…/pull/N` or `…/issues/N` links, each once,
    /// the latest mention first.
    pub numbers: Vec<u64>,
}

/// What `agent`'s session `id` says about `repo`: its prompts, replies and
/// tool output (`gh pr create` prints the link there). A bare `#N` counts
/// only when the session ran in a checkout of `repo`; elsewhere it names
/// some other repository's issue.
pub fn session_mentions(roots: &Roots, agent: CLIAgent, id: &str, repo: &RepoSlug) -> Mentions {
    session_file(roots, agent, id)
        .and_then(|path| read_as::<Texts>(&path, agent))
        .map(|t| {
            let bare = t.cwd.as_deref().is_some_and(|cwd| checkout_of(cwd, repo));
            mentions_in(t.messages.iter().map(|m| m.text.as_str()), repo, bare)
        })
        .unwrap_or_default()
}

/// Whether any of `cwd`'s GitHub remotes is `repo`.
fn checkout_of(cwd: &Path, repo: &RepoSlug) -> bool {
    let Ok(out) = git_output(cwd, &["remote", "-v"]) else {
        return false;
    };
    github_remotes(&String::from_utf8_lossy(&out.stdout))
        .iter()
        .any(|r| {
            r.slug.owner.eq_ignore_ascii_case(&repo.owner)
                && r.slug.name.eq_ignore_ascii_case(&repo.name)
        })
}

/// The agents whose history is searched.
const SEARCHED: [CLIAgent; 4] = [
    CLIAgent::Claude,
    CLIAgent::QoderCLI,
    CLIAgent::QoderCLICn,
    CLIAgent::Codex,
];

/// `agent`'s transcript files under `roots`; none for an agent not in
/// [`SEARCHED`].
fn files_for(roots: &Roots, agent: CLIAgent) -> Vec<Found> {
    match agent {
        CLIAgent::Claude => claude_files(&roots.claude.join("projects"), agent),
        CLIAgent::QoderCLI => claude_files(&roots.qoder.join("projects"), agent),
        CLIAgent::QoderCLICn => claude_files(&roots.qoder_cn.join("projects"), agent),
        CLIAgent::Codex => codex_files(&roots.codex),
        _ => Vec::new(),
    }
}

fn session_file(roots: &Roots, agent: CLIAgent, id: &str) -> Option<PathBuf> {
    let files = files_for(roots, agent);
    let rollout = format!("-{id}");
    files
        .into_iter()
        .find(|f| {
            f.path
                .file_stem()
                .and_then(|s| s.to_str())
                .is_some_and(|s| match agent {
                    CLIAgent::Codex => s.ends_with(&rollout),
                    _ => s == id,
                })
        })
        .map(|f| f.path)
}

static MENTION: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"(?i)github\.com/([\w.-]+)/([\w.-]+)/(pull|issues)/(\d+)|(?:^|[^\w/.#&-])(?:([\w.-]+)/([\w.-]+))?#(\d+)\b",
    )
    .expect("the mention pattern compiles")
});

/// `bare`: whether a `#N` with no `owner/repo` is taken to be `repo`'s.
fn mentions_in<'a>(texts: impl Iterator<Item = &'a str>, repo: &RepoSlug, bare: bool) -> Mentions {
    let ours = |owner: Option<regex::Match>, name: Option<regex::Match>| match (owner, name) {
        (Some(o), Some(n)) => {
            o.as_str().eq_ignore_ascii_case(&repo.owner)
                && n.as_str().eq_ignore_ascii_case(&repo.name)
        }
        _ => bare,
    };
    let mut seen: Vec<u64> = Vec::new();
    for text in texts {
        for c in MENTION.captures_iter(text) {
            let number = match (c.get(4), c.get(7)) {
                (Some(n), _) if ours(c.get(1), c.get(2)) => n,
                (None, Some(n)) if ours(c.get(5), c.get(6)) => n,
                _ => continue,
            };
            if let Ok(number) = number.as_str().parse() {
                seen.push(number);
            }
        }
    }
    let mut out = Mentions::default();
    for &number in seen.iter().rev() {
        if out.numbers.len() < MAX_MENTIONS && !out.numbers.contains(&number) {
            out.numbers.push(number);
        }
    }
    out
}

/// Best score first, then the most recently written.
fn rank(hits: &mut [HistoryHit]) {
    hits.sort_by_key(|h| (std::cmp::Reverse(h.score), std::cmp::Reverse(h.updated)));
}

/// What one transcript says, as far as a search cares.
#[derive(Default)]
pub(super) struct Transcript {
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
    pub(super) fn push(&mut self, from_user: bool, text: &str) {
        let text = text.trim();
        if !text.is_empty() {
            self.messages.push(Message {
                from_user,
                lower: text.to_lowercase(),
                text: text.to_owned(),
            });
        }
    }

    pub(super) fn bytes(&self) -> usize {
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

pub(super) fn read(path: &Path, agent: CLIAgent) -> Option<Transcript> {
    read_as::<IgnoredAny>(path, agent)
}

/// `read`, with tool output read as `Tools`.
fn read_as<Tools>(path: &Path, agent: CLIAgent) -> Option<Transcript>
where
    Tools: for<'a> Deserialize<'a> + ToolText,
{
    let mut reader = BufReader::new(File::open(path).ok()?);
    let mut transcript = Transcript::default();
    if agent != CLIAgent::Codex {
        transcript.id = Some(path.file_stem()?.to_str()?.to_owned());
    }
    let mut line = Vec::new();
    while reader.read_until(b'\n', &mut line).ok()? > 0 {
        let keep = match agent {
            CLIAgent::Codex => codex_line::<Tools>(&line, &mut transcript),
            _ => {
                claude_line::<Tools>(&line, &mut transcript);
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
/// unparsed: tool output runs to megabytes a line. `Tools` is what a tool
/// result's output is read as: `IgnoredAny` for search, [`Texts`] where it
/// counts (see [`session_mentions`]).
#[derive(Deserialize)]
#[serde(bound(deserialize = "Tools: Deserialize<'de> + ToolText"))]
struct ClaudeRecord<Tools> {
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
    message: Option<ClaudeMessage<Tools>>,
}

#[derive(Deserialize)]
#[serde(bound(deserialize = "Tools: Deserialize<'de> + ToolText"))]
struct ClaudeMessage<Tools> {
    #[serde(default = "Texts::empty")]
    content: Texts<Tools>,
}

/// The text parts of a message's `content`: the string itself, or the
/// `{"type": "text"}` parts of a list, and in `tools` the output of its
/// `tool_result` parts read as `Tools`. Tool calls and thinking are skipped.
struct Texts<Tools = IgnoredAny> {
    said: Vec<String>,
    tools: Vec<String>,
    kind: std::marker::PhantomData<Tools>,
}

impl<Tools> Texts<Tools> {
    fn empty() -> Self {
        Texts {
            said: Vec::new(),
            tools: Vec::new(),
            kind: std::marker::PhantomData,
        }
    }
}

/// What a tool's output reads as: nothing, or its text.
trait ToolText {
    fn into_texts(self) -> Vec<String>;
}

impl ToolText for IgnoredAny {
    fn into_texts(self) -> Vec<String> {
        Vec::new()
    }
}

impl ToolText for Texts {
    fn into_texts(self) -> Vec<String> {
        self.said
    }
}

#[derive(Deserialize)]
struct Part<Tools> {
    #[serde(rename = "type")]
    kind: Option<String>,
    text: Option<String>,
    content: Option<Tools>,
}

impl<'de, Tools: Deserialize<'de> + ToolText> Deserialize<'de> for Texts<Tools> {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        struct V<Tools>(std::marker::PhantomData<Tools>);
        impl<'de, Tools: Deserialize<'de> + ToolText> Visitor<'de> for V<Tools> {
            type Value = Texts<Tools>;
            fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
                f.write_str("a string or a list of parts")
            }
            fn visit_str<E>(self, s: &str) -> Result<Texts<Tools>, E> {
                let mut out = Texts::empty();
                out.said.push(s.to_owned());
                Ok(out)
            }
            fn visit_unit<E>(self) -> Result<Texts<Tools>, E> {
                Ok(Texts::empty())
            }
            fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Texts<Tools>, A::Error> {
                let mut out = Texts::empty();
                while let Some(part) = seq.next_element::<Part<Tools>>()? {
                    match part.kind.as_deref() {
                        Some("text") => out.said.extend(part.text),
                        Some("tool_result") => {
                            out.tools
                                .extend(part.content.into_iter().flat_map(ToolText::into_texts));
                        }
                        _ => {}
                    }
                }
                Ok(out)
            }
            fn visit_map<A: serde::de::MapAccess<'de>>(
                self,
                mut map: A,
            ) -> Result<Texts<Tools>, A::Error> {
                while map.next_entry::<IgnoredAny, IgnoredAny>()?.is_some() {}
                Ok(Texts::empty())
            }
        }
        d.deserialize_any(V(std::marker::PhantomData))
    }
}

fn claude_line<'a, Tools: Deserialize<'a> + ToolText>(line: &'a [u8], t: &mut Transcript) {
    let Ok(r) = serde_json::from_slice::<ClaudeRecord<Tools>>(line) else {
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
    let Some(content) = r.message.map(|m| m.content) else {
        return;
    };
    for text in content.tools {
        t.push(false, &text);
    }
    for text in content.said {
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
struct CodexRecord<Tools> {
    #[serde(rename = "type")]
    kind: Option<String>,
    payload: Option<CodexPayload<Tools>>,
}

#[derive(Deserialize)]
struct CodexPayload<Tools> {
    #[serde(rename = "type")]
    kind: Option<String>,
    message: Option<String>,
    id: Option<String>,
    cwd: Option<String>,
    source: Option<serde_json::Value>,
    thread_source: Option<String>,
    /// A `function_call_output`'s output, read as `Tools`.
    output: Option<Tools>,
}

/// `false` once the rollout turns out to be one Codex ran for itself.
fn codex_line<'a, Tools: Deserialize<'a> + ToolText>(line: &'a [u8], t: &mut Transcript) -> bool {
    let Ok(r) = serde_json::from_slice::<CodexRecord<Tools>>(line) else {
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
        (Some("response_item"), Some("function_call_output")) => {
            for text in p.output.into_iter().flat_map(ToolText::into_texts) {
                t.push(false, &text);
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

    fn slug(owner: &str, name: &str) -> RepoSlug {
        RepoSlug {
            owner: owner.into(),
            name: name.into(),
        }
    }

    #[test]
    fn mentions_are_this_repos_refs_and_links_latest_first() {
        let repo = slug("acme", "widgets");
        let texts = [
            "see #12 and acme/other#99, then Acme/Widgets#13",
            "https://github.com/acme/widgets/pull/14 and https://github.com/foo/bar/pull/15",
            "https://github.com/acme/widgets/issues/16, https://github.com/acme/widgets/issues/12",
            "#12 again (#17) but not a#18, x/y/z#19, &#20; or #21abc",
        ];
        let m = mentions_in(texts.into_iter(), &repo, true);
        assert_eq!(m.numbers, vec![17, 12, 16, 14, 13]);
    }

    #[test]
    fn outside_the_repo_only_qualified_mentions_count() {
        let repo = slug("acme", "widgets");
        let texts = [
            "fixed #12 and https://github.com/other/thing/pull/30",
            "acme/widgets#13, https://github.com/Acme/Widgets/issues/14",
        ];
        let m = mentions_in(texts.into_iter(), &repo, false);
        assert_eq!(m.numbers, vec![14, 13]);
    }

    #[test]
    fn a_sessions_mentions_include_tool_output_and_other_sessions_stay_out() {
        use serde_json::json;
        let home = tempfile::tempdir().unwrap();
        let checkout = home.path().join("widgets");
        std::fs::create_dir(&checkout).unwrap();
        for args in [
            &["init", "--quiet"][..],
            &["remote", "add", "origin", "git@github.com:acme/widgets.git"],
        ] {
            assert!(git_output(&checkout, args).unwrap().success());
        }
        write(
            &home.path().join(".claude/projects/-work-app/s1.jsonl"),
            &[
                json!({"type": "user", "cwd": checkout, "message": {"content": "look at #3"}}),
                json!({"type": "assistant", "message": {"content": [
                    {"type": "tool_use", "name": "Bash", "input": {"command": "gh pr create"}}
                ]}}),
                json!({"type": "user", "message": {"content": [
                    {"type": "tool_result", "tool_use_id": "t", "content": "https://github.com/acme/widgets/pull/4\n"},
                    {"type": "tool_result", "tool_use_id": "u", "content": [{"type": "text", "text": "merged #5"}]}
                ]}}),
                json!({"type": "assistant", "message": {"content": [{"type": "text", "text": "Opened #4."}]}}),
            ],
        );
        write(
            &home.path().join(".claude/projects/-work-app/s2.jsonl"),
            &[json!({"type": "user", "message": {"content": "#77"}})],
        );
        write(
            &home.path().join(".claude/projects/-elsewhere/s3.jsonl"),
            &[
                json!({"type": "user", "cwd": home.path(), "message": {"content": "#78 and acme/widgets#79"}}),
            ],
        );
        let codex = home
            .path()
            .join(".codex/sessions/2026/01/02/rollout-2026-01-02T00-00-00-c1.jsonl");
        write(
            &codex,
            &[
                json!({"type": "session_meta", "payload": {"id": "c1", "source": "cli"}}),
                json!({"type": "response_item", "payload": {"type": "function_call_output", "output": "acme/widgets#8"}}),
            ],
        );
        let roots = roots(home.path());
        let repo = slug("acme", "widgets");
        let m = session_mentions(&roots, CLIAgent::Claude, "s1", &repo);
        assert_eq!(m.numbers, vec![4, 5, 3]);
        assert_eq!(
            session_mentions(&roots, CLIAgent::Codex, "c1", &repo).numbers,
            vec![8]
        );
        assert_eq!(
            session_mentions(&roots, CLIAgent::Claude, "nope", &repo),
            Mentions::default()
        );
        assert_eq!(
            session_mentions(&roots, CLIAgent::Claude, "s3", &repo).numbers,
            vec![79],
            "a bare #N outside a checkout of the repo is some other repo's"
        );
        // Search still leaves tool output out.
        assert!(search(&roots, "merged").is_empty());
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
            let cached: usize = super::super::history_cache::cached_files();
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

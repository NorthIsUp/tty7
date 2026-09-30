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

use serde::Deserialize;

use crate::core::cli_agent::{AgentSessionState, CLIAgent};

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Entry {
    pid: i32,
    session_id: String,
    kind: Option<String>,
    job_id: Option<String>,
}

/// Where Claude keeps its per-process session files on this machine.
pub fn sessions_dir() -> Option<PathBuf> {
    crate::core::agent_history::Roots::local().map(|roots| roots.claude.join("sessions"))
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

/// How to reopen an agent session, decided on the host that has its files.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ResumePlan {
    /// It is running in the background as this job: `claude --resume` would
    /// be refused, so attach instead.
    Attach(String),
    Resume,
    /// Claude saved no transcript for it (it never took a turn), so a resume
    /// would stop at "No conversation found": start it fresh under its id.
    Fresh,
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
        if project.path().join(&file).is_file() {
            return ResumePlan::Resume;
        }
    }
    ResumePlan::Fresh
}

/// Fill in the session id `argv` names for a pane no hook has spoken for
/// yet, which is what lets a reboot resume Claude without its hooks. `true`
/// when `session` changed. `miss` remembers the argv that named nothing, so
/// an unmappable `claude attach` doesn't reread the sessions dir every poll.
pub fn adopt_argv_session(
    agent: Option<CLIAgent>,
    argv: Option<&[String]>,
    session: &mut Option<AgentSessionState>,
    miss: &mut Option<Vec<String>>,
) -> bool {
    let (Some(agent), Some(argv)) = (agent, argv) else {
        return false;
    };
    if session.as_ref().is_some_and(|s| s.session_id.is_some()) || miss.as_deref() == Some(argv) {
        return false;
    }
    // `claude attach <job>` names only its job; the session file maps it back.
    let named = agent
        .session_id_in_argv(argv)
        .or_else(|| session_for_job(&sessions_dir()?, attached_job(argv)?));
    let Some(id) = named else {
        *miss = Some(argv.to_vec());
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

    #[test]
    fn an_argv_session_is_adopted_once_and_a_miss_is_remembered() {
        let argv = |a: &[&str]| a.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        let (mut session, mut miss) = (None, None);
        let named = argv(&["claude", "--session-id", ID]);
        let claude = Some(CLIAgent::Claude);
        assert!(adopt_argv_session(
            claude,
            Some(&named),
            &mut session,
            &mut miss
        ));
        let got = session.clone().unwrap();
        assert_eq!(got.session_id.as_deref(), Some(ID));
        assert_eq!(got.launch_argv.as_deref(), Some(&named[..]));
        assert!(!adopt_argv_session(
            claude,
            Some(&named),
            &mut session,
            &mut miss
        ));

        let (mut session, mut miss) = (None, None);
        let orphan = argv(&["claude", "attach", "ffffffff"]);
        assert!(!adopt_argv_session(
            claude,
            Some(&orphan),
            &mut session,
            &mut miss
        ));
        assert_eq!(miss.as_deref(), Some(&orphan[..]));
        assert!(session.is_none());
    }

    #[test]
    fn an_attach_argv_names_its_job() {
        let argv = |a: &[&str]| a.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert_eq!(
            attached_job(&argv(&["/usr/local/bin/claude", "attach", "6011098d"])),
            Some("6011098d")
        );
        assert_eq!(attached_job(&argv(&["claude", "--resume", ID])), None);
        assert_eq!(attached_job(&argv(&["tmux", "attach", "main"])), None);
    }
}

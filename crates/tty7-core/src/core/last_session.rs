//! The session a pane's agent left behind (`PaneRecord::last_session`), for
//! any agent: set from what the agent reports, cleared when the shell is the
//! user's again, and read back as what the pane resumes.

use crate::core::cli_agent::CLIAgent;
use crate::core::machine::{AgentFacts, PaneRecord};

/// After `record.agent` takes an observation: a session the agent reported
/// becomes the pane's `last_session`. Nothing else replaces it, so an agent
/// with no session (`claude --version`) or one gone (a crash) keeps it;
/// `closed` (`claude_background::closed_on_leaving`) lets it go.
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

/// `agent` with the `last` session filled in where a fresh agent of the same
/// kind has not reported one yet, or `last` when no agent is there.
pub fn with_last_session(
    agent: Option<AgentFacts>,
    last: Option<&AgentFacts>,
) -> Option<AgentFacts> {
    let (facts, session_id, launch_argv) = merged(agent.as_ref(), last)?;
    Some(AgentFacts {
        agent: facts.agent,
        session_id: session_id.map(str::to_string),
        launch_argv: launch_argv.map(<[String]>::to_vec),
        status: facts.status.clone(),
    })
}

/// [`with_last_session`], borrowed: the facts it keeps, and the session and
/// argv it fills in.
fn merged<'a>(
    agent: Option<&'a AgentFacts>,
    last: Option<&'a AgentFacts>,
) -> Option<(&'a AgentFacts, Option<&'a str>, Option<&'a [String]>)> {
    let own = |f: &'a AgentFacts| (f, f.session_id.as_deref(), f.launch_argv.as_deref());
    match (agent, last) {
        (None, last) => last.map(own),
        (Some(a), Some(l)) if a.session_id.is_none() && a.agent == l.agent => Some((
            a,
            l.session_id.as_deref(),
            a.launch_argv.as_deref().or(l.launch_argv.as_deref()),
        )),
        (a, _) => a.map(own),
    }
}

/// What pane `r`'s agent resumes: [`with_last_session`], its recorded session
/// swapped for the one its argv names when another pane of the same agent
/// claims the recorded one, by argv or record: a Codex report misrouted
/// before #127 left one pane holding another's session, and both would
/// resume it. A record no other pane claims stays, since `/new` inside a
/// resumed session makes it newer than the argv.
pub fn pane_agent(r: &PaneRecord, panes: &[PaneRecord]) -> Option<AgentFacts> {
    let mut facts = with_last_session(r.agent.clone(), r.last_session.as_ref())?;
    let named = |agent: CLIAgent, argv: Option<&[String]>| {
        argv.and_then(|argv| agent.session_id_in_argv(argv))
    };
    let (Some(recorded), Some(own)) = (
        facts.session_id.as_deref(),
        named(facts.agent, facts.launch_argv.as_deref()),
    ) else {
        return Some(facts);
    };
    let claimed = recorded != own
        && panes
            .iter()
            .filter(|p| p.id != r.id)
            .filter_map(|p| merged(p.agent.as_ref(), p.last_session.as_ref()))
            .any(|(o, session, argv)| {
                o.agent == facts.agent
                    && (session == Some(recorded)
                        || named(o.agent, argv).as_deref() == Some(recorded))
            });
    if claimed {
        log::info!(
            "pane {}'s session {recorded} belongs to another pane; it resumes {own}",
            r.id
        );
        facts.session_id = Some(own);
    }
    Some(facts)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn argv(a: &[&str]) -> Vec<String> {
        a.iter().map(|s| s.to_string()).collect()
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
    fn a_session_another_pane_claims_gives_way_to_the_one_the_argv_names() {
        const X: &str = "01a17a3f-c6dd-7353-8a3f-3c60c83a2234";
        const Y: &str = "01a15fed-9183-7e12-a490-168f72e2f39b";
        const Z: &str = "01a16b02-21b6-74a3-a463-880dd2c4c1b1";
        let codex = |session: Option<&str>, resumes: &str| AgentFacts {
            agent: CLIAgent::Codex,
            session_id: session.map(str::to_string),
            launch_argv: Some(argv(&["node", "/bin/codex", "resume", resumes])),
            status: None,
        };
        let pane = |id, facts: AgentFacts| {
            let mut p = PaneRecord::new(id);
            p.agent = Some(facts);
            p
        };
        let session = |i: usize, panes: &[PaneRecord]| {
            pane_agent(&panes[i], panes).and_then(|f| f.session_id)
        };
        // 865 holds 913's session, which 913's argv names.
        let panes = [pane(865, codex(Some(X), Y)), pane(913, codex(None, X))];
        assert_eq!(session(0, &panes).as_deref(), Some(Y));
        assert_eq!(session(1, &panes), None, "913 is untouched");

        let by_record = [pane(865, codex(Some(X), Y)), pane(913, codex(Some(X), Z))];
        assert_eq!(
            session(0, &by_record).as_deref(),
            Some(Y),
            "another pane's record claims it too"
        );
        let alone = [pane(865, codex(Some(X), Y))];
        assert_eq!(
            session(0, &alone).as_deref(),
            Some(X),
            "a `/new` inside the resumed session keeps the newer record"
        );
        let mut claude = codex(Some(X), Z);
        claude.agent = CLIAgent::Claude;
        let other_agent = [pane(865, codex(Some(X), Y)), pane(913, claude)];
        assert_eq!(
            session(0, &other_agent).as_deref(),
            Some(X),
            "only the same agent's panes claim"
        );
    }
}

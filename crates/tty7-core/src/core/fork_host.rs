//! The fork's host calls, kept off upstream's `Host` trait but for one hook,
//! [`Host::fork`]: a host that answers them hands back a [`ForkHost`]. Only
//! the local host does; the history they read is this machine's.
//!
//! Callers go through [`ForkCalls`], which answers for any host: its own
//! answer, or nothing found (and a plain resume) where it has none.

use std::io;

use crate::core::agent_history::Roots;
use crate::core::claude_background::{self, ResumePlan};
use crate::core::cli_agent::CLIAgent;
use crate::core::github::RepoSlug;
use crate::core::history_search::{self, HistoryHit, Mentions};
use crate::host::local::LocalHost;
use crate::host::{Host, guard_off_ui};

pub trait ForkHost {
    /// Past sessions whose conversation contains `query`
    /// ([`history_search::search`]).
    fn search_agent_history(&self, query: &str) -> io::Result<Vec<HistoryHit>>;

    /// The issues and pull requests of `repo` that `agent`'s session `id`
    /// mentions ([`history_search::session_mentions`]).
    fn agent_session_mentions(
        &self,
        agent: CLIAgent,
        id: &str,
        repo: &RepoSlug,
    ) -> io::Result<Mentions>;

    /// How to reopen `agent`'s session `session_id`
    /// ([`claude_background::resume_plan`]).
    fn resume_plan(&self, agent: CLIAgent, session_id: &str) -> io::Result<ResumePlan>;
}

/// [`ForkHost`]'s calls on any host.
pub trait ForkCalls {
    fn search_agent_history(&self, query: &str) -> io::Result<Vec<HistoryHit>>;
    fn agent_session_mentions(
        &self,
        agent: CLIAgent,
        id: &str,
        repo: &RepoSlug,
    ) -> io::Result<Mentions>;
    fn resume_plan(&self, agent: CLIAgent, session_id: &str) -> io::Result<ResumePlan>;
}

impl ForkCalls for dyn Host + '_ {
    fn search_agent_history(&self, query: &str) -> io::Result<Vec<HistoryHit>> {
        self.fork()
            .map_or(Ok(Vec::new()), |f| f.search_agent_history(query))
    }

    fn agent_session_mentions(
        &self,
        agent: CLIAgent,
        id: &str,
        repo: &RepoSlug,
    ) -> io::Result<Mentions> {
        self.fork().map_or(Ok(Mentions::default()), |f| {
            f.agent_session_mentions(agent, id, repo)
        })
    }

    fn resume_plan(&self, agent: CLIAgent, session_id: &str) -> io::Result<ResumePlan> {
        self.fork()
            .map_or(Ok(ResumePlan::Resume), |f| f.resume_plan(agent, session_id))
    }
}

/// This machine's agent history roots, or an error saying there are none.
fn roots() -> io::Result<Roots> {
    Roots::local().ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "no home directory"))
}

impl ForkHost for LocalHost {
    fn search_agent_history(&self, query: &str) -> io::Result<Vec<HistoryHit>> {
        guard_off_ui();
        Ok(history_search::search(&roots()?, query))
    }

    fn agent_session_mentions(
        &self,
        agent: CLIAgent,
        id: &str,
        repo: &RepoSlug,
    ) -> io::Result<Mentions> {
        guard_off_ui();
        Ok(history_search::session_mentions(&roots()?, agent, id, repo))
    }

    fn resume_plan(&self, agent: CLIAgent, session_id: &str) -> io::Result<ResumePlan> {
        guard_off_ui();
        Ok(claude_background::resume_plan(
            &roots()?.claude,
            agent,
            session_id,
        ))
    }
}

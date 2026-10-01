//! The fork's host calls, kept off upstream's `Host` trait but for one hook,
//! [`Host::fork`](crate::host::Host::fork). Only the local host answers them,
//! the history they read being this machine's; every other host hands back
//! [`NoFork`].

use std::io;

use crate::core::agent_history::Roots;
use crate::core::cli_agent::CLIAgent;
use crate::core::github::RepoSlug;
use crate::core::history_search::{self, HistoryHit, Mentions};
use crate::host::guard_off_ui;
use crate::host::local::LocalHost;

/// Each call's default is the answer of a host that has none of it: nothing
/// found.
pub trait ForkHost {
    /// Past sessions whose conversation contains `query`
    /// ([`history_search::search`]).
    fn search_agent_history(&self, _query: &str) -> io::Result<Vec<HistoryHit>> {
        Ok(Vec::new())
    }

    /// The issues and pull requests of `repo` that `agent`'s session `id`
    /// mentions ([`history_search::session_mentions`]).
    fn agent_session_mentions(
        &self,
        _agent: CLIAgent,
        _id: &str,
        _repo: &RepoSlug,
    ) -> io::Result<Mentions> {
        Ok(Mentions::default())
    }
}

/// Every host but the local one.
pub struct NoFork;

impl ForkHost for NoFork {}

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
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_host_without_the_fork_calls_finds_nothing() {
        let host: &dyn ForkHost = &NoFork;
        assert!(host.search_agent_history("x").unwrap().is_empty());
        let repo = RepoSlug {
            owner: "o".into(),
            name: "r".into(),
        };
        let mentions = host.agent_session_mentions(CLIAgent::Claude, "id", &repo);
        assert_eq!(mentions.unwrap(), Mentions::default());
    }
}

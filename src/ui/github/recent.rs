//! The branches the pane's repository was on before this one, newest first,
//! so the GitHub panel can keep their pull requests one click away.
//!
//! Read from HEAD's reflog: the order the branches were checked out in, which
//! is what "recent" means to whoever just switched away from one.

use std::collections::HashMap;
use std::time::Instant;

use gpui::Context;

use crate::ui::app::Tty7App;
use crate::ui::github::{BRANCH_TTL, BranchHead};
use crate::ui::host_ops::SharedHost;
use crate::ui::scm::state::RepoKey;

/// The current branch and the ones before it.
pub(crate) const SHOWN: usize = 5;

/// How far back the reflog is read for them.
const REFLOG_DEPTH: &str = "300";

#[derive(Default)]
pub(crate) struct RecentBranches {
    repos: HashMap<RepoKey, Lookup>,
}

#[derive(Default)]
struct Lookup {
    branches: Vec<BranchHead>,
    read_at: Option<Instant>,
    loading: bool,
}

/// Branch names in the order `checkout: moving from A to B` lines name them,
/// newest first, each once.
fn from_reflog(subjects: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for line in subjects.lines() {
        let Some(rest) = line.strip_prefix("checkout: moving from ") else {
            continue;
        };
        let Some((from, to)) = rest.split_once(" to ") else {
            continue;
        };
        for name in [to, from] {
            if !out.iter().any(|b| b == name) {
                out.push(name.to_string());
            }
        }
    }
    out
}

/// Up to `SHOWN - 1` branches besides `current`, as `BranchHead`s. Names the
/// reflog remembers that are no longer branches (deleted, or a detached sha)
/// are skipped.
fn read(h: &dyn tty7_core::host::Host, root: &std::path::Path, current: &str) -> Vec<BranchHead> {
    use tty7_core::core::git::git;
    let Some(subjects) = git(
        h,
        root,
        &["reflog", "show", "--format=%gs", "-n", REFLOG_DEPTH, "HEAD"],
    ) else {
        return Vec::new();
    };
    let Some(refs) = git(
        h,
        root,
        &[
            "for-each-ref",
            "--format=%(refname:short) %(upstream:short)",
            "refs/heads",
        ],
    ) else {
        return Vec::new();
    };
    let upstreams: HashMap<&str, Option<String>> = refs
        .lines()
        .filter_map(|l| {
            let (name, up) = l.split_once(' ').unwrap_or((l, ""));
            (!name.is_empty()).then(|| (name, (!up.is_empty()).then(|| up.to_string())))
        })
        .collect();
    from_reflog(&subjects)
        .into_iter()
        .filter(|b| b != current)
        .filter_map(|branch| {
            let upstream = upstreams.get(branch.as_str())?.clone();
            Some(BranchHead { branch, upstream })
        })
        .take(SHOWN - 1)
        .collect()
}

impl Tty7App {
    /// The branches `repo` was on before `current`, re-read with the branch.
    pub(crate) fn github_recent_branches(
        &mut self,
        host: SharedHost,
        repo: &RepoKey,
        current: &str,
        cx: &mut Context<Self>,
    ) -> Vec<BranchHead> {
        let entry = self.github.recent.repos.entry(repo.clone()).or_default();
        let shown = entry.branches.clone();
        let due = !entry.loading && entry.read_at.is_none_or(|t| t.elapsed() > BRANCH_TTL);
        // A switch makes the list stale at once: the branch just left belongs
        // at its head, and the one now current must leave it.
        let switched = shown.iter().any(|b| b.branch == current);
        if due || (switched && !entry.loading) {
            entry.loading = true;
            let root = repo.root.clone();
            let repo = repo.clone();
            let current = current.to_string();
            crate::ui::host_ops::HostOps::run(
                host,
                cx,
                move |h| read(h, &root, &current),
                move |this, branches, cx| {
                    let entry = this.github.recent.repos.entry(repo).or_default();
                    entry.loading = false;
                    entry.read_at = Some(Instant::now());
                    if entry.branches != branches {
                        entry.branches = branches;
                        cx.notify();
                    }
                },
            );
        }
        shown.into_iter().filter(|b| b.branch != current).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_reflog_names_branches_newest_first_and_once() {
        let subjects = "\
checkout: moving from feat/b to main
commit: wip
checkout: moving from feat/a to feat/b
reset: moving to HEAD~1
checkout: moving from main to feat/a
checkout: moving from feat/b to 1a2b3c4d
";
        assert_eq!(
            from_reflog(subjects),
            ["main", "feat/b", "feat/a", "1a2b3c4d"]
        );
    }
}

//! Stacked pull requests: one whose base branch is another listed pull
//! request's head branch sits on top of it. A list draws each stack as one
//! contiguous run with every pull request above the one it is based on: a
//! pull request is stacked after its base, so this keeps the list's
//! newest-first order inside the run, and the trunk-most ends at the bottom
//! beside the branch the stack is based on.

use std::collections::HashMap;

use super::model::{Item, PullRefs};

/// How a row is drawn in its stack. Each lane is a column of the connector;
/// a pull request shares its base's lane when it is the first one based on
/// it, and a sibling gets a lane of its own, so a fork never reads as one
/// pull request based on the other. A row in no stack is the default.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct StackShape<'a> {
    /// How many lanes the row's stack uses.
    pub lanes: usize,
    /// The lane the row's glyph sits in.
    pub lane: usize,
    /// A line leaves the glyph upward, to the row just above.
    pub up: bool,
    /// A line leaves the glyph downward, to its base or to the trunk.
    pub down: bool,
    /// Lanes whose line runs through this row to a base further down.
    pub through: Vec<usize>,
    /// Lanes whose line ends in this row's glyph, coming in from the side.
    pub joins: Vec<usize>,
    /// Under the stack's bottom row: the branch the stack is based on. `None`
    /// when that branch is another listed pull request's, as in a cycle.
    pub trunk: Option<&'a str>,
}

/// The branch of a head label `owner:branch` when it lives in `owner`'s
/// repository. GitHub owners are case-insensitive, and a remote URL can
/// spell one either way.
fn own_branch<'a>(head_label: &'a str, owner: &str) -> Option<&'a str> {
    let (who, branch) = head_label.split_once(':')?;
    who.eq_ignore_ascii_case(owner).then_some(branch)
}

/// `rows` with each stack gathered into one run where its first member
/// sorted; rows in no stack keep their place. `owner` is the repository's:
/// a base branch only names another pull request's head when that head lives
/// in the same repository, never in a fork.
pub fn stack_order<'a>(rows: &[&'a Item], owner: &str) -> Vec<(&'a Item, StackShape<'a>)> {
    let refs = |i: usize| -> Option<&'a PullRefs> { rows[i].pull.as_ref() };
    let mut by_head: HashMap<&str, usize> = HashMap::new();
    for i in 0..rows.len() {
        if let Some(branch) = refs(i).and_then(|r| own_branch(&r.head_label, owner)) {
            by_head.entry(branch).or_insert(i);
        }
    }
    let parent: Vec<Option<usize>> = (0..rows.len())
        .map(|i| {
            refs(i)
                .and_then(|r| by_head.get(r.base_ref.as_str()).copied())
                .filter(|&p| p != i)
        })
        .collect();
    let mut children: Vec<Vec<usize>> = vec![Vec::new(); rows.len()];
    for (i, p) in parent.iter().enumerate() {
        if let Some(p) = *p {
            children[p].push(i);
        }
    }

    // Roots first, then whatever a cycle left unvisited, each from its
    // earliest member. `from` is the row a walk reached each row by, which in
    // a cycle is not always its `parent`.
    let mut seen = vec![false; rows.len()];
    let mut from: Vec<Option<usize>> = vec![None; rows.len()];
    let mut lane = vec![0; rows.len()];
    let mut groups: Vec<(Vec<usize>, usize)> = Vec::new();
    let starts = (0..rows.len())
        .filter(|&i| parent[i].is_none())
        .chain(0..rows.len());
    for start in starts {
        if seen[start] {
            continue;
        }
        let mut group = Vec::new();
        let mut lanes = 1;
        let mut heir_taken = vec![false; rows.len()];
        let mut todo = vec![(start, None)];
        while let Some((i, via)) = todo.pop() {
            if std::mem::replace(&mut seen[i], true) {
                continue;
            }
            from[i] = via;
            // Lanes are handed out in walk order, so a lane's line never
            // crosses the side branch of a row it passes.
            lane[i] = match via {
                Some(p) if !std::mem::replace(&mut heir_taken[p], true) => lane[p],
                Some(_) => {
                    lanes += 1;
                    lanes - 1
                }
                None => 0,
            };
            group.push(i);
            todo.extend(children[i].iter().rev().map(|&c| (c, Some(i))));
        }
        // Pre-order reversed: every pull request lands above its base, the
        // first based on it right above it.
        group.reverse();
        groups.push((group, lanes));
    }
    groups.sort_by_key(|(g, _)| g.iter().min().copied());

    let mut out = Vec::with_capacity(rows.len());
    for (group, lanes) in groups {
        let start = out.len();
        let solo = group.len() == 1;
        let mut row_of = HashMap::new();
        for (k, &i) in group.iter().enumerate() {
            row_of.insert(i, start + k);
            let mut shape = StackShape::default();
            if !solo {
                shape.lanes = lanes;
                shape.lane = lane[i];
                shape.down = from[i].is_some();
            }
            if !solo && from[i].is_none() && parent[i].is_none() {
                shape.trunk = refs(i).map(|r| r.base_ref.as_str());
                shape.down = shape.trunk.is_some();
            }
            out.push((rows[i], shape));
        }
        for &i in &group {
            let Some(p) = from[i] else { continue };
            let (r, rp) = (row_of[&i], row_of[&p]);
            if lane[i] == lane[p] {
                out[rp].1.up = true;
            } else {
                for (_, shape) in &mut out[r + 1..rp] {
                    shape.through.push(lane[i]);
                }
                out[rp].1.joins.push(lane[i]);
            }
        }
    }
    out
}

/// The Graphite page of pull request `number` of `owner/repo`.
pub fn graphite_url(owner: &str, repo: &str, number: u64) -> String {
    format!("https://app.graphite.dev/github/pr/{owner}/{repo}/{number}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::github::model::ItemState;

    fn pr(number: u64, head: &str, base: &str) -> Item {
        Item {
            number,
            title: String::new(),
            state: ItemState::Open,
            is_pr: true,
            author: String::new(),
            labels: Vec::new(),
            comments: 0,
            created_at: 0,
            updated_at: 0,
            html_url: String::new(),
            pull: Some(PullRefs {
                head_ref: head.into(),
                head_label: format!("acme:{head}"),
                base_ref: base.into(),
            }),
        }
    }

    fn order_as(items: &[Item], owner: &str) -> Vec<u64> {
        let rows: Vec<&Item> = items.iter().collect();
        stack_order(&rows, owner)
            .into_iter()
            .map(|(i, _)| i.number)
            .collect()
    }

    fn order(items: &[Item]) -> Vec<u64> {
        order_as(items, "acme")
    }

    /// The list as the panel draws it, one line per row and one between
    /// rows for the connectors that cross the gap: `●` a glyph, `│` a line,
    /// `╯` a side lane's line ending in the glyph left of it, `•` a trunk.
    fn drawn(items: &[Item]) -> Vec<String> {
        let rows: Vec<&Item> = items.iter().collect();
        let mut out = Vec::new();
        let mut first = true;
        for (item, s) in stack_order(&rows, "acme") {
            let lanes = s.lanes.max(1);
            if !std::mem::replace(&mut first, false) {
                let gap: String = (0..lanes)
                    .map(|c| {
                        let line =
                            (c == s.lane && s.up) || s.through.contains(&c) || s.joins.contains(&c);
                        if line { '│' } else { ' ' }
                    })
                    .collect();
                out.push(gap.trim_end().to_string());
            }
            let row: String = (0..lanes)
                .map(|c| match c {
                    _ if c == s.lane => '●',
                    _ if s.joins.contains(&c) => '╯',
                    _ if s.through.contains(&c) => '│',
                    _ => ' ',
                })
                .collect();
            out.push(format!("{row} #{}", item.number));
            if let Some(trunk) = s.trunk {
                out.push(format!("• {trunk}"));
            }
        }
        out
    }

    #[test]
    fn a_stack_is_gathered_top_first_where_its_first_member_sorted() {
        let items = [
            pr(1, "solo", "main"),
            pr(3, "c", "b"),
            pr(9, "other", "main"),
            pr(2, "b", "a"),
            pr(4, "a", "main"),
        ];
        assert_eq!(
            drawn(&items),
            [
                "● #1", "", "● #3", "│", "● #2", "│", "● #4", "• main", "", "● #9",
            ]
        );
    }

    #[test]
    fn issues_and_unstacked_rows_keep_their_order() {
        let mut issue = pr(5, "x", "y");
        issue.is_pr = false;
        issue.pull = None;
        let items = [pr(7, "y", "main"), issue, pr(8, "z", "main")];
        assert_eq!(order(&items), [7, 5, 8]);
        assert!(drawn(&items).iter().all(|l| !l.contains('│')));
    }

    #[test]
    fn a_fork_s_branch_is_not_a_base_in_this_repository() {
        let mut fork = pr(1, "feat", "main");
        fork.pull.as_mut().unwrap().head_label = "bob:feat".into();
        let items = [fork, pr(2, "more", "feat")];
        assert_eq!(drawn(&items), ["● #1", "", "● #2"]);
    }

    #[test]
    fn the_owner_matches_however_the_remote_spells_it() {
        let items = [pr(1, "a", "main"), pr(2, "b", "a")];
        assert_eq!(order_as(&items, "ACME"), [2, 1]);
        assert_eq!(order_as(&items, "Acme"), [2, 1]);
        assert_eq!(order_as(&items, "acm"), [1, 2], "a prefix is not the owner");
    }

    #[test]
    fn siblings_on_one_base_are_drawn_as_a_fork_not_a_line() {
        let items = [pr(1, "a", "main"), pr(2, "b", "a"), pr(3, "c", "a")];
        assert_eq!(
            drawn(&items),
            [" ● #3", " │", "●│ #2", "││", "●╯ #1", "• main"]
        );
    }

    #[test]
    fn a_side_lane_passes_a_whole_substack() {
        let items = [
            pr(1, "a", "main"),
            pr(2, "b", "a"),
            pr(3, "c", "b"),
            pr(4, "d", "a"),
        ];
        assert_eq!(
            drawn(&items),
            [
                " ● #4",
                " │",
                "●│ #3",
                "││",
                "●│ #2",
                "││",
                "●╯ #1",
                "• main",
            ]
        );
    }

    #[test]
    fn a_cycle_shows_every_row_once_and_no_trunk() {
        let items = [pr(1, "a", "b"), pr(2, "b", "a"), pr(3, "c", "c")];
        assert_eq!(drawn(&items), ["● #2", "│", "● #1", "", "● #3"]);
    }

    #[test]
    fn graphite_links_by_owner_repo_and_number() {
        assert_eq!(
            graphite_url("acme", "widgets", 42),
            "https://app.graphite.dev/github/pr/acme/widgets/42"
        );
    }
}

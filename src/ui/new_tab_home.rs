//! Where a new tab lands in the sidebar: the group its own cwd resolves to,
//! last in it, and never the active tab's group just because it was active.
//! A tab whose cwd has no answer yet sits in Ungrouped and is moved to the end
//! of its group once the probe answers, unless the user has filed or dragged
//! it by then. Kept out of `app.rs`: [`Tty7App::seat_new_tab`] asks
//! [`Tty7App::new_tab_home`] for the slot, `render` calls
//! [`Tty7App::home_new_tabs`], and `apply_tab_order` forgets the waiting tabs.

use gpui::Context;
use tty7_core::core::machine::TabId;

use crate::core::config::{Config, TabBarPosition};
use crate::core::group_key::{GroupId, GroupKey, place};
use crate::terminal::git_status::GitStatusCache;
use crate::ui::app::{Tab, Tty7App};

/// The slot a new tab of group `key` is inserted at, among tabs drawn in
/// `keys`. Beside an active tab of the same group it takes `upstream` (the
/// `new_tab_position` setting); otherwise it goes after the group's last tab,
/// or at the end when the group has none yet.
pub(crate) fn seat_index(
    keys: &[Option<GroupKey>],
    active: usize,
    key: &Option<GroupKey>,
    upstream: usize,
) -> usize {
    if keys.get(active) == Some(key) {
        return upstream;
    }
    keys.iter()
        .rposition(|k| k == key)
        .map_or(keys.len(), |i| i + 1)
}

/// Where tab `from` is reinserted, once removed, to sit last in its group.
pub(crate) fn home_after_removal(keys: &[Option<GroupKey>], from: usize) -> usize {
    let key = &keys[from];
    keys.iter()
        .enumerate()
        .filter(|&(i, _)| i != from)
        .map(|(_, k)| k)
        .enumerate()
        .filter(|&(_, k)| k == key)
        .map(|(j, _)| j + 1)
        .last()
        .unwrap_or(keys.len() - 1)
}

impl Tty7App {
    fn sidebar_on_left(cx: &gpui::App) -> bool {
        cx.global::<Config>().tab_bar_position == TabBarPosition::Left
    }

    fn drawn_key(&self, tab: &Tab, cx: &gpui::App) -> Option<GroupKey> {
        place(
            tab.group.get(),
            &self.sidebar_groups,
            cx.global::<Config>().sidebar_auto_grouping,
            tab.auto_group.borrow().clone(),
        )
    }

    /// The index `tab`, not yet in `self.tabs`, is inserted at.
    pub(crate) fn new_tab_home(&self, tab: &Tab, cx: &gpui::App) -> usize {
        let upstream = self.new_tab_insert_at(cx);
        if !Self::sidebar_on_left(cx) {
            return upstream;
        }
        let keys = self.sidebar_group_keys(cx);
        seat_index(&keys, self.active, &self.drawn_key(tab, cx), upstream)
    }

    /// Wait for tab `index`'s cwd when it has no group yet.
    pub(crate) fn note_unhomed(&mut self, index: usize, cx: &gpui::App) {
        let Some(tab) = self.tabs.get(index) else {
            return;
        };
        if Self::sidebar_on_left(cx) && self.drawn_key(tab, cx).is_none() {
            self.unhomed.push((tab.tree_id.get(), tab.group.get()));
        }
    }

    /// Move each waiting tab whose cwd has now resolved to the end of its
    /// group. One the user filed since (its pinned group changed) stays put;
    /// a drag clears the whole list in `apply_tab_order`.
    pub(crate) fn home_new_tabs(&mut self, cx: &mut Context<Self>) {
        if self.unhomed.is_empty() {
            return;
        }
        let waiting = std::mem::take(&mut self.unhomed);
        let mut keys = self.sidebar_group_keys(cx);
        let mut order: Vec<usize> = (0..self.tabs.len()).collect();
        let mut still = Vec::new();
        for (id, group) in waiting {
            let Some(at) = order.iter().position(|&i| self.tabs[i].tree_id.get() == id) else {
                continue;
            };
            let tab = &self.tabs[order[at]];
            if !cwd_answered(tab, cx) {
                still.push((id, group));
                continue;
            }
            if tab.group.get() != group || keys[at].is_none() {
                continue;
            }
            let to = home_after_removal(&keys, at);
            let key = keys.remove(at);
            keys.insert(to, key);
            let moved = order.remove(at);
            order.insert(to, moved);
        }
        self.apply_tab_order(&order, cx);
        self.unhomed = still;
    }
}

/// Whether the probe has answered for `tab`'s cwd, so its group is final.
/// A remote pane has no probe to wait for.
fn cwd_answered(tab: &Tab, cx: &gpui::App) -> bool {
    let Some(leaf) = tab.pane.first_leaf() else {
        return false;
    };
    let Some(view) = leaf.terminal() else {
        return false;
    };
    let view = view.read(cx);
    if view.remote_context().is_some() {
        return true;
    }
    view.git_status_cwd().is_some_and(|cwd| {
        cx.try_global::<GitStatusCache>()
            .and_then(|c| c.known_repo_for(view.host_id(), cwd))
            .is_some()
    })
}

/// Waiting tabs, by tree id, with the pinned group each had when it opened.
pub(crate) type Unhomed = Vec<(TabId, Option<GroupId>)>;

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use gpui::BorrowAppContext as _;

    use super::*;
    use crate::core::group_key::AutoKey;
    use crate::terminal::git_status::RepoSnapshot;
    use crate::terminal::view::quiet_test_pane;
    use crate::ui::app::test_window::harness_with_tabs;
    use crate::ui::host_ops::HostId;
    use crate::ui::pane::{Pane, PaneSlot};

    fn repo(s: &str) -> Option<GroupKey> {
        Some(GroupKey::Auto(AutoKey::Repo(PathBuf::from(s))))
    }

    #[test]
    fn a_path_in_another_group_lands_at_its_end() {
        let keys = [repo("/a"), repo("/a"), repo("/b"), repo("/a"), None];
        assert_eq!(seat_index(&keys, 4, &repo("/a"), 5), 4, "after the last /a");
        assert_eq!(
            seat_index(&keys, 0, &repo("/b"), 1),
            3,
            "not beside the active /a"
        );
    }

    #[test]
    fn an_unknown_path_goes_ungrouped_not_to_the_active_group() {
        let keys = [repo("/a"), None, repo("/a")];
        assert_eq!(
            seat_index(&keys, 0, &None, 1),
            2,
            "after the last ungrouped tab"
        );
        let keys = [repo("/a"), repo("/a")];
        assert_eq!(seat_index(&keys, 0, &None, 1), 2, "none yet: the end");
        assert_eq!(
            seat_index(&keys, 0, &repo("/c"), 1),
            2,
            "new group: the end"
        );
    }

    #[test]
    fn beside_its_own_group_it_keeps_the_new_tab_position_setting() {
        let keys = [repo("/a"), repo("/a"), repo("/a")];
        assert_eq!(seat_index(&keys, 0, &repo("/a"), 1), 1);
    }

    #[test]
    fn a_resolved_tab_moves_to_the_end_of_its_group() {
        let keys = [repo("/a"), repo("/b"), repo("/a"), repo("/b")];
        assert_eq!(home_after_removal(&keys, 1), 3, "after the other /b");
        let keys = [repo("/a"), repo("/c"), repo("/a")];
        assert_eq!(
            home_after_removal(&keys, 1),
            2,
            "alone in its group: the end"
        );
    }

    fn cold_tab(
        app: &gpui::Entity<Tty7App>,
        vcx: &mut gpui::VisualTestContext,
        cwd: &str,
    ) -> TabId {
        let cwd = PathBuf::from(cwd);
        app.update_in(vcx, |app, window, cx| {
            let (view, _stream) = quiet_test_pane(9, window, cx);
            view.update(cx, |v, _| v.set_git_status_cwd_for_test(Some(cwd)));
            let tab = Tab::new(Pane::leaf(PaneSlot::Ready(view)));
            let id = tab.tree_id.get();
            app.seat_new_tab(tab, window, cx);
            id
        })
    }

    fn answer(vcx: &mut gpui::VisualTestContext, cwd: &str, root: Option<&str>) {
        vcx.update(|_, cx| {
            cx.update_global::<GitStatusCache, _>(|cache, _| {
                cache.finish_probe(
                    HostId::LOCAL,
                    std::path::Path::new(cwd),
                    root.map(|root| RepoSnapshot {
                        root: PathBuf::from(root),
                        home: PathBuf::from(root),
                        branch: "main".into(),
                        counts: Some((0, 0)),
                    }),
                );
            })
        });
    }

    fn index_of(
        app: &gpui::Entity<Tty7App>,
        vcx: &mut gpui::VisualTestContext,
        id: TabId,
    ) -> usize {
        app.update(vcx, |app, _| {
            app.tabs.iter().position(|t| t.tree_id.get() == id).unwrap()
        })
    }

    /// Tab 0 in no repo, tab 1 in /w/b, tab 2 in /w/a and active. A tab
    /// opened in /w/b before its probe answers waits last in Ungrouped (after
    /// tab 0), then moves after the /w/b tab once it does.
    #[gpui::test]
    fn a_tab_opened_cold_moves_to_its_group_when_the_probe_answers(cx: &mut gpui::TestAppContext) {
        let (app, mut vcx, _streams) = harness_with_tabs(cx, 3);
        for (i, dir, root) in [
            (0, "/w/plain", None),
            (1, "/w/b", Some("/w/b")),
            (2, "/w/a", Some("/w/a")),
        ] {
            app.update(&mut vcx, |app, cx| {
                let view = app.tabs[i].pane.first_leaf().unwrap();
                let view = view.terminal().unwrap();
                view.update(cx, |v, _| v.set_git_status_cwd_for_test(Some(dir.into())));
            });
            answer(&mut vcx, dir, root);
        }
        app.update_in(&mut vcx, |app, window, cx| app.activate(2, window, cx));

        let id = cold_tab(&app, &mut vcx, "/w/b/src");
        assert_eq!(
            index_of(&app, &mut vcx, id),
            1,
            "unknown: last in Ungrouped"
        );
        let other = cold_tab(&app, &mut vcx, "/w/c");
        vcx.run_until_parked();
        assert_eq!(
            app.update(&mut vcx, |app, _| app.unhomed.len()),
            2,
            "both wait"
        );

        answer(&mut vcx, "/w/b/src", Some("/w/b"));
        app.update(&mut vcx, |app, cx| app.home_new_tabs(cx));
        assert_eq!(index_of(&app, &mut vcx, id), 3, "after the /w/b tab");
        assert_eq!(index_of(&app, &mut vcx, other), 1);
        app.update(&mut vcx, |app, cx| {
            assert_eq!(app.sidebar_group_keys(cx)[3], repo("/w/b"));
            assert_eq!(app.unhomed.len(), 1, "the /w/c tab still waits");
        });
    }

    /// A tab filed into a pinned group while it waited is the user's; the
    /// probe answering does not move it.
    #[gpui::test]
    fn a_hand_grouped_tab_is_not_moved(cx: &mut gpui::TestAppContext) {
        let (app, mut vcx, _streams) = harness_with_tabs(cx, 2);
        app.update(&mut vcx, |app, cx| {
            let view = app.tabs[0].pane.first_leaf().unwrap();
            let view = view.terminal().unwrap();
            view.update(cx, |v, _| {
                v.set_git_status_cwd_for_test(Some("/w/b".into()))
            });
        });
        answer(&mut vcx, "/w/b", Some("/w/b"));
        let id = cold_tab(&app, &mut vcx, "/w/b/src");
        let before = index_of(&app, &mut vcx, id);
        app.update(&mut vcx, |app, cx| {
            let group = crate::core::group_key::PinnedGroup::label("work");
            let gid = group.id;
            app.edit_groups(cx, |g| g.pinned.push(group));
            app.set_tab_group(before, Some(gid), cx);
        });
        answer(&mut vcx, "/w/b/src", Some("/w/b"));
        app.update(&mut vcx, |app, cx| app.home_new_tabs(cx));
        assert_eq!(index_of(&app, &mut vcx, id), before);
        assert!(app.update(&mut vcx, |app, _| app.unhomed.is_empty()));
    }
}

//! A pinned folder group holds the tabs whose cwd is in its folder. This
//! overrides upstream's "a tab in one only ever leaves by hand"
//! (`group_key.rs`) for folder groups: a tab kept in one whose cwd is outside
//! its folder leaves, into the pinned folder it is in or back to auto
//! grouping, even one filed there before a restart by a cwd since corrected.
//!
//! Label groups have no folder and are never touched here. Tabs an older
//! build misfiled are repaired once, in the machine tree
//! (`tty7_core::core::group_migrate`).
//!
//! A tab can be pinned to the group it is in now ("Pin to Current Group", or
//! by putting it in a group by hand): it then neither walks out nor in, and its
//! auto group stays the one it last resolved to. The pins live in
//! [`WorkspaceGroups::held`], so they persist and every window agrees.

use gpui::{Context, WeakEntity};
use gpui_component::menu::{PopupMenu, PopupMenuItem};

use crate::core::group_key::{GroupId, WorkspaceGroups};
use crate::ui::app::{Tab, Tty7App};
use crate::ui::i18n::{L10nKey, t};

/// Whether `tab` is pinned to the group it is in now.
pub(crate) fn held(groups: &WorkspaceGroups, tab: &Tab) -> bool {
    groups.held.contains(&tab.tree_id.get())
}

/// Whether `tab`'s auto group stays the one it last resolved to: it is held
/// and drawn in it. One held in a pinned group keeps the answer warm, so if
/// the group is deleted it lands where its cwd is.
pub(crate) fn frozen(groups: &WorkspaceGroups, tab: &Tab) -> bool {
    held(groups, tab) && !tab.group.get().is_some_and(|g| groups.contains(g))
}

impl Tty7App {
    /// Pin tab `index` to the group it is in now, or (`on` false) hand it back
    /// to automatic regrouping.
    // shortcut: a closed tab's pin stays in `held` (one uuid each); prune when
    // the list is seen to grow.
    pub(crate) fn hold_tab_group(&mut self, index: usize, on: bool, cx: &mut Context<Self>) {
        let Some(tab) = self.tabs.get(index) else {
            return;
        };
        if on == held(&self.sidebar_groups, tab) {
            return;
        }
        let id = tab.tree_id.get();
        self.edit_groups(cx, |groups| match on {
            true => groups.held.push(id),
            false => groups.held.retain(|t| *t != id),
        });
    }
}

/// The tab menu's "Pin to Current Group" toggle for tab `index`.
pub(crate) fn hold_item(
    menu: PopupMenu,
    index: usize,
    app: &WeakEntity<Tty7App>,
    cx: &Context<PopupMenu>,
) -> PopupMenu {
    let Some(on) = app.upgrade().and_then(|app| {
        let app = app.read(cx);
        Some(held(&app.sidebar_groups, app.tabs.get(index)?))
    }) else {
        return menu;
    };
    let app = app.clone();
    menu.item(
        PopupMenuItem::new(t(L10nKey::SidebarPinToCurrentGroup))
            .checked(on)
            .on_click(move |_, _window, cx| {
                let _ = app.update(cx, |this, cx| this.hold_tab_group(index, !on, cx));
            }),
    )
}

/// Where a tab kept in `kept` goes now that its (answered) cwd is in pinned
/// folder `inside`, for a tab not [`held`]: `Some` is the new group
/// (`Some(None)` hands it back to auto grouping), `None` leaves it be.
pub(crate) fn walked_out(
    groups: &WorkspaceGroups,
    kept: Option<GroupId>,
    inside: Option<GroupId>,
) -> Option<Option<GroupId>> {
    let kept = kept?;
    groups.get(kept)?.folder.as_ref()?;
    (inside != Some(kept)).then_some(inside)
}

/// Whether a new tab opened beside one kept in `kept` inherits it. Only a
/// label group: a folder group is joined by the new tab's own cwd, and one
/// it opens outside of is not its group.
pub(crate) fn inherits(groups: &WorkspaceGroups, kept: GroupId) -> bool {
    groups.get(kept).is_some_and(|g| g.folder.is_none())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::group_key::{EntryWatch, PinnedGroup};
    use crate::ui::app::test_window::harness_with_tabs;
    use crate::ui::reorder;
    use crate::ui::tab_sidebar::{folder, plant_repo, repo};
    use gpui::TestAppContext;
    use std::path::{Path, PathBuf};

    #[test]
    fn an_unheld_tab_outside_its_folder_group_walks_out() {
        let clara = PinnedGroup::folder(Path::new("/w/clara"));
        let tty7 = PinnedGroup::folder(Path::new("/w/tty7"));
        let work = PinnedGroup::label("work");
        let (c, t, w) = (clara.id, tty7.id, work.id);
        let groups = WorkspaceGroups {
            pinned: vec![clara, tty7, work],
            ..Default::default()
        };
        assert_eq!(walked_out(&groups, Some(c), None), Some(None));
        assert_eq!(walked_out(&groups, Some(c), Some(t)), Some(Some(t)));
        assert_eq!(walked_out(&groups, Some(c), Some(c)), None);
        assert_eq!(walked_out(&groups, Some(w), None), None, "label");
        assert!(inherits(&groups, w) && !inherits(&groups, c));
    }

    /// A tab filed into a folder group by a cwd since corrected leaves it,
    /// even when restored that way (a baseline watch); a held one stays.
    #[gpui::test]
    fn a_restored_tab_outside_its_folder_group_leaves_unless_held(cx: &mut TestAppContext) {
        let (app, mut vcx, _streams) = harness_with_tabs(cx, 2);

        let probed = app.update(&mut vcx, |app, cx| {
            let probed = folder(app, "/w/probed", cx);
            for i in 0..2 {
                app.tabs[i].group.set(Some(probed));
                app.tabs[i].folder_watch.set(EntryWatch::baseline());
                plant_repo(app, i, "/w/other", "/w/other", cx);
            }
            app.hold_tab_group(1, true, cx);
            cx.notify();
            probed
        });
        vcx.run_until_parked();
        app.update(&mut vcx, |app, cx| {
            assert_eq!(app.tabs[0].group.get(), None, "not held: left");
            assert_eq!(app.sidebar_group_keys(cx)[0], repo("/w/other"));
            assert_eq!(app.tabs[1].group.get(), Some(probed), "held: stays");
        });
    }

    /// A tab dragged into a folder group from outside its folder stays; one
    /// the folder pulled in walks back out with its cwd.
    #[gpui::test]
    fn a_folder_group_keeps_a_dragged_tab_and_lets_a_walker_go(cx: &mut TestAppContext) {
        let (app, mut vcx, _streams) = harness_with_tabs(cx, 2);

        let probed = app.update(&mut vcx, |app, cx| {
            let probed = folder(app, "/w/probed", cx);
            plant_repo(app, 0, "/w/probed/sub", "/w/probed", cx);
            plant_repo(app, 1, "/w/other", "/w/other", cx);
            cx.notify();
            probed
        });
        vcx.run_until_parked();
        app.update(&mut vcx, |app, cx| {
            assert_eq!(app.tabs[0].group.get(), Some(probed), "walked in");
            app.set_tab_group(1, Some(probed), cx);
        });
        for _ in 0..3 {
            app.update(&mut vcx, |_, cx| cx.notify());
            vcx.run_until_parked();
        }
        app.update(&mut vcx, |app, cx| {
            assert_eq!(app.tabs[1].group.get(), Some(probed), "dragged in: stays");
            plant_repo(app, 0, "/w/other", "/w/other", cx);
            cx.notify();
        });
        vcx.run_until_parked();
        app.update(&mut vcx, |app, cx| {
            assert_eq!(app.tabs[0].group.get(), None, "walked out");
            assert_eq!(app.sidebar_group_keys(cx)[0], repo("/w/other"));
        });
    }

    /// A tab pinned to its auto group stays there when its cwd moves repo,
    /// and follows the cwd again once unpinned.
    #[gpui::test]
    fn a_held_tab_ignores_auto_regrouping(cx: &mut TestAppContext) {
        let (app, mut vcx, _streams) = harness_with_tabs(cx, 1);

        app.update(&mut vcx, |app, cx| {
            plant_repo(app, 0, "/w/alpha", "/w/alpha", cx);
            cx.notify();
        });
        vcx.run_until_parked();
        app.update(&mut vcx, |app, cx| {
            assert_eq!(app.sidebar_group_keys(cx)[0], repo("/w/alpha"));
            app.hold_tab_group(0, true, cx);
            folder(app, "/w/beta", cx);
            plant_repo(app, 0, "/w/beta", "/w/beta", cx);
            cx.notify();
        });
        vcx.run_until_parked();
        app.update(&mut vcx, |app, cx| {
            assert_eq!(app.tabs[0].group.get(), None, "did not walk into a folder");
            assert_eq!(app.sidebar_group_keys(cx)[0], repo("/w/alpha"), "held");
            app.hold_tab_group(0, false, cx);
            app.edit_groups(cx, |g| g.pinned.clear());
        });
        vcx.run_until_parked();
        app.update(&mut vcx, |app, cx| {
            assert_eq!(app.sidebar_group_keys(cx)[0], repo("/w/beta"), "released");
        });
    }

    /// A drag into a group pins the tab there: a folder group keeps it after
    /// its cwd walks out. A drop below the divider unpins it.
    #[gpui::test]
    fn a_drag_pins_the_tab_to_its_new_group(cx: &mut TestAppContext) {
        let (app, mut vcx, _streams) = harness_with_tabs(cx, 1);

        let (probed, dragged) = app.update(&mut vcx, |app, cx| {
            let probed = folder(app, "/w/probed", cx);
            plant_repo(app, 0, "/w/probed/sub", "/w/probed", cx);
            cx.notify();
            (probed, app.tabs[0].tree_id.get())
        });
        vcx.run_until_parked();
        app.update(&mut vcx, |app, cx| {
            assert_eq!(app.tabs[0].group.get(), Some(probed), "walked in");
            let work = PinnedGroup::label("work");
            let w = work.id;
            app.edit_groups(cx, |g| g.pinned.push(work));
            app.regroup_tab(dragged, reorder::Regroup::Into(w), cx);
            app.regroup_tab(dragged, reorder::Regroup::Into(probed), cx);
            assert!(held(&app.sidebar_groups, &app.tabs[0]), "a drag pins");
            plant_repo(app, 0, "/w/other", "/w/other", cx);
            cx.notify();
        });
        vcx.run_until_parked();
        app.update(&mut vcx, |app, cx| {
            assert_eq!(app.tabs[0].group.get(), Some(probed), "held: stays");
            app.regroup_tab(dragged, reorder::Regroup::ToAuto, cx);
            assert!(!held(&app.sidebar_groups, &app.tabs[0]), "out unpins");
        });
        vcx.run_until_parked();
        app.update(&mut vcx, |app, cx| {
            assert_eq!(app.sidebar_group_keys(cx)[0], repo("/w/other"));
        });
    }

    /// Groups landing from another window reset every watch to baseline. A
    /// tab dragged in by hand must survive that.
    #[gpui::test]
    fn adopting_groups_leaves_a_dragged_tab_alone(cx: &mut TestAppContext) {
        let (app, mut vcx, _streams) = harness_with_tabs(cx, 1);

        let probed = app.update(&mut vcx, |app, cx| {
            let probed = folder(app, "/w/probed", cx);
            plant_repo(app, 0, "/w/other", "/w/other", cx);
            app.set_tab_group(0, Some(probed), cx);
            probed
        });
        vcx.run_until_parked();
        app.update(&mut vcx, |app, cx| {
            let mut groups = app.sidebar_groups.clone();
            groups.ungrouped_collapsed = !groups.ungrouped_collapsed;
            app.adopt_sidebar_groups(groups, cx);
        });
        for _ in 0..3 {
            app.update(&mut vcx, |_, cx| cx.notify());
            vcx.run_until_parked();
        }
        app.update(&mut vcx, |app, _| {
            assert_eq!(app.tabs[0].group.get(), Some(probed));
        });
    }

    /// Nested folders: out of the inner folder into the outer one, and back.
    #[gpui::test]
    fn a_tab_walks_between_nested_folder_groups(cx: &mut TestAppContext) {
        let (app, mut vcx, _streams) = harness_with_tabs(cx, 1);

        let (mono, pkg) = app.update(&mut vcx, |app, cx| {
            let mono = folder(app, "/w/mono", cx);
            let pkg = folder(app, "/w/mono/pkg", cx);
            plant_repo(app, 0, "/w/mono/pkg/src", "/w/mono", cx);
            cx.notify();
            (mono, pkg)
        });
        vcx.run_until_parked();
        app.update(&mut vcx, |app, cx| {
            assert_eq!(
                app.tabs[0].group.get(),
                Some(pkg),
                "walked into the package"
            );
            plant_repo(app, 0, "/w/mono/lib", "/w/mono", cx);
            cx.notify();
        });
        vcx.run_until_parked();
        app.update(&mut vcx, |app, cx| {
            assert_eq!(app.tabs[0].group.get(), Some(mono), "out into the root");
            plant_repo(app, 0, "/w/mono/pkg/src", "/w/mono", cx);
            cx.notify();
        });
        vcx.run_until_parked();
        app.update(&mut vcx, |app, _| {
            assert_eq!(app.tabs[0].group.get(), Some(pkg), "and back");
        });
    }

    /// ⌘T beside a tab kept in a folder group, in a cwd outside that folder,
    /// lands in the new cwd's group, not the folder group it was opened from.
    #[gpui::test]
    fn a_tab_spawned_beside_a_folder_group_does_not_inherit_it(cx: &mut TestAppContext) {
        let (app, mut vcx, _streams) = harness_with_tabs(cx, 1);

        app.update(&mut vcx, |app, cx| {
            let clara = folder(app, "/w/clara", cx);
            app.set_tab_group(0, Some(clara), cx);
            app.active = 0;
            let place = app.spawn_group(Some(&PathBuf::from("/w/tsmux")), cx);
            assert_eq!(place.group, None);
            let place = app.spawn_group(Some(&PathBuf::from("/w/clara/src")), cx);
            assert_eq!(place.group, Some(clara), "inside it, its cwd files it");
        });
    }
}

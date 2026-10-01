//! A pinned folder group holds the tabs whose cwd is in its folder. This
//! overrides upstream's "a tab in one only ever leaves by hand"
//! (`group_key.rs`) for folder groups: a tab last seen inside the folder of
//! the group it is kept in walks back out with its cwd, into the pinned
//! folder it is now in or back to auto grouping. A tab dragged in from
//! outside the folder is never in it, so it stays, until it is dragged out
//! or its cwd goes into the folder and out again, which evicts it like any
//! other.
//!
//! Label groups have no folder and are never touched here. Tabs an older
//! build misfiled are repaired once, in the machine tree
//! (`tty7_core::core::group_migrate`).

use crate::core::group_key::{EntryWatch, GroupId, WorkspaceGroups};

/// Where a tab kept in `kept` goes now that its (answered) cwd is in pinned
/// folder `inside`, given the watch as it stood before this look: `Some` is
/// the new group (`Some(None)` hands it back to auto grouping), `None` leaves
/// it be.
pub(crate) fn walked_out(
    groups: &WorkspaceGroups,
    kept: Option<GroupId>,
    before: EntryWatch,
    inside: Option<GroupId>,
) -> Option<Option<GroupId>> {
    let kept = kept?;
    groups.get(kept)?.folder.as_ref()?;
    (inside != Some(kept) && before.was_in(kept)).then_some(inside)
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
    use crate::core::group_key::PinnedGroup;
    use crate::ui::app::test_window::harness_with_tabs;
    use crate::ui::tab_sidebar::{folder, plant_repo, repo};
    use gpui::TestAppContext;
    use std::path::{Path, PathBuf};

    fn seen_in(g: Option<GroupId>) -> EntryWatch {
        let mut w = EntryWatch::fresh();
        w.observe(g);
        w
    }

    #[test]
    fn only_a_tab_last_seen_inside_its_folder_group_walks_out() {
        let clara = PinnedGroup::folder(Path::new("/w/clara"));
        let tty7 = PinnedGroup::folder(Path::new("/w/tty7"));
        let work = PinnedGroup::label("work");
        let (c, t, w) = (clara.id, tty7.id, work.id);
        let groups = WorkspaceGroups {
            pinned: vec![clara, tty7, work],
            ..Default::default()
        };
        assert_eq!(
            walked_out(&groups, Some(c), seen_in(Some(c)), None),
            Some(None)
        );
        assert_eq!(
            walked_out(&groups, Some(c), seen_in(Some(c)), Some(t)),
            Some(Some(t))
        );
        assert_eq!(
            walked_out(&groups, Some(c), seen_in(Some(c)), Some(c)),
            None
        );
        assert_eq!(
            walked_out(&groups, Some(c), seen_in(None), None),
            None,
            "dragged in"
        );
        assert_eq!(
            walked_out(&groups, Some(c), EntryWatch::baseline(), None),
            None,
            "restored"
        );
        assert_eq!(
            walked_out(&groups, Some(w), seen_in(Some(w)), None),
            None,
            "label"
        );
        assert!(inherits(&groups, w) && !inherits(&groups, c));
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

//! One-shot repair of the machine tree for tabs an older build filed in a
//! folder group automatically: ⌘T inherited the active tab's pinned group
//! whatever the new tab's cwd, so tabs sat in folder groups their repos are
//! not in. Runs once per tree, recorded by a marker beside `machine.json`
//! written only after the repaired tree is, so a tab dragged into such a
//! group after the repair is never touched by it. A tab dragged in by hand
//! before the upgrade looks the same as a misfiled one, and is moved once
//! too. A fresh install, or a tree with no workspaces, has nothing to repair
//! and is marked done with its first save; one that failed to load is not,
//! so the next good
//! load is repaired. A tab whose paths do not resolve (a volume not mounted
//! yet, a folder deleted since) is skipped and stays where it is: the repair
//! does not wait for it, since a folder that never comes back would keep it
//! running, and moving hand-dragged tabs, on every start.
//!
//! A worktree with no cwd on record is judged by its repo home: when that
//! home is pinned elsewhere it is left alone, and when it is not pinned at
//! all the tab goes back to auto grouping, even if the worktree itself sat
//! inside its folder group's folder.

use std::path::{Path, PathBuf};

use crate::core::group_key::{AutoKey, GroupId, PinnedGroup, pinned_folder_for};
use crate::core::machine::{Machine, PaneNode};

const MARKER: &str = "group-membership-repaired";

/// Repair `machine`, loaded from `path`, unless the marker beside it says
/// that already happened. `Some(moved)` asks for the marker, which the store
/// writes only once it has written the tree itself; `None` asks for nothing.
///
/// `existed` is whether the file was there before the load, `loaded` whether
/// it loaded (an empty tree from a failed load did not). A fresh install has
/// nothing to repair: its marker comes with the tree's first save, so tabs
/// dragged into folder groups later are never moved by it, and nothing is
/// written beside a tree that does not exist yet. A failed load asks for
/// nothing, so the next good load is repaired.
pub fn run_once(machine: &mut Machine, path: &Path, existed: bool, loaded: bool) -> Option<bool> {
    if path.with_file_name(MARKER).exists() {
        return None;
    }
    match (existed, loaded) {
        (false, _) => Some(false),
        (true, false) => None,
        (true, true) => {
            repair(machine);
            Some(true)
        }
    }
}

/// Record that the repaired tree at `path` is on disk.
pub(crate) fn mark_done(path: &Path) {
    let marker = path.with_file_name(MARKER);
    if let Err(e) = std::fs::write(&marker, "") {
        log::warn!("could not write {}: {e}", marker.display());
    }
}

/// Move each tab kept in a folder group it is not in to the pinned folder it
/// is in, else back to auto grouping. Answers whether anything moved.
///
/// "In" is the live rule (`pinned_folder_for`) on the tab's last cwd and its
/// last repo. With no cwd on record the repo stands in for it, so a tab is
/// left alone when its repo holds the folder (a monorepo package) or sits
/// under another pinned folder (a worktree whose home is pinned elsewhere):
/// without the cwd neither says which side the tab is on. Paths are compared
/// with symlinks resolved, and one that does not resolve is compared as
/// spelled too; the tab stays if either says it is in its group.
fn repair(machine: &mut Machine) -> bool {
    let mut moved = false;
    for ws in &mut machine.workspaces {
        let raw = &ws.groups.pinned;
        let resolved: Vec<PinnedGroup> = raw
            .iter()
            .map(|g| PinnedGroup {
                folder: g
                    .folder_path()
                    .map(|f| real(f).0.to_string_lossy().into_owned()),
                ..g.clone()
            })
            .collect();
        for tab in &mut ws.tabs {
            let Some(kept) = tab.group else { continue };
            let Some(folder) = raw
                .iter()
                .find(|g| g.id == kept)
                .and_then(|g| g.folder_path())
            else {
                continue;
            };
            let Some(AutoKey::Repo(repo)) = &tab.last_auto else {
                continue;
            };
            let cwd = first_pane(&tab.root)
                .and_then(|id| machine.panes.iter().find(|p| p.id == id))
                .and_then(|p| p.cwd.as_deref())
                .map(Path::new);
            let (repo_real, repo_ok) = real(repo);
            let (cwd_real, cwd_ok) = cwd.map_or((None, false), |c| {
                let (r, ok) = real(c);
                (Some(r), ok)
            });
            let (folder_real, folder_ok) = real(folder);
            if !repo_ok && !folder_ok && !cwd_ok {
                continue;
            }
            let Some(inside) = place(
                &resolved,
                kept,
                &folder_real,
                cwd_real.as_deref(),
                &repo_real,
            ) else {
                continue;
            };
            let unresolved = !repo_ok || (cwd.is_some() && !cwd_ok);
            if inside == Some(kept)
                || (unresolved
                    && place(raw, kept, folder, cwd, repo).is_none_or(|g| g == Some(kept)))
            {
                continue;
            }
            tab.group = inside;
            moved = true;
        }
    }
    moved
}

/// The pinned folder a tab kept in `kept` (whose folder is `folder`) is in,
/// or `None` when the paths cannot say.
fn place(
    pinned: &[PinnedGroup],
    kept: GroupId,
    folder: &Path,
    cwd: Option<&Path>,
    repo: &Path,
) -> Option<Option<GroupId>> {
    if let Some(cwd) = cwd {
        return Some(pinned_folder_for(pinned, Some(cwd), Some(repo)));
    }
    let by_repo = pinned_folder_for(pinned, Some(repo), Some(repo));
    (!folder.starts_with(repo) && by_repo.is_none_or(|g| g == kept)).then_some(by_repo)
}

/// `path` with symlinks resolved, and whether that worked; the raw path when
/// it did not.
fn real(path: &Path) -> (PathBuf, bool) {
    match path.canonicalize() {
        Ok(p) => (p, true),
        Err(_) => (path.to_path_buf(), false),
    }
}

fn first_pane(node: &PaneNode) -> Option<u64> {
    match node {
        PaneNode::Leaf { pane } => Some(*pane),
        PaneNode::Split { a, .. } => first_pane(a),
    }
}

#[cfg(test)]
mod tests {
    use crate::core::group_key::{AutoKey, GroupId, PinnedGroup};
    use crate::core::machine::{MACHINE_FILE, Machine, MachineStore, Tab};
    use serde_json::{Value, json};
    use std::path::{Path, PathBuf};

    const MARKER: &str = super::MARKER;

    fn tab(group: GroupId, repo: &Path, pane: u64, asleep: bool) -> Value {
        let mut t = Tab::leaf(pane);
        t.group = Some(group);
        t.last_auto = Some(AutoKey::Repo(repo.to_path_buf()));
        t.hibernated = asleep;
        serde_json::to_value(t).unwrap()
    }

    fn write(path: &Path, doc: &Value) {
        serde_json::from_value::<Machine>(doc.clone()).expect("the fixture is a machine tree");
        std::fs::write(path, doc.to_string()).unwrap();
    }

    fn groups_of(store: &MachineStore) -> Vec<Option<GroupId>> {
        store.machine().workspaces[0]
            .tabs
            .iter()
            .map(|t| t.group)
            .collect()
    }

    /// The live tree's bad state, plus the tabs the repair must not move.
    /// Repaired on the first open, and never again.
    #[test]
    fn misfiled_tabs_are_repaired_on_the_first_open_only() {
        let dir = tempfile::TempDir::new().unwrap();
        let root = dir.path();
        let path = root.join(MACHINE_FILE);
        let d = |rel: &str| -> PathBuf {
            let p = root.join(rel);
            std::fs::create_dir_all(&p).unwrap();
            p
        };
        let (clara_d, tty7_d, mono_d, pkg_d) = (d("clara"), d("tty7"), d("mono"), d("mono/pkg"));
        let (tsmux_d, clawmux_d, wt_d) = (d("tsmux"), d("clawmux"), d("clara/wt"));
        let tty7_src = d("tty7/src");
        // Spelled through /tmp, a symlink to /private/tmp on macOS.
        let tmp = tempfile::TempDir::new_in("/tmp").unwrap();
        let sym_raw = tmp.path().join("sym");
        std::fs::create_dir_all(&sym_raw).unwrap();
        let sym_real = sym_raw.canonicalize().unwrap();

        let clara = PinnedGroup::folder(&clara_d);
        let tty7 = PinnedGroup::folder(&tty7_d);
        let pkg = PinnedGroup::folder(&pkg_d);
        let sym = PinnedGroup::folder(&sym_raw);
        let work = PinnedGroup::label("work");
        let (c, t, p, s, w) = (clara.id, tty7.id, pkg.id, sym.id, work.id);
        let doc = json!({
            "workspaces": [{
                "groups": {"pinned": [clara, tty7, pkg, sym, work]},
                "tabs": [
                    tab(c, &tsmux_d, 1, false),
                    tab(c, &tty7_d, 2, true),
                    tab(t, &clawmux_d, 3, false),
                    tab(c, &clara_d, 4, false),
                    tab(p, &mono_d, 5, false),
                    tab(w, &tsmux_d, 6, false),
                    tab(c, &tty7_d, 7, false),
                    tab(c, &tty7_d, 8, false),
                    tab(s, &sym_real, 9, false),
                ],
            }],
            "panes": [
                {"id": 2, "cwd": tty7_src},
                {"id": 7, "cwd": wt_d},
            ],
        });
        write(&path, &doc);

        let store = MachineStore::open(&path);
        assert_eq!(
            groups_of(&store),
            vec![
                None,    // tsmux: out of clara, auto
                Some(t), // tty7 (asleep), cwd in tty7: into tty7's folder
                None,    // clawmux: out of tty7, auto
                Some(c), // clara's own tab stays
                Some(p), // a package inside its repo stays
                Some(w), // a label group is the user's
                Some(c), // a worktree under clara, home tty7: where it sits
                Some(c), // its repo is pinned elsewhere, no cwd: can't tell
                Some(s), // /tmp and /private/tmp are one folder
            ]
        );
        assert!(root.join(MARKER).exists());
        drop(store);

        // The user drags a tsmux tab into clara by hand after the repair.
        let mut doc: Value =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        doc["workspaces"][0]["tabs"][0]["group"] = json!(c);
        write(&path, &doc);

        let store = MachineStore::open(&path);
        assert_eq!(groups_of(&store)[0], Some(c), "the repair ran once");
    }

    /// A fresh install has nothing to repair: marked done, so tabs dragged
    /// into folder groups later are never moved by it, and with no file
    /// written beside a tree that does not exist yet.
    #[test]
    fn a_fresh_install_is_marked_done() {
        let dir = tempfile::TempDir::new().unwrap();
        let store = MachineStore::open(dir.path().join(MACHINE_FILE));
        assert_eq!(
            std::fs::read_dir(dir.path()).unwrap().count(),
            0,
            "nothing is written beside a tree that is not there yet"
        );
        store
            .workspace_create(None, Some("api".into()), None)
            .unwrap();
        store.flush();
        assert!(
            dir.path().join(MARKER).exists(),
            "it comes with the first save"
        );
    }

    /// The marker says the repaired tree is on disk; a failed write must
    /// leave the repair to run again.
    #[test]
    fn a_repair_that_could_not_be_written_is_not_marked_done() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join(MACHINE_FILE);
        let elsewhere = tempfile::TempDir::new().unwrap();
        let clara = PinnedGroup::folder(dir.path());
        let c = clara.id;
        write(
            &path,
            &json!({
                "workspaces": [{
                    "groups": {"pinned": [clara]},
                    "tabs": [tab(c, elsewhere.path(), 1, false)],
                }],
            }),
        );
        // Fails the write by squatting on `write_atomic_private`'s temp name.
        // A read-only directory would fail the marker's write as well, and
        // this test would pass with the marker written before the tree.
        let tmp = format!(".{MACHINE_FILE}.tmp.{}", std::process::id());
        std::fs::create_dir(dir.path().join(tmp)).unwrap();

        let store = MachineStore::open(&path);
        assert_eq!(groups_of(&store), vec![None], "repaired in memory");
        assert!(!dir.path().join(MARKER).exists());
    }

    #[test]
    fn a_tree_with_no_workspaces_is_marked_done() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join(MACHINE_FILE);
        write(&path, &json!({"workspaces": []}));
        let _store = MachineStore::open(&path);
        assert!(dir.path().join(MARKER).exists());
    }

    /// A tree that failed to load reads as empty. That is not a tree
    /// repaired: the next good load still needs it.
    #[test]
    fn a_tree_that_failed_to_load_is_not_marked_done() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join(MACHINE_FILE);
        std::fs::write(&path, "not json").unwrap();
        let store = MachineStore::open(&path);
        assert!(store.machine().workspaces.is_empty());
        assert!(!dir.path().join(MARKER).exists());
    }

    /// A tab whose paths do not resolve is skipped, once: the repair is
    /// still marked done, so a tab dragged in by hand afterwards stays put
    /// however long the dead folder group lingers.
    #[test]
    fn a_tab_that_cannot_be_judged_is_skipped_once() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join(MACHINE_FILE);
        let clara_d = dir.path().join("clara");
        std::fs::create_dir_all(&clara_d).unwrap();
        let elsewhere = tempfile::TempDir::new().unwrap();
        let gone = PinnedGroup::folder(Path::new("/nonexistent/a"));
        let clara = PinnedGroup::folder(&clara_d);
        let (g, c) = (gone.id, clara.id);
        let doc = json!({
            "workspaces": [{
                "groups": {"pinned": [gone, clara]},
                "tabs": [
                    tab(g, Path::new("/nonexistent/b"), 1, false),
                    tab(c, &clara_d, 2, false),
                ],
            }],
        });
        write(&path, &doc);
        let store = MachineStore::open(&path);
        assert_eq!(groups_of(&store), vec![Some(g), Some(c)]);
        assert!(dir.path().join(MARKER).exists());
        drop(store);

        // Dragged into clara by hand from another repo.
        let mut doc: Value =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        doc["workspaces"][0]["tabs"][1] = tab(c, elsewhere.path(), 2, false);
        write(&path, &doc);
        let store = MachineStore::open(&path);
        assert_eq!(
            groups_of(&store),
            vec![Some(g), Some(c)],
            "the dragged tab stays"
        );
    }

    /// A cwd that no longer exists is compared as spelled too: under a
    /// folder pinned through /tmp it is still inside, though only the folder
    /// resolves to /private/tmp.
    #[test]
    fn an_unresolved_cwd_is_compared_as_spelled() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join(MACHINE_FILE);
        let tmp = tempfile::TempDir::new_in("/tmp").unwrap();
        let folder = tmp.path().join("proj");
        std::fs::create_dir_all(&folder).unwrap();
        let elsewhere = tempfile::TempDir::new().unwrap();
        let proj = PinnedGroup::folder(&folder);
        let p = proj.id;
        write(
            &path,
            &json!({
                "workspaces": [{
                    "groups": {"pinned": [proj]},
                    "tabs": [tab(p, elsewhere.path(), 1, false)],
                }],
                "panes": [{"id": 1, "cwd": folder.join("deleted")}],
            }),
        );
        let store = MachineStore::open(&path);
        assert_eq!(groups_of(&store), vec![Some(p)]);
    }
}

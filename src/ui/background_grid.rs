//! The grid a new pane starts at. A pane is laid out only once its tab is
//! shown, so one spawned at a placeholder grid in a tab nobody is looking at
//! (opened with ⇧, woken by Continue All or a restart, made by `tty7 tab new`,
//! or in a hidden hotkey window) runs its agent at that grid, and opening the
//! tab resizes it: the agent repaints the whole screen in front of the user.
//! So each window remembers the grid of its plain single-pane terminal tab,
//! new panes spawn at it, and a never-shown pane follows it when it changes.

use std::collections::HashSet;
use std::time::{Duration, Instant};

use gpui::{Context, EntityId, Task, Window, px};

use crate::terminal::TermSize;
use crate::ui::app::Tty7App;
use crate::ui::pane::{Pane, PaneSlot};

/// How long a new grid must hold before never-shown panes follow it, so a
/// window drag resizes each hidden agent once, not every frame.
const SETTLE: Duration = Duration::from_millis(200);

/// What a pane spawns at before any tab of its window has been laid out.
const PLACEHOLDER: (TermSize, u16, u16) = (TermSize { cols: 80, rows: 24 }, 8, 17);

/// A whole-tab terminal grid, its cell in device pixels at `scale`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Grid {
    size: TermSize,
    cell_w: u16,
    cell_h: u16,
    scale: f32,
}

#[cfg(test)]
impl Grid {
    pub(crate) fn for_test(cols: usize, rows: usize) -> Grid {
        Grid {
            size: TermSize::new(cols, rows),
            cell_w: 16,
            cell_h: 34,
            scale: 2.,
        }
    }
}

/// The size, and device-pixel cell, to spawn a pane at.
pub(crate) fn spawn_size(grid: Option<Grid>) -> (TermSize, u16, u16) {
    grid.map_or(PLACEHOLDER, |g| (g.size, g.cell_w, g.cell_h))
}

/// One window's whole-tab grid, kept on its `Tty7App`.
#[derive(Default)]
pub(crate) struct WholeTabGrid {
    latest: Option<(Grid, Instant)>,
    /// Repaints once a new grid has held; a newer grid drops the older wait.
    settle: Option<Task<()>>,
    /// Panes some frame has laid out; the rest have never been shown.
    shown: HashSet<EntityId>,
}

impl WholeTabGrid {
    /// The grid a new tab's pane spawns at, once this window has one.
    pub(crate) fn get(&self) -> Option<Grid> {
        self.latest.map(|(grid, _)| grid)
    }
}

/// Each frame: note the grid of the active tab when it is a plain terminal
/// filling the tab, and once the grid has held for [`SETTLE`], give it to the
/// never-shown pane of every other single-pane tab that is not at it yet (one
/// adopted, or spawned at an older grid). A tab that has been shown keeps its
/// own size, as does a split tab, whose panes follow its layout.
pub(crate) fn track(app: &mut Tty7App, window: &mut Window, cx: &mut Context<Tty7App>) {
    let docked = app.document_dock_px(window, cx).is_some();
    let now = cx.background_executor().now();
    let Some(tab) = app.tabs.get(app.active) else {
        return;
    };
    let mut changed = false;
    for leaf in tab.pane.leaves() {
        app.whole_tab_grid.shown.insert(leaf.entity_id());
    }
    if let Pane::Leaf(PaneSlot::Ready(view)) = &tab.pane
        && !docked
        && let Some((size, cell_w, cell_h)) = view.read(cx).terminal.laid_out_grid()
    {
        let grid = Grid {
            size,
            cell_w,
            cell_h,
            scale: window.scale_factor(),
        };
        if app.whole_tab_grid.get() != Some(grid) {
            app.whole_tab_grid.latest = Some((grid, now));
            changed = true;
        }
    }
    if changed {
        app.whole_tab_grid.settle = Some(cx.spawn_in(window, async move |this, cx| {
            cx.background_executor().timer(SETTLE).await;
            let _ = this.update(cx, |_, cx| cx.notify());
        }));
        return;
    }
    let Some((grid, since)) = app.whole_tab_grid.latest else {
        return;
    };
    if now.duration_since(since) < SETTLE {
        return;
    }
    let want = Some((grid.size, grid.cell_w, grid.cell_h));
    let mut live = HashSet::new();
    for (i, tab) in app.tabs.iter().enumerate() {
        live.extend(tab.pane.leaves().iter().map(PaneSlot::entity_id));
        let Pane::Leaf(PaneSlot::Ready(view)) = &tab.pane else {
            continue;
        };
        if i == app.active
            || app.whole_tab_grid.shown.contains(&view.entity_id())
            || view.read(cx).terminal.laid_out_grid() == want
        {
            continue;
        }
        view.update(cx, |view, cx| {
            view.set_grid_size(
                grid.size.cols,
                grid.size.rows,
                px(f32::from(grid.cell_w) / grid.scale),
                px(f32::from(grid.cell_h) / grid.scale),
                grid.scale,
                cx,
            );
        });
    }
    app.whole_tab_grid.shown.retain(|id| live.contains(id));
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use gpui::{TestAppContext, VisualTestContext};

    use super::*;
    use crate::daemon::protocol::{ClientMsg, WinSize};
    use crate::ui::app::test_window::harness_with_tabs;

    fn resizes(stream: &crate::daemon::transport::Stream) -> Arc<Mutex<Vec<WinSize>>> {
        let seen: Arc<Mutex<Vec<WinSize>>> = Default::default();
        let mut reader = stream.try_clone().unwrap();
        let out = seen.clone();
        std::thread::spawn(move || {
            while let Ok(msg) = ClientMsg::read(&mut reader) {
                if let ClientMsg::Resize(ws) = msg {
                    out.lock().unwrap().push(ws);
                }
            }
        });
        seen
    }

    /// Draw, let `SETTLE` pass on the executor's clock, draw again.
    fn settle(vcx: &mut VisualTestContext) {
        vcx.update(|window, _| window.refresh());
        vcx.run_until_parked();
        vcx.executor().advance_clock(SETTLE * 2);
        vcx.run_until_parked();
        vcx.update(|window, _| window.refresh());
        vcx.run_until_parked();
    }

    /// The link is a socket a reader thread drains, so a resize sent this
    /// frame lands a moment later, outside the executor.
    fn seen(log: &Arc<Mutex<Vec<WinSize>>>, n: usize) -> Vec<WinSize> {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
        while log.lock().unwrap().len() < n && std::time::Instant::now() < deadline {
            std::thread::yield_now();
        }
        log.lock().unwrap().clone()
    }

    fn grid_of(app: &gpui::Entity<Tty7App>, vcx: &mut VisualTestContext) -> Option<Grid> {
        app.read_with(vcx, |app, _| app.whole_tab_grid.get())
    }

    #[gpui::test]
    fn a_background_tab_is_sized_before_it_is_opened(cx: &mut TestAppContext) {
        let (app, mut vcx, streams) = harness_with_tabs(cx, 2);
        let (front, back) = (resizes(&streams[0]), resizes(&streams[1]));
        settle(&mut vcx);
        let shown = *seen(&front, 1).last().expect("the shown tab was laid out");
        assert_eq!(seen(&back, 1), [shown], "the hidden tab has the shown grid");
        app.update_in(&mut vcx, |app, window, cx| app.activate(1, window, cx));
        settle(&mut vcx);
        assert_eq!(seen(&back, 2).len(), 1, "opening it resizes nothing");
    }

    #[gpui::test]
    fn a_hidden_tab_follows_a_resize_once_it_settles(cx: &mut TestAppContext) {
        let (_app, mut vcx, streams) = harness_with_tabs(cx, 2);
        let back = resizes(&streams[1]);
        settle(&mut vcx);
        assert_eq!(seen(&back, 1).len(), 1);
        vcx.simulate_resize(gpui::size(px(900.), px(600.)));
        vcx.run_until_parked();
        assert_eq!(seen(&back, 2).len(), 1, "not before the grid settles");
        settle(&mut vcx);
        let after = seen(&back, 2);
        assert_eq!(after.len(), 2, "once, after it settles");
        assert!(after[1].cols < after[0].cols, "to the narrower window");
    }

    /// `adopt_workspace` swaps in a session's tabs while the window's grid
    /// stays put; they are swapped in here the same way, as quiet panes.
    #[gpui::test]
    fn tabs_adopted_into_a_window_with_a_grid_get_it(cx: &mut TestAppContext) {
        let (app, mut vcx, streams) = harness_with_tabs(cx, 1);
        let front = resizes(&streams[0]);
        settle(&mut vcx);
        let shown = *seen(&front, 1).last().expect("laid out");
        let adopted = app.update_in(&mut vcx, |app, window, cx| {
            let (view, stream) = crate::terminal::view::quiet_test_pane(9, window, cx);
            app.tabs
                .push(crate::ui::app::Tab::new(Pane::leaf(PaneSlot::Ready(view))));
            stream
        });
        let adopted = resizes(&adopted);
        settle(&mut vcx);
        assert_eq!(
            seen(&adopted, 1),
            [shown],
            "the adopted background tab has the grid"
        );
    }

    #[gpui::test]
    fn a_docked_document_column_is_not_the_whole_tab(cx: &mut TestAppContext) {
        let (app, mut vcx, _streams) = harness_with_tabs(cx, 2);
        settle(&mut vcx);
        let whole = grid_of(&app, &mut vcx).expect("recorded");
        app.update_in(&mut vcx, |app, window, cx| {
            app.toggle_code_panel(window, cx)
        });
        settle(&mut vcx);
        let docked = app.update_in(&mut vcx, |app, window, cx| app.document_dock_px(window, cx));
        assert!(docked.is_some(), "the code panel docks a column");
        assert_eq!(
            grid_of(&app, &mut vcx),
            Some(whole),
            "the column's narrower grid is ignored"
        );
    }

    #[gpui::test]
    fn a_new_pane_spawns_at_the_shown_tabs_grid(cx: &mut TestAppContext) {
        let (app, mut vcx, streams) = harness_with_tabs(cx, 1);
        let front = resizes(&streams[0]);
        settle(&mut vcx);
        let shown = *seen(&front, 1).last().expect("laid out");
        let (size, cell_w, cell_h) = spawn_size(grid_of(&app, &mut vcx));
        assert_eq!(
            (size.cols as u16, size.rows as u16, cell_w, cell_h),
            (shown.cols, shown.rows, shown.cell_w, shown.cell_h),
            "what new_terminal hands the spawn, with no frame of the new tab's own"
        );
    }

    #[test]
    fn a_window_with_no_grid_spawns_at_the_placeholder() {
        let grid = Grid {
            size: TermSize::new(200, 50),
            cell_w: 16,
            cell_h: 34,
            scale: 2.,
        };
        assert_eq!(spawn_size(Some(grid)), (TermSize::new(200, 50), 16, 34));
        assert_eq!(spawn_size(None), PLACEHOLDER);
    }
}

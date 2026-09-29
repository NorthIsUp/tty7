//! The fork's agent-resume features, kept out of `app.rs` so rebasing on
//! upstream touches as little of it as possible: which restored tabs sleep,
//! and Continue All Agents (`--continue`, the palette command).

use gpui::{App, Context, Window};

use crate::core::config::Config;
use crate::core::session::SessionPane;
use crate::ui::app::Tty7App;
use crate::ui::windows::WindowRegistry;

impl Tty7App {
    /// Wake every sleeping tab with an agent session in it, one every
    /// `continue_stagger_ms`, and tell each agent `continue_prompt` — the
    /// morning after a reboot, in one step. Tabs are found by id when their
    /// turn comes, so closing or moving one meanwhile is harmless.
    pub(crate) fn continue_all_agents(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.tabs.is_empty() {
            self.continue_when_tabs_land = true;
            return;
        }
        let cfg = cx.global::<Config>();
        let prompt = cfg.continue_prompt.clone();
        let stagger = std::time::Duration::from_millis(cfg.continue_stagger_ms);
        let ids: Vec<_> = self
            .tabs
            .iter()
            .filter(|t| t.asleep_layout().is_some_and(layout_has_agent_session))
            .map(|t| t.tree_id.get())
            .collect();
        log::info!(
            "continue all agents: {} of {} tabs asleep with an agent session",
            ids.len(),
            self.tabs.len()
        );
        cx.spawn_in(window, async move |this, cx| {
            for (n, id) in ids.into_iter().enumerate() {
                if n > 0 {
                    smol::Timer::after(stagger).await;
                }
                let woke = this.update_in(cx, |this, window, cx| {
                    if let Some(i) = this.tabs.iter().position(|t| t.tree_id.get() == id) {
                        this.wake_tab_with(i, Some(&prompt), window, cx);
                    }
                });
                if woke.is_err() {
                    return;
                }
            }
        })
        .detach();
    }
}

/// `tty7-app --continue`: Continue All Agents in the window launch opened.
pub fn continue_in_launch_window(cx: &mut App) {
    let Some(ws) = WindowRegistry::most_recent(cx) else {
        log::warn!("--continue: no window to continue in");
        return;
    };
    let (Some(handle), Some(app)) = (
        WindowRegistry::window_for(cx, ws),
        WindowRegistry::app_for(cx, ws),
    ) else {
        return;
    };
    let _ = handle.update(cx, |_, window, cx| {
        let _ = app.update(cx, |this, cx| this.continue_all_agents(window, cx));
    });
}

pub(crate) fn layout_has_agent_session(pane: &SessionPane) -> bool {
    match pane {
        SessionPane::Leaf {
            agent,
            agent_session_id,
            ..
        } => agent.is_some() && agent_session_id.is_some(),
        SessionPane::Split { a, b, .. } => {
            layout_has_agent_session(a) || layout_has_agent_session(b)
        }
    }
}

pub(crate) fn layout_has_live_pane(
    pane: &SessionPane,
    alive: Option<&std::collections::HashMap<u64, Option<String>>>,
) -> bool {
    match pane {
        SessionPane::Leaf { pane_id, .. } => {
            pane_id.is_some_and(|id| alive.is_some_and(|a| a.contains_key(&id)))
        }
        SessionPane::Split { a, b, .. } => {
            layout_has_live_pane(a, alive) || layout_has_live_pane(b, alive)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::layout_has_live_pane;

    #[test]
    fn a_tab_sleeps_through_restore_only_when_nothing_of_it_survived() {
        use crate::core::session::{SessionAxis, SessionPane};
        let leaf = |id: Option<u64>| SessionPane::Leaf {
            cwd: None,
            pane_id: id,
            shell: None,
            ssh_spec: None,
            agent: None,
            agent_session_id: None,
            agent_launch_argv: None,
        };
        let split = SessionPane::Split {
            axis: SessionAxis::Horizontal,
            ratio: 0.5,
            a: Box::new(leaf(Some(1))),
            b: Box::new(leaf(Some(2))),
        };
        let alive: std::collections::HashMap<u64, Option<String>> = [(2, None)].into();
        assert!(layout_has_live_pane(&split, Some(&alive)));
        assert!(!layout_has_live_pane(&leaf(Some(1)), Some(&alive)));
        assert!(!layout_has_live_pane(&leaf(None), Some(&alive)));
        assert!(!layout_has_live_pane(&split, Some(&Default::default())));
    }
}

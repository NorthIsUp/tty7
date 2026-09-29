//! The fork's agent-resume features, kept out of `app.rs` so rebasing on
//! upstream touches as little of it as possible: which restored tabs sleep,
//! and waking their agents (`--continue`, the palette command, and
//! `resume_agents_on_launch`).

use gpui::{App, Context, Window};

use crate::core::config::Config;
use crate::core::session::SessionPane;
use crate::ui::app::Tty7App;
use crate::ui::windows::WindowRegistry;
use tty7_core::core::machine::TabId;

impl Tty7App {
    /// Continue All Agents: [`Self::wake_agent_tabs`] with `continue_prompt`.
    pub(crate) fn continue_all_agents(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let prompt = cx.global::<Config>().continue_prompt.clone();
        self.wake_agent_tabs(Some(prompt), window, cx);
    }

    /// Wake every sleeping tab with an agent session in it, one every
    /// `continue_stagger_ms`, and tell each agent `prompt` — the morning after
    /// a reboot, in one step. Tabs are found by id when their turn comes, so
    /// closing or moving one meanwhile is harmless.
    pub(crate) fn wake_agent_tabs(
        &mut self,
        prompt: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.tabs.is_empty() {
            self.continue_when_tabs_land = Some(prompt);
            return;
        }
        let stagger = std::time::Duration::from_millis(cx.global::<Config>().continue_stagger_ms);
        let ids = agent_tabs(
            self.tabs
                .iter()
                .map(|t| (t.tree_id.get(), t.asleep_layout())),
        );
        log::info!(
            "waking agents: {} of {} tabs asleep with an agent session",
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
                        this.wake_tab_with(i, prompt.as_deref(), window, cx);
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

/// The tabs a wake picks: asleep, with an agent session somewhere in them.
fn agent_tabs<'a>(tabs: impl IntoIterator<Item = (TabId, Option<&'a SessionPane>)>) -> Vec<TabId> {
    tabs.into_iter()
        .filter(|(_, asleep)| asleep.is_some_and(layout_has_agent_session))
        .map(|(id, _)| id)
        .collect()
}

/// What launch wakes agents with: `--continue` sends `continue_prompt`,
/// `resume_agents_on_launch` resumes without a prompt, else nothing wakes.
/// One answer, so a launch with both wakes each tab once.
fn launch_wake(continue_flag: bool, cfg: &Config) -> Option<Option<String>> {
    if continue_flag {
        Some(Some(cfg.continue_prompt.clone()))
    } else if cfg.resume_agents_on_launch {
        Some(None)
    } else {
        None
    }
}

/// Launch's agent wake (`--continue`, `resume_agents_on_launch`) in the
/// window launch opened.
pub fn wake_launch_window(cx: &mut App, continue_flag: bool) {
    let Some(prompt) = launch_wake(continue_flag, cx.global::<Config>()) else {
        return;
    };
    let Some(ws) = WindowRegistry::most_recent(cx) else {
        log::warn!("launch wake: no window to wake agents in");
        return;
    };
    let (Some(handle), Some(app)) = (
        WindowRegistry::window_for(cx, ws),
        WindowRegistry::app_for(cx, ws),
    ) else {
        return;
    };
    let _ = handle.update(cx, |_, window, cx| {
        let _ = app.update(cx, |this, cx| this.wake_agent_tabs(prompt, window, cx));
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
    use super::{agent_tabs, launch_wake, layout_has_live_pane};
    use crate::core::config::Config;
    use crate::core::session::SessionPane;
    use tty7_core::core::cli_agent::CLIAgent;
    use tty7_core::core::machine::TabId;

    fn agent_leaf(session: Option<&str>) -> SessionPane {
        SessionPane::Leaf {
            cwd: None,
            pane_id: None,
            shell: None,
            ssh_spec: None,
            agent: session.map(|_| CLIAgent::Claude),
            agent_session_id: session.map(Into::into),
            agent_launch_argv: None,
        }
    }

    #[test]
    fn a_wake_picks_only_sleeping_tabs_with_an_agent_session() {
        let agent = agent_leaf(Some("s1"));
        let shell = agent_leaf(None);
        let (asleep_agent, asleep_shell, awake) = (TabId::new(), TabId::new(), TabId::new());
        let picked = agent_tabs([
            (asleep_agent, Some(&agent)),
            (asleep_shell, Some(&shell)),
            (awake, None),
        ]);
        assert_eq!(picked, vec![asleep_agent]);
    }

    #[test]
    fn launch_wakes_once_with_the_prompt_only_under_continue() {
        let mut cfg = Config::default();
        cfg.continue_prompt = "go".into();
        assert!(cfg.resume_agents_on_launch);
        assert_eq!(launch_wake(true, &cfg), Some(Some("go".into())));
        assert_eq!(launch_wake(false, &cfg), Some(None));
        cfg.resume_agents_on_launch = false;
        assert_eq!(launch_wake(true, &cfg), Some(Some("go".into())));
        assert_eq!(launch_wake(false, &cfg), None);
    }

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

//! The sidebar row's pid, CPU% and memory: every process in each of a tab's
//! pane trees, summed.
//!
//! One poll for the whole app, over the panes the sidebar last drew, so a
//! window full of tabs costs one round of `QueryProcs` per interval rather than
//! a poll per row.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use gpui::{
    AnyElement, App, Context, Global, IntoElement as _, ParentElement as _, Styled as _, Window,
    div, px,
};
use gpui_component::{ActiveTheme as _, h_flex};
use tty7_core::core::fork_config::ForkConfig;

use crate::core::config::Config;
use crate::daemon::procstat::compact_bytes;
use crate::daemon::protocol::PaneProcs;
use crate::ui::app::{Tab, Tty7App};
use crate::ui::host_ops::SharedHost;
use crate::ui::i18n::{L10nKey, t};
use crate::ui::pane::Pane;
use crate::ui::proc_usage::{CpuTracker, format_cpu, totals};
use crate::ui::tab_strip::measure_text;
use crate::ui::theme::tabular_figures;

const POLL: Duration = Duration::from_secs(3);

#[derive(Default)]
struct PaneUsage {
    procs: PaneProcs,
    cpu: CpuTracker,
}

#[derive(Default)]
pub(crate) struct TabUsage {
    wanted: HashMap<u64, Option<SharedHost>>,
    panes: HashMap<u64, PaneUsage>,
    polling: bool,
}

impl Global for TabUsage {}

/// What a row shows: the front pane's shell pid, and the tab's summed CPU% and
/// resident memory.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Usage {
    pub pid: Option<u32>,
    pub cpu: Option<f64>,
    pub rss: Option<u64>,
}

/// Which parts a row shows, from the settings.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Show {
    pub pid: bool,
    pub cpu: bool,
    pub memory: bool,
    pub annotate: bool,
}

impl Show {
    pub(crate) fn of(fork: &ForkConfig) -> Show {
        Show {
            pid: fork.tab_usage_pid,
            cpu: fork.tab_usage_cpu,
            memory: fork.tab_usage_memory,
            annotate: fork.tab_usage_annotate,
        }
    }

    /// Whether anything is drawn, so the poll can stay off; `annotate` only
    /// colours numbers already shown.
    pub(crate) fn any(&self) -> bool {
        self.pid || self.cpu || self.memory
    }
}

/// Where a tab counts as high-use.
const HOT_CPU: f64 = 80.0;
const HOT_RSS: u64 = 4 << 30;

/// One part of a row's numbers; `hot` ones take the warning colour.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Cell {
    pub text: String,
    pub hot: bool,
}

/// The Processes panel's figures, padded so a mono font keeps the width still
/// as the number moves.
fn cpu_text(pct: f64) -> String {
    format!("{:>5}", format_cpu(pct))
}

fn memory_text(bytes: u64) -> String {
    format!("{:>7}", compact_bytes(bytes))
}

impl Usage {
    pub(crate) fn cells(&self, show: Show) -> Vec<Cell> {
        let cpu_hot = show.annotate && self.cpu.is_some_and(|p| p >= HOT_CPU);
        let rss_hot = show.annotate && self.rss.is_some_and(|b| b >= HOT_RSS);
        let mut out = Vec::new();
        if let Some(pid) = self.pid.filter(|_| show.pid) {
            out.push(Cell {
                text: pid.to_string(),
                hot: false,
            });
        }
        if let Some(pct) = self.cpu.filter(|_| show.cpu) {
            out.push(Cell {
                text: cpu_text(pct),
                hot: cpu_hot,
            });
        }
        if let Some(bytes) = self.rss.filter(|_| show.memory) {
            out.push(Cell {
                text: memory_text(bytes),
                hot: rss_hot,
            });
        }
        out
    }
}

/// What a sidebar row draws at the end of its branch or title line, and the
/// width it takes out of the label.
pub(crate) struct RowUsage {
    pub el: AnyElement,
    pub w: f32,
}

/// A remote pane runs on the peer, so only its host can walk the process tree;
/// `None` from the host means it could not be asked, not an empty answer (see
/// `Host::pane_procs`). The same dispatch as Info → Processes' poll.
fn pane_procs(host: Option<&SharedHost>, pane_id: u64) -> Option<PaneProcs> {
    match host {
        Some(host) => host.pane_procs(pane_id),
        None => Some(crate::terminal::RemoteTerminal::query_procs(pane_id)),
    }
}

fn same_panes(a: &HashMap<u64, Option<SharedHost>>, b: &HashMap<u64, Option<SharedHost>>) -> bool {
    a.len() == b.len()
        && a.iter().all(|(id, host)| {
            b.get(id).is_some_and(|other| match (host, other) {
                (Some(x), Some(y)) => Arc::ptr_eq(x, y),
                (None, None) => true,
                _ => false,
            })
        })
}

impl TabUsage {
    /// Points the poll at every pane of `tabs`, or at none while nothing is
    /// shown, and starts it when it has something to ask.
    pub(crate) fn want(cx: &mut App, tabs: &[Tab]) {
        let panes: HashMap<u64, Option<SharedHost>> =
            match Show::of(&cx.global::<Config>().fork).any() {
                true => tabs
                    .iter()
                    .flat_map(|t| t.pane.terminals())
                    .map(|v| {
                        let v = v.read(cx);
                        let host = (!v.host_id().is_local()).then(|| v.host(cx)).flatten();
                        (v.pane_id, host)
                    })
                    .collect(),
                false => HashMap::new(),
            };
        // Called from render: touching the global marks it changed, which
        // draws another frame, so it is written only when the panes moved.
        if cx
            .try_global::<Self>()
            .is_some_and(|u| same_panes(&u.wanted, &panes))
        {
            return;
        }
        let usage = cx.default_global::<Self>();
        usage.wanted = panes;
        usage.panes.retain(|id, _| usage.wanted.contains_key(id));
        if !usage.polling && !usage.wanted.is_empty() {
            usage.polling = true;
            Self::poll(cx);
        }
    }

    /// A row's numbers for `pane`'s tab, or `None` when it has none to show
    /// or the title would keep under 96px of `avail` beside them.
    pub(crate) fn row(
        window: &Window,
        cx: &App,
        pane: &Pane,
        meta_size: f32,
        avail: f32,
    ) -> Option<RowUsage> {
        const GAP: f32 = 6.;
        let show = Show::of(&cx.global::<Config>().fork);
        let ids: Vec<u64> = pane
            .terminals()
            .iter()
            .map(|v| v.read(cx).pane_id)
            .collect();
        let front = pane
            .focused_or_first(window, cx)
            .map(|v| v.read(cx).pane_id);
        let cells = Self::of(cx, &ids, front)?.cells(show);
        if cells.is_empty() {
            return None;
        }
        // Small, mono and padded to fixed widths, so a number that moves from
        // 1.0 to 1.2 GB never nudges the row.
        let size = meta_size * 0.85;
        let mono = cx.theme().mono_font_family.clone();
        let font = gpui::Font {
            features: tabular_figures(),
            ..gpui::font(mono.clone())
        };
        let w: f32 = cells
            .iter()
            .map(|c| measure_text(window.text_system(), &font, size, &c.text) + GAP)
            .sum();
        if avail - w < 96. {
            return None;
        }
        let (muted, hot) = (cx.theme().muted_foreground, cx.theme().warning);
        let el = h_flex()
            .flex_shrink_0()
            .gap(px(GAP))
            .text_size(px(size))
            .font_family(mono)
            .children(cells.into_iter().map(|c| {
                div()
                    .whitespace_nowrap()
                    .text_color(if c.hot { hot } else { muted })
                    .child(c.text)
            }))
            .into_any_element();
        Some(RowUsage { el, w })
    }

    fn of(cx: &App, panes: &[u64], front: Option<u64>) -> Option<Usage> {
        let usage = cx.try_global::<Self>()?;
        let (mut cpu, mut rss) = (None::<f64>, None::<u64>);
        for pane in panes.iter().filter_map(|id| usage.panes.get(id)) {
            let (pct, bytes) = totals(&pane.procs.procs, &pane.cpu);
            if let Some(pct) = pct {
                *cpu.get_or_insert(0.) += pct;
            }
            if let Some(bytes) = bytes {
                *rss.get_or_insert(0) += bytes;
            }
        }
        let pid = front
            .and_then(|id| usage.panes.get(&id))
            .and_then(|p| p.procs.procs.first())
            .filter(|p| p.depth == 0)
            .map(|p| p.pid);
        (pid.is_some() || cpu.is_some() || rss.is_some()).then_some(Usage { pid, cpu, rss })
    }

    fn poll(cx: &mut App) {
        cx.spawn(async move |cx| {
            loop {
                cx.background_executor().timer(POLL).await;
                // Nothing wanted ends the loop; the next `want` with panes
                // starts another.
                let wanted = cx.update(|cx| {
                    let usage = cx.global_mut::<Self>();
                    usage.polling = !usage.wanted.is_empty();
                    usage.polling.then(|| usage.wanted.clone())
                });
                let Some(wanted) = wanted else { break };
                // One task per pane, so a slow host holds up only its own.
                let asks: Vec<_> = wanted
                    .into_iter()
                    .map(|(id, host)| {
                        cx.background_executor()
                            .spawn(async move { pane_procs(host.as_ref(), id).map(|p| (id, p)) })
                    })
                    .collect();
                let mut answers: Vec<(u64, PaneProcs)> = Vec::new();
                for ask in asks {
                    // `None`: the last answer stays until the host can be asked.
                    answers.extend(ask.await);
                }
                let now = Instant::now();
                if answers.is_empty() {
                    continue;
                }
                cx.update(|cx| {
                    // An answer the same as the last costs no frame: a quiet
                    // machine, or a test with no daemon, stays idle.
                    let changed = {
                        let usage = cx.global::<Self>();
                        answers.iter().any(|(id, procs)| {
                            usage.wanted.contains_key(id)
                                && usage.panes.get(id).is_none_or(|p| p.procs != *procs)
                        })
                    };
                    if !changed {
                        return;
                    }
                    let usage = cx.global_mut::<Self>();
                    for (id, procs) in answers {
                        if !usage.wanted.contains_key(&id) {
                            continue;
                        }
                        let pane = usage.panes.entry(id).or_default();
                        pane.cpu.sample(&procs.procs, now);
                        pane.procs = procs;
                    }
                    cx.refresh_windows();
                });
            }
        })
        .detach();
    }
}

impl Tty7App {
    /// The four rows for Settings' Tabs group.
    pub(crate) fn tab_usage_settings(&self, cx: &mut Context<Self>) -> [AnyElement; 4] {
        type Get = fn(&ForkConfig) -> bool;
        type Field = fn(&mut ForkConfig) -> &mut bool;
        let rows: [(&str, L10nKey, L10nKey, Get, Field); 4] = [
            (
                "tab-usage-pid",
                L10nKey::SettingsTabUsagePid,
                L10nKey::SettingsTabUsagePidDesc,
                |f| f.tab_usage_pid,
                |f| &mut f.tab_usage_pid,
            ),
            (
                "tab-usage-cpu",
                L10nKey::SettingsTabUsageCpu,
                L10nKey::SettingsTabUsageCpuDesc,
                |f| f.tab_usage_cpu,
                |f| &mut f.tab_usage_cpu,
            ),
            (
                "tab-usage-memory",
                L10nKey::SettingsTabUsageMemory,
                L10nKey::SettingsTabUsageMemoryDesc,
                |f| f.tab_usage_memory,
                |f| &mut f.tab_usage_memory,
            ),
            (
                "tab-usage-annotate",
                L10nKey::SettingsTabUsageAnnotate,
                L10nKey::SettingsTabUsageAnnotateDesc,
                |f| f.tab_usage_annotate,
                |f| &mut f.tab_usage_annotate,
            ),
        ];
        rows.map(|(id, title, desc, get, field)| {
            let on = get(&cx.global::<Config>().fork);
            let switch = self.settings_switch(id, on, cx, move |this, on, _, cx| {
                this.update_config(cx, |c| *field(&mut c.fork) = on)
            });
            self.settings_row(t(title), t(desc), switch, cx)
                .into_any_element()
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numbers_keep_their_width_as_they_move() {
        assert_eq!(cpu_text(3.24), " 3.2%");
        assert_eq!(cpu_text(12.0), "  12%");
        assert_eq!(cpu_text(100.0), " 100%");
        assert_eq!(memory_text(512 << 20), " 512 MB");
        assert_eq!(memory_text(1288490188), " 1.2 GB");
        let widths: Vec<usize> = [1u64 << 20, 999 << 20, 3 << 30, 200 << 30]
            .map(|b| memory_text(b).chars().count())
            .to_vec();
        assert!(widths.iter().all(|w| *w == 7), "{widths:?}");
    }

    #[test]
    fn annotate_only_colours_what_is_shown() {
        let usage = Usage {
            pid: Some(4821),
            cpu: Some(95.0),
            rss: Some(1 << 30),
        };
        let off = Show {
            pid: false,
            cpu: false,
            memory: false,
            annotate: true,
        };
        assert!(!off.any());
        assert!(usage.cells(off).is_empty());
        let all = Show {
            pid: true,
            cpu: true,
            memory: true,
            annotate: true,
        };
        let texts: Vec<_> = usage
            .cells(all)
            .into_iter()
            .map(|c| (c.text, c.hot))
            .collect();
        assert_eq!(
            texts,
            [
                ("4821".to_string(), false),
                ("  95%".to_string(), true),
                (" 1.0 GB".to_string(), false)
            ]
        );
        let plain = Show {
            annotate: false,
            ..all
        };
        assert!(usage.cells(plain).iter().all(|c| !c.hot));
    }

    #[test]
    fn want_compares_the_whole_pane_map() {
        let a: HashMap<u64, Option<SharedHost>> = [(1, None)].into();
        assert!(same_panes(&a, &a.clone()));
        assert!(!same_panes(&a, &[(2, None)].into()));
        assert!(!same_panes(&a, &HashMap::new()));
    }
}

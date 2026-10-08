//! The sidebar row's pid, CPU% and memory: every process in each of a tab's
//! pane trees, summed.
//!
//! One poll for the whole app, over the panes the sidebar last drew, so a
//! window full of tabs costs one round of `QueryProcs` per interval rather than
//! a poll per row.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use gpui::{AnyElement, App, Context, Global, IntoElement as _};

use crate::core::config::Config;
use crate::ui::app::Tty7App;
use crate::ui::i18n::{L10nKey, t};

use crate::daemon::protocol::PaneProcs;
use crate::ui::host_ops::SharedHost;
use crate::ui::proc_usage::{CpuTracker, totals};

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
    pub(crate) fn of(fork: &tty7_core::core::fork_config::ForkConfig) -> Show {
        Show {
            pid: fork.tab_usage_pid,
            cpu: fork.tab_usage_cpu,
            memory: fork.tab_usage_memory,
            annotate: fork.tab_usage_annotate,
        }
    }

    /// Whether anything could ever be drawn, so the poll can stay off.
    pub(crate) fn any(&self) -> bool {
        self.pid || self.cpu || self.memory || self.annotate
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

/// Always one decimal, padded to six columns, so a mono font keeps the width
/// still as the number moves (`  3.2%`, ` 12.0%`, `100.0%`).
fn cpu_text(pct: f64) -> String {
    format!("{:>5.1}%", pct.clamp(0., 999.9))
}

/// MB below a gigabyte, GB above, always `###.#` (`512.0 MB`, `  1.0 GB`).
fn memory_text(bytes: u64) -> String {
    const MB: f64 = 1024. * 1024.;
    let (v, unit) = match bytes as f64 / MB {
        mb if mb < 1000. => (mb, "MB"),
        mb => (mb / 1024., "GB"),
    };
    format!("{:>5.1} {unit}", v.min(999.9))
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
        if let Some(pct) = self.cpu.filter(|_| show.cpu || cpu_hot) {
            out.push(Cell {
                text: cpu_text(pct),
                hot: cpu_hot,
            });
        }
        if let Some(bytes) = self.rss.filter(|_| show.memory || rss_hot) {
            out.push(Cell {
                text: memory_text(bytes),
                hot: rss_hot,
            });
        }
        out
    }
}

impl TabUsage {
    /// Replaces the panes the poll asks about, and starts the poll the first
    /// time anyone wants one.
    pub(crate) fn want(cx: &mut App, panes: impl IntoIterator<Item = (u64, Option<SharedHost>)>) {
        let usage = cx.default_global::<Self>();
        usage.wanted = panes.into_iter().collect();
        usage.panes.retain(|id, _| usage.wanted.contains_key(id));
        if !usage.polling {
            usage.polling = true;
            Self::poll(cx);
        }
    }

    pub(crate) fn of(cx: &App, panes: &[u64], front: Option<u64>) -> Option<Usage> {
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
                let wanted = cx.update(|cx| cx.global::<Self>().wanted.clone());
                let answers: Vec<(u64, PaneProcs)> = cx
                    .background_executor()
                    .spawn(async move {
                        wanted
                            .into_iter()
                            .filter_map(|(id, host)| {
                                // `None` from a host means it could not be
                                // asked; the last answer stays until one can.
                                let procs = match host {
                                    Some(host) => host.pane_procs(id)?,
                                    None => crate::terminal::RemoteTerminal::query_procs(id),
                                };
                                Some((id, procs))
                            })
                            .collect()
                    })
                    .await;
                let now = Instant::now();
                cx.update(|cx| {
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
        type Field = fn(&mut tty7_core::core::fork_config::ForkConfig) -> &mut bool;
        let rows: [(&str, L10nKey, L10nKey, Field); 4] = [
            (
                "tab-usage-pid",
                L10nKey::SettingsTabUsagePid,
                L10nKey::SettingsTabUsagePidDesc,
                |f| &mut f.tab_usage_pid,
            ),
            (
                "tab-usage-cpu",
                L10nKey::SettingsTabUsageCpu,
                L10nKey::SettingsTabUsageCpuDesc,
                |f| &mut f.tab_usage_cpu,
            ),
            (
                "tab-usage-memory",
                L10nKey::SettingsTabUsageMemory,
                L10nKey::SettingsTabUsageMemoryDesc,
                |f| &mut f.tab_usage_memory,
            ),
            (
                "tab-usage-annotate",
                L10nKey::SettingsTabUsageAnnotate,
                L10nKey::SettingsTabUsageAnnotateDesc,
                |f| &mut f.tab_usage_annotate,
            ),
        ];
        let show = Show::of(&cx.global::<Config>().fork);
        let mut on = [show.pid, show.cpu, show.memory, show.annotate].into_iter();
        rows.map(|(id, title, desc, field)| {
            let on = on.next().unwrap_or_default();
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
        assert_eq!(cpu_text(3.24), "  3.2%");
        assert_eq!(cpu_text(12.0), " 12.0%");
        assert_eq!(cpu_text(100.0), "100.0%");
        assert_eq!(memory_text(512 << 20), "512.0 MB");
        assert_eq!(memory_text(1 << 30), "  1.0 GB");
        assert_eq!(memory_text(1288490188), "  1.2 GB");
        assert_eq!(memory_text(47 << 30), " 47.0 GB");
        let widths: Vec<usize> = [1u64 << 20, 999 << 20, 3 << 30, 200 << 30]
            .map(|b| memory_text(b).chars().count())
            .to_vec();
        assert!(widths.iter().all(|w| *w == 8), "{widths:?}");
    }

    #[test]
    fn a_hot_tab_shows_what_is_high_even_when_switched_off() {
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
        assert_eq!(
            usage.cells(off),
            [Cell {
                text: " 95.0%".into(),
                hot: true
            }]
        );
        let quiet = Show {
            annotate: false,
            ..off
        };
        assert!(usage.cells(quiet).is_empty());
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
                (" 95.0%".to_string(), true),
                ("  1.0 GB".to_string(), false)
            ]
        );
    }
}

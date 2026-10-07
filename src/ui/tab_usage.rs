//! The sidebar row's pid, CPU% and memory: every process in each of a tab's
//! pane trees, summed.
//!
//! One poll for the whole app, over the panes the sidebar last drew, so a
//! window full of tabs costs one round of `QueryProcs` per interval rather than
//! a poll per row.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use gpui::{App, Global};

use crate::daemon::procstat::compact_bytes;
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

impl Usage {
    pub(crate) fn label(&self) -> Option<String> {
        let parts: Vec<String> = [
            self.pid.map(|pid| format!("pid {pid}")),
            self.cpu.map(|pct| match pct < 10.0 {
                true => format!("{pct:.1}%"),
                false => format!("{pct:.0}%"),
            }),
            self.rss.map(compact_bytes),
        ]
        .into_iter()
        .flatten()
        .collect();
        (!parts.is_empty()).then(|| parts.join(" · "))
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
        let out = Usage { pid, cpu, rss };
        out.label().map(|_| out)
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_label_names_what_is_known_and_skips_the_rest() {
        let all = Usage {
            pid: Some(4821),
            cpu: Some(12.4),
            rss: Some(3 << 30),
        };
        assert_eq!(all.label().as_deref(), Some("pid 4821 · 12% · 3.0 GB"));
        let some = Usage {
            pid: None,
            cpu: Some(0.3),
            rss: None,
        };
        assert_eq!(some.label().as_deref(), Some("0.3%"));
        let none = Usage {
            pid: None,
            cpu: None,
            rss: None,
        };
        assert_eq!(none.label(), None);
    }
}

//! The fork's new tab page: New Tab asks what to open and where before it
//! opens anything — a terminal or an agent, in a directory picked from the
//! open tabs, the ones used most, and the children of `dir_roots`. Kept out of
//! `app.rs` so rebasing on upstream touches as little of it as possible;
//! `new_tab_page: false` gives upstream New Tab back unchanged.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use gpui::{
    AnyElement, App, Context, Entity, FontWeight, HighlightStyle, KeyDownEvent, MouseButton,
    MouseDownEvent, ScrollHandle, SharedString, StyledText, Subscription, Window, div, prelude::*,
    px, rems,
};
use gpui_component::input::{Input, InputEvent, InputState};
use gpui_component::{ActiveTheme as _, h_flex, v_flex};

use crate::core::actions::{NewTabPageNextKind, NewTabPagePrevKind};
use crate::core::cli_agent::CLIAgent;
use crate::core::config::{Config, ProfileUsage, unix_now};
use crate::ui::agent_launch::most_recent;
use crate::ui::app::Tty7App;
use crate::ui::home::display_path;
use crate::ui::host_ops::HostOps;
use crate::ui::i18n::{L10nKey, t};
use crate::ui::path_display::{abbreviate_home, local_home};
use crate::ui::search::fuzzy_score;
use crate::ui::switcher::CARD_TOP;
use tty7_core::host::Host;
use tty7_core::host::local::LocalHost;

const CARD_W: f32 = 560.0;
const CARD_RADIUS: f32 = 12.0;
const LIST_H: f32 = 336.0;
const ROW_H: f32 = 28.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    Terminal,
    Agent(CLIAgent),
}

impl Kind {
    fn label(self) -> SharedString {
        match self {
            Kind::Terminal => t(L10nKey::NewTabPageTerminal).into(),
            Kind::Agent(agent) => agent.display_name().into(),
        }
    }
}

pub(crate) struct NewTabPage {
    query: Entity<InputState>,
    kinds: Vec<Kind>,
    kind: usize,
    /// Every candidate with its canonical form, in [`candidates`] order;
    /// `rows` is what the query leaves of them.
    dirs: Vec<Dir>,
    /// The query itself, when it names a directory: resolved off the UI
    /// thread, and only for the query it was asked for.
    typed: Option<(String, Dir)>,
    rows: Vec<PathBuf>,
    selected: usize,
    home: Option<PathBuf>,
    scroll: ScrollHandle,
    _subs: Vec<Subscription>,
}

/// A directory as shown, and as resolved for deduplication.
pub(crate) type Dir = (PathBuf, PathBuf);

impl NewTabPage {
    fn refilter(&mut self, cx: &App) {
        let query = self.query.read(cx).value();
        let typed = self
            .typed
            .as_ref()
            .filter(|(asked, _)| asked.as_str() == query.trim())
            .map(|(_, dir)| dir);
        self.rows = filter(query.as_ref(), &self.dirs, self.home.as_deref(), typed);
        self.selected = 0;
    }
}

/// `~` and `~/rest` against `home`; anything else as written. `~user` is
/// not expanded: there is no portable way to ask for another user's home.
fn expand(path: &str, home: Option<&Path>) -> Option<PathBuf> {
    match path.strip_prefix('~') {
        None => Some(PathBuf::from(path)),
        Some("") => home.map(Path::to_path_buf),
        Some(rest) => Some(home?.join(rest.strip_prefix('/')?)),
    }
}

/// The directories the page offers, before any query: the active tab's, the
/// other tabs', the most used, then the children of each root. A path is
/// offered once, at its first place, however it is spelled — and only while
/// it is a directory, so a frecency entry for a deleted checkout drops out
/// by itself.
pub(crate) fn candidates(
    host: &dyn Host,
    active: Option<&Path>,
    tab_cwds: &[PathBuf],
    frecency: &HashMap<String, ProfileUsage>,
    roots: &[String],
    home: Option<&Path>,
    now: u64,
) -> Vec<Dir> {
    let mut used: Vec<(&String, f64)> = frecency.iter().map(|(p, u)| (p, u.score(now))).collect();
    // The key breaks ties so the order does not follow the map's hashing.
    used.sort_by(|a, b| b.1.total_cmp(&a.1).then_with(|| a.0.cmp(b.0)));
    let children = roots.iter().flat_map(|root| {
        let Some(root) = expand(root, home) else {
            return Vec::new();
        };
        let mut kids: Vec<PathBuf> = host
            .read_dir(&root, None)
            .unwrap_or_default()
            .into_iter()
            .filter(|e| !e.name.starts_with('.'))
            .map(|e| host.join(&root, &e.name))
            .collect();
        kids.sort();
        kids
    });
    let mut seen = HashSet::new();
    active
        .map(Path::to_path_buf)
        .into_iter()
        .chain(tab_cwds.iter().cloned())
        .chain(used.into_iter().map(|(p, _)| PathBuf::from(p)))
        .chain(children)
        .filter(|p| host.stat(p).is_ok_and(|m| m.is_dir))
        .filter_map(|p| {
            let real = host.canonicalize(&p).ok()?;
            seen.insert(real.clone()).then_some((p, real))
        })
        .collect()
}

/// The query as a directory, when it is one: `~/x/` and `~/x` rebuilt from
/// components so both land on one frecency key.
pub(crate) fn resolve_typed(host: &dyn Host, query: &str, home: Option<&Path>) -> Option<Dir> {
    let typed = expand(query.trim(), home)
        .filter(|p| host.is_absolute(p) && host.stat(p).is_ok_and(|m| m.is_dir))?
        .components()
        .collect::<PathBuf>();
    let real = host.canonicalize(&typed).ok()?;
    Some((typed, real))
}

/// What a row's text is matched against: the directory's own name for a
/// bare word — matched against the whole path, any long parent spells out
/// most queries by accident — and the `~` form once the query has a `/`.
fn match_text(query: &str, dir: &Path, home: Option<&Path>) -> Option<String> {
    match query.contains('/') {
        true => Some(abbreviate_home(&dir.to_string_lossy(), home).to_string()),
        false => Some(dir.file_name()?.to_string_lossy().into_owned()),
    }
}

/// Byte ranges of `text` to highlight for `query`: a contiguous match when
/// there is one, else the letters taken left to right, the way the scorer
/// accepts them. Case-insensitive; empty when nothing matches.
pub(crate) fn match_ranges(query: &str, text: &str) -> Vec<std::ops::Range<usize>> {
    let needle: Vec<char> = query
        .chars()
        .flat_map(char::to_lowercase)
        .filter(|c| !c.is_whitespace())
        .collect();
    if needle.is_empty() {
        return Vec::new();
    }
    let lower: Vec<(usize, char)> = text
        .char_indices()
        .map(|(i, c)| (i, c.to_lowercase().next().unwrap_or(c)))
        .collect();
    let end_of = |at: usize| lower.get(at + 1).map_or(text.len(), |(i, _)| *i);
    if let Some(start) = (0..lower.len()).find(|&s| {
        needle.len() <= lower.len() - s
            && needle.iter().enumerate().all(|(k, c)| lower[s + k].1 == *c)
    }) {
        return vec![lower[start].0..end_of(start + needle.len() - 1)];
    }
    let mut ranges: Vec<std::ops::Range<usize>> = Vec::new();
    let mut want = needle.iter().peekable();
    for (at, (i, c)) in lower.iter().enumerate() {
        if want.peek() == Some(&c) {
            want.next();
            match ranges.last_mut() {
                Some(last) if last.end == *i => last.end = end_of(at),
                _ => ranges.push(*i..end_of(at)),
            }
        }
    }
    match want.peek() {
        None => ranges,
        Some(_) => Vec::new(),
    }
}

/// `dirs` narrowed to the query and best match first, scored against the
/// `~` form the row shows so the home prefix never matches. A query that is
/// itself a directory leads, so a path nobody has visited is one Enter away.
pub(crate) fn filter(
    query: &str,
    dirs: &[Dir],
    home: Option<&Path>,
    typed: Option<&Dir>,
) -> Vec<PathBuf> {
    let query = query.trim();
    if query.is_empty() {
        return dirs.iter().map(|(d, _)| d.clone()).collect();
    }
    let mut scored: Vec<(i32, &Dir)> = dirs
        .iter()
        .filter_map(|dir| Some((fuzzy_score(query, &match_text(query, &dir.0, home)?)?, dir)))
        .collect();
    scored.sort_by_key(|(score, _)| std::cmp::Reverse(*score));
    typed
        .map(|(d, _)| d.clone())
        .into_iter()
        .chain(
            scored
                .into_iter()
                .filter(|(_, (_, real))| typed.is_none_or(|(_, t)| t != real))
                .map(|(_, (d, _))| d.clone()),
        )
        .collect()
}

/// Where `query` matched in a row's `shown` text: against the directory's
/// name at the end of it for a bare word, against the whole of it otherwise,
/// mirroring [`match_text`].
fn row_highlights(query: &str, dir: &Path, shown: &str) -> Vec<std::ops::Range<usize>> {
    if query.is_empty() || query.contains('/') {
        return match_ranges(query, shown);
    }
    let Some(name) = dir.file_name().map(|n| n.to_string_lossy().into_owned()) else {
        return Vec::new();
    };
    let Some(offset) = shown
        .len()
        .checked_sub(name.len())
        .filter(|_| shown.ends_with(&name))
    else {
        return Vec::new();
    };
    match_ranges(query, &name)
        .into_iter()
        .map(|r| r.start + offset..r.end + offset)
        .collect()
}

/// The kind the page starts on, as an index into Terminal-then-`offered`:
/// the agent launched last, or Terminal when none has been. `most_recent`
/// alone would fall back to the first agent on `PATH`.
fn initial_kind(offered: &[CLIAgent], usage: &HashMap<String, ProfileUsage>) -> usize {
    most_recent(offered, usage)
        .filter(|agent| usage.get(agent.slug()).is_some_and(|u| u.last_used > 0))
        .and_then(|agent| offered.iter().position(|&o| o == agent))
        .map_or(0, |at| at + 1)
}

/// Only [`Tty7App::commit_new_tab_page`] calls this: a cancelled page is
/// not a use.
fn bump_frecency(cfg: &mut Config, dir: &Path, now: u64) {
    let used = cfg
        .dir_frecency
        .entry(dir.to_string_lossy().into_owned())
        .or_default();
    used.count = used.count.saturating_add(1);
    used.last_used = now;
}

impl Tty7App {
    /// New Tab's first stop. False leaves New Tab to upstream: the page is
    /// off, or the workspace is remote, where neither the directories nor
    /// the agents on `PATH` are this machine's to list.
    pub(crate) fn open_new_tab_page(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        if !cx.global::<Config>().new_tab_page || !self.can_spawn_locally(cx) {
            return false;
        }
        if self.new_tab_page.is_some() {
            return true;
        }
        let cwds: Vec<Option<PathBuf>> = self
            .tabs
            .iter()
            .map(|tab| {
                tab.pane
                    .focused_or_first(window, cx)
                    .and_then(|leaf| leaf.read(cx).spawnable_cwd())
            })
            .collect();
        let active = cwds.get(self.active).cloned().flatten();
        let tab_cwds: Vec<PathBuf> = cwds.into_iter().flatten().collect();
        let offered = self.offered_agents(cx);
        let home = local_home();
        let cfg = cx.global::<Config>();
        let kind = initial_kind(&offered, &cfg.agent_frecency);
        // The open tabs are directories already, so they show at once; the
        // listing that has to touch the disk lands a moment later.
        let mut seen = HashSet::new();
        let dirs: Vec<Dir> = active
            .iter()
            .chain(&tab_cwds)
            .filter(|d| seen.insert((*d).clone()))
            .map(|d| (d.clone(), d.clone()))
            .collect();
        let (frecency, roots, now) = (cfg.dir_frecency.clone(), cfg.dir_roots.clone(), unix_now());
        let listing_home = home.clone();
        // The page only opens on a local workspace, so this machine is the host.
        HostOps::run(
            LocalHost::shared(),
            cx,
            move |host| {
                candidates(
                    host,
                    active.as_deref(),
                    &tab_cwds,
                    &frecency,
                    &roots,
                    listing_home.as_deref(),
                    now,
                )
            },
            |this: &mut Self, dirs, cx| {
                if let Some(page) = this.new_tab_page.as_mut() {
                    page.dirs = dirs;
                    page.refilter(cx);
                    cx.notify();
                }
            },
        );
        let kinds = std::iter::once(Kind::Terminal)
            .chain(offered.into_iter().map(Kind::Agent))
            .collect();
        let query =
            cx.new(|cx| InputState::new(window, cx).placeholder(t(L10nKey::NewTabPagePlaceholder)));
        query.update(cx, |state, cx| state.focus(window, cx));
        let subs = vec![cx.subscribe_in(
            &query,
            window,
            |this, _input, ev: &InputEvent, _window, cx| {
                if !matches!(ev, InputEvent::Change) {
                    return;
                }
                let Some(page) = this.new_tab_page.as_mut() else {
                    return;
                };
                page.refilter(cx);
                let asked = page.query.read(cx).value().trim().to_string();
                let home = page.home.clone();
                HostOps::run(
                    LocalHost::shared(),
                    cx,
                    move |host| (resolve_typed(host, &asked, home.as_deref()), asked),
                    |this: &mut Self, (typed, asked), cx| {
                        if let Some(page) = this.new_tab_page.as_mut() {
                            page.typed = typed.map(|dir| (asked, dir));
                            page.refilter(cx);
                            cx.notify();
                        }
                    },
                );
                cx.notify();
            },
        )];
        self.new_tab_page = Some(NewTabPage {
            query,
            kinds,
            kind,
            rows: dirs.iter().map(|(d, _)| d.clone()).collect(),
            dirs,
            typed: None,
            selected: 0,
            home,
            scroll: ScrollHandle::new(),
            _subs: subs,
        });
        cx.notify();
        true
    }

    fn close_new_tab_page(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.new_tab_page.take().is_some() {
            self.focus_active(window, cx);
            cx.notify();
        }
    }

    fn commit_new_tab_page(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(page) = self.new_tab_page.as_ref() else {
            return;
        };
        // No row: nothing to open, and the page stays for another query.
        let Some(dir) = page.rows.get(page.selected).cloned() else {
            return;
        };
        let kind = page.kinds[page.kind];
        self.close_new_tab_page(window, cx);
        match kind {
            Kind::Terminal => self.new_tab_at(dir.clone(), window, cx),
            Kind::Agent(agent) => self.launch_agent_in(agent, Some(dir.clone()), window, cx),
        }
        self.update_config(cx, |cfg| bump_frecency(cfg, &dir, unix_now()));
    }

    fn new_tab_page_query_is_empty(&self, cx: &App) -> bool {
        self.new_tab_page
            .as_ref()
            .is_none_or(|page| page.query.read(cx).value().is_empty())
    }

    /// Tab arrives as an action: a key listener never sees it (see keymap.rs).
    fn step_new_tab_page_kind(&mut self, forward: bool, cx: &mut Context<Self>) {
        let Some(page) = self.new_tab_page.as_mut() else {
            return;
        };
        let kinds = page.kinds.len();
        page.kind = match forward {
            true => (page.kind + 1) % kinds,
            false => (page.kind + kinds - 1) % kinds,
        };
        cx.notify();
    }

    /// The page is modal: every key is taken here, in the capture phase and so
    /// ahead of the app's own bindings, except plain typing and the editing
    /// chords the query box needs.
    fn on_new_tab_page_key(
        &mut self,
        ev: &KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let (key, mods) = (ev.keystroke.key.as_str(), ev.keystroke.modifiers);
        let chord = mods.platform || mods.control || mods.alt;
        match key {
            "escape" => self.close_new_tab_page(window, cx),
            "enter" => self.commit_new_tab_page(window, cx),
            // With a query typed, the arrows are the caret's.
            "left" | "right" if !chord && self.new_tab_page_query_is_empty(cx) => {
                self.step_new_tab_page_kind(key == "right", cx)
            }
            "[" | "{" | "]" | "}" if mods.platform && mods.shift => {
                self.step_new_tab_page_kind(matches!(key, "]" | "}"), cx)
            }
            "up" | "down" => {
                let Some(page) = self.new_tab_page.as_mut() else {
                    return;
                };
                page.selected = match key {
                    "up" => page.selected.saturating_sub(1),
                    _ => (page.selected + 1).min(page.rows.len().saturating_sub(1)),
                };
                page.scroll.scroll_to_item(page.selected);
            }
            _ => match key.parse::<usize>() {
                Ok(n @ 1..=9) if mods.control => {
                    if let Some(page) = self.new_tab_page.as_mut()
                        && n <= page.kinds.len()
                    {
                        page.kind = n - 1;
                    }
                }
                // The box's own editing chords; any other chord is swallowed.
                _ if mods.platform && matches!(key, "a" | "c" | "v" | "x" | "z") => return,
                _ if chord => {}
                _ => return,
            },
        }
        cx.stop_propagation();
        cx.notify();
    }

    pub(crate) fn render_new_tab_page(
        &self,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let page = self.new_tab_page.as_ref()?;
        let theme = cx.theme();
        let (fg, muted, border) = (theme.foreground, theme.muted_foreground, theme.border);
        let (primary, primary_fg) = (theme.primary, theme.primary_foreground);
        let hit = HighlightStyle {
            color: Some(theme.primary),
            font_weight: Some(FontWeight::BOLD),
            ..Default::default()
        };
        let query = page.query.read(cx).value().trim().to_string();
        let (hover, picked) = {
            let sf = &cx.global::<crate::ui::presets::Surfaces>().popover;
            (gpui::rgb(sf.hover), gpui::rgb(sf.selected))
        };
        let scrim = crate::ui::presets::scrim_fill(cx);

        let chips =
            h_flex()
                .flex_1()
                .flex_wrap()
                .gap(px(6.))
                .children(page.kinds.iter().enumerate().map(|(at, &kind)| {
                    let on = at == page.kind;
                    div()
                        .id(("new-tab-kind", at))
                        .h(px(24.))
                        .px(px(10.))
                        .flex()
                        .items_center()
                        .rounded(px(6.))
                        .text_size(rems(12. / 16.))
                        .cursor_pointer()
                        .when(on, |chip| chip.bg(primary).text_color(primary_fg))
                        .when(!on, |chip| {
                            chip.border_1()
                                .border_color(border)
                                .text_color(fg)
                                .hover(|chip| chip.bg(hover))
                        })
                        .when(at < 9, |chip| {
                            chip.child(div().mr(px(5.)).opacity(0.6).child(format!("^{}", at + 1)))
                        })
                        .child(kind.label())
                        .on_click(cx.listener(move |this, _, _window, cx| {
                            if let Some(page) = this.new_tab_page.as_mut() {
                                page.kind = at;
                            }
                            cx.notify();
                        }))
                }));

        let rows = page.rows.iter().enumerate().map(|(at, dir)| {
            h_flex()
                .id(("new-tab-dir", at))
                .flex_shrink_0()
                .h(px(ROW_H))
                .px(px(10.))
                .items_center()
                .rounded(crate::ui::rounding::ROW_RADIUS)
                .cursor_pointer()
                .text_color(fg)
                .when(at == page.selected, |row| row.bg(picked))
                .when(at != page.selected, |row| row.hover(|row| row.bg(hover)))
                .child(div().min_w_0().truncate().child({
                    let shown = display_path(dir, page.home.as_deref()).to_string();
                    let ranges = row_highlights(&query, dir, &shown);
                    StyledText::new(shown).with_highlights(ranges.into_iter().map(|r| (r, hit)))
                }))
                .on_click(cx.listener(move |this, _, window, cx| {
                    if let Some(page) = this.new_tab_page.as_mut() {
                        page.selected = at;
                    }
                    this.commit_new_tab_page(window, cx);
                }))
        });

        let viewport = window.viewport_size();
        let card_w = CARD_W.min(viewport.width.as_f32() - 48.).max(320.);
        let list_h = LIST_H
            .min(viewport.height.as_f32() - CARD_TOP - 160.)
            .max(3. * ROW_H);
        let list = v_flex()
            .id("new-tab-dirs")
            .track_scroll(&page.scroll)
            .max_h(px(list_h))
            .overflow_y_scroll()
            .p(px(6.))
            .gap(px(1.))
            .children(rows)
            .when(page.rows.is_empty(), |list| {
                list.child(
                    div()
                        .px(px(10.))
                        .py(px(10.))
                        .text_color(muted)
                        .child(t(L10nKey::SwitcherNoMatch)),
                )
            });

        let card = v_flex()
            .w(px(card_w))
            .map(|card| crate::ui::theme::floating_surface(card, cx))
            .rounded(px(CARD_RADIUS))
            .overflow_hidden()
            .child(
                h_flex()
                    .px(px(14.))
                    .pt(px(12.))
                    .pb(px(10.))
                    .gap(px(12.))
                    .items_center()
                    .child(
                        div()
                            .text_size(rems(13. / 16.))
                            .text_color(muted)
                            .child(t(L10nKey::NewTabPageTitle)),
                    )
                    .child(chips),
            )
            .child(
                div()
                    .h(px(40.))
                    .px(px(14.))
                    .flex()
                    .items_center()
                    .border_t_1()
                    .border_b_1()
                    .border_color(border)
                    .child(Input::new(&page.query).appearance(false).pl_0()),
            )
            .child(list)
            .child(
                div()
                    .px(px(14.))
                    .py(px(8.))
                    .border_t_1()
                    .border_color(border)
                    .text_size(rems(11. / 16.))
                    .text_color(muted)
                    .child(t(L10nKey::NewTabPageHint)),
            );

        Some(
            div()
                .absolute()
                .inset_0()
                .flex()
                .items_start()
                .justify_center()
                .pt(px(CARD_TOP))
                .bg(scrim)
                .key_context("NewTabPage")
                .on_action(cx.listener(|this, _: &NewTabPageNextKind, _, cx| {
                    this.step_new_tab_page_kind(true, cx)
                }))
                .on_action(cx.listener(|this, _: &NewTabPagePrevKind, _, cx| {
                    this.step_new_tab_page_kind(false, cx)
                }))
                .capture_key_down(cx.listener(Self::on_new_tab_page_key))
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(|this, _: &MouseDownEvent, window, cx| {
                        // Spent on the dismissal, as the switcher's scrim is:
                        // the click must not also land on whatever is beneath.
                        cx.stop_propagation();
                        this.close_new_tab_page(window, cx);
                    }),
                )
                .child(div().id("new-tab-card").occlude().child(card))
                .into_any_element(),
        )
    }
}

#[cfg(test)]
mod tests {
    use gpui::{TestAppContext, VisualTestContext};

    use super::*;
    use crate::core::session::Session;
    use crate::ui::windows::WindowRegistry;

    const NOW: u64 = 1_800_000_000;
    const DAY: u64 = 86_400;

    fn tree() -> (tempfile::TempDir, PathBuf) {
        let tmp = tempfile::TempDir::new().unwrap();
        let home = tmp.path().canonicalize().unwrap();
        for dir in [
            "src/alpha",
            "src/beta",
            "src/.hidden",
            "work",
            "hot",
            "warm",
        ] {
            std::fs::create_dir_all(home.join(dir)).unwrap();
        }
        std::fs::write(home.join("src/notes.txt"), "").unwrap();
        (tmp, home)
    }

    fn host() -> tty7_core::host::SharedHost {
        LocalHost::new()
    }

    fn shown(dirs: Vec<Dir>) -> Vec<PathBuf> {
        dirs.into_iter().map(|(d, _)| d).collect()
    }

    /// Resolved the way [`candidates`] resolves them, so a typed path meets
    /// the same canonical form (`\\?\C:\…` on Windows).
    fn as_dirs(paths: &[PathBuf]) -> Vec<Dir> {
        let host = host();
        paths
            .iter()
            .map(|p| {
                (
                    p.clone(),
                    host.canonicalize(p).unwrap_or_else(|_| p.clone()),
                )
            })
            .collect()
    }

    /// [`filter`] the way the page runs it: the typed row resolved first.
    fn run(query: &str, paths: &[PathBuf], home: Option<&Path>) -> Vec<PathBuf> {
        let typed = resolve_typed(&*host(), query, home);
        filter(query, &as_dirs(paths), home, typed.as_ref())
    }

    fn used(pairs: &[(&Path, u32, u64)]) -> HashMap<String, ProfileUsage> {
        pairs
            .iter()
            .map(|(p, count, last_used)| {
                (
                    p.to_string_lossy().into_owned(),
                    ProfileUsage {
                        count: *count,
                        last_used: *last_used,
                    },
                )
            })
            .collect()
    }

    #[test]
    fn candidates_come_active_tab_first_then_tabs_then_frecency_then_root_children() {
        let (_tmp, home) = tree();
        let frecency = used(&[
            (&home.join("warm"), 1, NOW - DAY),
            (&home.join("hot"), 9, NOW),
            (&home.join("gone"), 50, NOW),
        ]);
        let got = shown(candidates(
            &*host(),
            Some(&home.join("work")),
            &[home.join("src/beta"), home.join("work")],
            &frecency,
            &["~/src".into(), "~/missing".into(), "~/src/notes.txt".into()],
            Some(&home),
            NOW,
        ));
        assert_eq!(
            got,
            vec![
                home.join("work"),
                home.join("src/beta"),
                home.join("hot"),
                home.join("warm"),
                home.join("src/alpha"),
            ],
            "duplicates collapse, and a gone frecency entry, a missing root, a \
             file and a dot-directory are all skipped"
        );
    }

    #[test]
    fn the_same_directory_spelled_twice_is_offered_once() {
        let (_tmp, home) = tree();
        let got = shown(candidates(
            &*host(),
            Some(&home.join("src/alpha")),
            &[home.join("src/../src/alpha")],
            &HashMap::new(),
            &[home.join("src").to_string_lossy().into_owned()],
            None,
            NOW,
        ));
        assert_eq!(got, vec![home.join("src/alpha"), home.join("src/beta")]);
    }

    #[test]
    fn an_empty_query_keeps_the_candidate_order() {
        let dirs = vec![PathBuf::from("/b"), PathBuf::from("/a")];
        assert_eq!(filter("  ", &as_dirs(&dirs), None, None), dirs);
    }

    #[test]
    fn a_fuzzy_query_ranks_the_closer_match_first_and_drops_misses() {
        let home = Path::new("/Users/me");
        let dirs = vec![
            home.join("src/tty7-resume"),
            home.join("src/tty7"),
            home.join("code/web"),
        ];
        assert_eq!(
            run("tty7", &dirs, Some(home)),
            vec![home.join("src/tty7"), home.join("src/tty7-resume")]
        );
        // Scored against the `~` form: the home prefix itself matches nothing.
        assert_eq!(run("Users", &dirs, Some(home)), Vec::<PathBuf>::new());
        // A long parent must not spell the query: "beta" is in "f14681bd…t…a".
        let deep = vec![
            PathBuf::from("/tmp/f14681bd-scratch/src/alpha"),
            PathBuf::from("/tmp/f14681bd-scratch/src/beta-repo"),
        ];
        assert_eq!(run("beta", &deep, None), vec![deep[1].clone()]);
        assert_eq!(run("src/al", &deep, None), vec![deep[0].clone()]);
    }

    #[test]
    fn a_typed_existing_directory_is_offered_first() {
        let (_tmp, home) = tree();
        let dirs = vec![home.join("src/alpha"), home.join("work")];
        assert_eq!(
            run("~/src", &dirs, Some(&home)),
            vec![home.join("src"), home.join("src/alpha")]
        );
        // Already a candidate: moved to the front, not listed twice.
        assert_eq!(run("~/work", &dirs, Some(&home)), vec![home.join("work")]);
        // Not a directory: only fuzzy matches.
        assert_eq!(
            run("~/src/notes.txt", &dirs, Some(&home)),
            Vec::<PathBuf>::new()
        );
    }

    #[test]
    fn the_page_starts_on_the_agent_launched_last_or_on_terminal_when_none_was() {
        let offered = [CLIAgent::ALL[0], CLIAgent::ALL[1]];
        assert_eq!(initial_kind(&offered, &HashMap::new()), 0);
        let usage = HashMap::from([(
            offered[1].slug().to_string(),
            ProfileUsage {
                count: 1,
                last_used: NOW,
            },
        )]);
        assert_eq!(initial_kind(&offered, &usage), 2);
    }

    #[test]
    fn a_typed_directory_with_a_trailing_slash_is_offered_without_it() {
        let (_tmp, home) = tree();
        // PathBuf equality ignores a trailing slash; the stored key does not.
        assert_eq!(
            run("~/src/", &[], Some(&home))[0].to_string_lossy(),
            home.join("src").to_string_lossy()
        );
    }

    #[test]
    fn a_match_highlights_the_run_it_found_or_the_letters_in_order() {
        assert_eq!(match_ranges("stu", "claude-stuff"), vec![7..10]);
        assert_eq!(match_ranges("STU", "claude-stuff"), vec![7..10], "any case");
        assert_eq!(
            match_ranges("csl", "cc-statusline"),
            vec![0..1, 3..4, 9..10],
            "no run: the letters left to right"
        );
        assert!(match_ranges("zz", "cc-statusline").is_empty());
        assert!(match_ranges("", "x").is_empty());
    }

    #[test]
    fn a_bare_word_highlights_in_the_directory_name_of_the_shown_row() {
        let dir = Path::new("/Users/me/src/claude-stuff");
        let shown = "~/src/claude-stuff";
        assert_eq!(row_highlights("stu", dir, shown), vec![13..16]);
        // A `/` query matches the whole shown path, as the filter does.
        assert_eq!(row_highlights("src/cl", dir, shown), vec![2..8]);
    }

    #[test]
    fn opening_a_directory_counts_a_use_and_stamps_it() {
        let mut cfg = Config::default();
        let dir = Path::new("/Users/me/src/x");
        bump_frecency(&mut cfg, dir, NOW - DAY);
        bump_frecency(&mut cfg, dir, NOW);
        let u = &cfg.dir_frecency["/Users/me/src/x"];
        assert_eq!((u.count, u.last_used), (2, NOW));
    }

    #[gpui::test]
    fn new_tab_is_upstream_when_the_page_is_off_and_esc_opens_nothing(cx: &mut TestAppContext) {
        crate::core::config::pin_test_config_dir();
        cx.executor().allow_parking();
        cx.update(|cx| {
            gpui_component::init(cx);
            cx.set_global(Config::default());
            crate::ui::keymap::init(cx);
            WindowRegistry::init(cx);
        });
        let window = cx.add_window(|window, cx| {
            let app =
                cx.new(|cx| Tty7App::with_session(None, Some(Session::default()), window, cx));
            gpui_component::Root::new(app, window, cx)
        });
        let app = window
            .update(cx, |root, _, _| {
                root.view()
                    .clone()
                    .downcast::<Tty7App>()
                    .ok()
                    .expect("window root wraps a Tty7App")
            })
            .unwrap();
        let mut vcx = VisualTestContext::from_window(window.into(), cx);
        vcx.run_until_parked();

        vcx.update(|_, cx| cx.global_mut::<Config>().new_tab_page = false);
        let opened = app.update_in(&mut vcx, |app, window, cx| {
            app.open_new_tab_page(window, cx)
        });
        assert!(
            !opened,
            "with the page off, New Tab falls through to upstream"
        );

        vcx.update(|_, cx| cx.global_mut::<Config>().new_tab_page = true);
        let tabs = app.update(&mut vcx, |app, _| app.tabs.len());
        let opened = app.update_in(&mut vcx, |app, window, cx| {
            app.open_new_tab_page(window, cx)
        });
        assert!(opened);
        vcx.run_until_parked();
        assert!(app.update(&mut vcx, |app, _| app.new_tab_page.is_some()));

        vcx.simulate_keystrokes("escape");
        vcx.run_until_parked();
        app.update(&mut vcx, |app, cx| {
            assert!(app.new_tab_page.is_none(), "Esc closes the page");
            assert_eq!(app.tabs.len(), tabs, "Esc opens nothing");
            assert!(
                cx.global::<Config>().dir_frecency.is_empty(),
                "Esc counts no use"
            );
        });
    }
}

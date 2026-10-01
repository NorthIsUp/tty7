//! Background tabs: ⇧Enter or ⇧-click on the new tab page, or on a palette row
//! that opens a tab, opens it behind the current one, the way ⌘-click does in
//! a browser. The tab is created and started (a typed agent command still
//! waits for the shell's first prompt on the pane's own poll, focused or not);
//! only activation is skipped. Kept out of `app.rs`: upstream's
//! `Tty7App::new_tab_slot` hands its tab to [`Tty7App::seat_new_tab`].

use std::rc::Rc;

use gpui::{App, Context, KeyBinding, KeyBindingContextPredicate, Keystroke, Modifiers, Window};

use crate::ui::app::{Tab, Tty7App};
use crate::ui::search::{CommandKind, KEY_CONTEXT};

/// The modifier that sends a new tab to the background. ⌥ is taken (split).
pub(crate) fn wanted_by(mods: Modifiers) -> bool {
    mods.shift
}

/// [`wanted_by`] off the live keyboard: the palette confirms rows through an
/// event that carries no modifiers, as the New Tab menu does.
pub(crate) fn wanted(window: &Window) -> bool {
    wanted_by(window.modifiers())
}

/// The palette rows whose Enter opens a tab, and so take ⇧.
pub(crate) fn opens_tab(kind: &CommandKind) -> bool {
    matches!(
        kind,
        CommandKind::ResumeSession { .. }
            | CommandKind::ForkSession { .. }
            | CommandKind::LaunchAgent(_)
    )
}

/// ⇧Enter confirms a palette row as Enter does; [`wanted`] reads the ⇧ back
/// when the row runs. gpui-component binds the list's bare Enter to an action
/// it keeps private, so this borrows that binding's. Bound before the keymap
/// takes its base snapshot, so a rebuild keeps it.
pub(crate) fn bind_shift_enter(cx: &mut App) {
    let enter = Keystroke::parse("enter").expect("a valid keystroke");
    let confirm = cx
        .key_bindings()
        .borrow()
        .bindings()
        .find(|b| {
            b.predicate().is_some_and(|p| p.to_string() == "List")
                && b.match_keystrokes(std::slice::from_ref(&enter)) == Some(false)
        })
        .map(|b| b.action().boxed_clone());
    let Some(confirm) = confirm else {
        log::warn!("no List binding for enter; the palette's ⇧Enter stays unbound");
        return;
    };
    let context = KeyBindingContextPredicate::parse(&format!("{KEY_CONTEXT} > List")).ok();
    match KeyBinding::load(
        "shift-enter",
        confirm,
        context.map(Rc::new),
        false,
        None,
        cx.keyboard_mapper().as_ref(),
    ) {
        Ok(binding) => cx.bind_keys([binding]),
        Err(e) => log::warn!("palette ⇧Enter: {e}"),
    }
}

/// The active index once a tab is inserted at `insert_at`: the new tab in
/// the foreground, else the tab that was active, shifted past the insert.
pub(crate) fn active_after_insert(active: usize, insert_at: usize, background: bool) -> usize {
    match background {
        false => insert_at,
        true if insert_at <= active => active + 1,
        true => active,
    }
}

impl Tty7App {
    /// Run `open` with every tab it creates through `new_tab_slot` left
    /// unfocused when `background` is set.
    pub(crate) fn in_background<R>(
        &mut self,
        background: bool,
        open: impl FnOnce(&mut Self) -> R,
    ) -> R {
        self.open_in_background = background;
        let out = open(self);
        self.open_in_background = false;
        out
    }

    /// [`Self::in_background`] when the modifiers held ask for it
    /// ([`wanted`]).
    pub(crate) fn maybe_background<R>(
        &mut self,
        window: &mut Window,
        open: impl FnOnce(&mut Self, &mut Window) -> R,
    ) -> R {
        let background = wanted(window);
        self.in_background(background, |this| open(this, window))
    }

    /// Insert a just-opened tab where `new_tab_position` says, and make it the
    /// active one unless [`Self::in_background`] asked otherwise. With no tab
    /// open there is nothing to stay on, so it is activated anyway.
    pub(crate) fn seat_new_tab(&mut self, tab: Tab, window: &mut Window, cx: &mut Context<Self>) {
        let background = self.open_in_background && !self.tabs.is_empty();
        if !background {
            self.remember_active_pane(window, cx);
            self.maximized = None;
        }
        let insert_at = self.new_tab_insert_at(cx);
        self.tabs.insert(insert_at, tab);
        self.active = active_after_insert(self.active, insert_at, background);
        if !background {
            self.focus_active(window, cx);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::terminal::view::quiet_test_pane;
    use crate::ui::app::test_window;
    use crate::ui::pane::{Pane, PaneSlot};

    #[test]
    fn a_background_insert_keeps_the_active_tab_where_it_is() {
        assert_eq!(active_after_insert(2, 3, false), 3);
        assert_eq!(active_after_insert(2, 3, true), 2, "after current");
        assert_eq!(active_after_insert(2, 5, true), 2, "at the end");
        assert_eq!(active_after_insert(2, 0, true), 3, "before it: shifted");
    }

    #[test]
    fn shift_asks_for_the_background() {
        assert!(wanted_by(Modifiers::shift()));
        assert!(!wanted_by(Modifiers::none()));
        assert!(!wanted_by(Modifiers::alt()));
    }

    #[gpui::test]
    fn a_background_open_adds_a_tab_and_leaves_active_alone(cx: &mut gpui::TestAppContext) {
        let (app, mut vcx, _streams) = test_window::harness_with_tabs(cx, 2);
        app.update_in(&mut vcx, |app, window, cx| app.activate(1, window, cx));
        let seat = |app: &gpui::Entity<Tty7App>, vcx: &mut gpui::VisualTestContext, bg: bool| {
            app.update_in(vcx, |app, window, cx| {
                let (view, _stream) = quiet_test_pane(9, window, cx);
                let tab = Tab::new(Pane::leaf(PaneSlot::Ready(view)));
                app.in_background(bg, |app| app.seat_new_tab(tab, window, cx));
            });
            vcx.run_until_parked();
            app.update(vcx, |app, _| (app.tabs.len(), app.active))
        };

        assert_eq!(seat(&app, &mut vcx, true), (3, 1), "added, still on tab 1");
        assert!(!app.update(&mut vcx, |app, _| app.open_in_background));
        assert_eq!(seat(&app, &mut vcx, false), (4, 2), "foreground: activated");
    }

    /// ⇧Enter confirms the row too: the list binds only a bare Enter.
    #[gpui::test]
    fn shift_return_confirms_the_picked_row(cx: &mut gpui::TestAppContext) {
        let (app, mut vcx, _streams) = test_window::harness_with_tabs(cx, 2);
        app.update_in(&mut vcx, |app, _, _| app.tabs[1].last_used.set(5));
        app.update_in(&mut vcx, |app, window, cx| {
            app.open_search(crate::ui::search::SearchTab::All, "", window, cx)
        });
        vcx.run_until_parked();
        vcx.simulate_keystrokes("shift-enter");
        vcx.run_until_parked();
        app.read_with(&vcx, |app, _| {
            assert!(app.search.is_none(), "⇧Enter picks the row");
            assert_eq!(app.active, 1);
        });
    }
}

//! The right panel's GitHub tab.

use gpui::{AnyElement, Context, Window};

use crate::ui::app::Tty7App;
use crate::ui::i18n::{L10nKey, t};

impl Tty7App {
    pub(crate) fn render_panel_github(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let title = self.panel_title(t(L10nKey::PanelGitHubTitle), None, None, window, cx);
        let body = self.panel_empty(t(L10nKey::PanelGitHubTitle), None, cx);
        self.panel_scroll(body, title)
    }
}

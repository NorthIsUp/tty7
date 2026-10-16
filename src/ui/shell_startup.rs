//! The Shell group's Shell startup row: whether a pane's zsh, bash or fish
//! reads the user's rc files.

use gpui::{AnyElement, Context, IntoElement as _};
use tty7_core::core::fork_config::ShellStartup;

use crate::core::config::Config;
use crate::ui::app::Tty7App;
use crate::ui::i18n::{L10nKey, t};

impl Tty7App {
    pub(crate) fn shell_startup_row(&self, cx: &mut Context<Self>) -> AnyElement {
        let modes = [
            ShellStartup::User,
            ShellStartup::NoRcs,
            ShellStartup::NoRcsEnvlibs,
        ];
        let current = cx.global::<Config>().fork.shell_startup;
        let choice = self.settings_choice(
            "shell-startup",
            &[
                t(L10nKey::SettingsShellStartupUser),
                t(L10nKey::SettingsShellStartupNoRcs),
                t(L10nKey::SettingsShellStartupEnvlibs),
            ],
            modes.iter().position(|m| *m == current).unwrap_or(0),
            cx,
            move |this, ix, _, cx| {
                let mode = modes.get(ix).copied().unwrap_or_default();
                this.update_config(cx, |c| c.fork.shell_startup = mode)
            },
        );
        self.settings_row(
            t(L10nKey::SettingsShellStartup),
            t(L10nKey::SettingsShellStartupDesc),
            choice,
            cx,
        )
        .into_any_element()
    }
}

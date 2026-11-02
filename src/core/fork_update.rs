//! Where the NorthIsUp fork's updates come from (FORK.md).
//!
//! CI builds (tty7-niu) check the fork's GitHub releases. A local install
//! (tty7-niu-dev, `mise run install-app`) never checks GitHub: its channel is
//! the bundle in ~/Applications, and a new build landing there raises the
//! update prompt.

use gpui::App;

/// The GitHub repo every update check and release link reads. A macro, not a
/// `const`, so `update.rs` can `concat!` it into its URL constants.
macro_rules! update_repo {
    () => {
        "NorthIsUp/tty7"
    };
}
pub(crate) use update_repo;

/// Whether this process runs from a local install, which checks no feed.
pub fn is_local_install() -> bool {
    local::running().is_some()
}

/// Prompts to restart once the bundle this process runs from is replaced by
/// a newer local build. A no-op anywhere else.
pub fn watch(cx: &mut App) {
    local::watch(cx);
}

#[cfg(target_os = "macos")]
mod local {
    use gpui::{App, PromptButton, PromptLevel};
    use std::os::unix::process::CommandExt as _;
    use std::path::{Path, PathBuf};
    use std::process::{Command, Stdio};
    use std::sync::OnceLock;
    use std::time::Duration;

    use crate::ui::i18n::{L10nKey, t, t_fmt};

    /// Written by `bundle-macos.sh` when `TTY7_LOCAL_BUILD_ID` is set.
    const BUILD_ID: &str = "Contents/Resources/local-build-id";
    const POLL: Duration = Duration::from_secs(5);

    /// The bundle and the build id it held when this process started.
    /// `install-app` swaps the bundle by rename, so the path keeps naming
    /// whatever build is installed now.
    pub(super) fn running() -> Option<&'static (PathBuf, String)> {
        static RUNNING: OnceLock<Option<(PathBuf, String)>> = OnceLock::new();
        RUNNING
            .get_or_init(|| {
                let exe = std::env::current_exe().ok()?;
                let app = exe
                    .ancestors()
                    .find(|path| path.extension().is_some_and(|ext| ext == "app"))?
                    .to_path_buf();
                let id = read_id(&app)?;
                Some((app, id))
            })
            .as_ref()
    }

    fn read_id(app: &Path) -> Option<String> {
        let id = std::fs::read_to_string(app.join(BUILD_ID)).ok()?;
        Some(id.trim().to_string()).filter(|id| !id.is_empty())
    }

    pub(super) fn watch(cx: &mut App) {
        let Some((app, current)) = running().cloned() else {
            return;
        };
        cx.spawn(async move |cx| {
            // "Later" answers for that build only; the next install asks again.
            let mut declined: Option<String> = None;
            loop {
                cx.background_executor().timer(POLL).await;
                let Some(installed) = read_id(&app) else {
                    continue;
                };
                if installed == current || declined.as_ref() == Some(&installed) {
                    continue;
                }
                let Some(window) = cx.update(|cx| cx.windows().first().copied()) else {
                    continue;
                };
                let detail = t_fmt(
                    L10nKey::UpdateDialogDetail,
                    &[
                        ("version", installed.as_str()),
                        ("current", current.as_str()),
                    ],
                );
                let answer = cx.update(|cx| {
                    window.update(cx, |_root, window, cx| {
                        window.prompt(
                            PromptLevel::Info,
                            t(L10nKey::UpdateDialogTitle),
                            Some(&detail),
                            &[
                                PromptButton::ok(t(L10nKey::SettingsUpdateAndRelaunch)),
                                PromptButton::cancel(t(L10nKey::UpdateDialogLater)),
                            ],
                            cx,
                        )
                    })
                });
                let Ok(answer) = answer else {
                    continue;
                };
                if let Ok(0) = answer.await {
                    relaunch_after_exit(&app);
                    cx.update(|cx| cx.quit());
                    return;
                }
                declined = Some(installed);
            }
        })
        .detach();
    }

    /// `open` once this process is gone: LaunchServices starts the new build
    /// with a Dock launch's environment, not this process's.
    fn relaunch_after_exit(app: &Path) {
        let spawned = Command::new("/bin/sh")
            .args([
                "-c",
                r#"while kill -0 "$0" 2>/dev/null; do sleep 0.2; done; exec /usr/bin/open "$1""#,
            ])
            .arg(std::process::id().to_string())
            .arg(app)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .process_group(0)
            .spawn();
        if let Err(error) = spawned {
            log::warn!(
                "could not schedule the relaunch of {}: {error}",
                app.display()
            );
        }
    }
}

#[cfg(not(target_os = "macos"))]
mod local {
    use gpui::App;
    use std::path::PathBuf;

    pub(super) fn running() -> Option<&'static (PathBuf, String)> {
        None
    }

    pub(super) fn watch(_cx: &mut App) {}
}

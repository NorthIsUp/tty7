//! A command a new pane runs once, at its first prompt — a dead pane's agent,
//! resumed — without it ever becoming the pane's shell: the spawn records the
//! plain shell, so a split or the next restore gets a shell, not the command
//! again. A shell with tty7's integration (zsh, bash, fish) is handed it in
//! `TTY7_RUN_ONCE` and runs it itself; one without runs it from `-lic` and
//! then `exec`s itself.

use portable_pty::CommandBuilder;

use crate::core::shell_quote::{quote_for_shell, runs_once};
use crate::daemon::protocol::ShellSpec;
use crate::daemon::shell_integration::{ShellKind, integrated_kind};

/// Arrange for `cmd`, the pane's shell, to run `line` once. `shell` is the
/// spawn's resolved shell, `None` for the login shell.
pub(crate) fn apply(cmd: &mut CommandBuilder, line: &str, shell: Option<&ShellSpec>) {
    let program = shell
        .map(|s| s.program.clone())
        .or_else(|| {
            let argv0 = cmd.get_argv().first()?;
            Some(argv0.to_string_lossy().trim_start_matches('-').to_string())
        })
        .unwrap_or_else(crate::core::shells::login_shell);
    let custom = shell.is_some_and(|s| !s.args.is_empty() && !s.args_are_tty7_defaults);
    match integrated_kind(Some(&program), custom) {
        Some(ShellKind::Zsh | ShellKind::Bash | ShellKind::Fish) => {
            cmd.env("TTY7_RUN_ONCE", line);
        }
        _ if runs_once(&program) => {
            let args = shell.map_or(&[][..], |s| s.args.as_slice());
            let then = match custom {
                true => args
                    .iter()
                    .map(|a| quote_for_shell(a, Some(&program)))
                    .collect(),
                false => vec!["-l".to_string()],
            };
            let script = format!(
                "{line}; exec {} {}",
                quote_for_shell(&program, Some(&program)),
                then.join(" ")
            );
            *cmd.get_argv_mut() = vec![program.into(), "-lic".into(), script.into()];
        }
        _ => log::warn!("{program} cannot be handed a command to run once; it starts plain"),
    }
}

#[cfg(test)]
mod tests {
    use super::apply;
    use crate::daemon::protocol::ShellSpec;
    use portable_pty::CommandBuilder;

    fn spec(program: &str, args: &[&str]) -> ShellSpec {
        ShellSpec {
            program: program.into(),
            args: args.iter().map(|a| a.to_string()).collect(),
            args_are_tty7_defaults: false,
        }
    }

    #[test]
    fn an_integrated_shell_is_told_and_keeps_its_argv() {
        let zsh = spec("/bin/zsh", &[]);
        let mut cmd = CommandBuilder::new("/bin/zsh");
        apply(&mut cmd, "claude --resume x", Some(&zsh));
        assert_eq!(cmd.get_argv(), &["/bin/zsh"]);
        assert_eq!(
            cmd.get_env("TTY7_RUN_ONCE").and_then(|v| v.to_str()),
            Some("claude --resume x")
        );
    }

    #[test]
    fn a_plain_shell_runs_it_then_becomes_itself() {
        let dash = spec("/bin/dash", &[]);
        let mut cmd = CommandBuilder::new("/bin/dash");
        apply(&mut cmd, "claude --resume x", Some(&dash));
        assert_eq!(
            cmd.get_argv(),
            &["/bin/dash", "-lic", "claude --resume x; exec /bin/dash -l"]
        );

        let custom = spec("/bin/zsh", &["-o", "no_rcs"]);
        let mut cmd = CommandBuilder::new("/bin/zsh");
        apply(&mut cmd, "x", Some(&custom));
        assert_eq!(cmd.get_argv()[2], "x; exec /bin/zsh -o no_rcs");
        assert!(cmd.get_env("TTY7_RUN_ONCE").is_none());
    }

    #[test]
    fn any_other_shell_starts_plain() {
        let nu = spec("nu", &[]);
        let mut cmd = CommandBuilder::new("nu");
        apply(&mut cmd, "x", Some(&nu));
        assert_eq!(cmd.get_argv(), &["nu"]);
        assert!(cmd.get_env("TTY7_RUN_ONCE").is_none());
    }
}

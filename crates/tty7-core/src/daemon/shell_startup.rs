//! `shell_startup` in the config: a pane's zsh, bash or fish that reads none
//! of the user's rc files yet keeps tty7's integration, and optionally
//! activates the env tools it finds in their place.

use std::path::{Path, PathBuf};

use super::shell_integration::{Injection, ShellKind};
use crate::core::config::Config;
use crate::core::fork_config::ShellStartup;

/// Read by the zsh redirectors and the bash rcfile: the directory whose
/// startup files they source instead of the user's.
pub(crate) const STARTUP_ENV: &str = "TTY7_SHELL_STARTUP";

/// The env tools a no-rcs shell activates, by absolute path.
#[derive(Debug, PartialEq)]
enum Activation {
    /// mise reads `.tool-versions` and `.env` itself, so nothing else runs.
    Mise(PathBuf),
    Others {
        /// The `asdf` binary and the shims directory.
        asdf: Option<(PathBuf, PathBuf)>,
        /// `.env` is loaded by a built-in snippet only when this is `None`.
        direnv: Option<PathBuf>,
    },
}

pub(crate) fn apply(kind: &ShellKind, injection: &mut Injection) {
    apply_mode(
        Config::load().fork.shell_startup,
        kind,
        injection,
        detect_here,
    );
}

fn apply_mode(
    mode: ShellStartup,
    kind: &ShellKind,
    injection: &mut Injection,
    detect: impl FnOnce() -> Activation,
) {
    // Empty, not absent: a daemon started from a no-rcs pane inherits it.
    injection.env.insert(STARTUP_ENV.into(), String::new());
    let shell = match kind {
        ShellKind::Zsh => "zsh",
        ShellKind::Bash => "bash",
        ShellKind::Fish => "fish",
        _ => return,
    };
    if mode == ShellStartup::User {
        return;
    }
    let activation = (mode == ShellStartup::NoRcsEnvlibs).then(detect);
    let script = script(shell, activation.as_ref());
    if shell == "fish" {
        injection
            .args
            .splice(0..0, ["--no-config".into(), "-C".into(), script]);
        return;
    }
    let Some(dir) = injection.dir.as_ref().map(|d| d.join("startup")) else {
        return;
    };
    let file = dir.join(if shell == "zsh" { ".zshrc" } else { ".bashrc" });
    if let Err(e) = std::fs::create_dir_all(&dir).and_then(|()| std::fs::write(&file, script)) {
        log::warn!("shell_startup: {e}; this pane reads the user's rc files");
        return;
    }
    injection
        .env
        .insert(STARTUP_ENV.into(), dir.to_string_lossy().into_owned());
}

fn detect_here() -> Activation {
    let home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_default();
    let path = std::env::var_os("PATH").unwrap_or_default();
    let asdf_data = std::env::var_os("ASDF_DATA_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| home.join(".asdf"));
    let mut search: Vec<PathBuf> = std::env::split_paths(&path).collect();
    // A daemon started from the Dock has a bare PATH: where the tools'
    // installers put them.
    search.extend([
        home.join(".local/bin"),
        home.join(".local/share/mise/bin"),
        "/opt/homebrew/bin".into(),
        "/usr/local/bin".into(),
        home.join(".asdf/bin"),
    ]);
    detect(&search, &asdf_data)
}

fn detect(search: &[PathBuf], asdf_data: &Path) -> Activation {
    let find = |name: &str| search.iter().map(|d| d.join(name)).find(|p| p.is_file());
    if let Some(mise) = find("mise") {
        return Activation::Mise(mise);
    }
    Activation::Others {
        asdf: find("asdf").map(|bin| (bin, asdf_data.join("shims"))),
        direnv: find("direnv"),
    }
}

/// What a no-rcs `shell` runs where the user's rc files would have: macOS's
/// system PATH, then `activation`'s tools.
fn script(shell: &str, activation: Option<&Activation>) -> String {
    let fish = shell == "fish";
    let mut out = String::new();
    if cfg!(target_os = "macos") {
        out.push_str(if fish {
            "test -x /usr/libexec/path_helper; and /usr/libexec/path_helper -c | source\n"
        } else {
            "[ -x /usr/libexec/path_helper ] && eval \"$(/usr/libexec/path_helper -s)\"\n"
        });
    }
    let path_add = |out: &mut String, dir: &Path| {
        let dir = quote(&dir.to_string_lossy(), fish);
        out.push_str(&if fish {
            format!("contains -- {dir} $PATH; or set -gx PATH {dir} $PATH\n")
        } else {
            format!("case \":$PATH:\" in *:{dir}:*) ;; *) export PATH={dir}:\"$PATH\" ;; esac\n")
        });
    };
    let hook = |out: &mut String, bin: &Path, verb: &str| {
        let bin = quote(&bin.to_string_lossy(), fish);
        out.push_str(&if fish {
            format!("{bin} {verb} fish | source\n")
        } else {
            format!("eval \"$({bin} {verb} {shell})\"\n")
        });
    };
    match activation {
        None => {}
        Some(Activation::Mise(mise)) => {
            path_add(&mut out, parent(mise));
            match shell {
                "zsh" => out.push_str(&DEFER_ZSH.replace(
                    "{cmd}",
                    &format!("{} activate zsh", quote(&mise.to_string_lossy(), false)),
                )),
                _ => hook(&mut out, mise, "activate"),
            }
        }
        Some(Activation::Others { asdf, direnv }) => {
            if let Some((bin, shims)) = asdf {
                path_add(&mut out, parent(bin));
                path_add(&mut out, shims);
            }
            match direnv {
                Some(direnv) => {
                    path_add(&mut out, parent(direnv));
                    hook(&mut out, direnv, "hook");
                }
                None => {
                    if !fish {
                        out.push_str(DOTENV_POSIX);
                    }
                    out.push_str(match shell {
                        "fish" => DOTENV_FISH,
                        "zsh" => DOTENV_ZSH_HOOK,
                        _ => DOTENV_BASH_HOOK,
                    });
                }
            }
        }
    }
    out
}

fn parent(bin: &Path) -> &Path {
    bin.parent().unwrap_or(Path::new("/"))
}

fn quote(s: &str, fish: bool) -> String {
    if fish {
        format!("'{}'", s.replace('\\', r"\\").replace('\'', r"\'"))
    } else {
        format!("'{}'", s.replace('\'', r"'\''"))
    }
}

// `mise activate`'s output is read once zle is idle, after the first prompt is
// drawn: mise takes 40-150ms just to start, which was most of a new pane's wait.
// Its output already carries the directory's env, so nothing has to rerun.
// A command entered sooner waits for it in preexec, so none runs without it.
// tty7's precmd moves back to the end so the prompt mark still comes last.
const DEFER_ZSH: &str = r#"exec {__tty7_defer_fd}< <({cmd} 2>/dev/null)
__tty7_defer_done() {
  local fd=$__tty7_defer_fd code= line
  [[ -n $fd ]] || return 0
  unset __tty7_defer_fd
  zle -F $fd 2>/dev/null
  while IFS= read -r -u $fd line; do code+=$line$'\n'; done
  code+=$line
  exec {fd}<&-
  add-zsh-hook -d preexec __tty7_defer_done
  eval "$code"
  if (( $+functions[__tty7_precmd] )); then
    add-zsh-hook -d precmd __tty7_precmd
    add-zsh-hook precmd __tty7_precmd
  fi
}
autoload -Uz add-zsh-hook
add-zsh-hook preexec __tty7_defer_done
zle -F $__tty7_defer_fd __tty7_defer_done 2>/dev/null || __tty7_defer_done
"#;

// Parses `KEY=value` lines rather than sourcing the file: a `.env` in any
// cloned repo would otherwise run its code on `cd`.
// shortcut: a variable stays set after leaving its directory; direnv unloads.
const DOTENV_POSIX: &str = r#"__tty7_dotenv() {
  [ -r .env ] || return 0
  local __l __k __v
  while IFS= read -r __l || [ -n "$__l" ]; do
    __l=${__l#export }
    case $__l in *=*) ;; *) continue ;; esac
    __k=${__l%%=*}
    case $__k in ''|[0-9]*|*[!A-Za-z0-9_]*) continue ;; esac
    __v=${__l#*=}
    case $__v in
      \"*\") __v=${__v#\"}; __v=${__v%\"} ;;
      \'*\') __v=${__v#\'}; __v=${__v%\'} ;;
    esac
    export "$__k=$__v"
  done < .env
}
"#;

const DOTENV_ZSH_HOOK: &str =
    "autoload -Uz add-zsh-hook\nadd-zsh-hook chpwd __tty7_dotenv\n__tty7_dotenv\n";

const DOTENV_BASH_HOOK: &str = r#"__tty7_dotenv_pwd=
__tty7_dotenv_cd() { [ "$PWD" = "$__tty7_dotenv_pwd" ] || { __tty7_dotenv_pwd=$PWD; __tty7_dotenv; }; }
PROMPT_COMMAND="__tty7_dotenv_cd${PROMPT_COMMAND:+;$PROMPT_COMMAND}"
"#;

const DOTENV_FISH: &str = r#"function __tty7_dotenv --on-variable PWD
  test -r .env; or return
  for line in (string match -rv '^\s*(#|$)' < .env)
    set -l kv (string split -m1 = -- (string replace -r '^export\s+' '' -- $line))
    set -q kv[2]; and string match -qr '^[A-Za-z_][A-Za-z0-9_]*$' -- $kv[1]; or continue
    set -gx $kv[1] (string trim -c '"\'' -- $kv[2])
  end
end
__tty7_dotenv
"#;

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::process::Command;

    use super::super::shell_integration::{bash_rcfile, zsh_redirectors};
    use super::*;

    fn injection(dir: Option<PathBuf>) -> Injection {
        Injection {
            env: HashMap::new(),
            args: vec!["-C".into(), "integration".into()],
            replaces_argv: false,
            dir,
        }
    }

    fn exe(dir: &Path, name: &str) -> PathBuf {
        std::fs::create_dir_all(dir).unwrap();
        let p = dir.join(name);
        std::fs::write(&p, "").unwrap();
        p
    }

    fn mise() -> Activation {
        Activation::Mise("/opt/x/mise".into())
    }

    #[test]
    fn mise_wins_and_hides_the_rest() {
        let t = tempfile::tempdir().unwrap();
        let (a, b) = (t.path().join("a"), t.path().join("b"));
        exe(&a, "asdf");
        exe(&a, "direnv");
        let mise = exe(&b, "mise");
        assert_eq!(detect(&[a, b], t.path()), Activation::Mise(mise));
    }

    #[test]
    fn without_mise_asdf_and_direnv_both_activate() {
        let t = tempfile::tempdir().unwrap();
        let a = t.path().join("a");
        let asdf = exe(&a, "asdf");
        let direnv = exe(&a, "direnv");
        let data = t.path().join("data");
        assert_eq!(
            detect(&[a], &data),
            Activation::Others {
                asdf: Some((asdf, data.join("shims"))),
                direnv: Some(direnv),
            }
        );
    }

    #[test]
    fn the_first_dir_on_the_search_path_wins() {
        let t = tempfile::tempdir().unwrap();
        let (a, b) = (t.path().join("a"), t.path().join("b"));
        let first = exe(&a, "direnv");
        exe(&b, "direnv");
        let Activation::Others { direnv, .. } = detect(&[a, b], t.path()) else {
            panic!()
        };
        assert_eq!(direnv, Some(first));
    }

    #[test]
    fn dotenv_loads_only_without_direnv() {
        let none = Activation::Others {
            asdf: None,
            direnv: None,
        };
        let direnv = Activation::Others {
            asdf: None,
            direnv: Some("/d/direnv".into()),
        };
        for shell in ["zsh", "bash", "fish"] {
            assert!(
                script(shell, Some(&none)).contains("__tty7_dotenv"),
                "{shell}"
            );
            let s = script(shell, Some(&direnv));
            assert!(!s.contains("__tty7_dotenv"), "{shell}: {s}");
            assert!(s.contains("'/d/direnv' hook"), "{shell}: {s}");
        }
        assert!(script("zsh", Some(&none)).contains("chpwd"));
        assert!(script("bash", Some(&none)).contains("PROMPT_COMMAND"));
    }

    #[test]
    fn mise_activates_for_its_own_shell() {
        assert!(
            script("zsh", Some(&mise())).contains("< <('/opt/x/mise' activate zsh 2>/dev/null)")
        );
        assert!(script("bash", Some(&mise())).contains("activate bash"));
        assert!(script("fish", Some(&mise())).contains("'/opt/x/mise' activate fish | source"));
    }

    #[test]
    fn no_rcs_without_envlibs_activates_nothing() {
        for shell in ["zsh", "bash", "fish"] {
            let s = script(shell, None);
            assert!(!s.contains("activate") && !s.contains("dotenv"), "{s}");
            assert_eq!(s.contains("path_helper"), cfg!(target_os = "macos"));
        }
    }

    #[test]
    fn user_mode_changes_nothing_but_clears_an_inherited_dir() {
        let t = tempfile::tempdir().unwrap();
        let mut inj = injection(Some(t.path().into()));
        apply_mode(
            ShellStartup::User,
            &ShellKind::Zsh,
            &mut inj,
            || unreachable!(),
        );
        assert_eq!(inj.env[STARTUP_ENV], "");
        assert_eq!(inj.args, ["-C", "integration"]);
        assert!(!t.path().join("startup").exists());
    }

    #[test]
    fn zsh_and_bash_get_a_startup_dir_in_their_injection_dir() {
        for (kind, file) in [(ShellKind::Zsh, ".zshrc"), (ShellKind::Bash, ".bashrc")] {
            let t = tempfile::tempdir().unwrap();
            let mut inj = injection(Some(t.path().into()));
            apply_mode(ShellStartup::NoRcsEnvlibs, &kind, &mut inj, mise);
            let dir = t.path().join("startup");
            assert_eq!(inj.env[STARTUP_ENV], dir.to_string_lossy());
            let body = std::fs::read_to_string(dir.join(file)).unwrap();
            assert!(body.contains("activate"), "{body}");
        }
    }

    #[test]
    fn fish_runs_without_config_and_envlibs_detection_is_lazy() {
        let mut inj = injection(None);
        apply_mode(
            ShellStartup::NoRcs,
            &ShellKind::Fish,
            &mut inj,
            || unreachable!(),
        );
        assert_eq!(inj.args[..2], ["--no-config", "-C"]);
        assert_eq!(inj.args[3..], ["-C", "integration"]);
    }

    #[test]
    fn other_shells_keep_the_users_startup() {
        let mut inj = injection(None);
        apply_mode(
            ShellStartup::NoRcsEnvlibs,
            &ShellKind::Nushell,
            &mut inj,
            mise,
        );
        assert_eq!(inj.args, ["-C", "integration"]);
        assert_eq!(inj.env[STARTUP_ENV], "");
    }

    #[test]
    fn the_posix_dotenv_parser_sets_values_and_runs_nothing() {
        let t = tempfile::tempdir().unwrap();
        std::fs::write(
            t.path().join(".env"),
            "# c\nA=1\nexport B=\"two words\"\nC='x'\nD=$(touch pwned)\nbad-key=3\n9X=4\n",
        )
        .unwrap();
        for sh in ["bash", "zsh"] {
            let out = Command::new(sh)
                .current_dir(t.path())
                .arg("-c")
                .arg(format!(
                    "{DOTENV_POSIX}__tty7_dotenv; printf '%s|' \"$A\" \"$B\" \"$C\" \"$D\""
                ))
                .output()
                .unwrap();
            assert_eq!(
                String::from_utf8_lossy(&out.stdout),
                "1|two words|x|$(touch pwned)|",
                "{sh}: {}",
                String::from_utf8_lossy(&out.stderr)
            );
            assert!(!t.path().join("pwned").exists());
        }
    }

    /// What an interactive `shell` prints from rc files that each echo their
    /// owner, with `TTY7_SHELL_STARTUP` set when `startup`.
    fn sourced(shell: &str, startup: bool) -> String {
        let t = tempfile::tempdir().unwrap();
        let (home, ours, dir) = (
            t.path().join("home"),
            t.path().join("ours"),
            t.path().join("startup"),
        );
        for d in [&home, &ours, &dir] {
            std::fs::create_dir(d).unwrap();
        }
        let mut cmd = Command::new(shell);
        if shell == "zsh" {
            for (name, body) in zsh_redirectors() {
                std::fs::write(ours.join(name), body).unwrap();
                std::fs::write(home.join(name), format!("echo USER{name}\n")).unwrap();
            }
            std::fs::write(dir.join(".zshrc"), "echo STARTUP\n").unwrap();
            cmd.env("ZDOTDIR", &ours);
        } else {
            std::fs::write(ours.join("rc"), bash_rcfile()).unwrap();
            std::fs::write(home.join(".bashrc"), "echo USER.bashrc\n").unwrap();
            std::fs::write(dir.join(".bashrc"), "echo STARTUP\n").unwrap();
            cmd.arg("--rcfile").arg(ours.join("rc"));
        }
        let out = cmd
            .args(["-i", "-c", "echo INT=$TTY7_SHELL_INTEGRATION"])
            .env("HOME", &home)
            .env("TTY7_SHELL_INTEGRATION", "")
            .env_remove("TTY7_USER_ZDOTDIR")
            .env(
                STARTUP_ENV,
                if startup {
                    dir.as_os_str()
                } else {
                    "".as_ref()
                },
            )
            .output()
            .unwrap();
        String::from_utf8_lossy(&out.stdout).into_owned()
    }

    #[cfg(unix)]
    #[test]
    fn the_redirectors_and_rcfile_swap_the_users_files_for_the_startup_dir() {
        for shell in ["zsh", "bash"] {
            if Command::new(shell).arg("-c").arg("true").status().is_err() {
                continue;
            }
            let user = sourced(shell, false);
            assert!(
                user.contains("USER") && !user.contains("STARTUP"),
                "{shell}: {user}"
            );
            let ours = sourced(shell, true);
            assert!(
                !ours.contains("USER") && ours.contains("STARTUP"),
                "{shell}: {ours}"
            );
            assert!(
                ours.contains("INT=1"),
                "{shell}: integration still loads: {ours}"
            );
        }
    }
}

//! `shell_startup` in the config: a pane's zsh, bash or fish that reads none
//! of the user's rc files yet keeps tty7's integration, and optionally
//! activates the env tools it finds in their place.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use base64::Engine as _;

use super::shell_integration::{Injection, ShellKind};
use crate::core::config::Config;
use crate::core::fork_config::ShellStartup;

/// Read by the zsh redirectors and the bash rcfile: the directory whose
/// startup files they source instead of the user's.
pub(crate) const STARTUP_ENV: &str = "TTY7_SHELL_STARTUP";

/// The key mise's `env_cache` encrypts with. `mise activate` mints a new one
/// unless the shell inherits one, so without a shared key no pane could read
/// the cache another pane wrote.
const MISE_KEY_ENV: &str = "__MISE_ENV_CACHE_KEY";

/// The shells a startup script is written for.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Sh {
    Zsh,
    Bash,
    Fish,
}

impl Sh {
    fn name(self) -> &'static str {
        match self {
            Sh::Zsh => "zsh",
            Sh::Bash => "bash",
            Sh::Fish => "fish",
        }
    }
}

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
        mise_cache_key(),
    );
}

fn apply_mode(
    mode: ShellStartup,
    kind: &ShellKind,
    injection: &mut Injection,
    detect: impl FnOnce() -> Activation,
    mise_key: Option<&str>,
) {
    // Empty, not absent: a daemon started from a no-rcs pane inherits it.
    injection.env.insert(STARTUP_ENV.into(), String::new());
    let sh = match kind {
        ShellKind::Zsh => Sh::Zsh,
        ShellKind::Bash => Sh::Bash,
        ShellKind::Fish => Sh::Fish,
        ShellKind::PowerShell | ShellKind::Nushell | ShellKind::Wsl => return,
    };
    if mode == ShellStartup::User {
        return;
    }
    let activation = (mode == ShellStartup::NoRcsEnvlibs).then(detect);
    if let (Some(Activation::Mise(_)), Some(key)) = (&activation, mise_key) {
        injection.env.insert(MISE_KEY_ENV.into(), key.into());
    }
    let script = script(sh, activation.as_ref());
    let rc = match sh {
        Sh::Zsh => ".zshrc",
        Sh::Bash => ".bashrc",
        Sh::Fish => {
            injection
                .args
                .splice(0..0, ["--no-config".into(), "-C".into(), script]);
            return;
        }
    };
    let Some(dir) = injection.dir.as_ref().map(|d| d.join("startup")) else {
        return;
    };
    let file = dir.join(rc);
    if let Err(e) = std::fs::create_dir_all(&dir).and_then(|()| std::fs::write(&file, script)) {
        log::warn!("shell_startup: {e}; this pane reads the user's rc files");
        return;
    }
    injection
        .env
        .insert(STARTUP_ENV.into(), dir.to_string_lossy().into_owned());
}

/// One key for every pane of this daemon. `None` when the daemon already
/// carries the user's own key, which the panes inherit instead.
fn mise_cache_key() -> Option<&'static str> {
    static KEY: OnceLock<Option<String>> = OnceLock::new();
    KEY.get_or_init(|| {
        if std::env::var_os(MISE_KEY_ENV).is_some() {
            return None;
        }
        mint_mise_cache_key()
            .inspect_err(|e| log::warn!("shell_startup: no mise env cache key: {e}"))
            .ok()
    })
    .as_deref()
}

/// mise's own format: 32 random bytes for ChaCha20-Poly1305, standard base64.
fn mint_mise_cache_key() -> Result<String, getrandom::Error> {
    let mut key = [0u8; 32];
    getrandom::fill(&mut key)?;
    Ok(base64::engine::general_purpose::STANDARD.encode(key))
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

/// What a no-rcs `sh` runs where the user's rc files would have: macOS's
/// system PATH, then `activation`'s tools.
fn script(sh: Sh, activation: Option<&Activation>) -> String {
    let mut out = String::new();
    if cfg!(target_os = "macos") {
        out.push_str(match sh {
            Sh::Fish => {
                "test -x /usr/libexec/path_helper; and /usr/libexec/path_helper -c | source\n"
            }
            Sh::Zsh | Sh::Bash => {
                "[ -x /usr/libexec/path_helper ] && eval \"$(/usr/libexec/path_helper -s)\"\n"
            }
        });
    }
    let path_add = |out: &mut String, dir: &Path| {
        let dir = quote(&dir.to_string_lossy(), sh);
        out.push_str(&match sh {
            Sh::Fish => format!("contains -- {dir} $PATH; or set -gx PATH {dir} $PATH\n"),
            Sh::Zsh | Sh::Bash => {
                format!(
                    "case \":$PATH:\" in *:{dir}:*) ;; *) export PATH={dir}:\"$PATH\" ;; esac\n"
                )
            }
        });
    };
    let hook = |out: &mut String, bin: &Path, verb: &str| {
        let bin = quote(&bin.to_string_lossy(), sh);
        out.push_str(&match sh {
            Sh::Fish => format!("{bin} {verb} fish | source\n"),
            Sh::Zsh | Sh::Bash => format!("eval \"$({bin} {verb} {})\"\n", sh.name()),
        });
    };
    let Some(activation) = activation else {
        return out;
    };
    let tool = match activation {
        Activation::Mise(_) => "mise",
        Activation::Others {
            direnv: Some(_), ..
        } => "direnv",
        Activation::Others { direnv: None, .. } => ".env",
    };
    let msg = quote(&format!("loading env ({tool})..."), sh);
    let (here, start, end) = match sh {
        Sh::Fish => (ENV_HERE_FISH, HINT_START_FISH, HINT_END_FISH),
        Sh::Zsh | Sh::Bash => (ENV_HERE_POSIX, HINT_START_POSIX, HINT_END_POSIX),
    };
    out.push_str(here);
    // zsh's mise load runs after the prompt, so DEFER_ZSH shows its own hint.
    let blocking = !matches!((sh, activation), (Sh::Zsh, Activation::Mise(_)));
    if blocking {
        out.push_str(&start.replace("{msg}", &msg));
    }
    match activation {
        Activation::Mise(mise) => {
            path_add(&mut out, parent(mise));
            match sh {
                Sh::Zsh => out.push_str(
                    &DEFER_ZSH
                        .replace("{mise}", &quote(&mise.to_string_lossy(), sh))
                        .replace("{msg}", &msg),
                ),
                Sh::Bash | Sh::Fish => hook(&mut out, mise, "activate"),
            }
        }
        Activation::Others { asdf, direnv } => {
            if let Some((bin, shims)) = asdf {
                path_add(&mut out, parent(bin));
                path_add(&mut out, shims);
            }
            match direnv {
                Some(direnv) => {
                    path_add(&mut out, parent(direnv));
                    hook(&mut out, direnv, "hook");
                }
                None => match sh {
                    Sh::Fish => out.push_str(DOTENV_FISH),
                    Sh::Zsh => out.push_str(&format!("{DOTENV_POSIX}{DOTENV_ZSH_HOOK}")),
                    Sh::Bash => out.push_str(&format!("{DOTENV_POSIX}{DOTENV_BASH_HOOK}")),
                },
            }
        }
    }
    if blocking {
        out.push_str(end);
    }
    out
}

// Whether the env tools have anything to load here: a config file in $PWD or
// above. Builtins only, so a pane in /tmp pays nothing and shows no hint.
const ENV_HERE_POSIX: &str = r#"__tty7_env_here() {
  local d=$PWD f
  while :; do
    for f in mise.toml .mise.toml mise/config.toml .mise/config.toml .tool-versions .env .envrc; do
      [ -e "$d/$f" ] && return 0
    done
    [ -n "$d" ] || return 1
    d=${d%/*}
  done
}
"#;

const ENV_HERE_FISH: &str = r#"function __tty7_env_here
  set -l d $PWD
  while true
    for f in mise.toml .mise.toml mise/config.toml .mise/config.toml .tool-versions .env .envrc
      test -e "$d/$f"; and return 0
    end
    test -n "$d"; or return 1
    set d (string replace -r '/[^/]*$' '' -- $d)
  end
end
"#;

// Shown while a load that blocks the first prompt runs, then erased.
const HINT_START_POSIX: &str = "__tty7_env_here && { __tty7_hint=1; printf '%s' {msg} >&2; }\n";
const HINT_END_POSIX: &str =
    "[ -z \"$__tty7_hint\" ] || printf '\\r\\033[K' >&2; unset __tty7_hint\n";
const HINT_START_FISH: &str =
    "__tty7_env_here; and set -g __tty7_hint 1; and printf '%s' {msg} >&2\n";
const HINT_END_FISH: &str =
    "set -q __tty7_hint; and printf '\\r\\e[K' >&2; and set -e __tty7_hint\n";

fn parent(bin: &Path) -> &Path {
    bin.parent().unwrap_or(Path::new("/"))
}

fn quote(s: &str, sh: Sh) -> String {
    match sh {
        Sh::Fish => format!("'{}'", s.replace('\\', r"\\").replace('\'', r"\'")),
        Sh::Zsh | Sh::Bash => format!("'{}'", s.replace('\'', r"'\''")),
    }
}

// `mise activate`'s output is read once zle is idle, after the first prompt is
// drawn: mise takes 40-150ms just to start, which was most of a new pane's wait.
// Its output already carries the directory's env, so nothing has to rerun.
// A command entered sooner waits for it in preexec, so none runs without it.
// tty7's precmd moves back to the end so the prompt mark still comes last.
// While it is pending, and only if `__tty7_env_here`, a hint shows under the
// prompt (`zle -M`) or, for a command waiting in preexec, on stderr until the
// load lands. Both wait 150ms first, the prompt's on a timer fd, so a fast load
// shows nothing.
//
// The output's own last step, `_mise_hook --force`, is a full `mise hook-env`
// (90-250ms), so that one exact line is swapped for hook-env's output, and its
// stderr replayed, computed here too. Without that line, or if hook-env fails,
// the output passes through untouched and runs as it always has.
// shortcut: hook-env sees the env as of the fork; anything that edits PATH later
// (e.g. /etc/zlogin) is overwritten. Upgrade by re-running hook-env if $PATH
// changed since the fork.
const DEFER_ZSH: &str = r#"__tty7_defer_activate() {
  local out m=$'\n_mise_hook --force\n' err hook
  out=$({mise} activate zsh 2>/dev/null) || return
  out=$'\n'$out$'\n'
  if [[ $out == *$m* ]]; then
    eval "${out%%$m*}" >/dev/null 2>&1
    err=$(mktemp 2>/dev/null) || err=/dev/null
    if hook=$({mise} hook-env --shell-pid $$ --force -s zsh 2>$err); then
      [[ -s $err ]] && hook+=$'\n'"print -ru2 -- ${(q)$(<$err)}"
      out=${out%%$m*}$'\n'$hook$'\n'${out#*$m}
    fi
    [[ $err == /dev/null ]] || rm -f $err
  fi
  print -r -- "$out"
}
exec {__tty7_defer_fd}< <(__tty7_defer_activate)
zmodload zsh/zselect 2>/dev/null
__tty7_defer_msg=
__tty7_env_here && __tty7_defer_msg={msg}
# The hint's delay in centiseconds; the env sets it only for tty7's tests.
: ${__tty7_hint_cs:=15}
__tty7_defer_ready() { zselect -t 0 -r $__tty7_defer_fd 2>/dev/null; }
__tty7_defer_done() {
  local fd=$__tty7_defer_fd code hint=
  [[ -n $fd ]] || return 0
  __tty7_hint_off
  if ! zle && [[ -n $__tty7_defer_msg ]] && ! zselect -t $__tty7_hint_cs -r $fd 2>/dev/null; then
    hint=1
    print -rn -- "$__tty7_defer_msg" >&2
  fi
  # A Ctrl-C here aborts the function with the fd and hooks in place, so the
  # next command or idle prompt reads it instead.
  {
    IFS= read -rd '' -u $fd code
  } always {
    [[ -n $hint ]] && print -rn -- $'\r\e[K' >&2
  }
  zle -F $fd 2>/dev/null
  exec {fd}<&-
  unset __tty7_defer_fd
  add-zsh-hook -d preexec __tty7_defer_done
  if zle; then
    [[ -n $__tty7_defer_shown ]] && zle -M ''
    zle -I
  fi
  eval "$code"
  # `__tty7_precmd` is shell_integration.rs's prompt-mark hook.
  [[ -n ${precmd_functions[(r)__tty7_precmd]} ]] &&
    precmd_functions=(${precmd_functions:#__tty7_precmd} __tty7_precmd)
}
__tty7_hint_off() {
  [[ -n $__tty7_hint_fd ]] || return 0
  zle -F $__tty7_hint_fd 2>/dev/null
  exec {__tty7_hint_fd}<&-
  unset __tty7_hint_fd
}
__tty7_hint_show() {
  __tty7_hint_off
  [[ -n $__tty7_defer_fd ]] && ! __tty7_defer_ready || return 0
  zle -M "$__tty7_defer_msg"
  __tty7_defer_shown=1
}
__tty7_defer_hint() {
  add-zle-hook-widget -d line-init __tty7_defer_hint
  [[ -n $__tty7_defer_fd && -n $__tty7_defer_msg ]] || return 0
  exec {__tty7_hint_fd}< <(sleep $(( __tty7_hint_cs / 100.0 )))
  zle -F $__tty7_hint_fd __tty7_hint_show
}
autoload -Uz add-zsh-hook add-zle-hook-widget
zle -N __tty7_defer_hint
add-zle-hook-widget line-init __tty7_defer_hint
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
        for shell in [Sh::Zsh, Sh::Bash, Sh::Fish] {
            assert!(
                script(shell, Some(&none)).contains("__tty7_dotenv"),
                "{shell:?}"
            );
            let s = script(shell, Some(&direnv));
            assert!(!s.contains("__tty7_dotenv"), "{shell:?}: {s}");
            assert!(s.contains("'/d/direnv' hook"), "{shell:?}: {s}");
        }
        assert!(script(Sh::Zsh, Some(&none)).contains("chpwd"));
        assert!(script(Sh::Bash, Some(&none)).contains("PROMPT_COMMAND"));
    }

    #[test]
    fn mise_activates_for_its_own_shell() {
        assert!(
            script(Sh::Zsh, Some(&mise()))
                .contains("out=$('/opt/x/mise' activate zsh 2>/dev/null)")
        );
        assert!(script(Sh::Bash, Some(&mise())).contains("activate bash"));
        assert!(script(Sh::Fish, Some(&mise())).contains("'/opt/x/mise' activate fish | source"));
    }

    #[test]
    fn no_rcs_without_envlibs_activates_nothing() {
        for shell in [Sh::Zsh, Sh::Bash, Sh::Fish] {
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
            None,
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
            apply_mode(ShellStartup::NoRcsEnvlibs, &kind, &mut inj, mise, None);
            let dir = t.path().join("startup");
            assert_eq!(inj.env[STARTUP_ENV], dir.to_string_lossy());
            let body = std::fs::read_to_string(dir.join(file)).unwrap();
            assert!(body.contains("activate"), "{body}");
        }
    }

    #[test]
    fn the_mise_cache_key_goes_only_to_panes_that_activate_mise() {
        let key = base64::engine::general_purpose::STANDARD
            .decode(mint_mise_cache_key().unwrap())
            .unwrap();
        assert_eq!(key.len(), 32);
        assert_ne!(
            mint_mise_cache_key().unwrap(),
            mint_mise_cache_key().unwrap()
        );
        let pane = |mode, detect: fn() -> Activation| {
            let t = tempfile::tempdir().unwrap();
            let mut inj = injection(Some(t.path().into()));
            apply_mode(mode, &ShellKind::Zsh, &mut inj, detect, Some("k"));
            inj.env.get(MISE_KEY_ENV).cloned()
        };
        assert_eq!(pane(ShellStartup::NoRcsEnvlibs, mise).as_deref(), Some("k"));
        let others = || Activation::Others {
            asdf: None,
            direnv: None,
        };
        assert_eq!(pane(ShellStartup::NoRcsEnvlibs, others), None);
        assert_eq!(pane(ShellStartup::NoRcs, mise), None);
        assert_eq!(mise_cache_key(), mise_cache_key());
    }

    #[test]
    fn fish_runs_without_config_and_envlibs_detection_is_lazy() {
        let mut inj = injection(None);
        apply_mode(
            ShellStartup::NoRcs,
            &ShellKind::Fish,
            &mut inj,
            || unreachable!(),
            None,
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
            None,
        );
        assert_eq!(inj.args, ["-C", "integration"]);
        assert_eq!(inj.env[STARTUP_ENV], "");
    }

    #[test]
    fn the_hint_names_the_tool_and_wraps_only_loads_that_block_the_prompt() {
        let direnv = Activation::Others {
            asdf: None,
            direnv: Some("/d/direnv".into()),
        };
        let dotenv = Activation::Others {
            asdf: None,
            direnv: None,
        };
        for (activation, tool) in [(mise(), "mise"), (direnv, "direnv"), (dotenv, ".env")] {
            for sh in [Sh::Zsh, Sh::Bash, Sh::Fish] {
                let s = script(sh, Some(&activation));
                assert!(
                    s.contains(&format!("'loading env ({tool})...'")),
                    "{sh:?}: {s}"
                );
                let blocking = !(sh == Sh::Zsh && tool == "mise");
                assert_eq!(
                    s.contains("__tty7_hint=1;") || s.contains("set -g __tty7_hint 1"),
                    blocking,
                    "{sh:?}: {s}"
                );
            }
        }
        assert!(!script(Sh::Zsh, None).contains("loading env"));
    }

    #[test]
    fn env_here_finds_a_config_above_pwd_and_nothing_elsewhere() {
        let t = tempfile::tempdir().unwrap();
        let deep = t.path().join("a/b/c");
        std::fs::create_dir_all(&deep).unwrap();
        let empty = tempfile::tempdir().unwrap();
        std::fs::write(t.path().join("a/.tool-versions"), "").unwrap();
        for sh in ["bash", "zsh"] {
            let here = |dir: &Path| {
                Command::new(sh)
                    .arg("-c")
                    .arg(format!("{ENV_HERE_POSIX}__tty7_env_here"))
                    .current_dir(dir)
                    .status()
                    .unwrap()
                    .success()
            };
            assert!(here(&deep), "{sh}");
            assert!(!here(empty.path()), "{sh}");
        }
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

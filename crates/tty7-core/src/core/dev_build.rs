//! Whether this binary is a cargo build rather than an install. A build must
//! not write the user's global state outside tty7's config dir — the `tty7`
//! CLI link on PATH, the agent hooks in `~/.claude/settings.json` and friends —
//! or it points the real install at a binary the next `cargo clean` or worktree
//! removal deletes.
//!
//! No profile names: a build is anything under a dir cargo marked with
//! `CACHEDIR.TAG` (which also covers `CARGO_TARGET_DIR` outside the repo), or
//! under a `target` dir beside a `Cargo.toml`. Both the path as launched and
//! its canonical form are checked, so a symlinked target dir counts too.

use std::path::Path;
use std::sync::OnceLock;

/// The running executable is a build. Cached: the answer cannot change.
pub fn running_dev_build() -> bool {
    static DEV: OnceLock<bool> = OnceLock::new();
    *DEV.get_or_init(|| std::env::current_exe().is_ok_and(|exe| is_dev_build(&exe)))
}

pub fn is_dev_build(exe: &Path) -> bool {
    under_a_build_dir(exe) || exe.canonicalize().is_ok_and(|c| under_a_build_dir(&c))
}

fn under_a_build_dir(exe: &Path) -> bool {
    exe.ancestors().skip(1).any(|dir| {
        dir.join("CACHEDIR.TAG").is_file()
            || (dir.file_name() == Some("target".as_ref())
                && dir.parent().is_some_and(|p| p.join("Cargo.toml").is_file()))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn tmpdir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("tty7-dev-build-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn a_target_dir_beside_a_cargo_toml_is_a_build() {
        let repo = tmpdir("repo");
        std::fs::write(repo.join("Cargo.toml"), "").unwrap();
        for rel in [
            "target/fast/tty7-app",
            "target/debug/tty7-app",
            "target/release/tty7-app",
            "target/aarch64-apple-darwin/release/tty7-app",
            "target/fast/bundle/osx/tty7-niu-dev.app/Contents/MacOS/tty7-app",
        ] {
            assert!(is_dev_build(&repo.join(rel)), "{rel} is a build");
        }
        let _ = std::fs::remove_dir_all(&repo);
    }

    #[test]
    fn a_cachedir_tag_marks_a_build_with_no_target_in_its_path() {
        let dir = tmpdir("cachedir");
        std::fs::write(dir.join("CACHEDIR.TAG"), "").unwrap();
        assert!(is_dev_build(&dir.join("fast/tty7-app")));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_symlink_into_a_build_dir_is_a_build() {
        let dir = tmpdir("symlink");
        std::fs::create_dir_all(dir.join("build/fast")).unwrap();
        std::fs::write(dir.join("build/CACHEDIR.TAG"), "").unwrap();
        std::fs::write(dir.join("build/fast/tty7-app"), "").unwrap();
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(dir.join("build/fast"), dir.join("link")).unwrap();
            assert!(is_dev_build(&dir.join("link/tty7-app")));
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn installs_are_not_builds() {
        for p in [
            "/Users/me/Applications/tty7-niu-dev.app/Contents/MacOS/tty7-app",
            "/Applications/tty7.app/Contents/MacOS/tty7-app",
            "/Volumes/Target/Applications/tty7.app/Contents/MacOS/tty7-app",
            "/Users/me/.cargo/bin/tty7-app",
            "/opt/homebrew/Cellar/tty7/1.0/bin/tty7-app",
            "/opt/tty7/release/tty7-app",
        ] {
            assert!(!is_dev_build(Path::new(p)), "{p} is installed");
        }
    }

    #[test]
    fn the_test_binary_is_a_build() {
        assert!(running_dev_build());
    }
}

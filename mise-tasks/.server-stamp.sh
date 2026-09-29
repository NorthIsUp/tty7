# Sourced by launch and reload. A window from a build whose server code
# differs replaces the running server, and every pane's processes end with
# it; the stamp records which server code the running server was started from.
stamp="$MISE_PROJECT_ROOT/target/fast/.server-stamp"
# What install-app installs and launch runs.
app_name=tty7-niu-dev
app="$HOME/Applications/$app_name.app"

server_code() {
  { git -C "$MISE_PROJECT_ROOT" ls-files -s crates/tty7-core Cargo.toml Cargo.lock
    git -C "$MISE_PROJECT_ROOT" diff -- crates/tty7-core Cargo.toml Cargo.lock; } | shasum | cut -c1-16
}

server_pid() {
  "$MISE_PROJECT_ROOT/target/fast/tty7" status 2>/dev/null | awk '$1 == "pid" { print $2 }'
}

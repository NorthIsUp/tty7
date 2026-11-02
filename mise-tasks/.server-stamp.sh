# Sourced by launch, reload and install-app. The stamp records which server
# code the running server runs, so a reload knows when to restart it in place.
stamp="$MISE_PROJECT_ROOT/target/fast/.server-stamp"
# What install-app installs and launch runs; overridable for a scratch rig.
app_name=tty7-niu-dev
app="${TTY7_DEV_APP:-$HOME/Applications/$app_name.app}"
exe="$app/Contents/MacOS/tty7-app"

server_code() {
  { git -C "$MISE_PROJECT_ROOT" ls-files -s crates/tty7-core Cargo.toml Cargo.lock
    git -C "$MISE_PROJECT_ROOT" diff -- crates/tty7-core Cargo.toml Cargo.lock; } | shasum | cut -c1-16
}

server_pid() {
  "$app/Contents/MacOS/tty7" status 2>/dev/null | awk '$1 == "pid" { print $2 }'
}

# ps shows the argv of the image the pid runs now, so an in-place restart shows.
on_bundle() {
  [[ "$(ps -o command= -p "$1")" == "$exe --daemon"* ]]
}

# Windows only: the server runs the same binary with flags. The target/fast
# path is where launch ran windows from before the bundle.
window_pids() {
  ps -axo pid=,command= | awk -v a="$exe" -v b="$MISE_PROJECT_ROOT/target/fast/tty7-app" \
    'NF == 2 && ($2 == a || $2 == b) { print $1 }'
}

# A server not running this checkout's code from the bundle is replaced in
# place: same pid, every pane kept. Joining it instead would leave a new
# window on old server code.
ensure_server() {
  local pid
  pid=$(server_pid || true)
  [ -n "$pid" ] || return 0
  if [ "$(cat "$stamp" 2>/dev/null || true)" = "$pid $(server_code)" ] && on_bundle "$pid"; then
    return 0
  fi
  TTY7_SERVER_EXE="$exe" "$app/Contents/MacOS/tty7" server restart
  pid=$(server_pid || true)
  if [ -z "$pid" ] || ! on_bundle "$pid"; then
    echo "server restart left server ${pid:-(none)} not running $exe: $(ps -o command= -p "${pid:-0}" || true)" >&2
    return 1
  fi
  echo "$pid $(server_code)" > "$stamp"
}

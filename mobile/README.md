# tty7 mobile

Watch and drive your desktop's tty7 panes and agents from a phone.

```
phone (Tauri: WebView + Rust)  ──iroh──▶  tty7-gateway  ──local sockets──▶  tty7 server
   xterm.js ◀─ raw bytes ─ tty7-mobile-client          (crates/tty7-gateway)
```

- **Transport**: [iroh](https://iroh.computer). The phone dials the desktop by public key.
  Connections hole-punch to a direct path when the network allows and fall back to a relay
  when it doesn't. Either way they're end-to-end encrypted. No port forwarding, no VPN.
- **Protocol**: `crates/tty7-mobile-proto`. The phone never speaks the daemon's own
  protocols. The gateway exposes a small vocabulary: pair, a live tree of workspaces, tabs,
  panes and agent status, and one stream per open pane.
- **Panes are observed, not attached.** A phone never resizes a pane or takes it away from
  the desktop window showing it. Keystrokes go in beside the observer (`SendInput`). The
  terminal keeps the desktop's size, and the app shrinks the font to fit the width.
- **Auth**: pairing with a one-time code, valid 10 minutes, single use. After that the
  gateway admits only the phone keys on its device list (`<config dir>/mobile/devices.json`).

## Run it

On the desktop, next to a running tty7:

```sh
cargo run -p tty7-gateway -- serve     # keep this running
cargo run -p tty7-gateway -- pair      # prints a QR code + a tty7pair:… code
cargo run -p tty7-gateway -- devices   # paired phones; `revoke <name>` removes one
```

### The app on the desktop (fastest loop)

Tauri builds the same app for macOS, which is the quickest way to work on the UI:

```sh
cd mobile
npm install
npm run tauri dev
```

Paste the `tty7pair:` code and tap **Pair**.

### iOS

Needs the full Xcode, not just the Command Line Tools, plus an Apple developer account to
run on a device.

```sh
rustup target add aarch64-apple-ios aarch64-apple-ios-sim
cd mobile
npm run tauri ios init       # once: generates src-tauri/gen/apple
npm run tauri ios dev        # simulator, or pick a connected device
```

### Android

Needs Android Studio's SDK and NDK, with `ANDROID_HOME` and `NDK_HOME` set.

```sh
rustup target add aarch64-linux-android armv7-linux-androideabi x86_64-linux-android
cd mobile
npm run tauri android init
npm run tauri android dev
```

### Without a phone

`crates/tty7-mobile-client/examples/probe.rs` is a phone in a shell. It pairs, prints the
tree, and times keystroke echo on a pane:

```sh
cargo run -p tty7-mobile-client --example probe -- pair '<code>'
cargo run -p tty7-mobile-client --example probe -- tree
cargo run -p tty7-mobile-client --example probe -- type 1 'echo hi'
```

## Not done yet

- **QR scanning.** The app takes a pasted code for now. Next step is
  `tauri-plugin-barcode-scanner` on mobile.
- **Push notifications** when an agent starts waiting. The gateway sees the status change,
  but APNs/FCM delivery needs a small native plugin and a push relay.
- **Keychain / Keystore** for the phone's key. It lives in the app's private data
  directory today.
- **Remote machines.** The gateway serves the local machine's panes. Panes the desktop
  reaches over SSH aren't exposed yet.
- **Take over.** Attaching a pane at the phone's size, for when the desktop isn't in use.
- **A self-hosted iroh relay** for production, instead of n0's public ones.

import "@xterm/xterm/css/xterm.css";
import "./style.css";

import { Terminal } from "@xterm/xterm";

import * as api from "./api";
import type { AgentView, Host, LinkInfo, PaneView, Tree } from "./api";

const app = document.getElementById("app")!;

// ---------------------------------------------------------------------------
// A tiny DOM helper. Three screens do not justify a framework, and the
// terminal — the one heavy view — is xterm.js either way.

type Child = Node | string | null | undefined | false;

function h<K extends keyof HTMLElementTagNameMap>(
  tag: K,
  props: Partial<HTMLElementTagNameMap[K]> & { class?: string } = {},
  ...children: Child[]
): HTMLElementTagNameMap[K] {
  const el = document.createElement(tag);
  const { class: cls, ...rest } = props;
  if (cls) el.className = cls;
  Object.assign(el, rest);
  for (const c of children) if (c) el.append(c);
  return el;
}

function show(...children: Child[]) {
  app.replaceChildren(...(children.filter(Boolean) as (Node | string)[]));
}

function errorText(e: unknown) {
  return typeof e === "string" ? e : e instanceof Error ? e.message : String(e);
}

// Each screen registers what to do when the app comes back from the
// background, and how to let go of its streams when it is left.
let onResume: (() => void) | null = null;
let onLeave: (() => void) | null = null;

function leave() {
  onLeave?.();
  onLeave = null;
  onResume = null;
}

document.addEventListener("visibilitychange", () => {
  if (document.visibilityState === "visible") onResume?.();
});

// ---------------------------------------------------------------------------
// Machines

async function hostsScreen() {
  leave();
  const list = h("div", { class: "list" });
  const status = h("p", { class: "status" });
  show(
    h("header", {}, h("h1", {}, "tty7")),
    h("main", {}, list, pairForm(status), status),
  );

  const hosts = await api.hosts();
  if (hosts.length === 0) {
    list.append(
      h(
        "p",
        { class: "hint" },
        "No machines yet. On your computer run ",
        h("code", {}, "tty7-gateway pair"),
        " and paste the code below.",
      ),
    );
  }
  for (const host of hosts) {
    list.append(
      h(
        "button",
        { class: "row", onclick: () => hostScreen(host) },
        h("span", { class: "row-title" }, host.name),
        h("span", { class: "row-sub" }, host.id.slice(0, 12)),
      ),
    );
  }
}

function pairForm(status: HTMLElement) {
  const code = h("textarea", {
    class: "code",
    placeholder: "tty7pair:…",
    rows: 3,
    autocapitalize: "off",
    spellcheck: false,
  });
  const name = h("input", {
    class: "field",
    value: guessDeviceName(),
    placeholder: "This phone's name",
  });
  const button = h("button", { class: "primary" }, "Pair");
  button.onclick = async () => {
    button.disabled = true;
    status.textContent = "Pairing…";
    try {
      const host = await api.pair(code.value.trim(), name.value.trim() || "phone");
      status.textContent = "";
      hostScreen(host);
    } catch (e) {
      status.textContent = errorText(e);
    } finally {
      button.disabled = false;
    }
  };
  return h("section", { class: "pair" }, h("h2", {}, "Pair a machine"), code, name, button);
}

function guessDeviceName() {
  const ua = navigator.userAgent;
  if (/iPhone/.test(ua)) return "iPhone";
  if (/iPad/.test(ua)) return "iPad";
  if (/Android/.test(ua)) return "Android";
  return "tty7 mobile";
}

// ---------------------------------------------------------------------------
// One machine: its agents first, then everything else.

function hostScreen(host: Host) {
  leave();
  const link = h("span", { class: "link" }, "connecting…");
  const body = h("main", {}, h("p", { class: "status" }, "Connecting…"));
  const back = h("button", { class: "back", onclick: () => hostsScreen() }, "‹");
  const menu = h("button", { class: "icon", title: "Forget this machine" }, "⋯");
  menu.onclick = async () => {
    if (confirm(`Forget ${host.name}? You will need a new pairing code.`)) {
      await api.forget(host.id);
      hostsScreen();
    }
  };
  show(h("header", {}, back, h("h1", {}, host.name), link, menu), body);

  let alive = true;
  let lastTree: Tree | null = null;
  onLeave = () => {
    alive = false;
  };

  const start = async () => {
    try {
      await api.watch(host.id, (msg) => {
        if (!alive) return;
        switch (msg.type) {
          case "tree":
            lastTree = msg.tree;
            body.replaceChildren(...renderTree(host, msg.tree));
            break;
          case "link":
            renderLink(link, msg.link);
            break;
          case "error":
            body.prepend(h("p", { class: "status error" }, msg.message));
            break;
          case "closed":
            link.textContent = "offline";
            link.className = "link offline";
            break;
        }
      });
    } catch (e) {
      if (!alive) return;
      link.textContent = "offline";
      link.className = "link offline";
      body.replaceChildren(
        h("p", { class: "status error" }, errorText(e)),
        h("button", { class: "primary", onclick: start }, "Retry"),
        ...(lastTree ? renderTree(host, lastTree) : []),
      );
    }
  };
  // Back from the background the stream may be dead, or fine and merely
  // behind: watching again covers both, since the gateway sends the whole
  // tree on every new watch.
  onResume = start;
  start();
}

function renderLink(el: HTMLElement, link: LinkInfo) {
  el.className = `link ${link.path}`;
  el.textContent =
    link.path === "connecting" ? "connecting…" : `${link.path} · ${link.rtt_ms} ms`;
}

function renderTree(host: Host, tree: Tree): Node[] {
  const out: Node[] = [];
  const waiting: [string, PaneView][] = [];
  for (const ws of tree.workspaces)
    for (const tab of ws.tabs)
      for (const pane of tab.panes)
        if (pane.agent && (pane.agent.status === "waiting" || pane.agent.status === "done"))
          waiting.push([`${ws.name} · ${tab.name}`, pane]);

  if (waiting.length) {
    out.push(
      h(
        "section",
        { class: "needs-you" },
        h("h2", {}, "Needs you"),
        ...waiting.map(([where, pane]) => paneRow(host, pane, where)),
      ),
    );
  }
  for (const ws of tree.workspaces) {
    out.push(
      h(
        "section",
        { class: "workspace" },
        h("h2", {}, ws.name),
        ...ws.tabs.map((tab) =>
          h(
            "div",
            { class: tab.hibernated ? "tab asleep" : "tab" },
            h("div", { class: "tab-name" }, tab.name, tab.hibernated && h("span", { class: "tag" }, "asleep")),
            ...tab.panes.map((pane) => paneRow(host, pane, null)),
          ),
        ),
      ),
    );
  }
  if (tree.workspaces.length === 0) {
    out.push(h("p", { class: "hint" }, "No workspaces on this machine."));
  }
  return out;
}

function paneRow(host: Host, pane: PaneView, where: string | null) {
  return h(
    "button",
    { class: "row pane", onclick: () => terminalScreen(host, pane) },
    h(
      "span",
      { class: "row-title" },
      pane.agent && agentDot(pane.agent),
      pane.title,
    ),
    h("span", { class: "row-sub" }, where ?? pane.agent?.message ?? pane.cwd ?? ""),
    where && pane.agent?.message && h("span", { class: "row-sub" }, pane.agent.message),
  );
}

function agentDot(agent: AgentView) {
  return h("span", { class: `dot ${agent.status}`, title: `${agent.kind}: ${agent.status}` });
}

// ---------------------------------------------------------------------------
// A terminal. The pane keeps the size its desktop window gave it — the phone
// watches rather than attaches — so the font shrinks to fit the width.

const KEYS: [string, string][] = [
  ["esc", "\x1b"],
  ["tab", "\t"],
  ["ctrl", ""],
  ["↑", "\x1b[A"],
  ["↓", "\x1b[B"],
  ["←", "\x1b[D"],
  ["→", "\x1b[C"],
  ["^C", "\x03"],
  ["|", "|"],
  ["/", "/"],
  ["~", "~"],
  ["-", "-"],
];

function terminalScreen(host: Host, pane: PaneView) {
  leave();
  const title = h("h1", {}, pane.title);
  const state = h("span", { class: "link" });
  const back = h("button", { class: "back", onclick: () => hostScreen(host) }, "‹");
  const screen = h("div", { class: "term" });
  const keybar = h("div", { class: "keybar" });
  show(h("header", {}, back, title, state), screen, keybar);

  const term = new Terminal({
    cols: 80,
    rows: 24,
    fontSize: 12,
    fontFamily: "Menlo, 'SF Mono', ui-monospace, monospace",
    scrollback: 5000,
    cursorBlink: false,
    theme: { background: "#0d0f14", foreground: "#d6d9e0" },
  });
  term.open(screen);

  let handle: number | null = null;
  let ctrl = false;
  let cols = 80;
  const ctrlKey = document.createElement("button");

  const send = (data: string) => {
    if (handle === null) return;
    if (ctrl && data.length === 1) {
      const c = data.toUpperCase().charCodeAt(0);
      if (c >= 64 && c <= 95) data = String.fromCharCode(c - 64);
      ctrl = false;
      ctrlKey.classList.remove("on");
    }
    api.paneInput(handle, data).catch(() => {});
  };
  term.onData(send);

  for (const [label, seq] of KEYS) {
    const key = label === "ctrl" ? ctrlKey : document.createElement("button");
    key.textContent = label;
    // Keep focus in the terminal so the soft keyboard stays up.
    key.onpointerdown = (e) => e.preventDefault();
    key.onclick = () => {
      if (label === "ctrl") {
        ctrl = !ctrl;
        key.classList.toggle("on", ctrl);
      } else {
        send(seq);
      }
      term.focus();
    };
    keybar.append(key);
  }

  // Fits the pane's width to the screen. The row count follows the pane too,
  // and the view scrolls vertically if it is taller than what is left.
  const fit = () => {
    const width = screen.clientWidth - 8;
    const probe = 0.6; // a monospace cell is ~0.6em wide
    const size = Math.max(5, Math.min(14, Math.floor((width / cols / probe) * 10) / 10));
    term.options.fontSize = size;
  };
  window.addEventListener("resize", fit);

  let alive = true;
  const open = async () => {
    state.textContent = "connecting…";
    state.className = "link connecting";
    term.reset();
    try {
      handle = await api.paneOpen(
        host.id,
        pane.id,
        (bytes) => term.write(bytes),
        (event) => {
          if (!alive) return;
          switch (event.type) {
            case "size":
              cols = event.cols;
              term.resize(event.cols, event.rows);
              fit();
              break;
            case "agent":
              state.textContent = event.agent ? `${event.agent.kind} · ${event.agent.status}` : "";
              state.className = `link ${event.agent?.status ?? ""}`;
              break;
            case "exited":
              state.textContent = event.code === null ? "closed" : `exited ${event.code}`;
              state.className = "link offline";
              handle = null;
              break;
            case "error":
              state.textContent = event.message;
              state.className = "link offline";
              break;
            case "cwd":
              break;
          }
        },
      );
      if (state.textContent === "connecting…") {
        state.textContent = "";
        state.className = "link";
      }
      term.focus();
    } catch (e) {
      state.textContent = errorText(e);
      state.className = "link offline";
    }
  };

  onLeave = () => {
    alive = false;
    window.removeEventListener("resize", fit);
    if (handle !== null) api.paneClose(handle);
    term.dispose();
  };
  // The pane replays its screen on every open, so coming back from the
  // background is a fresh open onto a reset terminal — never a gap.
  onResume = () => {
    if (handle !== null) api.paneClose(handle);
    handle = null;
    open();
  };
  open();
}

hostsScreen();

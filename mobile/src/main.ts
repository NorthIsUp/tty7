import "@xterm/xterm/css/xterm.css";
import "./style.css";

import { Terminal } from "@xterm/xterm";
import type { ITheme } from "@xterm/xterm";

import * as api from "./api";
import type {
  AgentStatus,
  AgentView,
  Host,
  LinkInfo,
  PaneView,
  TabView,
  Tree,
  WorkspaceView,
} from "./api";
import { agentLook, icon } from "./icons";
import logoUrl from "./assets/logo.svg?url";

const app = document.getElementById("app")!;

// ---------------------------------------------------------------------------
// A tiny DOM helper. Four screens do not justify a framework, and the
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

/** One of the drawn icons, as an element. */
function ico(name: keyof typeof icon, cls = "icon") {
  const el = h("span", { class: cls });
  el.innerHTML = icon[name];
  return el;
}

/** An error from the Rust side, read as a sentence. */
function sentence(text: string) {
  const t = text.trim();
  if (!t) return "";
  return `${t[0].toUpperCase()}${t.slice(1)}${/[.!?]$/.test(t) ? "" : "."}`;
}

function errorText(e: unknown) {
  return typeof e === "string" ? e : e instanceof Error ? e.message : String(e);
}

// ---------------------------------------------------------------------------
// Navigation. Every screen is pushed or popped: the new one slides in over the
// old, the way a phone's own apps move, and back undoes it.

// Each screen registers what to do when the app comes back from the
// background, and how to let go of its streams when it is left.
let onResume: (() => void) | null = null;
let onLeave: (() => void) | null = null;

document.addEventListener("visibilitychange", () => {
  if (document.visibilityState === "visible") onResume?.();
});

const still = matchMedia("(prefers-reduced-motion: reduce)");

function go(direction: "push" | "pop", render: () => HTMLElement) {
  const swap = () => {
    onLeave?.();
    onLeave = null;
    onResume = null;
    app.replaceChildren(render());
  };
  if (!document.startViewTransition || still.matches || !app.firstChild) {
    swap();
    return;
  }
  document.documentElement.dataset.nav = direction;
  document
    .startViewTransition(swap)
    .finished.finally(() => delete document.documentElement.dataset.nav);
}

interface ScreenParts {
  title: string;
  back?: { label: string; onclick: () => void };
  trailing?: Child[];
  /** Under the large title: the connection line on a machine. */
  subtitle?: Child;
  body: Child[];
  footer?: Child;
}

/** A screen with a large title that folds into the bar as it scrolls away. */
function screen(parts: ScreenParts) {
  const bar = h(
    "header",
    { class: "nav" },
    h(
      "div",
      { class: "nav-lead" },
      parts.back &&
        h(
          "button",
          { class: "nav-back", onclick: parts.back.onclick, ariaLabel: `Back to ${parts.back.label}` },
          ico("back"),
          h("span", {}, parts.back.label),
        ),
    ),
    h("div", { class: "nav-title" }, parts.title),
    h("div", { class: "nav-trail" }, ...(parts.trailing ?? [])),
  );
  const large = h("h1", { class: "large-title" }, parts.title);
  const scroll = h(
    "main",
    { class: "scroll" },
    h("div", { class: "title-block" }, large, parts.subtitle),
    ...parts.body,
  );
  new IntersectionObserver(
    ([entry]) => bar.classList.toggle("folded", !entry.isIntersecting),
    { root: scroll, threshold: 0, rootMargin: "-8px 0px 0px 0px" },
  ).observe(large);
  return h("div", { class: "screen" }, bar, scroll, parts.footer);
}

function section(title: Child, ...rows: Child[]) {
  return h(
    "section",
    { class: "group" },
    title && h("h2", { class: "group-title" }, title),
    h("div", { class: "card" }, ...rows),
  );
}

// ---------------------------------------------------------------------------
// Machines

function hostsScreen(direction: "push" | "pop" = "pop") {
  go(direction, () => {
    const body = h("div", { class: "stack" });
    const pairButton = h(
      "button",
      { class: "nav-icon", ariaLabel: "Pair a machine", onclick: () => pairScreen() },
      ico("plus"),
    );
    const view = screen({ title: "Machines", trailing: [pairButton], body: [body] });

    api.hosts().then((hosts) => {
      if (hosts.length === 0) {
        pairButton.hidden = true;
        body.replaceChildren(
          h(
            "div",
            { class: "empty" },
            h("img", { class: "empty-mark", src: logoUrl, alt: "" }),
            h("h2", { class: "empty-title" }, "Pair your computer"),
            h(
              "p",
              { class: "empty-body" },
              "Reach the panes open in tty7 on your computer, and type into them from here.",
            ),
            h("button", { class: "button primary", onclick: () => pairScreen() }, "Pair a machine"),
          ),
        );
        return;
      }
      body.replaceChildren(
        section(
          null,
          ...hosts.map((host) =>
            h(
              "button",
              { class: "row", onclick: () => hostScreen(host, "push") },
              h("span", { class: "tile" }, ico("machine")),
              h("span", { class: "row-text" }, h("span", { class: "row-title" }, host.name)),
              ico("chevron", "icon row-chevron"),
            ),
          ),
        ),
      );
    });
    return view;
  });
}

// ---------------------------------------------------------------------------
// Pairing

function pairScreen() {
  go("push", () => {
    const code = h("textarea", {
      class: "field-input code",
      placeholder: "tty7pair:…",
      rows: 4,
      autocapitalize: "off",
      spellcheck: false,
      ariaLabel: "Pairing code",
    });
    const name = h("input", {
      class: "field-input",
      value: guessDeviceName(),
      placeholder: "This phone",
      ariaLabel: "This phone's name",
      autocapitalize: "words",
    });
    const error = h("p", { class: "field-error", role: "alert" });
    const submit = h("button", { class: "button primary wide" }, "Pair");

    const sync = () => {
      submit.disabled = code.value.trim() === "";
      error.textContent = "";
    };

    const canPaste = typeof navigator.clipboard?.readText === "function";
    const paste = h("button", { class: "chip", hidden: !canPaste }, ico("paste"), "Paste");
    paste.onclick = async () => {
      try {
        code.value = (await navigator.clipboard.readText()).trim();
        sync();
      } catch {
        paste.hidden = true;
        code.focus();
      }
    };

    code.oninput = sync;
    sync();

    submit.onclick = async () => {
      submit.disabled = true;
      submit.classList.add("busy");
      submit.textContent = "Pairing…";
      try {
        const host = await api.pair(code.value.trim(), name.value.trim() || "phone");
        hostScreen(host, "push");
      } catch (e) {
        error.textContent = errorText(e);
        submit.classList.remove("busy");
        submit.textContent = "Pair";
        submit.disabled = false;
      }
    };

    return screen({
      title: "Pair a machine",
      back: { label: "Machines", onclick: () => hostsScreen() },
      body: [
        h(
          "ol",
          { class: "steps" },
          h("li", {}, h("span", {}, "On your computer, run ", h("code", {}, "tty7-gateway pair"), ".")),
          h("li", {}, h("span", {}, "Copy the code it prints and paste it below.")),
        ),
        h(
          "section",
          { class: "group" },
          h("div", { class: "group-head" }, h("h2", { class: "group-title" }, "Pairing code"), paste),
          h("div", { class: "card field" }, code),
          error,
        ),
        h(
          "section",
          { class: "group" },
          h("h2", { class: "group-title" }, "Name"),
          h("div", { class: "card field" }, name),
          h(
            "p",
            { class: "group-note" },
            "How this phone shows up in ",
            h("code", {}, "tty7-gateway devices"),
            ".",
          ),
        ),
      ],
      footer: h("div", { class: "dock" }, submit),
    });
  });
}

function guessDeviceName() {
  const ua = navigator.userAgent;
  if (/iPhone/.test(ua)) return "iPhone";
  if (/iPad/.test(ua)) return "iPad";
  if (/Android/.test(ua)) return "Android";
  return "tty7 mobile";
}

// ---------------------------------------------------------------------------
// One machine: what needs you first, then every workspace as the desktop
// groups it.

/** Tries again after a dropped connection, sooner at first: 1s, 2s, 4s … up
 * to 15s between tries, for as long as the screen is up. */
function retrier(run: () => void) {
  let attempt = 0;
  let timer: number | undefined;
  return {
    schedule() {
      clearTimeout(timer);
      timer = window.setTimeout(run, Math.min(15_000, 1000 * 2 ** attempt++));
    },
    /** It worked: the next drop starts from the shortest wait again. */
    reset() {
      attempt = 0;
      clearTimeout(timer);
    },
    cancel() {
      clearTimeout(timer);
    },
  };
}

/** How long a machine may stay silent before the screen says so. */
const SLOW_CONNECT_MS = 10_000;

function hostScreen(host: Host, direction: "push" | "pop" = "pop") {
  go(direction, () => {
    const link = h("p", { class: "link" });
    renderLink(link, { path: "connecting", rtt_ms: 0 });
    const notice = h("div", { class: "notice-slot" });
    const body = h("div", { class: "stack" }, skeleton());

    let alive = true;
    let lastTree: Tree | null = null;
    let slow: number | undefined;

    const menu = menuButton([
      { label: "Refresh", icon: "refresh", run: () => api.refresh(host.id).catch(() => start()) },
      {
        label: "Forget this machine",
        icon: "trash",
        danger: true,
        run: async () => {
          if (confirm(`Forget ${host.name}? You'll need a new pairing code to reach it again.`)) {
            await api.forget(host.id);
            hostsScreen();
          }
        },
      },
    ]);

    const view = screen({
      title: host.name,
      back: { label: "Machines", onclick: () => hostsScreen() },
      trailing: [menu],
      subtitle: link,
      body: [notice, body],
    });

    const retry = retrier(() => start());
    let offlineNow = false;
    // The notice on screen is the dropped connection's, for a tree to clear.
    let dropped = false;
    // A network that comes back is the moment to try, not the next tick.
    const online = () => {
      if (offlineNow) start();
    };
    window.addEventListener("online", online);

    onLeave = () => {
      alive = false;
      clearTimeout(slow);
      retry.cancel();
      window.removeEventListener("online", online);
    };

    const failed = (message: string) =>
      notice.replaceChildren(noticeCard({ title: "Couldn't open a tab", body: [sentence(message)] }));

    const offline = (message: string) => {
      offlineNow = true;
      dropped = true;
      retry.schedule();
      renderLink(link, null);
      notice.replaceChildren(
        noticeCard({
          title: `Can't reach ${host.name}`,
          body: [
            sentence(message),
            " Check that ",
            h("code", {}, "tty7-gateway serve"),
            ` is running on ${host.name}.`,
          ],
          actions: [
            { label: "Try now", run: start },
            { label: "Pair again", run: () => pairScreen() },
          ],
        }),
      );
      if (!lastTree) body.replaceChildren();
    };

    const start = async () => {
      offlineNow = false;
      retry.cancel();
      clearTimeout(slow);
      // Retrying under a tree, the dropped connection's notice stays up to
      // say why the tree may be stale, until a new one replaces it.
      if (!(lastTree && dropped)) notice.replaceChildren();
      renderLink(link, { path: "connecting", rtt_ms: 0 });
      if (!lastTree) body.replaceChildren(skeleton());
      // A machine that never answers leaves nothing to show but a spinner;
      // say what is likely wrong while still trying.
      slow = window.setTimeout(() => {
        if (alive && !lastTree)
          notice.replaceChildren(
            noticeCard({
              title: `Still looking for ${host.name}`,
              body: [
                "Check that ",
                h("code", {}, "tty7-gateway serve"),
                " is running there. If it restarted or the network changed since you paired, pairing again gives this phone its new address.",
              ],
              tone: "warn",
              actions: [{ label: "Pair again", run: () => pairScreen() }],
            }),
          );
      }, SLOW_CONNECT_MS);
      try {
        await api.watch(host.id, (msg) => {
          if (!alive) return;
          switch (msg.type) {
            case "tree":
              clearTimeout(slow);
              retry.reset();
              if (!lastTree || dropped) notice.replaceChildren();
              dropped = false;
              lastTree = msg.tree;
              body.replaceChildren(...renderTree(host, msg.tree, failed));
              break;
            case "link":
              renderLink(link, msg.link);
              break;
            case "error":
              clearTimeout(slow);
              notice.replaceChildren(noticeCard({ title: msg.message, tone: "warn" }));
              if (!lastTree) body.replaceChildren();
              break;
            case "closed":
              offline("The connection closed.");
              break;
          }
        });
      } catch (e) {
        if (alive) {
          clearTimeout(slow);
          offline(errorText(e));
        }
      }
    };
    // Back from the background the stream may be dead, or fine and merely
    // behind: watching again covers both, since the gateway sends the whole
    // tree on every new watch.
    onResume = start;
    start();
    return view;
  });
}

function renderLink(el: HTMLElement, link: LinkInfo | null) {
  el.className = `link ${link?.path ?? "offline"}`;
  const text =
    link === null
      ? "Offline"
      : link.path === "connecting"
        ? "Connecting…"
        : `${link.path === "direct" ? "Direct" : "Relay"} · ${link.rtt_ms} ms`;
  el.replaceChildren(h("span", { class: "link-dot" }), text);
}

function skeleton() {
  const row = () =>
    h(
      "div",
      { class: "row skeleton" },
      h("span", { class: "avatar" }),
      h("span", { class: "row-text" }, h("span", { class: "bone" }), h("span", { class: "bone short" })),
    );
  return h("div", { class: "group", ariaHidden: "true" }, h("div", { class: "card" }, row(), row(), row()));
}

const STATUS_WORD: Record<AgentStatus, string> = {
  working: "Working",
  waiting: "Needs input",
  done: "Done",
  idle: "Idle",
};

/** Which machine a pane or workspace is on: a remote the desktop is linked
 * to, or null for the paired machine itself. */
type Place = { key: string; name: string } | null;

function renderTree(host: Host, tree: Tree, failed: (message: string) => void): Node[] {
  const out: Node[] = [];
  const remotes = tree.remotes ?? [];

  out.push(...tree.workspaces.map((ws) => workspaceGroup(host, null, ws, failed)));
  if (tree.workspaces.length === 0) {
    out.push(
      h(
        "div",
        { class: "empty" },
        h("span", { class: "tile large" }, ico("terminal")),
        h("h2", { class: "empty-title" }, "Nothing open"),
        h("p", { class: "empty-body" }, `Open a tab in tty7 on ${host.name} and it appears here.`),
      ),
    );
  }

  // Then every machine the desktop reaches over SSH, as the desktop's
  // sidebar lists them: its own heading, its workspaces under it.
  for (const remote of remotes) {
    const state = !remote.connected
      ? "Link down"
      : remote.error
        ? "Not answering"
        : remote.pending
          ? "Reading…"
          : "Connected";
    const tone = !remote.connected || remote.error ? "offline" : remote.pending ? "connecting" : "direct";
    out.push(
      h(
        "section",
        { class: "remote" },
        h(
          "div",
          { class: "remote-head" },
          h("span", { class: "tile" }, ico("server")),
          h(
            "div",
            { class: "remote-titles" },
            h("h2", { class: "remote-name" }, remote.name),
            h(
              "p",
              { class: `link ${tone}` },
              h("span", { class: "link-dot" }),
              `SSH · ${state}`,
            ),
          ),
        ),
        !remote.connected &&
          h(
            "p",
            { class: "remote-note" },
            `tty7 on ${host.name} lost its link to ${remote.name}. Reconnect it there to reach its workspaces.`,
          ),
        remote.error && h("p", { class: "remote-note" }, sentence(remote.error)),
        remote.pending && skeleton(),
        remote.connected &&
          !remote.error &&
          !remote.pending &&
          remote.workspaces.length === 0 &&
          h("p", { class: "remote-note" }, `No workspaces on ${remote.name}.`),
        ...remote.workspaces.map((ws) => workspaceGroup(host, remote, ws, failed)),
      ),
    );
  }
  return out;
}

/** A workspace is one card of its tabs, named as the desktop's sidebar names
 * them; a split tab gives each of its panes a row. */
function workspaceGroup(host: Host, place: Place, ws: WorkspaceView, failed: (message: string) => void) {
  return h(
    "section",
    { class: "group" },
    h(
      "div",
      { class: "group-head" },
      h("h2", { class: "group-title" }, workspaceName(ws.name)),
      newTabButton(host, place, ws, failed),
    ),
    h(
      "div",
      { class: "card" },
      ...ws.tabs.flatMap((tab) => tab.panes.map((pane) => paneRow(host, place, tab, pane))),
    ),
  );
}

/** Starts a shell in a new tab at the end of a workspace, in the directory
 * its last tab is in, sized to this screen, and opens it. */
function newTabButton(host: Host, place: Place, ws: WorkspaceView, failed: (message: string) => void) {
  const button = h("button", { class: "head-action" }, ico("plus"), "New tab");
  button.onclick = async () => {
    button.disabled = true;
    const cwd = ws.tabs.at(-1)?.panes[0]?.cwd ?? null;
    try {
      const created = await api.tabNew(host.id, place?.key ?? null, ws.id, cwd, phoneGrid());
      terminalScreen(host, place, { id: created.pane_id, title: "shell", cwd }, "New tab");
    } catch (e) {
      failed(errorText(e));
      button.disabled = false;
    }
  };
  return button;
}

/** The grid that fills this screen at the readable size: what a tab started
 * here is spawned at, since no desktop window is showing it yet. */
function phoneGrid() {
  const cellW = READABLE_PX * CELL_EM;
  const cellH = READABLE_PX * 1.18;
  // The terminal screen's bar and key bar, and the xterm padding.
  const chrome = 48 + 56 + 16;
  return {
    cols: Math.max(20, Math.floor((window.innerWidth - 12) / cellW)),
    rows: Math.max(5, Math.floor((window.innerHeight - chrome) / cellH)),
  };
}

/** The desktop leaves a workspace unnamed as "-". */
function workspaceName(name: string) {
  return name && name !== "-" ? name : "Untitled workspace";
}

/** A pane's row: its tab's name first, as on the desktop, then what the pane
 * is doing — its agent's state, or where its shell is. */
function paneRow(host: Host, place: Place, tab: TabView, pane: PaneView) {
  const agent = pane.agent;
  const sub: Child[] = [];
  if (agent && agent.status !== "idle")
    sub.push(h("span", { class: `status-word ${agent.status}` }, STATUS_WORD[agent.status]));
  // A tab named after its directory says the cwd already; what tells its
  // panes apart then is what runs in them.
  const namedByPath = /^[~/]/.test(tab.name);
  const split = tab.panes.length > 1 || namedByPath ? pane.title : null;
  const dir = agent || namedByPath ? null : shortPath(pane.cwd);
  const detail = [split, agent?.message ?? dir].filter(Boolean).join(" · ");
  if (detail) sub.push(sub.length ? ` · ${detail}` : detail);
  return h(
    "button",
    {
      class: tab.hibernated ? "row asleep" : "row",
      onclick: () => terminalScreen(host, place, pane, tab.name),
    },
    avatar(agent),
    h(
      "span",
      { class: "row-text" },
      h(
        "span",
        { class: "row-title" },
        tab.name,
        tab.hibernated && h("span", { class: "tag" }, "Asleep"),
      ),
      sub.length > 0 && h("span", { class: "row-sub" }, ...sub),
    ),
    ico("chevron", "icon row-chevron"),
  );
}

/** A pane's avatar, as the desktop's tab strip draws it: the agent's mark on
 * its brand colour, or a terminal, with the status dot as a badge. Waiting is
 * hollow, so it differs from Done in shape and not only in hue. */
function avatar(agent: AgentView | null | undefined, cls = "avatar") {
  const el = h("span", { class: cls });
  if (agent) {
    const look = agentLook(agent.kind);
    el.style.setProperty("--field", look.field);
    el.style.setProperty("--mark-ink", look.ink);
    el.classList.add("agent");
    el.title = `${look.name}: ${STATUS_WORD[agent.status]}`;
    if (look.mark) {
      const mark = h("span", { class: "mark" });
      mark.style.setProperty("--mark", `url("${look.mark}")`);
      el.append(mark);
    } else el.append(ico("terminal"));
    if (agent.status !== "idle") el.append(h("span", { class: `badge ${agent.status}` }));
  } else el.append(ico("terminal"));
  return el;
}

/** The last two segments of a path: the part that tells panes apart. */
function shortPath(path: string | null | undefined) {
  if (!path) return "";
  const parts = path.split("/").filter(Boolean);
  return parts.length <= 2 ? path : `…/${parts.slice(-2).join("/")}`;
}

interface NoticeParts {
  title: string;
  body?: Child[];
  tone?: "warn" | "bad";
  actions?: { label: string; run: () => void }[];
}

function noticeCard({ title, body, tone = "bad", actions = [] }: NoticeParts) {
  return h(
    "div",
    { class: `notice ${tone}`, role: "status" },
    ico("alert", "icon notice-icon"),
    h(
      "div",
      { class: "notice-text" },
      h("p", { class: "notice-title" }, title),
      body && h("p", { class: "notice-body" }, ...body),
      actions.length > 0 &&
        h(
          "div",
          { class: "notice-actions" },
          ...actions.map((a) => h("button", { class: "button tinted small", onclick: a.run }, a.label)),
        ),
    ),
  );
}

interface MenuItem {
  label: string;
  icon: keyof typeof icon;
  danger?: boolean;
  run: () => void;
}

/** A trailing ⋯ button with a small menu that drops from it. */
function menuButton(items: MenuItem[]) {
  const wrap = h("div", { class: "menu-wrap" });
  const list = h("div", { class: "menu", role: "menu", hidden: true });
  const outside = (e: Event) => {
    if (!wrap.contains(e.target as Node)) close();
  };
  const close = () => {
    list.hidden = true;
    document.removeEventListener("pointerdown", outside, true);
  };
  for (const item of items)
    list.append(
      h(
        "button",
        {
          class: item.danger ? "menu-item danger" : "menu-item",
          role: "menuitem",
          onclick: () => {
            close();
            item.run();
          },
        },
        h("span", {}, item.label),
        ico(item.icon),
      ),
    );
  const button = h("button", { class: "nav-icon", ariaLabel: "More" }, ico("more"));
  button.onclick = () => {
    if (!list.hidden) return close();
    list.hidden = false;
    document.addEventListener("pointerdown", outside, true);
  };
  wrap.append(button, list);
  return wrap;
}

// ---------------------------------------------------------------------------
// A terminal. The pane keeps the size its desktop window gave it — the phone
// watches rather than attaches — so the font shrinks to fit the width.

// The desktop's Light and Dark presets (src/ui/presets.rs), so a pane reads
// the same on the phone as in the window it lives in.
const ANSI = {
  light: ["#24292e", "#d1242f", "#1a7f37", "#9a6700", "#0969da", "#8250df", "#1b7c83", "#6e7781", "#57606a", "#cf222e", "#1f883d", "#bf8700", "#218bff", "#a475f9", "#3192aa", "#8c959f"],
  dark: ["#616161", "#ff8272", "#b4fa72", "#fefdc2", "#a5d5fe", "#ff8ffd", "#d0d1fe", "#f1f1f1", "#8e8e8e", "#ffc4bd", "#d6fcb9", "#fefdd5", "#c1e3fe", "#ffb1fe", "#e5e6fe", "#feffff"],
};
const NAMES = ["black", "red", "green", "yellow", "blue", "magenta", "cyan", "white"] as const;

const darkScheme = matchMedia("(prefers-color-scheme: dark)");

function terminalTheme(): ITheme {
  const dark = darkScheme.matches;
  const ansi = dark ? ANSI.dark : ANSI.light;
  const theme: Record<string, string> = dark
    ? { background: "#191b20", foreground: "#e2e5eb", cursor: "#78a8f5", cursorAccent: "#191b20", selectionBackground: "#78a8f555" }
    : { background: "#ffffff", foreground: "#0f1419", cursor: "#1f6bf0", cursorAccent: "#ffffff", selectionBackground: "#1f6bf033" };
  NAMES.forEach((n, i) => {
    theme[n] = ansi[i];
    theme[`bright${n[0].toUpperCase()}${n.slice(1)}`] = ansi[i + 8];
  });
  return theme as ITheme;
}

/** A key on the bar: a sequence to send, the ctrl latch, or the clipboard.
 * `text` is what it shows when that is not its label. */
type Key = {
  label: string;
  seq: string;
  text?: string;
  icon?: keyof typeof icon;
  latch?: boolean;
  paste?: boolean;
};

const KEYS: Key[] = [
  { label: "esc", seq: "\x1b" },
  { label: "tab", seq: "\t" },
  // Claude Code's mode switch, among others.
  { label: "Shift Tab", text: "⇧tab", seq: "\x1b[Z" },
  { label: "ctrl", seq: "", latch: true },
  { label: "^C", seq: "\x03" },
  { label: "Left", seq: "\x1b[D", icon: "left" },
  { label: "Right", seq: "\x1b[C", icon: "right" },
  { label: "Up", seq: "\x1b[A", icon: "up" },
  { label: "Down", seq: "\x1b[B", icon: "down" },
  { label: "Paste", seq: "", icon: "paste", paste: true },
  { label: "|", seq: "|" },
  { label: "/", seq: "/" },
  { label: "~", seq: "~" },
  { label: "-", seq: "-" },
];

/** Hack's advance width, in ems — what the fit divides by. */
const CELL_EM = 0.602;
/** The smallest font a pane is read at before it pans instead of shrinking. */
const READABLE_PX = 11;

/** What was typed into a pane's compose box and not sent yet, by pane. Kept
 * across leaving the pane, and across a send that did not get through. */
const drafts = new Map<string, string>();

function terminalScreen(host: Host, place: Place, pane: PaneView, title: string) {
  go("push", () => {
    // What the pane is doing and whether keystrokes will land, in words: the
    // one line under the title.
    const stateWord = h("span", {}, "Connecting…");
    const state = h("span", { class: "term-state connecting" }, h("span", { class: "link-dot" }), stateWord);
    // On a remote, the path says which machine it is on, the way a prompt does.
    const where = (path: string | null | undefined) =>
      place && path ? `${place.name}:${shortPath(path)}` : shortPath(path);
    const cwd = h("span", { class: "term-cwd" }, where(pane.cwd));
    const sub = h("span", { class: "term-sub" }, state, cwd);
    const zoom = h("button", { class: "nav-icon", ariaLabel: "Fit the whole width" }, ico("fit"));
    const select = h("button", { class: "nav-icon", ariaLabel: "Select text" }, ico("copy"));
    const take = h("button", { class: "nav-icon", ariaLabel: "Use this phone's screen size" }, ico("phone"));
    let head = avatar(pane.agent, "avatar small");
    const bar = h(
      "header",
      { class: "nav term-nav" },
      h(
        "div",
        { class: "nav-lead" },
        h("button", { class: "nav-back", ariaLabel: `Back to ${host.name}`, onclick: () => hostScreen(host) }, ico("back")),
      ),
      h(
        "div",
        { class: "term-head" },
        head,
        h("div", { class: "term-titles" }, h("span", { class: "term-title" }, title), sub),
      ),
      h("div", { class: "nav-trail" }, select, take, zoom),
    );
    const screenEl = h("div", { class: "term" });
    const banner = h("div", { class: "term-banner-slot" });
    // Selecting in xterm itself does not work by touch. The pane's text is
    // laid out here instead, as a plain page the phone selects in its own
    // way — handles, magnifier, Copy.
    const copyText = h("pre", { class: "term-copy-text" });
    const copyDone = h("button", { class: "button tinted small" }, "Done");
    const copyView = h(
      "div",
      { class: "term-copy", hidden: true },
      h("div", { class: "term-copy-bar" }, h("span", {}, "Select text to copy"), copyDone),
      copyText,
    );
    const keys = h("div", { class: "keys" });
    const keyboard = h("button", { class: "key key-kbd", ariaLabel: "Show or hide the keyboard" }, ico("keyboard"));
    const composeKey = h("button", { class: "key key-kbd", ariaLabel: "Write a message" }, ico("compose"));
    const keybar = h("div", { class: "keybar" }, keys, composeKey, keyboard);

    // The compose box: a real text field, so the phone's own keyboard works
    // in full — autocorrect, dictation, an IME's candidates, moving the
    // caret — and what is written goes in whole, then Enter.
    const draftKey = `${host.id}/${place?.key ?? ""}/${pane.id}`;
    const field = h("textarea", {
      class: "compose-input",
      rows: 1,
      value: drafts.get(draftKey) ?? "",
      placeholder: pane.agent ? `Reply to ${agentLook(pane.agent.kind).name}` : "Type a command",
      enterKeyHint: "send",
      ariaLabel: "Message",
    });
    const sendKey = h("button", { class: "compose-send", ariaLabel: "Send" }, ico("send"));
    const compose = h("div", { class: "compose", hidden: true }, field, sendKey);

    const view = h(
      "div",
      { class: "screen term-screen" },
      bar,
      h("div", { class: "term-wrap" }, screenEl, copyView, banner),
      compose,
      keybar,
    );

    const term = new Terminal({
      cols: 80,
      rows: 24,
      fontSize: 11,
      fontFamily: "Hack, Menlo, ui-monospace, monospace",
      scrollback: 5000,
      cursorBlink: false,
      theme: terminalTheme(),
    });

    let handle: number | null = null;
    // Whether keystrokes land: set by a successful open, cleared by anything
    // that says they no longer do.
    let live = false;
    let ctrl = false;
    let cols = 80;
    let alive = true;
    let composing = false;
    // The pane exited: nothing to reconnect to.
    let ended = false;
    // The phone's size: `wanted` is what the user asked for, `leased` what
    // the daemon confirmed, `sent` the grid last asked for.
    let wanted = false;
    let leased = false;
    let releasing = false;
    let sent = "";
    let ctrlKey: HTMLButtonElement | null = null;

    // Typing that does not get through is said, never dropped quietly: the
    // pane goes offline with a way back, rather than looking live.
    // A drop is retried on its own, sooner at first; the pane stays on
    // screen as it was until the new stream replaces it.
    const retry = retrier(() => reopen());
    const offline = (message: string) => {
      live = false;
      setState("connecting", "Reconnecting");
      showBanner(`${sentence(message)} Reconnecting…`, { label: "Try now", run: reopen });
      retry.schedule();
    };
    // Only the first refusal speaks: a key typed just before it fails on its
    // own, with a vaguer reason.
    const refused = (message: string) => {
      if (alive && live) offline(message);
    };
    const input = (data: string): Promise<boolean> => {
      if (handle === null || !live) return Promise.resolve(false);
      return api.paneInput(handle, data).then(
        () => true,
        (e) => {
          refused(errorText(e));
          return false;
        },
      );
    };
    const send = (data: string) => {
      if (ctrl && data.length === 1) {
        const c = data.toUpperCase().charCodeAt(0);
        if (c >= 64 && c <= 95) data = String.fromCharCode(c - 64);
        ctrl = false;
        ctrlKey?.classList.remove("on");
      }
      void input(data);
    };
    term.onData(send);

    const canPaste = typeof navigator.clipboard?.readText === "function";
    const paste = async () => {
      let text: string;
      try {
        text = await navigator.clipboard.readText();
      } catch {
        return; // Declined at the system's paste prompt.
      }
      if (!text) return;
      if (composing) {
        field.setRangeText(text, field.selectionStart, field.selectionEnd, "end");
        edited();
      } else term.paste(text);
    };

    for (const k of KEYS) {
      if (k.paste && !canPaste) continue;
      const key = h("button", { class: "key", ariaLabel: k.label }, k.icon ? ico(k.icon) : (k.text ?? k.label));
      if (k.latch) ctrlKey = key;
      // Keep focus where it is — the terminal or the compose box — so the
      // soft keyboard stays up.
      key.onpointerdown = (e) => e.preventDefault();
      key.onclick = () => {
        if (k.latch) {
          ctrl = !ctrl;
          key.classList.toggle("on", ctrl);
        } else if (k.paste) void paste();
        else send(k.seq);
        if (!composing) term.focus();
      };
      keys.append(key);
    }

    const grow = () => {
      // Off the page it measures nothing; the first fit comes once it is on.
      if (!field.isConnected) return;
      field.style.height = "auto";
      field.style.height = `${field.scrollHeight}px`;
    };
    const edited = () => {
      if (field.value) drafts.set(draftKey, field.value);
      else drafts.delete(draftKey);
      grow();
    };
    field.addEventListener("input", edited);
    const submit = async () => {
      const body = field.value.replace(/\r?\n/g, "\r");
      // Several lines go in as one paste where the program asked for that,
      // so an agent takes them as one message and a shell does not run each.
      const data = body.includes("\r") && term.modes.bracketedPasteMode ? `\x1b[200~${body}\x1b[201~` : body;
      if (data && !(await input(data))) return;
      // Enter as its own write: a program that tells pasting from typing by
      // how the bytes arrive would otherwise take it as part of the text.
      // An empty box sends Enter alone.
      if (!(await input("\r"))) return;
      field.value = "";
      edited();
    };
    field.addEventListener("keydown", (e) => {
      // Enter that confirms an IME's candidate is the IME's, not a send.
      if (e.key !== "Enter" || e.shiftKey || e.isComposing || e.keyCode === 229) return;
      e.preventDefault();
      void submit();
    });
    sendKey.onpointerdown = (e) => e.preventDefault();
    sendKey.onclick = () => void submit();

    const setComposing = (on: boolean, focus: boolean) => {
      composing = on;
      compose.hidden = !on;
      composeKey.classList.toggle("on", on);
      composeKey.ariaLabel = on ? "Type into the terminal" : "Write a message";
      if (!on) {
        field.blur();
        if (focus) term.focus();
        return;
      }
      grow();
      if (focus) field.focus();
    };
    composeKey.onpointerdown = (e) => e.preventDefault();
    composeKey.onclick = () => setComposing(!composing, true);
    // An agent's pane opens on the box, keyboard down: read first, then reply.
    if (pane.agent) setComposing(true, false);
    keyboard.onpointerdown = (e) => e.preventDefault();
    keyboard.onclick = () => {
      const active = document.activeElement;
      if (screenEl.contains(active)) term.blur();
      else if (active === field) field.blur();
      else if (composing) field.focus();
      else term.focus();
    };

    // Two ways to show a pane wider than the phone. Fitted, the font shrinks
    // until every column shows; readable, the font stays legible and the view
    // pans sideways, following the cursor. A pane that fits legibly is simply
    // fitted, and the toggle has nothing to offer.
    let readable = true;
    const fittedSize = () => {
      const width = screenEl.clientWidth - 12;
      return width <= 0 ? READABLE_PX : Math.min(14, Math.floor((width / cols / CELL_EM) * 10) / 10);
    };
    const fit = () => {
      // Taken over, the pane is exactly the phone's width at the readable
      // size: nothing to shrink, nothing to pan.
      if (leased) {
        zoom.hidden = true;
        term.options.fontSize = READABLE_PX;
        screenEl.classList.remove("panning");
        return;
      }
      const fitted = fittedSize();
      const cramped = fitted < READABLE_PX;
      zoom.hidden = !cramped;
      term.options.fontSize = cramped && readable ? READABLE_PX : Math.max(4, fitted);
      screenEl.classList.toggle("panning", cramped && readable);
      zoom.replaceChildren(ico(readable ? "fit" : "zoom"));
      zoom.ariaLabel = readable ? "Fit the whole width" : "Make the text readable";
      follow();
    };
    const follow = () => {
      if (!screenEl.classList.contains("panning")) return;
      const cell = screenEl.scrollWidth / cols;
      const x = term.buffer.active.cursorX * cell;
      const view = screenEl.clientWidth;
      if (x < screenEl.scrollLeft + cell * 4 || x > screenEl.scrollLeft + view - cell * 4)
        screenEl.scrollLeft = Math.max(0, x - view / 2);
    };
    zoom.onclick = () => {
      readable = !readable;
      fit();
    };

    // Taking the pane over (the lets are with the others above): it runs at
    // the phone's grid while this screen is open, and the desktop keeps its
    // window and can take it back.
    const takeGrid = () => {
      const width = screenEl.clientWidth - 12;
      const height = screenEl.clientHeight - 16;
      // A row's height, measured off what xterm drew and scaled to the size
      // the pane will be read at; the metrics guess only before the first draw.
      const drawn = screenEl.querySelector<HTMLElement>(".xterm-screen");
      const now = term.options.fontSize ?? READABLE_PX;
      const rowH =
        drawn && term.rows && drawn.clientHeight
          ? ((drawn.clientHeight / term.rows) * READABLE_PX) / now
          : READABLE_PX * 1.18;
      return {
        cols: Math.max(20, Math.floor(width / (READABLE_PX * CELL_EM))),
        rows: Math.max(5, Math.floor(height / rowH)),
      };
    };
    const askLease = () => {
      if (handle === null || !live || !wanted) return;
      const grid = takeGrid();
      const key = `${grid.cols}x${grid.rows}`;
      if (leased && key === sent) return;
      sent = key;
      api.paneLease(handle, grid).catch(() => {});
    };
    const showTake = () => {
      take.classList.toggle("on", wanted);
      take.ariaLabel = wanted ? "Give the pane back to the desktop's size" : "Use this phone's screen size";
    };
    take.onclick = () => {
      wanted = !wanted;
      showTake();
      if (wanted) {
        sent = "";
        askLease();
      } else if (handle !== null && leased) {
        releasing = true;
        api.paneLease(handle, null).catch(() => {});
      }
    };
    // The keyboard coming up or the phone turning changes the grid; asked
    // for once it settles, not on every step of the animation.
    let regrid: number | undefined;
    const regridSoon = () => {
      clearTimeout(regrid);
      if (wanted) regrid = window.setTimeout(askLease, 250);
    };
    const leaseEvent = (held: boolean, refused: string | null | undefined) => {
      const was = leased;
      leased = held;
      if (held) {
        banner.replaceChildren();
      } else {
        sent = "";
        const ours = releasing;
        releasing = false;
        if (refused) {
          wanted = false;
          showBanner(sentence(refused));
        } else if (was && !ours) {
          // Nobody here let go: the desktop took it back, or another device
          // took it over. Asking again is one tap, not automatic.
          wanted = false;
          showBanner("The desktop took this pane back.", {
            label: "Take over again",
            run: () => {
              banner.replaceChildren();
              take.click();
            },
          });
        }
        showTake();
      }
      fit();
    };
    term.onCursorMove(follow);
    // The whole buffer, scrollback included, as lines of text. A row the
    // terminal wrapped joins the one before it, so a long URL or path comes
    // out whole; its trailing blanks are real and kept.
    const bufferText = () => {
      const buf = term.buffer.active;
      const lines: string[] = [];
      for (let y = 0; y < buf.length; y++) {
        const line = buf.getLine(y);
        if (!line) continue;
        const text = line.translateToString(!buf.getLine(y + 1)?.isWrapped);
        if (line.isWrapped && lines.length) lines[lines.length - 1] += text;
        else lines.push(text);
      }
      while (lines.length && !lines[lines.length - 1].trim()) lines.pop();
      return lines.join("\n");
    };
    const selecting = (on: boolean) => {
      copyView.hidden = !on;
      select.classList.toggle("on", on);
      if (!on) {
        getSelection()?.removeAllRanges();
        copyText.textContent = "";
        return;
      }
      term.blur();
      field.blur();
      copyText.textContent = bufferText();
      copyText.scrollTop = copyText.scrollHeight;
    };
    select.onclick = () => selecting(copyView.hidden);
    copyDone.onclick = () => selecting(false);

    const retheme = () => {
      term.options.theme = terminalTheme();
    };
    window.addEventListener("resize", fit);
    window.addEventListener("resize", regridSoon);
    darkScheme.addEventListener("change", retheme);

    const setState = (cls: string, label: string) => {
      state.className = `term-state ${cls}`;
      stateWord.textContent = label;
    };
    const showBanner = (text: string, action?: { label: string; run: () => void }) =>
      banner.replaceChildren(
        h(
          "div",
          { class: "term-banner" },
          h("span", {}, text),
          action && h("button", { class: "button tinted small", onclick: action.run }, action.label),
        ),
      );

    // Which open is current. A stream that was replaced may still deliver
    // its last bytes or its error; those are not this pane's any more.
    let generation = 0;
    const open = async () => {
      const mine = ++generation;
      const current = () => alive && mine === generation;
      // The screen is cleared for the replay when it starts, not before, so a
      // retry that fails leaves the last good screen up.
      let replayed = false;
      const replay = () => {
        if (replayed) return;
        replayed = true;
        term.reset();
      };
      live = false;
      retry.cancel();
      if (!banner.hasChildNodes()) setState("connecting", "Connecting");
      try {
        const opened = await api.paneOpen(
          host.id,
          place?.key ?? null,
          pane.id,
          (bytes) => {
            if (!current()) return;
            replay();
            term.write(bytes);
          },
          (event) => {
            if (!current()) return;
            if (event.type === "size") replay();
            switch (event.type) {
              case "size":
                cols = event.cols;
                term.resize(event.cols, event.rows);
                fit();
                break;
              case "agent": {
                const next = avatar(event.agent, "avatar small");
                head.replaceWith(next);
                head = next;
                break;
              }
              case "cwd":
                cwd.textContent = where(event.path);
                break;
              case "exited":
                ended = true;
                live = false;
                handle = null;
                retry.cancel();
                setState("offline", "Closed");
                showBanner(
                  event.code === null ? "This pane closed." : `This pane exited with code ${event.code}.`,
                );
                break;
              case "error":
                offline(event.message);
                break;
              case "lease":
                leaseEvent(event.held, event.refused);
                break;
            }
          },
        );
        if (!current()) {
          api.paneClose(opened);
          return;
        }
        handle = opened;
        live = true;
        // A new stream starts at the desktop's size; the lease the old one
        // held ended with it. Asked for again if it is still wanted.
        leased = false;
        sent = "";
        askLease();
        retry.reset();
        banner.replaceChildren();
        setState("live", "Live");
        if (!composing) term.focus();
      } catch (e) {
        if (current()) offline(errorText(e));
      }
    };
    const reopen = () => {
      if (handle !== null) api.paneClose(handle);
      handle = null;
      open();
    };
    const online = () => {
      if (!live && !ended) reopen();
    };
    window.addEventListener("online", online);

    onLeave = () => {
      alive = false;
      retry.cancel();
      window.removeEventListener("online", online);
      window.removeEventListener("resize", fit);
      window.removeEventListener("resize", regridSoon);
      clearTimeout(regrid);
      darkScheme.removeEventListener("change", retheme);
      if (handle !== null) api.paneClose(handle);
      term.dispose();
    };
    // The pane replays its screen on every open, so coming back from the
    // background is a fresh open onto a reset terminal — never a gap.
    onResume = reopen;

    // Hack has to be loaded before xterm measures a cell, or the first fit is
    // taken with the fallback face's metrics.
    requestAnimationFrame(() => {
      document.fonts.load("12px Hack").finally(() => {
        if (!alive) return;
        term.open(screenEl);
        fit();
        grow();
        open();
      });
    });
    return view;
  });
}

hostsScreen("push");

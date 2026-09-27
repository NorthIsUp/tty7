// The app's icons, drawn in the desktop's grammar: 24-unit box, 1.8 stroke,
// round caps and joins. Agent marks are the desktop's own files.

const stroke = (body: string, width = 1.8) =>
  `<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="${width}" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">${body}</svg>`;

export const icon = {
  back: stroke(`<path d="M15 5 8 12l7 7"/>`, 2.2),
  chevron: stroke(`<path d="m9.5 6 6 6-6 6"/>`, 2),
  plus: stroke(`<path d="M12 5v14M5 12h14"/>`, 2),
  more: `<svg viewBox="0 0 24 24" fill="currentColor" aria-hidden="true"><circle cx="5.5" cy="12" r="1.7"/><circle cx="12" cy="12" r="1.7"/><circle cx="18.5" cy="12" r="1.7"/></svg>`,
  terminal: stroke(`<path d="M5.8 8.4 9.7 12l-3.9 3.6"/><path d="M12.9 15.6h5.6"/>`),
  machine: stroke(`<rect x="3" y="4" width="18" height="13" rx="2"/><path d="M8 21h8M12 17v4"/>`),
  keyboard: stroke(
    `<rect x="2.5" y="6" width="19" height="12" rx="2.5"/><path d="M6.5 10h.01M10 10h.01M14 10h.01M17.5 10h.01M8 14h8"/>`,
  ),
  up: stroke(`<path d="M12 18V6M6.5 11.5 12 6l5.5 5.5"/>`, 2),
  down: stroke(`<path d="M12 6v12M6.5 12.5 12 18l5.5-5.5"/>`, 2),
  left: stroke(`<path d="M18 12H6M11.5 6.5 6 12l5.5 5.5"/>`, 2),
  right: stroke(`<path d="M6 12h12M12.5 6.5 18 12l-5.5 5.5"/>`, 2),
  fit: stroke(`<path d="M4 9V5h4M20 9V5h-4M4 15v4h4M20 15v4h-4"/><path d="M8.5 12h7"/>`),
  zoom: stroke(`<circle cx="11" cy="11" r="6"/><path d="m20 20-4.2-4.2M8.5 11h5M11 8.5v5"/>`),
  refresh: stroke(`<path d="M20 11a8 8 0 1 0-2.3 5.7"/><path d="M20 5v6h-6"/>`),
  trash: stroke(`<path d="M4 7h16M9.5 7V4.5h5V7M6.5 7l1 13h9l1-13"/>`),
  paste: stroke(
    `<rect x="6" y="4.5" width="12" height="16" rx="2"/><path d="M9.5 4.5V3.5h5v1M9.5 10.5h5M9.5 14.5h5"/>`,
  ),
  alert: stroke(`<path d="M12 4 2.8 19.5h18.4Z"/><path d="M12 10v4M12 17h.01"/>`),
};

/** Which desktop mark and colours an agent wears — `accent_rgb`,
 * `icon_rgb` and `icon_path` in tty7-core's cli_agent.rs. */
const AGENTS: Record<string, { name: string; field: string; ink?: string; mark?: string }> = {
  claude: { name: "Claude Code", field: "#D97757" },
  codex: { name: "Codex", field: "#000000" },
  traecli: { name: "TraeCode", field: "#000000", ink: "#32F08C" },
  gemini: { name: "Gemini", field: "#4285F4" },
  aider: { name: "Aider", field: "#14B014", mark: "" },
  amp: { name: "Amp", field: "#F34E3F" },
  opencode: { name: "OpenCode", field: "#6E56CF" },
  copilot: { name: "Copilot", field: "#8957E5" },
  cursor: { name: "Cursor", field: "#9AA0A6" },
  goose: { name: "Goose", field: "#3ECC5F" },
  droid: { name: "Droid", field: "#EF6F2E" },
  pi: { name: "Pi", field: "#0EA5E9" },
  auggie: { name: "Auggie", field: "#16A34A", mark: "" },
  hermes: { name: "Hermes", field: "#8B5CF6", mark: "" },
  vibe: { name: "Vibe", field: "#FA520F", mark: "" },
  antigravity: { name: "Antigravity", field: "#3186FF", mark: "" },
  grok: { name: "Grok", field: "#000000" },
  qwen: { name: "Qwen Code", field: "#6D44E8" },
  omp: { name: "Oh My Pi", field: "#F97316" },
  kimi: { name: "Kimi Code", field: "#027AFF" },
  qodercli: { name: "Qoder CLI", field: "#FFFFFF", ink: "#000000" },
  crush: { name: "Crush", field: "#6B50FF" },
  codebuddy: { name: "CodeBuddy", field: "#1F1F1F" },
};

const marks = import.meta.glob<string>("./assets/agents/*.svg", {
  query: "?url",
  import: "default",
  eager: true,
});

export interface AgentLook {
  name: string;
  field: string;
  ink: string;
  /** URL of the mark, drawn as a mask so it takes `ink`; null draws the
   * terminal glyph instead, as the desktop does for agents with no mark. */
  mark: string | null;
}

export function agentLook(kind: string): AgentLook {
  const known = AGENTS[kind];
  const mark = marks[`./assets/agents/${kind}.svg`] ?? null;
  return {
    name: known?.name ?? kind,
    field: known?.field ?? "#6B7280",
    ink: known?.ink ?? "#FFFFFF",
    mark: known?.mark === "" ? null : mark,
  };
}

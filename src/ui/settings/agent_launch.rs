//! Settings → Agents: an agent's launch options, compact under its
//! integration row — always for an agent whose hooks are installed, from the
//! row's menu for the rest — its model and permission controls
//! ([`CLIAgent::model_flag`], [`CLIAgent::launch_choices`]) and its extra
//! argument lines, kept in `ForkConfig::agent_settings`.

use std::collections::{HashMap, HashSet};

use gpui::{EntityId, Global};

use crate::core::cli_agent::CLIAgent;
use crate::core::fork_config::AgentLaunchSettings;

use super::kit::{self, BtnKind, Tk, fs};
use super::shell::MenuEntry;
use super::*;

/// One agent's text fields on the page.
struct AgentInputs {
    model: Entity<InputState>,
    extras: Vec<Entity<InputState>>,
    _subs: Vec<Subscription>,
}

/// One window's open Launch options and their fields, by agent slug.
#[derive(Default)]
struct LaunchUi {
    open: HashSet<&'static str>,
    inputs: HashMap<&'static str, AgentInputs>,
}

/// Each window's [`LaunchUi`], by its app.
#[derive(Default)]
struct AgentLaunchUi(HashMap<EntityId, LaunchUi>);

impl Global for AgentLaunchUi {}

fn settings_key(cfg: &Config, agent: CLIAgent) -> Option<&String> {
    cfg.fork
        .agent_settings
        .keys()
        .find(|k| CLIAgent::from_slug(k) == Some(agent))
}

fn settings_of(cfg: &Config, agent: CLIAgent) -> AgentLaunchSettings {
    settings_key(cfg, agent)
        .and_then(|k| cfg.fork.agent_settings.get(k))
        .cloned()
        .unwrap_or_default()
}

/// Store `settings` as `agent`'s, dropping the entry when it adds nothing.
fn set_settings(cfg: &mut Config, agent: CLIAgent, settings: AgentLaunchSettings) {
    cfg.fork
        .agent_settings
        .retain(|k, _| CLIAgent::from_slug(k) != Some(agent));
    if settings != AgentLaunchSettings::default() {
        cfg.fork
            .agent_settings
            .insert(agent.slug().to_string(), settings);
    }
}

fn choice_title(key: &str) -> L10nKey {
    match key {
        "sandbox" => L10nKey::SettingsAgentSandbox,
        _ => L10nKey::SettingsAgentPermission,
    }
}

impl Tty7App {
    /// Whether `agent`'s row has its Launch options open.
    pub(crate) fn agent_launch_open(&self, agent: CLIAgent, cx: &Context<Self>) -> bool {
        cx.try_global::<AgentLaunchUi>()
            .and_then(|g| g.0.get(&cx.entity_id()))
            .is_some_and(|ui| ui.open.contains(agent.slug()))
    }

    /// Build every agent's fields against `window`, the page's, so the rows
    /// can draw them inline.
    pub(crate) fn build_agent_launch_inputs(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        for agent in CLIAgent::ALL {
            self.build_agent_inputs(agent, 0, window, cx);
        }
    }

    /// Open or close `agent`'s Launch options; its fields are built on open.
    pub(crate) fn toggle_agent_launch(
        &mut self,
        agent: CLIAgent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let id = cx.entity_id();
        let ui = cx
            .default_global::<AgentLaunchUi>()
            .0
            .entry(id)
            .or_default();
        if !ui.open.remove(agent.slug()) {
            ui.open.insert(agent.slug());
            self.build_agent_inputs(agent, 0, window, cx);
        }
        cx.notify();
    }

    /// (Re)build `agent`'s fields from its settings, with `blanks` empty
    /// extra rows after the saved ones.
    fn build_agent_inputs(
        &mut self,
        agent: CLIAgent,
        blanks: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let settings = settings_of(cx.global::<Config>(), agent);
        let mut subs = Vec::new();
        let mut field = |value: String, placeholder: &str, cx: &mut Context<Self>| {
            let input = cx.new(|cx| {
                InputState::new(window, cx)
                    .placeholder(placeholder.to_string())
                    .default_value(value)
            });
            subs.push(
                cx.subscribe_in(&input, window, move |this, _i, ev, _w, cx| {
                    if matches!(ev, InputEvent::PressEnter { .. } | InputEvent::Blur) {
                        this.commit_agent_launch(agent, None, cx);
                    }
                }),
            );
            input
        };
        let model = field(settings.model.clone(), t(L10nKey::SettingsAgentDefault), cx);
        let extras = settings
            .extra_args
            .iter()
            .cloned()
            .chain(std::iter::repeat_n(String::new(), blanks))
            .map(|line| field(line, "--add-dir ~/src", cx))
            .collect();
        let id = cx.entity_id();
        cx.default_global::<AgentLaunchUi>()
            .0
            .entry(id)
            .or_default()
            .inputs
            .insert(
                agent.slug(),
                AgentInputs {
                    model,
                    extras,
                    _subs: subs,
                },
            );
    }

    fn agent_inputs(
        &self,
        agent: CLIAgent,
        cx: &Context<Self>,
    ) -> Option<(Entity<InputState>, Vec<Entity<InputState>>)> {
        cx.try_global::<AgentLaunchUi>()
            .and_then(|g| g.0.get(&cx.entity_id())?.inputs.get(agent.slug()))
            .map(|i| (i.model.clone(), i.extras.clone()))
    }

    /// Save `agent`'s fields, leaving out extra row `skip`. A line whose
    /// quote never closes is not saved; its row says why.
    fn commit_agent_launch(
        &mut self,
        agent: CLIAgent,
        skip: Option<usize>,
        cx: &mut Context<Self>,
    ) {
        let Some((model, extras)) = self.agent_inputs(agent, cx) else {
            return;
        };
        let mut settings = settings_of(cx.global::<Config>(), agent);
        settings.model = model.read(cx).value().trim().to_string();
        settings.extra_args = extras
            .iter()
            .enumerate()
            .filter(|(i, _)| Some(*i) != skip)
            .map(|(_, input)| input.read(cx).value().trim().to_string())
            .filter(|line| !line.is_empty() && crate::ui::app::split_shell_args(line).is_ok())
            .collect();
        if settings != settings_of(cx.global::<Config>(), agent) {
            self.update_config(cx, |c| set_settings(c, agent, settings));
        }
        cx.notify();
    }

    fn pick_agent_choice(
        &mut self,
        agent: CLIAgent,
        key: &'static str,
        ix: usize,
        cx: &mut Context<Self>,
    ) {
        let Some(choice) = agent.launch_choices().iter().find(|c| c.key == key) else {
            return;
        };
        let mut settings = settings_of(cx.global::<Config>(), agent);
        match ix.checked_sub(1).and_then(|i| choice.options.get(i)) {
            Some(opt) => settings
                .choices
                .insert(key.to_string(), opt.value.to_string()),
            None => settings.choices.remove(key),
        };
        self.update_config(cx, |c| set_settings(c, agent, settings));
    }

    /// A text field in the panel's grid, dressed as the dropdowns beside it,
    /// with the line that says what is wrong with it.
    fn grid_field(
        &self,
        input: &Entity<InputState>,
        error: Option<String>,
        cx: &Context<Self>,
    ) -> AnyElement {
        let tk = Tk::of(cx);
        let focused = self.settings_input_focused(input, cx);
        v_flex()
            .gap(px(4.))
            .child(
                kit::raised_text_field(input, focused, error.is_some(), &tk, cx)
                    .w(px(kit::CONTROL_W)),
            )
            .when_some(error, |col, e| {
                col.child(div().text_size(fs(12.)).text_color(tk.danger).child(e))
            })
            .into_any_element()
    }

    /// `agent`'s launch options under its integration row, in the row's text
    /// column: a label column and a [`kit::CONTROL_W`] control column, so
    /// every agent's controls line up — model, each choice, one field per
    /// CLI option with its remove tile, then "+ Add CLI option".
    pub(crate) fn render_agent_launch_panel(
        &self,
        agent: CLIAgent,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let tk = Tk::of(cx);
        let cfg = cx.global::<Config>();
        let settings = settings_of(cfg, agent);
        let line = (settings != AgentLaunchSettings::default())
            .then(|| agent.launch_command(&cfg.launch_lines()));
        let inputs = self.agent_inputs(agent, cx);
        let slug = agent.slug();
        // A label with the flag it adds on hover, in the fixed label column.
        let label = |id: String, text: &'static str, hint: Option<String>| {
            div()
                .id(SharedString::from(id))
                .w(px(LAUNCH_LABEL_W))
                .flex_shrink_0()
                .text_size(fs(12.))
                .text_color(tk.k5)
                .truncate()
                .child(text)
                .when_some(hint, |d, hint| {
                    d.tooltip(move |w, cx| {
                        gpui_component::tooltip::Tooltip::new(hint.clone()).build(w, cx)
                    })
                })
        };
        let grid_row = |name: String| {
            h_flex()
                .gap(px(LAUNCH_GAP))
                .items_center()
                .debug_selector(move || name)
        };
        // The control column; the kit sizes its fields and dropdowns to it.
        let column = |name: String, control: AnyElement| {
            div()
                .w(px(kit::CONTROL_W))
                .flex_shrink_0()
                .debug_selector(move || name)
                .child(control)
        };
        let mut rows: Vec<AnyElement> = Vec::new();

        if let (Some(flag), Some((model, _))) = (agent.model_flag(), &inputs) {
            rows.push(
                grid_row(format!("agent-launch-{slug}-model"))
                    .child(label(
                        format!("agent-launch-{slug}-model-label"),
                        t(L10nKey::SettingsAgentModel),
                        Some(t_fmt(L10nKey::SettingsAgentModelDesc, &[("flag", flag)])),
                    ))
                    .child(column(
                        format!("agent-launch-{slug}-model-control"),
                        self.grid_field(model, None, cx),
                    ))
                    .into_any_element(),
            );
        }

        for choice in agent.launch_choices() {
            let picked = settings
                .choices
                .get(choice.key)
                .and_then(|v| choice.options.iter().find(|o| o.value == v));
            let key = choice.key;
            let pick = |ix: usize| -> super::shell::PickFn {
                std::rc::Rc::new(move |this, _w, cx| this.pick_agent_choice(agent, key, ix, cx))
            };
            let mut entries = vec![MenuEntry::item(
                t(L10nKey::SettingsAgentDefault),
                picked.is_none(),
                move |this, _w, cx| this.pick_agent_choice(agent, key, 0, cx),
            )];
            for (i, opt) in choice.options.iter().enumerate() {
                entries.push(MenuEntry::Item {
                    label: opt.label.into(),
                    checked: picked.is_some_and(|p| p.value == opt.value),
                    font: None,
                    danger: false,
                    trailing: None,
                    detail: opt.hint.map(Into::into),
                    on_pick: pick(i + 1),
                });
            }
            let control = self.settings_dropdown(
                format!("agent-launch-{slug}-{key}"),
                picked.map_or(t(L10nKey::SettingsAgentDefault), |o| o.label),
                None,
                entries,
                cx,
            );
            rows.push(
                grid_row(format!("agent-launch-{slug}-{key}-row"))
                    .child(label(
                        format!("agent-launch-{slug}-{key}-label"),
                        t(choice_title(key)),
                        picked.and_then(|o| o.hint).map(str::to_string),
                    ))
                    .child(column(
                        format!("agent-launch-{slug}-{key}-control"),
                        control,
                    ))
                    .into_any_element(),
            );
        }

        if let Some((_, extras)) = &inputs {
            for (i, input) in extras.iter().enumerate() {
                let error = crate::ui::app::split_shell_args(&input.read(cx).value())
                    .is_err()
                    .then(|| t(L10nKey::SettingsArgumentsInvalid).to_string());
                let remove = div()
                    .id(SharedString::from(format!(
                        "agent-launch-{slug}-remove-{i}"
                    )))
                    .debug_selector(move || format!("agent-launch-{slug}-remove-{i}"))
                    .role(gpui::Role::Button)
                    .aria_label(t(L10nKey::SettingsAgentRemoveArg))
                    .size(px(26.))
                    .flex_shrink_0()
                    .rounded(px(6.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .cursor_pointer()
                    .hover(move |s| s.bg(tk.k05))
                    .child(
                        Icon::empty()
                            .path("icons/settings/trash.svg")
                            .size(px(12.))
                            .text_color(tk.k5),
                    )
                    .tooltip(|w, cx| {
                        gpui_component::tooltip::Tooltip::new(t(L10nKey::SettingsAgentRemoveArg))
                            .build(w, cx)
                    })
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.commit_agent_launch(agent, Some(i), cx);
                        this.build_agent_inputs(agent, 0, window, cx);
                    }));
                let name = match i {
                    0 => t(L10nKey::SettingsAgentExtraArgs),
                    _ => "",
                };
                rows.push(
                    grid_row(format!("agent-launch-{slug}-extra-{i}"))
                        .items_start()
                        .child(
                            label(format!("agent-launch-{slug}-extra-{i}-label"), name, None)
                                .h(px(26.))
                                .flex()
                                .items_center(),
                        )
                        .child(column(
                            format!("agent-launch-{slug}-extra-{i}-control"),
                            self.grid_field(input, error, cx),
                        ))
                        .child(remove)
                        .into_any_element(),
                );
            }
            rows.push(
                grid_row(format!("agent-launch-{slug}-add-row"))
                    .h(px(26.))
                    .child(div().w(px(LAUNCH_LABEL_W)).flex_shrink_0())
                    .child(
                        div()
                            .debug_selector(move || format!("agent-launch-{slug}-add"))
                            .child(
                                kit::button(
                                    SharedString::from(format!("agent-launch-{slug}-add")),
                                    t(L10nKey::SettingsAgentAddArg),
                                    BtnKind::Link,
                                )
                                .on_click(cx.listener(
                                    move |this, _, window, cx| {
                                        this.commit_agent_launch(agent, None, cx);
                                        this.build_agent_inputs(agent, 1, window, cx);
                                    },
                                )),
                            ),
                    )
                    .into_any_element(),
            );
        }

        v_flex()
            .debug_selector(move || format!("agent-launch-{slug}"))
            .pl(px(LAUNCH_INDENT))
            .pb(px(12.))
            .gap(px(8.))
            .min_w_0()
            .children(rows)
            .when_some(line, |col, line| {
                col.child(
                    div()
                        .min_w_0()
                        .whitespace_normal()
                        .font_family(Tk::mono(cx))
                        .text_size(fs(11.5))
                        .text_color(tk.k45)
                        .child(t_fmt(
                            L10nKey::SettingsAgentLaunchesAs,
                            &[("command", &line)],
                        )),
                )
            })
            .into_any_element()
    }
}

/// The integration row's text column: its 20px disc and the 12px gap after.
const LAUNCH_INDENT: f32 = 32.;
const LAUNCH_LABEL_W: f32 = 120.;
const LAUNCH_GAP: f32 = 8.;

/// The collapsed row's note of what `agent`'s launch options set:
/// `model: o3 · never · +2 options`. `None` when nothing is.
pub(crate) fn launch_summary(cfg: &Config, agent: CLIAgent) -> Option<String> {
    let s = settings_of(cfg, agent);
    let mut parts = Vec::new();
    if agent.model_flag().is_some() && !s.model.trim().is_empty() {
        parts.push(t_fmt(
            L10nKey::SettingsAgentSummaryModel,
            &[("model", s.model.trim())],
        ));
    }
    for choice in agent.launch_choices() {
        if let Some(opt) = s
            .choices
            .get(choice.key)
            .and_then(|v| choice.options.iter().find(|o| o.value == v))
        {
            parts.push(opt.label.to_string());
        }
    }
    let extras = s.extra_args.iter().filter(|l| !l.trim().is_empty()).count();
    if extras > 0 {
        parts.push(t_plural(L10nKey::SettingsAgentSummaryExtras, extras, &[]));
    }
    (!parts.is_empty()).then(|| parts.join(" · "))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(cfg: &Config, agent: CLIAgent) -> String {
        agent.launch_command(&cfg.launch_lines())
    }

    fn with(agent: CLIAgent, settings: AgentLaunchSettings) -> Config {
        let mut cfg = Config::default();
        set_settings(&mut cfg, agent, settings);
        cfg
    }

    fn settings(model: &str, choices: &[(&str, &str)], extras: &[&str]) -> AgentLaunchSettings {
        AgentLaunchSettings {
            model: model.into(),
            choices: choices
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect(),
            extra_args: extras.iter().map(|s| s.to_string()).collect(),
        }
    }

    fn argv(line: &str) -> Vec<String> {
        crate::ui::app::split_shell_args(line).unwrap()
    }

    #[test]
    fn claude_settings_give_the_exact_argv() {
        let cfg = with(
            CLIAgent::Claude,
            settings("opus", &[("permission", "plan")], &["--add-dir ~/src"]),
        );
        assert_eq!(
            line(&cfg, CLIAgent::Claude),
            "claude --model opus --permission-mode plan --add-dir ~/src"
        );
    }

    #[test]
    fn codex_settings_give_the_exact_argv() {
        let cfg = with(
            CLIAgent::Codex,
            settings(
                "gpt-5.1-codex",
                &[("permission", "approve-for-me")],
                &["--search"],
            ),
        );
        assert_eq!(
            argv(&line(&cfg, CLIAgent::Codex)),
            [
                "codex",
                "--model",
                "gpt-5.1-codex",
                "--sandbox",
                "workspace-write",
                "--ask-for-approval",
                "on-request",
                "-c",
                "approvals_reviewer=auto_review",
                "--search"
            ]
        );
        // Each preset is the sandbox, approval policy and reviewer Codex's
        // own /permissions picker sets for it.
        let preset = |value: &str| {
            let cfg = with(CLIAgent::Codex, settings("", &[("permission", value)], &[]));
            line(&cfg, CLIAgent::Codex)
        };
        assert_eq!(
            preset("ask"),
            "codex --sandbox workspace-write --ask-for-approval on-request -c approvals_reviewer=user"
        );
        assert_eq!(
            preset("full-access"),
            "codex --sandbox danger-full-access --ask-for-approval never -c approvals_reviewer=user"
        );
        assert_eq!(
            preset("read-only"),
            "codex --sandbox read-only --ask-for-approval on-request -c approvals_reviewer=user"
        );
        let labels: Vec<&str> = CLIAgent::Codex.launch_choices()[0]
            .options
            .iter()
            .map(|o| o.label)
            .collect();
        assert_eq!(
            labels,
            [
                "Ask for approval",
                "Approve for me",
                "Full Access",
                "Read Only"
            ]
        );
    }

    /// The Sandbox and Approval policy pair #144 stored becomes the preset
    /// that sets it, or the flags it added when no preset does.
    #[test]
    fn a_codex_sandbox_and_approval_pair_becomes_its_preset() {
        let migrated = |choices: &[(&str, &str)]| {
            let mut cfg = with(CLIAgent::Codex, settings("o3", choices, &["--search"]));
            cfg.move_agent_launch_into_settings();
            settings_of(&cfg, CLIAgent::Codex)
        };
        assert_eq!(
            migrated(&[("sandbox", "workspace-write"), ("approval", "on-request")]),
            settings("o3", &[("permission", "ask")], &["--search"])
        );
        assert_eq!(
            migrated(&[("sandbox", "danger-full-access"), ("approval", "never")]),
            settings("o3", &[("permission", "full-access")], &["--search"])
        );
        assert_eq!(
            migrated(&[("sandbox", "read-only"), ("approval", "on-request")]),
            settings("o3", &[("permission", "read-only")], &["--search"])
        );
        assert_eq!(
            migrated(&[("sandbox", "workspace-write"), ("approval", "never")]),
            settings(
                "o3",
                &[],
                &[
                    "--sandbox workspace-write",
                    "--ask-for-approval never",
                    "--search"
                ]
            )
        );
        assert_eq!(
            migrated(&[("approval", "never")]),
            settings("o3", &[], &["--ask-for-approval never", "--search"])
        );
    }

    #[test]
    fn a_launch_line_spelling_a_preset_moves_into_it() {
        let mut cfg = Config::default();
        cfg.agent_launch.insert(
            "codex".into(),
            "codex -c approvals_reviewer=user --ask-for-approval never --model o3 --sandbox danger-full-access"
                .into(),
        );
        cfg.move_agent_launch_into_settings();
        assert_eq!(
            settings_of(&cfg, CLIAgent::Codex),
            settings("o3", &[("permission", "full-access")], &[])
        );
    }

    #[test]
    fn pi_settings_give_the_exact_argv() {
        let cfg = with(
            CLIAgent::Pi,
            settings("anthropic/claude-sonnet-4:high", &[], &["--thinking high"]),
        );
        assert_eq!(
            argv(&line(&cfg, CLIAgent::Pi)),
            [
                "pi",
                "--model",
                "anthropic/claude-sonnet-4:high",
                "--thinking",
                "high"
            ]
        );
        assert!(CLIAgent::Pi.launch_choices().is_empty());
    }

    #[test]
    fn cursor_settings_give_the_exact_argv() {
        let cfg = with(
            CLIAgent::Cursor,
            settings(
                "gpt-5[reasoning=high]",
                &[("permission", "force"), ("sandbox", "disabled")],
                &[],
            ),
        );
        // The brackets would glob unquoted.
        assert_eq!(
            line(&cfg, CLIAgent::Cursor),
            "cursor-agent --model 'gpt-5[reasoning=high]' --force --sandbox disabled"
        );
    }

    #[test]
    fn an_agent_without_structured_flags_still_takes_extras() {
        assert_eq!(CLIAgent::Gemini.model_flag(), None);
        let cfg = with(
            CLIAgent::Gemini,
            settings("ignored", &[("permission", "plan")], &["--yolo"]),
        );
        assert_eq!(line(&cfg, CLIAgent::Gemini), "gemini --yolo");
    }

    #[test]
    fn the_collapsed_row_sums_up_what_is_set() {
        let cfg = with(
            CLIAgent::Codex,
            settings(
                "o3",
                &[("permission", "approve-for-me")],
                &["--search", "--oss"],
            ),
        );
        let summary = launch_summary(&cfg, CLIAgent::Codex).unwrap();
        let parts: Vec<&str> = summary.split(" · ").collect();
        assert_eq!(parts.len(), 3, "{summary}");
        assert!(parts[0].contains("o3") && parts[1] == "Approve for me" && parts[2].contains('2'));
        assert_eq!(launch_summary(&cfg, CLIAgent::Claude), None);
    }

    #[test]
    fn defaults_add_nothing() {
        let cfg = with(CLIAgent::Claude, AgentLaunchSettings::default());
        assert!(cfg.fork.agent_settings.is_empty(), "nothing stored");
        for agent in CLIAgent::ALL {
            assert_eq!(line(&cfg, agent), agent.binary());
        }
        // Blank extras and an unknown choice value add nothing either.
        let cfg = with(
            CLIAgent::Codex,
            settings(" ", &[("sandbox", "everything")], &["", "  "]),
        );
        assert_eq!(line(&cfg, CLIAgent::Codex), "codex");
    }

    #[test]
    fn settings_follow_an_agent_launch_wrapper() {
        let mut cfg = with(CLIAgent::Claude, settings("opus", &[], &[]));
        cfg.agent_launch.insert("claude".into(), "cc --fast".into());
        assert_eq!(line(&cfg, CLIAgent::Claude), "cc --fast --model opus");
    }

    #[test]
    fn extras_survive_quoting() {
        let cfg = with(
            CLIAgent::Claude,
            settings(
                "",
                &[],
                &[
                    r#"--append-system-prompt "be terse, don't guess""#,
                    "--add-dir '/Users/me/My Projects'",
                ],
            ),
        );
        assert_eq!(
            argv(&line(&cfg, CLIAgent::Claude)),
            [
                "claude",
                "--append-system-prompt",
                "be terse, don't guess",
                "--add-dir",
                "/Users/me/My Projects"
            ]
        );
    }

    #[test]
    fn a_resume_and_a_fresh_restart_carry_the_settings() {
        let cfg = with(
            CLIAgent::Codex,
            settings("o3", &[("permission", "read-only")], &[]),
        );
        let lines = cfg.launch_lines();
        let flags = "--model o3 --sandbox read-only --ask-for-approval on-request -c approvals_reviewer=user";
        assert_eq!(
            crate::ui::agent_launch::resume_line(CLIAgent::Codex, "0199-x", &lines),
            Some(format!("codex resume 0199-x {flags}"))
        );
        assert_eq!(
            crate::ui::agent_launch::fork_line(CLIAgent::Codex, "0199-x", &lines),
            Some(format!("codex fork 0199-x {flags}"))
        );
        assert_eq!(
            CLIAgent::Codex.relaunch_command(None, &lines),
            format!("codex {flags}")
        );
        // A pane that recorded its own flags restarts with those instead.
        let ran: Vec<String> = ["codex", "--model", "gpt-5"].map(String::from).to_vec();
        assert_eq!(
            CLIAgent::Codex.relaunch_command(Some(&ran), &lines),
            "codex --model gpt-5"
        );
    }

    /// `agent_launch` lines from before the page existed load as its controls
    /// and still launch exactly as they did.
    #[test]
    fn old_agent_launch_lines_move_into_the_settings() {
        let old = [
            (
                "claude",
                "claude --dangerously-skip-permissions --model opus --permission-mode acceptEdits",
            ),
            (
                "codex",
                "codex -s read-only --ask-for-approval never --model o3",
            ),
            ("pi", "cc --fast"),
            ("gemini", "gemini"),
            ("Cursor", "cursor-agent \"quoted\""),
        ];
        let mut cfg = Config::default();
        for (slug, line) in old {
            cfg.agent_launch.insert(slug.to_string(), line.to_string());
        }
        let before = cfg.launch_lines();
        cfg.move_agent_launch_into_settings();

        assert_eq!(
            settings_of(&cfg, CLIAgent::Claude),
            settings(
                "opus",
                &[("permission", "acceptEdits")],
                &["--dangerously-skip-permissions"]
            )
        );
        // A sandbox and approval policy that are no preset stay extras.
        assert_eq!(
            settings_of(&cfg, CLIAgent::Codex),
            settings("o3", &[], &["-s read-only", "--ask-for-approval never"])
        );
        // A bare binary needs no settings at all.
        assert!(settings_key(&cfg, CLIAgent::Gemini).is_none());
        // A wrapper and a quoted line are left where they were.
        assert_eq!(
            cfg.agent_launch,
            HashMap::from([
                ("pi".to_string(), "cc --fast".to_string()),
                ("Cursor".to_string(), "cursor-agent \"quoted\"".to_string()),
            ])
        );
        for agent in CLIAgent::ALL {
            assert_eq!(
                argv(&agent.launch_command(&cfg.launch_lines()))
                    .iter()
                    .collect::<std::collections::BTreeSet<_>>(),
                argv(&agent.launch_command(&before))
                    .iter()
                    .collect::<std::collections::BTreeSet<_>>(),
                "{agent:?} launches with the same words"
            );
        }
        // Run again, nothing moves twice.
        let once = cfg.fork.agent_settings.clone();
        cfg.move_agent_launch_into_settings();
        assert_eq!(cfg.fork.agent_settings, once);
    }

    #[test]
    fn an_agent_with_settings_keeps_its_old_line_as_it_is() {
        let mut cfg = with(CLIAgent::Claude, settings("opus", &[], &[]));
        cfg.agent_launch
            .insert("claude".into(), "claude --model sonnet".into());
        cfg.move_agent_launch_into_settings();
        assert_eq!(cfg.agent_launch["claude"], "claude --model sonnet");
        assert_eq!(settings_of(&cfg, CLIAgent::Claude).model, "opus");
    }
}

#[cfg(test)]
mod gpui_tests {
    use super::super::{AgentHookRow, AgentHooksView, SettingsSection};
    use super::*;
    use crate::core::agent_hooks::{HookAgent, HooksState};
    use crate::core::session::Session;
    use gpui::{TestAppContext, VisualTestContext, size};

    /// The panel's grid, as laid out: one label column and one control
    /// column across agents, controls one width and height, the remove tiles
    /// on one line, "+ Add CLI option" in the control column, even spacing,
    /// and nothing past the row's right edge.
    #[gpui::test]
    fn launch_options_line_up_across_agents(cx: &mut TestAppContext) {
        crate::core::config::pin_test_config_dir();
        cx.executor().allow_parking();
        cx.update(|cx| {
            gpui_component::init(cx);
            let mut cfg = Config::default();
            let extras = |lines: &[&str]| AgentLaunchSettings {
                extra_args: lines.iter().map(|l| l.to_string()).collect(),
                ..Default::default()
            };
            cfg.fork
                .agent_settings
                .insert("claude".into(), extras(&["--add-dir ~/src", "--verbose"]));
            cfg.fork
                .agent_settings
                .insert("codex".into(), extras(&["--search"]));
            cx.set_global(cfg);
            crate::ui::keymap::init(cx);
        });
        let window = cx.add_window(|window, cx| {
            let app =
                cx.new(|cx| Tty7App::with_session(None, Some(Session::default()), window, cx));
            gpui_component::Root::new(app, window, cx)
        });
        cx.background_executor.run_until_parked();
        let app = window
            .update(cx, |root, _, _| {
                root.view().clone().downcast::<Tty7App>().ok().unwrap()
            })
            .unwrap();
        let mut vcx = VisualTestContext::from_window(window.into(), cx);
        app.update_in(&mut vcx, |app, window, cx| {
            app.open_settings_section(SettingsSection::Agents, window, cx);
        });
        vcx.run_until_parked();
        app.update_in(&mut vcx, |app, _, cx| {
            let s = app.active_settings_mut().unwrap();
            s.agent_hooks_seq += 1;
            s.agent_hooks_states = AgentHooksView::Ready(
                [HookAgent::Claude, HookAgent::Codex, HookAgent::Cursor]
                    .into_iter()
                    .map(|agent| AgentHookRow {
                        agent,
                        state: HooksState::Installed,
                        target: "~/.claude/settings.json".into(),
                    })
                    .collect(),
            );
            cx.notify();
        });
        vcx.simulate_resize(size(px(1100.), px(1400.)));
        vcx.run_until_parked();

        let at = |vcx: &mut VisualTestContext, name: &'static str| {
            vcx.debug_bounds(name)
                .unwrap_or_else(|| panic!("{name} was drawn"))
        };
        // Each agent's rows top to bottom, then their controls.
        let agents: [(&str, &[&'static str], &[&'static str]); 3] = [
            (
                "agent-launch-claude",
                &[
                    "agent-launch-claude-model",
                    "agent-launch-claude-permission-row",
                    "agent-launch-claude-extra-0",
                    "agent-launch-claude-extra-1",
                    "agent-launch-claude-add-row",
                ],
                &[
                    "agent-launch-claude-model-control",
                    "agent-launch-claude-permission-control",
                    "agent-launch-claude-extra-0-control",
                    "agent-launch-claude-extra-1-control",
                ],
            ),
            (
                "agent-launch-codex",
                &[
                    "agent-launch-codex-model",
                    "agent-launch-codex-permission-row",
                    "agent-launch-codex-extra-0",
                    "agent-launch-codex-add-row",
                ],
                &[
                    "agent-launch-codex-model-control",
                    "agent-launch-codex-permission-control",
                    "agent-launch-codex-extra-0-control",
                ],
            ),
            (
                "agent-launch-cursor",
                &[
                    "agent-launch-cursor-model",
                    "agent-launch-cursor-permission-row",
                    "agent-launch-cursor-sandbox-row",
                    "agent-launch-cursor-add-row",
                ],
                &[
                    "agent-launch-cursor-model-control",
                    "agent-launch-cursor-permission-control",
                    "agent-launch-cursor-sandbox-control",
                ],
            ),
        ];
        let first_row = at(&mut vcx, "agent-launch-claude-model");
        let first_control = at(&mut vcx, "agent-launch-claude-model-control");
        for (panel, rows, controls) in agents {
            let panel = at(&mut vcx, panel);
            assert_eq!(first_row.left() - panel.left(), px(LAUNCH_INDENT));
            let rows: Vec<_> = rows.iter().map(|r| at(&mut vcx, r)).collect();
            for pair in rows.windows(2) {
                assert_eq!(pair[0].size.height, px(26.), "one row height");
                assert_eq!(
                    pair[1].top() - pair[0].bottom(),
                    px(LAUNCH_GAP),
                    "even spacing"
                );
            }
            for row in &rows {
                assert_eq!(row.left(), first_row.left(), "one label column");
                assert!(row.right() <= panel.right());
            }
            for control in controls {
                let control = at(&mut vcx, control);
                assert_eq!(control.left(), first_control.left(), "one control column");
                assert_eq!(control.size.width, px(kit::CONTROL_W));
                assert_eq!(control.size.height, px(26.));
            }
        }
        assert_eq!(
            first_control.left() - first_row.left(),
            px(LAUNCH_LABEL_W + LAUNCH_GAP)
        );
        for add in [
            "agent-launch-claude-add",
            "agent-launch-codex-add",
            "agent-launch-cursor-add",
        ] {
            assert_eq!(at(&mut vcx, add).left(), first_control.left(), "{add}");
        }
        let trash_x = first_control.right() + px(LAUNCH_GAP);
        for remove in [
            "agent-launch-claude-remove-0",
            "agent-launch-claude-remove-1",
            "agent-launch-codex-remove-0",
        ] {
            assert_eq!(at(&mut vcx, remove).left(), trash_x, "{remove}");
        }
    }
}

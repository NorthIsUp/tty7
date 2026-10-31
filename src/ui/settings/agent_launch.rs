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
        "approval" => L10nKey::SettingsAgentApproval,
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

    /// `agent`'s launch options, compact under its integration row: the
    /// model and choices on one wrapping line, then one field per extra
    /// argument line and "+ Add option".
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
        let label = |text: &'static str| {
            div()
                .text_size(fs(12.))
                .text_color(tk.k45)
                .whitespace_nowrap()
                .child(text)
        };
        let mut controls = h_flex().flex_wrap().gap(px(10.)).items_center();

        if let (Some(flag), Some((model, _))) = (agent.model_flag(), &inputs) {
            controls = controls.child(
                h_flex()
                    .gap(px(6.))
                    .items_center()
                    .id(SharedString::from(format!(
                        "agent-launch-{}-model",
                        agent.slug()
                    )))
                    .tooltip(move |w, cx| {
                        gpui_component::tooltip::Tooltip::new(t_fmt(
                            L10nKey::SettingsAgentModelDesc,
                            &[("flag", flag)],
                        ))
                        .build(w, cx)
                    })
                    .child(label(t(L10nKey::SettingsAgentModel)))
                    .child(self.settings_text_input(model, 160., false, cx)),
            );
        }

        for choice in agent.launch_choices() {
            let mut labels = vec![t(L10nKey::SettingsAgentDefault)];
            labels.extend(choice.options.iter().map(|o| o.value));
            let selected = settings
                .choices
                .get(choice.key)
                .and_then(|v| choice.options.iter().position(|o| o.value == v))
                .map_or(0, |i| i + 1);
            let key = choice.key;
            let control = self.settings_choice(
                format!("agent-launch-{}-{key}", agent.slug()),
                &labels,
                selected,
                cx,
                move |this, ix, _w, cx| this.pick_agent_choice(agent, key, ix, cx),
            );
            controls = controls.child(
                h_flex()
                    .gap(px(6.))
                    .items_center()
                    .child(label(t(choice_title(key))))
                    .child(control),
            );
        }

        let mut extras_col = v_flex().gap(px(6.));
        if let Some((_, extras)) = &inputs {
            for (i, input) in extras.iter().enumerate() {
                let error = crate::ui::app::split_shell_args(&input.read(cx).value())
                    .is_err()
                    .then(|| t(L10nKey::SettingsArgumentsInvalid).to_string());
                extras_col = extras_col.child(
                    h_flex()
                        .gap(px(6.))
                        .items_start()
                        .child(self.settings_checked_input(input, 260., error, cx))
                        .child(
                            kit::button(
                                SharedString::from(format!(
                                    "agent-launch-{}-remove-{i}",
                                    agent.slug()
                                )),
                                t(L10nKey::SettingsAgentRemoveArg),
                                BtnKind::Link,
                            )
                            .on_click(cx.listener(
                                move |this, _, window, cx| {
                                    this.commit_agent_launch(agent, Some(i), cx);
                                    this.build_agent_inputs(agent, 0, window, cx);
                                },
                            )),
                        ),
                );
            }
            extras_col = extras_col.child(
                h_flex().child(
                    kit::button(
                        SharedString::from(format!("agent-launch-{}-add", agent.slug())),
                        t(L10nKey::SettingsAgentAddArg),
                        BtnKind::Link,
                    )
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.commit_agent_launch(agent, None, cx);
                        this.build_agent_inputs(agent, 1, window, cx);
                    })),
                ),
            );
        }

        v_flex()
            .pl(px(32.))
            .pb(px(10.))
            .gap(px(6.))
            .child(controls)
            .child(extras_col)
            .when_some(line, |col, line| {
                col.child(
                    div()
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
        if let Some(v) = s.choices.get(choice.key)
            && choice.options.iter().any(|o| o.value == v)
        {
            parts.push(v.clone());
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
                &[("sandbox", "workspace-write"), ("approval", "never")],
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
                "never",
                "--search"
            ]
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
            settings("o3", &[("approval", "never")], &["--search", "--oss"]),
        );
        let summary = launch_summary(&cfg, CLIAgent::Codex).unwrap();
        let parts: Vec<&str> = summary.split(" · ").collect();
        assert_eq!(parts.len(), 3, "{summary}");
        assert!(parts[0].contains("o3") && parts[1] == "never" && parts[2].contains('2'));
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
            settings("o3", &[("sandbox", "read-only")], &[]),
        );
        let lines = cfg.launch_lines();
        assert_eq!(
            crate::ui::agent_launch::resume_line(CLIAgent::Codex, "0199-x", &lines).as_deref(),
            Some("codex resume 0199-x --model o3 --sandbox read-only")
        );
        assert_eq!(
            crate::ui::agent_launch::fork_line(CLIAgent::Codex, "0199-x", &lines).as_deref(),
            Some("codex fork 0199-x --model o3 --sandbox read-only")
        );
        assert_eq!(
            CLIAgent::Codex.relaunch_command(None, &lines),
            "codex --model o3 --sandbox read-only"
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
        // `-s` is not the spelling the control writes, so it stays an extra.
        assert_eq!(
            settings_of(&cfg, CLIAgent::Codex),
            settings("o3", &[("approval", "never")], &["-s read-only"])
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

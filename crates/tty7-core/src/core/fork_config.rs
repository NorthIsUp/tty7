//! The fork's settings, kept out of upstream's `Config` so a rebase touches
//! one field there: `Config::fork`, flattened, so they sit at the top level
//! of `config.json` exactly as they did when they were `Config`'s own.

use std::collections::{BTreeMap, HashMap};

use serde::{Deserialize, Serialize};

use super::cli_agent::CLIAgent;
use super::config::{Config, ProfileUsage, de_lenient};
use super::shell_quote::quote_for_shell;

/// Every default is in [`Default`] below: the struct-level `serde(default)`
/// fills a missing key from it, so no field names its own.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(default)]
pub struct ForkConfig {
    /// Restore a tab whose panes all died (reboot, Quit and Stop) asleep, so
    /// it starts — and resumes its agent — when first opened, not at launch.
    pub restore_asleep: bool,
    /// What Continue All Agents, and with `continue_interrupted_agents` a
    /// resume of a cut-off turn, tells the agent. Empty says nothing.
    pub continue_prompt: String,
    /// How many tabs a wake starts at once, so they do not all cold-start and
    /// hit the API together; `None` is a quarter of the cores. 1 to 20.
    pub agent_wake_concurrency: Option<u8>,
    /// Wake every agent tab restore put to sleep at launch; tabs the user
    /// hibernated and shell-only tabs stay asleep.
    pub resume_agents_on_launch: bool,
    /// How a restored agent pane resumes its session.
    #[serde(deserialize_with = "de_lenient")]
    pub agent_resume_mode: AgentResumeMode,
    /// Tell a resumed agent `continue_prompt` when its transcript shows the
    /// turn was cut off or left background work unfinished; off resumes every
    /// agent without a word.
    pub continue_interrupted_agents: bool,
    /// New Tab opens the new tab page — pick an agent or a terminal and a
    /// directory — instead of a shell in the current tab's directory.
    pub new_tab_page: bool,
    /// Directories whose children the new tab page offers (`~` expands).
    pub dir_roots: Vec<String>,
    /// How often and how recently each directory was launched into from the
    /// new tab page, keyed by its absolute path.
    pub dir_frecency: HashMap<String, ProfileUsage>,
    /// Sidebar group colour overrides, by group name: `"clawmux": "#ffd7d7"`.
    pub group_colors: HashMap<String, String>,
    /// A 1px outline round each sidebar group header.
    pub group_outline: bool,
    /// A tinted fill behind each sidebar group header.
    pub group_background: bool,
    #[serde(deserialize_with = "de_lenient")]
    pub group_outline_color: GroupColorSource,
    #[serde(deserialize_with = "de_lenient")]
    pub group_background_color: GroupColorSource,
    /// Whether the fill and outline wrap the header alone or the whole group.
    #[serde(deserialize_with = "de_lenient")]
    pub group_background_scope: GroupBackgroundScope,
    /// Animate a sidebar group folding open and shut.
    pub animations: bool,
    /// Niceness every pane's shell starts at, so typing stays responsive while
    /// agents build; 0 leaves it alone. Unix only.
    pub nice: i32,
    /// The GitHub panel shows `origin` (the fork) until a remote is picked,
    /// instead of `upstream`.
    pub github_panel_prefer_origin: bool,
    /// The system-wide chord that shows and hides tty7, in keymap syntax
    /// (`alt-space`, `cmd-shift-t`); `null` or `""` turns it off. macOS only.
    pub global_hotkey: Option<String>,
    /// The hotkey summons the window over the whole screen the mouse is on,
    /// floating above other apps.
    pub global_hotkey_fullscreen: bool,
    /// Hide tty7 whenever it loses focus after the hotkey summoned it.
    pub global_hotkey_hide_on_blur: bool,
    /// How long the hotkey's fade in and out takes; 0 is instant.
    pub global_hotkey_fade_ms: u64,
    /// Settings → Agents' launch settings, by agent slug. Appended to the
    /// agent's `agent_launch` line ([`super::config::Config::launch_lines`]).
    #[serde(deserialize_with = "de_lenient")]
    pub agent_settings: HashMap<String, AgentLaunchSettings>,
}

/// A bare "continue" read as "move on", so a resumed agent that stopped on a
/// question answered it itself.
pub const DEFAULT_CONTINUE_PROMPT: &str = "This agent is resuming after a crash. Pick up where you left off and continue. This is not an instruction to make a specific choice if you were waiting for user input.";

impl Default for ForkConfig {
    fn default() -> Self {
        Self {
            restore_asleep: true,
            continue_prompt: DEFAULT_CONTINUE_PROMPT.into(),
            agent_wake_concurrency: None,
            resume_agents_on_launch: false,
            agent_resume_mode: AgentResumeMode::Typed,
            continue_interrupted_agents: false,
            new_tab_page: true,
            dir_roots: ["~/src", "~/code", "~/projects"].map(String::from).to_vec(),
            dir_frecency: HashMap::new(),
            group_colors: HashMap::new(),
            group_outline: false,
            group_background: true,
            group_outline_color: GroupColorSource::Hashed,
            group_background_color: GroupColorSource::Hashed,
            group_background_scope: GroupBackgroundScope::Header,
            animations: true,
            nice: 5,
            github_panel_prefer_origin: true,
            global_hotkey: Some("alt-space".into()),
            global_hotkey_fullscreen: false,
            global_hotkey_hide_on_blur: false,
            global_hotkey_fade_ms: 150,
            agent_settings: HashMap::new(),
        }
    }
}

/// One agent's launch settings. Every part left empty adds nothing, so the
/// agent's own config wins.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(default)]
pub struct AgentLaunchSettings {
    /// The value for [`CLIAgent::model_flag`].
    #[serde(skip_serializing_if = "String::is_empty")]
    pub model: String,
    /// [`CLIAgent::launch_choices`] key to the option picked.
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub choices: BTreeMap<String, String>,
    /// Argument lines in shell syntax, each typed as written after the flags
    /// above: `--add-dir ~/src`.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub extra_args: Vec<String>,
}

impl AgentLaunchSettings {
    /// Codex's separate Sandbox and Approval policy choices, from before its
    /// one Permissions preset: the pair a preset sets becomes that preset,
    /// any other stays as the flags it added, now extra lines.
    fn merge_codex_sandbox_and_approval(&mut self) {
        let sandbox = self.choices.remove("sandbox");
        let approval = self.choices.remove("approval");
        let preset = match (sandbox.as_deref(), approval.as_deref()) {
            (None, None) => return,
            (Some("workspace-write"), Some("on-request")) => Some("ask"),
            (Some("danger-full-access"), Some("never")) => Some("full-access"),
            (Some("read-only"), Some("on-request")) => Some("read-only"),
            _ => None,
        };
        match preset {
            Some(p) if !self.choices.contains_key("permission") => {
                self.choices.insert("permission".into(), p.into());
            }
            _ => {
                let flags = [("--sandbox", sandbox), ("--ask-for-approval", approval)];
                let lines = flags
                    .into_iter()
                    .filter_map(|(flag, v)| Some(format!("{flag} {}", v?)));
                self.extra_args.splice(0..0, lines);
            }
        }
    }

    /// What these settings add to `agent`'s launch line, as shell words:
    /// model, then each choice, then the extra lines. A choice value the
    /// agent does not offer (a hand-edited file) adds nothing.
    pub fn flags(&self, agent: CLIAgent) -> Vec<String> {
        let mut out = Vec::new();
        let model = self.model.trim();
        if let Some(flag) = agent.model_flag().filter(|_| !model.is_empty()) {
            out.push(flag.to_string());
            out.push(quote_for_shell(model, None));
        }
        for choice in agent.launch_choices() {
            let picked = self.choices.get(choice.key);
            if let Some(opt) = choice
                .options
                .iter()
                .find(|o| Some(o.value) == picked.map(String::as_str))
            {
                out.extend(opt.argv.iter().map(|a| a.to_string()));
            }
        }
        out.extend(
            self.extra_args
                .iter()
                .map(|line| line.trim())
                .filter(|line| !line.is_empty())
                .map(str::to_string),
        );
        out
    }

    /// `line` as settings, when it is `agent`'s bare binary followed by flags
    /// alone, every word plain enough to need no quoting: the model and the
    /// choices it spells come out as those, the rest as one extra line per
    /// flag with its values. `None` for anything else (a wrapper, a path,
    /// env assignments, a prompt, quoting), which stays an `agent_launch`
    /// line.
    pub fn from_launch_line(agent: CLIAgent, line: &str) -> Option<Self> {
        let mut words = line.split_whitespace();
        if words.next() != Some(agent.binary()) {
            return None;
        }
        let plain = |w: &str| {
            w.bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"-_=./,:@+~".contains(&b))
        };
        let words: Vec<&str> = words.collect();
        if !words.iter().all(|w| plain(w)) {
            return None;
        }
        let mut groups = flag_groups(&words)?;
        let mut out = Self::default();
        // A choice option can span several flags (Codex's presets), so it is
        // taken when every one of its groups is on the line.
        for choice in agent.launch_choices() {
            for opt in choice.options {
                let wanted = flag_groups(opt.argv).unwrap_or_default();
                let at: Option<Vec<usize>> = wanted
                    .iter()
                    .map(|w| groups.iter().position(|g| g == w))
                    .collect();
                if let Some(mut at) = at {
                    at.sort_unstable();
                    at.dedup();
                    if at.len() != wanted.len() {
                        continue;
                    }
                    for i in at.into_iter().rev() {
                        groups.remove(i);
                    }
                    out.choices
                        .insert(choice.key.to_string(), opt.value.to_string());
                    break;
                }
            }
        }
        for group in groups {
            if let ([flag, model], Some(model_flag)) = (group.as_slice(), agent.model_flag())
                && *flag == model_flag
                && out.model.is_empty()
            {
                out.model = model.to_string();
                continue;
            }
            out.extra_args.push(group.join(" "));
        }
        Some(out)
    }
}

/// `words` as flags with their values: `["-c", "k=v", "--x"]` is
/// `[["-c", "k=v"], ["--x"]]`. `None` when a value comes before any flag.
fn flag_groups<'a>(words: &[&'a str]) -> Option<Vec<Vec<&'a str>>> {
    let mut groups: Vec<Vec<&str>> = Vec::new();
    for &word in words {
        match (word.starts_with('-'), groups.last_mut()) {
            (true, _) => groups.push(vec![word]),
            (false, Some(group)) => group.push(word),
            (false, None) => return None,
        }
    }
    Some(groups)
}

impl Config {
    /// Each `agent_launch` line that is the agent's binary and plain flags
    /// becomes that agent's Settings → Agents controls, so the page shows
    /// what a launch runs. An agent that already has settings keeps its line
    /// as it is: moving it would add the same flags twice.
    pub fn move_agent_launch_into_settings(&mut self) {
        for (slug, s) in self.fork.agent_settings.iter_mut() {
            if CLIAgent::from_slug(slug) == Some(CLIAgent::Codex) {
                s.merge_codex_sandbox_and_approval();
            }
        }
        let settings = &mut self.fork.agent_settings;
        self.agent_launch.retain(|slug, line| {
            let Some(agent) = CLIAgent::from_slug(slug) else {
                return true;
            };
            if settings
                .keys()
                .any(|k| CLIAgent::from_slug(k) == Some(agent))
            {
                return true;
            }
            match AgentLaunchSettings::from_launch_line(agent, line.trim()) {
                Some(moved) => {
                    if moved != AgentLaunchSettings::default() {
                        settings.insert(agent.slug().to_string(), moved);
                    }
                    false
                }
                None => true,
            }
        });
    }

    /// `agent_launch` with each agent's Settings → Agents flags appended: the
    /// line every new tab, split, resume, fork and fresh restart starts from.
    pub fn launch_lines(&self) -> HashMap<String, String> {
        let mut out = self.agent_launch.clone();
        for (slug, settings) in &self.fork.agent_settings {
            let Some(agent) = CLIAgent::from_slug(slug) else {
                continue;
            };
            let flags = settings.flags(agent);
            if flags.is_empty() {
                continue;
            }
            let base = agent.launch_command(&self.agent_launch);
            out.retain(|k, _| CLIAgent::from_slug(k) != Some(agent));
            out.insert(
                agent.slug().to_string(),
                format!("{base} {}", flags.join(" ")),
            );
        }
        out
    }
}

/// Where a sidebar group header's outline or fill takes its colour from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum GroupColorSource {
    /// A neutral from the theme, the same for every group.
    Theme,
    /// The group's own colour: its `group_colors` override, else its hash.
    #[default]
    Hashed,
}

/// How a restored agent pane resumes its session.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AgentResumeMode {
    /// The resume line is typed at the new shell's first prompt.
    #[default]
    Typed,
    /// The new shell runs it itself, once, as the tab's program. Only for
    /// sh, bash, zsh, dash, ksh and fish; any other shell is typed into.
    Program,
}

/// How much of a sidebar group its fill and outline cover.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum GroupBackgroundScope {
    #[default]
    Header,
    /// The header and its rows, as one block.
    Group,
}

#[cfg(test)]
mod tests {
    use super::super::config::Config;

    /// A file written before `ForkConfig` existed: every fork key, none at
    /// its default.
    const OLD_FILE: &str = r##"{
        "font_size": 15.0,
        "restore_asleep": false,
        "continue_prompt": "go on",
        "agent_wake_concurrency": 3,
        "resume_agents_on_launch": true,
        "agent_resume_mode": "program",
        "continue_interrupted_agents": true,
        "new_tab_page": false,
        "dir_roots": ["~/work"],
        "dir_frecency": {"/Users/me/src/x": {"count": 2, "last_used": 9}},
        "group_colors": {"clawmux": "#ffd7d7"},
        "group_outline": true,
        "group_background": false,
        "group_outline_color": "theme",
        "group_background_color": "theme",
        "group_background_scope": "group",
        "animations": false,
        "nice": 0,
        "github_panel_prefer_origin": false,
        "global_hotkey": "cmd-shift-t",
        "global_hotkey_fullscreen": true,
        "global_hotkey_hide_on_blur": true,
        "global_hotkey_fade_ms": 0
    }"##;

    #[test]
    fn an_old_file_loads_and_saves_the_same_top_level_keys() {
        let old: serde_json::Value = serde_json::from_str(OLD_FILE).unwrap();
        let cfg: Config = serde_json::from_str(OLD_FILE).unwrap();
        assert!(!cfg.fork.restore_asleep);
        assert!(cfg.fork.resume_agents_on_launch);
        assert_eq!(cfg.fork.agent_resume_mode, super::AgentResumeMode::Program);
        assert!(cfg.fork.continue_interrupted_agents);
        assert_eq!(cfg.fork.continue_prompt, "go on");
        assert_eq!(cfg.fork.dir_frecency["/Users/me/src/x"].count, 2);
        assert_eq!(cfg.fork.nice, 0);
        assert_eq!(cfg.font_size, 15.0, "upstream's keys are untouched");

        let saved = serde_json::to_value(&cfg).unwrap();
        let saved = saved.as_object().unwrap();
        assert!(!saved.contains_key("fork"), "flattened, not nested");
        for (key, value) in old.as_object().unwrap() {
            assert_eq!(saved.get(key), Some(value), "{key}");
        }
        let defaults = serde_json::to_value(Config::default()).unwrap();
        for key in old.as_object().unwrap().keys() {
            assert!(defaults.get(key).is_some(), "a default file writes {key}");
        }
    }

    /// A fork key that upstream's `Config` also had would be written twice
    /// and read by only one of them.
    #[test]
    fn fork_keys_and_upstream_keys_do_not_overlap() {
        /// The top-level keys of a JSON object as written, duplicates kept
        /// (a `serde_json::Value` would fold them into one).
        struct Keys(Vec<String>);
        impl<'de> serde::Deserialize<'de> for Keys {
            fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
                struct V;
                impl<'de> serde::de::Visitor<'de> for V {
                    type Value = Keys;
                    fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
                        f.write_str("an object")
                    }
                    fn visit_map<A: serde::de::MapAccess<'de>>(
                        self,
                        mut map: A,
                    ) -> Result<Keys, A::Error> {
                        let mut keys = Vec::new();
                        while let Some((k, serde::de::IgnoredAny)) = map.next_entry()? {
                            keys.push(k);
                        }
                        Ok(Keys(keys))
                    }
                }
                d.deserialize_map(V)
            }
        }
        let keys = |text: String| serde_json::from_str::<Keys>(&text).unwrap().0;
        let fork = keys(serde_json::to_string(&super::ForkConfig::default()).unwrap());
        let all = keys(serde_json::to_string(&Config::default()).unwrap());
        let unique: std::collections::BTreeSet<_> = all.iter().collect();
        assert_eq!(unique.len(), all.len(), "a key written twice");
        assert!(
            fork.iter().all(|k| unique.contains(k)),
            "every fork key reaches the file"
        );
    }

    /// The GitHub panel's Session tab replaced the first two;
    /// `agent_wake_concurrency` replaced `continue_stagger_ms`.
    #[test]
    fn retired_keys_still_load() {
        let cfg: Config = serde_json::from_str(
            r#"{"github_panel_session_filter": false, "github_panel_default_list": "issues", "continue_stagger_ms": 500, "nice": 2}"#,
        )
        .unwrap();
        assert_eq!(cfg.fork.nice, 2);
        let saved = serde_json::to_value(&cfg).unwrap();
        assert!(saved.get("github_panel_default_list").is_none());
        assert!(saved.get("github_panel_session_filter").is_none());
        assert!(saved.get("continue_stagger_ms").is_none());
    }

    /// Upstream's behaviour until the user opts in.
    #[test]
    fn the_resume_settings_default_to_upstream_behaviour() {
        let cfg = super::ForkConfig::default();
        assert!(!cfg.resume_agents_on_launch);
        assert_eq!(cfg.agent_resume_mode, super::AgentResumeMode::Typed);
        assert!(!cfg.continue_interrupted_agents);
        assert_eq!(cfg.continue_prompt, super::DEFAULT_CONTINUE_PROMPT);
        assert_eq!(cfg.agent_wake_concurrency, None);
    }

    #[test]
    fn a_bad_fork_value_falls_back_without_failing_the_file() {
        let cfg: Config =
            serde_json::from_str(r#"{"font_size": 13.0, "group_outline_color": 7}"#).unwrap();
        assert_eq!(
            cfg.fork.group_outline_color,
            super::GroupColorSource::Hashed
        );
        assert_eq!(cfg.font_size, 13.0);
        assert_eq!(cfg.fork.continue_prompt, super::DEFAULT_CONTINUE_PROMPT);
        assert_eq!(cfg.fork.nice, 5);
    }
}

//! The fork's settings, kept out of upstream's `Config` so a rebase touches
//! one field there: `Config::fork`, flattened, so they sit at the top level
//! of `config.json` exactly as they did when they were `Config`'s own.

use std::collections::{BTreeSet, HashMap};

use serde::{Deserialize, Serialize};

use super::config::{ProfileUsage, de_lenient};

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
    /// Agents the new tab page leaves out, by slug, though they are on `PATH`.
    pub new_tab_hidden_agents: BTreeSet<String>,
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
    /// The sidebar row ends with the front pane's shell pid.
    pub tab_usage_pid: bool,
    /// …with the CPU% of every process in the tab's panes.
    pub tab_usage_cpu: bool,
    /// …with their resident memory.
    pub tab_usage_memory: bool,
    /// A tab past the high-use marks shows its CPU or memory in the warning
    /// colour even with those switched off.
    pub tab_usage_annotate: bool,
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
}

impl Default for ForkConfig {
    fn default() -> Self {
        Self {
            restore_asleep: true,
            continue_prompt: "continue".into(),
            agent_wake_concurrency: None,
            resume_agents_on_launch: false,
            agent_resume_mode: AgentResumeMode::Typed,
            continue_interrupted_agents: false,
            new_tab_page: true,
            new_tab_hidden_agents: BTreeSet::new(),
            dir_roots: ["~/src", "~/code", "~/projects"].map(String::from).to_vec(),
            dir_frecency: HashMap::new(),
            group_colors: HashMap::new(),
            group_outline: false,
            group_background: true,
            group_outline_color: GroupColorSource::Hashed,
            group_background_color: GroupColorSource::Hashed,
            group_background_scope: GroupBackgroundScope::Header,
            animations: true,
            tab_usage_pid: false,
            tab_usage_cpu: false,
            tab_usage_memory: false,
            tab_usage_annotate: true,
            nice: 5,
            github_panel_prefer_origin: true,
            global_hotkey: Some("alt-space".into()),
            global_hotkey_fullscreen: false,
            global_hotkey_hide_on_blur: false,
            global_hotkey_fade_ms: 150,
        }
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
        assert_eq!(cfg.continue_prompt, "continue");
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
        assert_eq!(cfg.fork.continue_prompt, "continue");
        assert_eq!(cfg.fork.nice, 5);
    }
}

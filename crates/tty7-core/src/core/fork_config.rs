//! The fork's settings, kept out of upstream's `Config` so a rebase touches
//! one field there: `Config::fork`, flattened, so they sit at the top level
//! of `config.json` exactly as they did when they were `Config`'s own.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use super::config::{ProfileUsage, de_lenient};

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(default)]
pub struct ForkConfig {
    /// Restore a tab whose panes all died (reboot, Quit and Stop) asleep, so
    /// it starts — and resumes its agent — when first opened, not at launch.
    #[serde(default = "yes")]
    pub restore_asleep: bool,
    /// What Continue All Agents tells each agent it resumes. Empty resumes
    /// them without a word.
    #[serde(default = "default_continue_prompt")]
    pub continue_prompt: String,
    /// The gap Continue All Agents leaves between one tab's resume and the
    /// next, so they do not all cold-start and hit the API at once.
    #[serde(default = "default_continue_stagger_ms")]
    pub continue_stagger_ms: u64,
    /// Wake every sleeping tab with an agent session at launch, staggered
    /// like Continue All Agents but with no prompt; shell-only tabs stay
    /// asleep.
    #[serde(default = "yes")]
    pub resume_agents_on_launch: bool,
    /// New Tab opens the new tab page — pick an agent or a terminal and a
    /// directory — instead of a shell in the current tab's directory.
    #[serde(default = "yes")]
    pub new_tab_page: bool,
    /// Directories whose children the new tab page offers (`~` expands).
    #[serde(default = "default_dir_roots")]
    pub dir_roots: Vec<String>,
    /// How often and how recently each directory was launched into from the
    /// new tab page, keyed by its absolute path.
    #[serde(default)]
    pub dir_frecency: HashMap<String, ProfileUsage>,
    /// Sidebar group colour overrides, by group name: `"clawmux": "#ffd7d7"`.
    #[serde(default)]
    pub group_colors: HashMap<String, String>,
    /// A 1px outline round each sidebar group header.
    #[serde(default)]
    pub group_outline: bool,
    /// A tinted fill behind each sidebar group header.
    #[serde(default = "yes")]
    pub group_background: bool,
    #[serde(default, deserialize_with = "de_lenient")]
    pub group_outline_color: GroupColorSource,
    #[serde(default, deserialize_with = "de_lenient")]
    pub group_background_color: GroupColorSource,
    /// Whether the fill and outline wrap the header alone or the whole group.
    #[serde(default, deserialize_with = "de_lenient")]
    pub group_background_scope: GroupBackgroundScope,
    /// Animate a sidebar group folding open and shut.
    #[serde(default = "yes")]
    pub animations: bool,
    /// Niceness every pane's shell starts at, so typing stays responsive while
    /// agents build; 0 leaves it alone. Unix only.
    #[serde(default = "default_nice")]
    pub nice: i32,
    /// The GitHub panel's pull requests show only those the focused pane's
    /// agent session mentions.
    #[serde(default = "yes")]
    pub github_panel_session_filter: bool,
    /// The GitHub panel shows `origin` (the fork) until a remote is picked,
    /// instead of `upstream`.
    #[serde(default = "yes")]
    pub github_panel_prefer_origin: bool,
    /// Which list the GitHub panel opens on.
    #[serde(default)]
    pub github_panel_default_list: GitHubPanelList,
    /// The system-wide chord that shows and hides tty7, in keymap syntax
    /// (`alt-space`, `cmd-shift-t`); `null` or `""` turns it off. macOS only.
    #[serde(default = "default_global_hotkey")]
    pub global_hotkey: Option<String>,
    /// The hotkey summons the window over the whole screen the mouse is on,
    /// floating above other apps.
    #[serde(default)]
    pub global_hotkey_fullscreen: bool,
    /// Hide tty7 whenever it loses focus after the hotkey summoned it.
    #[serde(default)]
    pub global_hotkey_hide_on_blur: bool,
    /// How long the hotkey's fade in and out takes; 0 is instant.
    #[serde(default = "default_global_hotkey_fade_ms")]
    pub global_hotkey_fade_ms: u64,
}

impl Default for ForkConfig {
    fn default() -> Self {
        Self {
            restore_asleep: true,
            continue_prompt: default_continue_prompt(),
            continue_stagger_ms: default_continue_stagger_ms(),
            resume_agents_on_launch: true,
            new_tab_page: true,
            dir_roots: default_dir_roots(),
            dir_frecency: HashMap::new(),
            group_colors: HashMap::new(),
            group_outline: false,
            group_background: true,
            group_outline_color: GroupColorSource::Hashed,
            group_background_color: GroupColorSource::Hashed,
            group_background_scope: GroupBackgroundScope::Header,
            animations: true,
            nice: default_nice(),
            github_panel_session_filter: true,
            github_panel_prefer_origin: true,
            github_panel_default_list: GitHubPanelList::Issues,
            global_hotkey: default_global_hotkey(),
            global_hotkey_fullscreen: false,
            global_hotkey_hide_on_blur: false,
            global_hotkey_fade_ms: default_global_hotkey_fade_ms(),
        }
    }
}

fn yes() -> bool {
    true
}

fn default_continue_prompt() -> String {
    "continue".into()
}

fn default_continue_stagger_ms() -> u64 {
    3000
}

fn default_global_hotkey() -> Option<String> {
    Some("alt-space".to_string())
}

fn default_global_hotkey_fade_ms() -> u64 {
    150
}

fn default_dir_roots() -> Vec<String> {
    ["~/src", "~/code", "~/projects"].map(String::from).to_vec()
}

fn default_nice() -> i32 {
    5
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

/// How much of a sidebar group its fill and outline cover.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum GroupBackgroundScope {
    #[default]
    Header,
    /// The header and its rows, as one block.
    Group,
}

/// The GitHub panel's list on open.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GitHubPanelList {
    #[default]
    Issues,
    PullRequests,
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
        "continue_stagger_ms": 500,
        "resume_agents_on_launch": false,
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
        "github_panel_session_filter": false,
        "github_panel_prefer_origin": false,
        "github_panel_default_list": "pull_requests",
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

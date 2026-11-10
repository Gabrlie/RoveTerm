//! User preferences. [`Preferences`] follow the user (and will sync between devices);
//! [`DevicePrefs`] describe this device only. Fields missing from a stored record take their
//! defaults, so older records stay readable.

use serde::{Deserialize, Serialize};

use crate::VaultError;

/// All preferences that follow the user, stored as one record.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Preferences {
    /// Window theme, accent, scale and language.
    pub appearance: AppearancePrefs,
    /// Terminal appearance and behaviour.
    pub terminal: TerminalPrefs,
    /// File panel behaviour.
    pub files: FilePrefs,
    /// Server status panel.
    pub status: StatusPrefs,
    /// AI approvals.
    pub ai: AiPrefs,
}

/// How much the user is asked before an AI agent acts (see `rove_ai::Mode`).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AiMode {
    /// Read-only operations run; modifications are asked about; dangerous ones always are.
    #[default]
    AskByRisk,
    /// Everything is asked about.
    AskAll,
    /// Everything runs except dangerous operations.
    Auto,
}

/// AI approval settings, which follow the user.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct AiPrefs {
    /// Default approval mode of new AI sessions.
    pub mode: AiMode,
    /// Extra programs treated as read-only (base names).
    pub read_only: Vec<String>,
    /// Text that makes any command containing it dangerous.
    pub dangerous: Vec<String>,
}

/// How the agent signs in.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentAuth {
    /// With the stored API key when there is one, else the agent's own sign-in.
    #[default]
    Auto,
    /// With the stored API key.
    ApiKey,
    /// With the agent's own account sign-in (such as `codex login` with ChatGPT).
    Account,
}

/// The AI agent program on this device.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct AgentPrefs {
    /// The agent's ACP program; empty finds Codex's adapter automatically.
    pub command: String,
    /// Arguments for `command`.
    pub args: Vec<String>,
    pub auth: AgentAuth,
}

/// Light or dark interface.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Theme {
    /// Follows the operating system.
    #[default]
    System,
    /// Always light.
    Light,
    /// Always dark.
    Dark,
}

/// Accent colour presets, named after the system palette.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Accent {
    /// Blue.
    #[default]
    Blue,
    /// Purple.
    Purple,
    /// Pink.
    Pink,
    /// Red.
    Red,
    /// Orange.
    Orange,
    /// Yellow.
    Yellow,
    /// Green.
    Green,
    /// Grey.
    Graphite,
}

/// Interface language.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Language {
    /// Follows the system language.
    #[default]
    #[serde(rename = "auto")]
    Auto,
    /// Simplified Chinese.
    #[serde(rename = "zh-CN")]
    ZhCn,
    /// English.
    #[serde(rename = "en")]
    En,
}

/// Window appearance.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct AppearancePrefs {
    /// Light, dark or following the system.
    pub theme: Theme,
    /// Accent colour.
    pub accent: Accent,
    /// Interface scale in percent (90–125).
    pub ui_scale: u16,
    /// Interface language.
    pub language: Language,
}

impl Default for AppearancePrefs {
    fn default() -> Self {
        Self {
            theme: Theme::System,
            accent: Accent::Blue,
            ui_scale: 100,
            language: Language::Auto,
        }
    }
}

/// Terminal cursor shape.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CursorStyle {
    /// Filled block.
    #[default]
    Block,
    /// Vertical bar.
    Bar,
    /// Underline.
    Underline,
}

/// What a right click in the terminal does.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RightClick {
    /// Opens the context menu.
    #[default]
    Menu,
    /// Pastes the clipboard.
    Paste,
}

/// How the terminal bell is shown.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Bell {
    /// Ignored.
    None,
    /// Briefly flashes the pane.
    #[default]
    Visual,
}

/// Terminal appearance and behaviour.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct TerminalPrefs {
    /// Preferred font family; empty uses the built-in font stack. The built-in stack and the
    /// bundled symbol font always follow it as fallbacks.
    pub font_family: String,
    /// Font size in CSS pixels (8–32).
    pub font_size: u16,
    /// Line height as a multiple of the font size (1.0–2.0).
    pub line_height: f32,
    /// Colour scheme id used with the light interface. The UI owns the scheme catalogue and
    /// falls back to its default for an unknown id.
    pub scheme_light: String,
    /// Colour scheme id used with the dark interface.
    pub scheme_dark: String,
    /// Cursor shape.
    pub cursor_style: CursorStyle,
    /// Whether the cursor blinks.
    pub cursor_blink: bool,
    /// Lines kept above the screen (1000–100000).
    pub scrollback: u32,
    /// Copies selected text to the clipboard as soon as it is selected.
    pub copy_on_select: bool,
    /// Right-click behaviour.
    pub right_click: RightClick,
    /// Bell behaviour.
    pub bell: Bell,
}

impl Default for TerminalPrefs {
    fn default() -> Self {
        Self {
            font_family: String::new(),
            font_size: 13,
            line_height: 1.15,
            scheme_light: "rove-light".into(),
            scheme_dark: "rove-dark".into(),
            cursor_style: CursorStyle::Block,
            cursor_blink: false,
            scrollback: 10_000,
            copy_on_select: false,
            right_click: RightClick::Menu,
            bell: Bell::Visual,
        }
    }
}

/// What happens when a transfer meets an existing name.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConflictChoice {
    /// Asks every time.
    #[default]
    Ask,
    /// Keeps both, renaming the new one.
    Rename,
    /// Replaces the existing item.
    Overwrite,
    /// Leaves the existing item and skips the new one.
    Skip,
}

/// File panel behaviour.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct FilePrefs {
    /// Default answer to a name clash on upload or download.
    pub default_conflict: ConflictChoice,
    /// Shows entries whose names start with a dot.
    pub show_hidden: bool,
    /// Asks before deleting.
    pub confirm_delete: bool,
}

impl Default for FilePrefs {
    fn default() -> Self {
        Self {
            default_conflict: ConflictChoice::Ask,
            show_hidden: false,
            confirm_delete: true,
        }
    }
}

/// Server status panel.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct StatusPrefs {
    /// Seconds between samples (2–30).
    pub interval_secs: u16,
}

impl Default for StatusPrefs {
    fn default() -> Self {
        Self { interval_secs: 3 }
    }
}

/// Preferences of this device only; never leave it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct DevicePrefs {
    /// Shows the system window material (Liquid Glass, vibrancy, Mica) where available.
    pub window_material: bool,
    /// Absolute directory downloads go to; empty uses the system download directory.
    pub download_dir: String,
    /// Transfers copying at the same time (1–8).
    pub max_transfers: u8,
    /// The AI agent program.
    pub agent: AgentPrefs,
    /// SSH connection timing on this device's networks.
    pub connection: ConnectionPrefs,
}

/// SSH connection timing; applies to connections opened after a change.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct ConnectionPrefs {
    /// Seconds allowed to open a connection (5–120).
    pub connect_timeout_secs: u16,
    /// Seconds between keepalives (5–600).
    pub keepalive_secs: u16,
    /// Unanswered keepalives before a connection counts as lost (1–10).
    pub keepalive_max: u8,
}

impl Default for ConnectionPrefs {
    fn default() -> Self {
        Self {
            connect_timeout_secs: 15,
            keepalive_secs: 30,
            keepalive_max: 3,
        }
    }
}

impl Default for DevicePrefs {
    fn default() -> Self {
        Self {
            window_material: true,
            download_dir: String::new(),
            max_transfers: 3,
            agent: AgentPrefs::default(),
            connection: ConnectionPrefs::default(),
        }
    }
}

fn check_range<T: PartialOrd + std::fmt::Display>(
    name: &str,
    value: T,
    range: std::ops::RangeInclusive<T>,
) -> Result<(), VaultError> {
    if range.contains(&value) {
        Ok(())
    } else {
        Err(VaultError::Invalid(format!(
            "{name} must be {}–{}",
            range.start(),
            range.end()
        )))
    }
}

/// A scheme id is a short lowercase slug.
fn check_scheme(name: &str, id: &str) -> Result<(), VaultError> {
    let valid = !id.is_empty()
        && id.len() <= 40
        && id
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-');
    if valid {
        Ok(())
    } else {
        Err(VaultError::Invalid(format!("{name} is not a scheme id")))
    }
}

impl Preferences {
    /// Trims text and checks ranges. Returns the normalized preferences.
    pub fn validate(mut self) -> Result<Self, VaultError> {
        let terminal = &mut self.terminal;
        terminal.font_family = terminal.font_family.trim().to_owned();
        if terminal.font_family.chars().count() > 100 {
            return Err(VaultError::Invalid("font family is too long".into()));
        }
        // The family is placed into a CSS font list; quotes, separators and control characters
        // would change its meaning.
        if terminal
            .font_family
            .chars()
            .any(|c| c.is_control() || matches!(c, '"' | '\'' | ',' | ';' | '\\' | '{' | '}'))
        {
            return Err(VaultError::Invalid(
                "font family contains characters that are not allowed".into(),
            ));
        }
        if !(8..=32).contains(&terminal.font_size) {
            return Err(VaultError::Invalid("font size must be 8–32".into()));
        }
        if !(1.0..=2.0).contains(&terminal.line_height) {
            return Err(VaultError::Invalid("line height must be 1.0–2.0".into()));
        }
        check_scheme("light scheme", &terminal.scheme_light)?;
        check_scheme("dark scheme", &terminal.scheme_dark)?;
        check_range("scrollback", terminal.scrollback, 1_000..=100_000)?;
        check_range("interface scale", self.appearance.ui_scale, 90..=125)?;
        check_range("status interval", self.status.interval_secs, 2..=30)?;
        self.ai.validate()?;
        Ok(self)
    }
}

/// Most entries in one AI rule list.
const MAX_AI_RULES: usize = 200;
/// Longest AI rule entry.
const MAX_AI_RULE: usize = 100;

impl AiPrefs {
    /// Trims entries, drops empty ones and checks sizes; read-only entries must be program names.
    fn validate(&mut self) -> Result<(), VaultError> {
        for (name, list) in [
            ("read-only", &mut self.read_only),
            ("dangerous", &mut self.dangerous),
        ] {
            *list = list
                .iter()
                .map(|entry| entry.trim().to_owned())
                .filter(|entry| !entry.is_empty())
                .collect();
            if list.len() > MAX_AI_RULES {
                return Err(VaultError::Invalid(format!("too many {name} rules")));
            }
            if let Some(bad) = list.iter().find(|entry| {
                entry.chars().count() > MAX_AI_RULE || entry.chars().any(char::is_control)
            }) {
                return Err(VaultError::Invalid(format!(
                    "{name} rule {bad:?} is not valid"
                )));
            }
        }
        if let Some(bad) = self.read_only.iter().find(|entry| {
            !entry
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || "-_.+".contains(c))
        }) {
            return Err(VaultError::Invalid(format!(
                "read-only rule {bad:?} is not a program name"
            )));
        }
        Ok(())
    }
}

impl AgentPrefs {
    /// Trims the command and checks sizes.
    fn validate(&mut self) -> Result<(), VaultError> {
        self.command = self.command.trim().to_owned();
        let bad = |text: &str| text.len() > 1024 || text.chars().any(char::is_control);
        if bad(&self.command) || self.args.len() > 32 || self.args.iter().any(|arg| bad(arg)) {
            return Err(VaultError::Invalid("the agent command is not valid".into()));
        }
        Ok(())
    }
}

impl DevicePrefs {
    /// Trims the download directory and checks ranges. Returns the normalized preferences. The
    /// directory must be absolute; whether it exists is checked when downloading.
    pub fn validate(mut self) -> Result<Self, VaultError> {
        self.download_dir = self.download_dir.trim().to_owned();
        if !self.download_dir.is_empty() {
            let path = std::path::Path::new(&self.download_dir);
            if self.download_dir.len() > 1024
                || self.download_dir.chars().any(char::is_control)
                || !path.is_absolute()
            {
                return Err(VaultError::Invalid(
                    "download directory must be an absolute path".into(),
                ));
            }
        }
        check_range("concurrent transfers", self.max_transfers, 1..=8)?;
        self.agent.validate()?;
        check_range(
            "connect timeout",
            self.connection.connect_timeout_secs,
            5..=120,
        )?;
        check_range(
            "keepalive interval",
            self.connection.keepalive_secs,
            5..=600,
        )?;
        check_range("keepalive limit", self.connection.keepalive_max, 1..=10)?;
        Ok(self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn with(family: &str, size: u16, line_height: f32) -> Preferences {
        Preferences {
            terminal: TerminalPrefs {
                font_family: family.into(),
                font_size: size,
                line_height,
                ..TerminalPrefs::default()
            },
            ..Preferences::default()
        }
    }

    #[test]
    fn validates_terminal_preferences() {
        let ok = with("  JetBrainsMono Nerd Font ", 14, 1.2)
            .validate()
            .unwrap();
        assert_eq!(ok.terminal.font_family, "JetBrainsMono Nerd Font");
        assert!(with("", 13, 1.0).validate().is_ok());
        assert!(with("Evil\", serif", 13, 1.2).validate().is_err());
        assert!(with("A;B", 13, 1.2).validate().is_err());
        assert!(with(&"x".repeat(101), 13, 1.2).validate().is_err());
        assert!(with("Menlo", 7, 1.2).validate().is_err());
        assert!(with("Menlo", 13, 2.5).validate().is_err());
        assert!(with("Menlo", 13, f32::NAN).validate().is_err());
    }

    #[test]
    fn missing_fields_take_defaults() {
        let parsed: Preferences = serde_json::from_str(r#"{"terminal":{"font_size":16}}"#).unwrap();
        assert_eq!(parsed.terminal.font_size, 16);
        assert_eq!(parsed.terminal.line_height, 1.15);
        let empty: Preferences = serde_json::from_str("{}").unwrap();
        assert_eq!(empty, Preferences::default());
    }

    #[test]
    fn checks_new_ranges_and_scheme_ids() {
        let mut prefs = Preferences::default();
        assert!(prefs.clone().validate().is_ok());
        prefs.terminal.scrollback = 999;
        assert!(prefs.clone().validate().is_err());
        prefs.terminal.scrollback = 5_000;
        prefs.terminal.scheme_dark = "Evil Scheme".into();
        assert!(prefs.clone().validate().is_err());
        prefs.terminal.scheme_dark = "tokyo-night".into();
        prefs.appearance.ui_scale = 130;
        assert!(prefs.clone().validate().is_err());
        prefs.appearance.ui_scale = 110;
        prefs.status.interval_secs = 1;
        assert!(prefs.clone().validate().is_err());
        prefs.status.interval_secs = 5;
        assert!(prefs.validate().is_ok());
    }

    #[test]
    fn reads_enums_by_their_wire_names_and_old_records() {
        let parsed: Preferences = serde_json::from_str(
            r#"{"appearance":{"theme":"dark","accent":"graphite","language":"zh-CN"},
                "terminal":{"font_size":15,"cursor_style":"bar","right_click":"paste"},
                "files":{"default_conflict":"overwrite"}}"#,
        )
        .unwrap();
        assert_eq!(parsed.appearance.theme, Theme::Dark);
        assert_eq!(parsed.appearance.accent, Accent::Graphite);
        assert_eq!(parsed.appearance.language, Language::ZhCn);
        assert_eq!(parsed.appearance.ui_scale, 100);
        assert_eq!(parsed.terminal.cursor_style, CursorStyle::Bar);
        assert_eq!(parsed.terminal.right_click, RightClick::Paste);
        assert_eq!(parsed.terminal.scheme_light, "rove-light");
        assert_eq!(parsed.files.default_conflict, ConflictChoice::Overwrite);
        assert!(parsed.files.confirm_delete);
        // A record written before these groups existed.
        let old: Preferences = serde_json::from_str(
            r#"{"terminal":{"font_family":"Menlo","font_size":14,"line_height":1.2}}"#,
        )
        .unwrap();
        assert_eq!(old.terminal.font_family, "Menlo");
        assert_eq!(old.appearance, AppearancePrefs::default());
        assert_eq!(old.status.interval_secs, 3);
    }

    #[test]
    fn validates_device_preferences() {
        let ok = DevicePrefs {
            download_dir: if cfg!(windows) {
                "  C:\\Downloads ".into()
            } else {
                "  /data/downloads ".into()
            },
            ..DevicePrefs::default()
        }
        .validate()
        .unwrap();
        assert!(!ok.download_dir.starts_with(' '));
        assert!(DevicePrefs::default().validate().is_ok());
        let relative = DevicePrefs {
            download_dir: "downloads".into(),
            ..DevicePrefs::default()
        };
        assert!(relative.validate().is_err());
        let too_many = DevicePrefs {
            max_transfers: 9,
            ..DevicePrefs::default()
        };
        assert!(too_many.validate().is_err());
    }
}

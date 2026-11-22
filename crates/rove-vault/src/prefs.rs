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

/// How a model provider is spoken to (see `rove_ai::llm::Format`).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderFormat {
    /// OpenAI Chat Completions, which most providers speak.
    #[default]
    Chat,
    /// OpenAI Responses.
    Responses,
    /// Anthropic Messages.
    Anthropic,
}

/// How much a model reasons, where the format can set it.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReasoningEffort {
    Minimal,
    Low,
    #[default]
    Medium,
    High,
    #[serde(rename = "xhigh")]
    XHigh,
    Max,
}

/// A model provider of the built-in assistant. Its API key is kept apart, on each device.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderPrefs {
    /// Stable id: lowercase letters, digits and `-`.
    pub id: String,
    pub name: String,
    pub format: ProviderFormat,
    /// Base URL including the API version path, such as `https://api.openai.com/v1`.
    pub base_url: String,
    /// The model new conversations start with.
    pub model: String,
    /// Other models of the provider to switch to in a conversation.
    #[serde(default)]
    pub models: Vec<String>,
    #[serde(default)]
    pub max_tokens: Option<u32>,
    /// Context windows of the models in tokens, as the provider's model list told them.
    #[serde(default)]
    pub contexts: std::collections::BTreeMap<String, u64>,
    /// Context window of the models the list says nothing about, when the user set one.
    #[serde(default)]
    pub context_window: Option<u64>,
    /// Older versions left it empty for "the model's default", read as `medium`.
    #[serde(default, deserialize_with = "effort_or_medium")]
    pub effort: ReasoningEffort,
}

fn effort_or_medium<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<ReasoningEffort, D::Error> {
    Ok(Option::<ReasoningEffort>::deserialize(deserializer)?.unwrap_or_default())
}

/// AI settings that follow the user.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct AiPrefs {
    /// Default approval mode of new AI sessions.
    pub mode: AiMode,
    /// Model providers of the built-in assistant.
    pub providers: Vec<ProviderPrefs>,
    /// The agent new conversations start with: `builtin:<provider id>` or `acp:<agent id>`.
    pub default_agent: Option<String>,
    /// Extra programs treated as read-only (base names).
    pub read_only: Vec<String>,
    /// Text that makes any command containing it dangerous.
    pub dangerous: Vec<String>,
    /// Most model requests of the built-in assistant in one turn (its tool calls included).
    pub max_steps: u32,
    /// How long the built-in assistant waits for a reply to start, and between its pieces.
    pub request_timeout_secs: u32,
    /// The judge of the auto mode: who decides ordinary modifications before the user is asked.
    pub reviewer: ReviewerKind,
    /// The model the `chosen` judge uses.
    pub review_model: Option<ReviewModel>,
    /// How sure the reviewer must be to let an operation run, in percent (50–99).
    pub review_confidence: u8,
    /// Tasks the fleet lead runs on a schedule while the app is open.
    pub schedules: Vec<Schedule>,
}

/// A task the fleet lead runs on its own at set times.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Schedule {
    #[serde(default)]
    pub owner: Option<uuid::Uuid>,
    /// Stable id: 1–40 lowercase letters, digits or `-`.
    pub id: String,
    pub name: String,
    /// What the fleet lead is told to do.
    pub instruction: String,
    pub repeat: Repeat,
    /// Changes the judge is sure of may be made; otherwise the run only checks and reports.
    #[serde(default)]
    pub allow_changes: bool,
    /// A run missed while the app was closed is made at the next start (the latest one only).
    #[serde(default = "yes")]
    pub catch_up: bool,
    #[serde(default = "yes")]
    pub enabled: bool,
    /// When it was made (Unix milliseconds): times before it are not due.
    pub created_at: i64,
}

fn yes() -> bool {
    true
}

/// When a schedule runs, in the device's local time.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Repeat {
    /// Once, at `at` (Unix milliseconds).
    Once { at: i64 },
    /// Every hour at `minute`.
    Hourly { minute: u8 },
    /// Every day at `hour`:`minute`.
    Daily { hour: u8, minute: u8 },
    /// Every week on `weekday` (0 Monday … 6 Sunday) at `hour`:`minute`.
    Weekly { weekday: u8, hour: u8, minute: u8 },
}

/// Most schedules kept.
const MAX_SCHEDULES: usize = 50;

impl Schedule {
    fn validate(&mut self) -> Result<(), VaultError> {
        let invalid = |message: &str| Err(VaultError::Invalid(message.into()));
        if self.id.is_empty()
            || self.id.len() > 40
            || !self
                .id
                .chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
        {
            return invalid("a schedule id is not valid");
        }
        self.name = self.name.trim().to_owned();
        if self.name.is_empty() || self.name.chars().count() > 60 {
            return invalid("a schedule name must be 1–60 characters");
        }
        self.instruction = self.instruction.trim().to_owned();
        if self.instruction.is_empty() || self.instruction.len() > 8000 {
            return invalid("a schedule's instruction must be 1–8000 bytes");
        }
        let ok = match self.repeat {
            Repeat::Once { at } => at > 0,
            Repeat::Hourly { minute } => minute < 60,
            Repeat::Daily { hour, minute } => hour < 24 && minute < 60,
            Repeat::Weekly {
                weekday,
                hour,
                minute,
            } => weekday < 7 && hour < 24 && minute < 60,
        };
        if !ok || self.created_at <= 0 {
            return invalid("a schedule's time is not valid");
        }
        Ok(())
    }
}

/// The judge of the auto mode.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReviewerKind {
    /// The session's own model (for local agents: the default provider's). Older records'
    /// `off` (when the review mode had a judge) reads as this.
    #[default]
    #[serde(alias = "off")]
    Model,
    /// A model chosen for judging (`review_model`), such as a cheaper one.
    Chosen,
    /// TypeSafe's Jev, with its own API key.
    Jev,
}

/// A provider's model, by the provider's id.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReviewModel {
    pub provider: String,
    pub model: String,
}

impl Default for AiPrefs {
    fn default() -> Self {
        Self {
            mode: AiMode::default(),
            providers: Vec::new(),
            default_agent: None,
            read_only: Vec::new(),
            dangerous: Vec::new(),
            max_steps: 40,
            request_timeout_secs: 180,
            reviewer: ReviewerKind::Model,
            review_model: None,
            schedules: Vec::new(),
            review_confidence: 80,
        }
    }
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

/// An ACP agent the user added by command.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CustomAgent {
    /// Stable id: `custom-` followed by lowercase letters, digits and `-`.
    pub id: String,
    pub name: String,
    pub command: String,
    #[serde(default)]
    pub args: Vec<String>,
}

/// ACP agents on this device.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct AgentsPrefs {
    /// Ids of detected agents the user turned off.
    pub disabled: Vec<String>,
    /// Agents added by command.
    pub custom: Vec<CustomAgent>,
    /// How agents sign in.
    pub auth: AgentAuth,
    /// Agents the user runs on their own (Claude Code, Codex in a terminal) may use the hosts
    /// saved here through RoveTerm's MCP server, with RoveTerm's approvals.
    pub expose: bool,
}

/// The AI agent program on this device, before agents were listed; read from older records and
/// moved into [`AgentsPrefs::custom`].
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

/// How CPU usage is presented.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CpuDisplay {
    #[default]
    Auto,
    PerCore,
    Summary,
}

/// Unit used to display network throughput.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NetworkUnit {
    #[default]
    Bytes,
    Bits,
}

/// How memory usage is formatted in the terminal status bar.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MemoryDisplay {
    #[default]
    FullPercent,
    Full,
    Used,
    Percent,
}

/// How network throughput is formatted in the terminal status bar.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NetworkDisplay {
    #[default]
    Split,
    Total,
}

/// Server status panel.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct StatusPrefs {
    /// Seconds between samples (2–30).
    pub interval_secs: u16,
    pub show_system: bool,
    pub show_cpu: bool,
    pub show_memory: bool,
    pub show_swap: bool,
    pub show_network: bool,
    pub show_disks: bool,
    pub cpu_display: CpuDisplay,
    pub network_unit: NetworkUnit,
    pub terminal_bar: bool,
    pub terminal_bar_cpu: bool,
    pub terminal_bar_memory: bool,
    pub terminal_bar_memory_display: MemoryDisplay,
    pub terminal_bar_network: bool,
    pub terminal_bar_network_display: NetworkDisplay,
    pub terminal_bar_latency: bool,
    pub terminal_bar_tailscale_path: bool,
}

impl Default for StatusPrefs {
    fn default() -> Self {
        Self {
            interval_secs: 3,
            show_system: true,
            show_cpu: true,
            show_memory: true,
            show_swap: true,
            show_network: true,
            show_disks: true,
            cpu_display: CpuDisplay::Auto,
            network_unit: NetworkUnit::Bytes,
            terminal_bar: true,
            terminal_bar_cpu: true,
            terminal_bar_memory: true,
            terminal_bar_memory_display: MemoryDisplay::FullPercent,
            terminal_bar_network: true,
            terminal_bar_network_display: NetworkDisplay::Split,
            terminal_bar_latency: false,
            terminal_bar_tailscale_path: true,
        }
    }
}

/// An external editor on this device. Empty `program` and `args` select the system opener.
///
/// Validation checks native absolute-path syntax and sizes without accessing the filesystem.
/// Callers must pass `args` as separate arguments, preserving their spaces; the application
/// substitutes `{file}` when it launches the editor, without composing a shell command.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct ExternalEditorPrefs {
    /// Absolute program path; on macOS an `.app` bundle is also accepted without arguments.
    pub program: String,
    /// Arguments passed directly to the program, with no trimming or shell expansion.
    pub args: Vec<String>,
}

/// Longest external-editor path or individual argument, in UTF-8 bytes.
const MAX_EDITOR_TEXT: usize = 4096;
/// Most external-editor arguments.
const MAX_EDITOR_ARGS: usize = 32;
/// Most UTF-8 bytes in all external-editor arguments together.
const MAX_EDITOR_ARGS_BYTES: usize = 32 * 1024;

impl ExternalEditorPrefs {
    /// Trims the program path and checks native absolute-path syntax, sizes and control
    /// characters. Empty configuration selects the system opener. Arguments require a program.
    /// Windows accepts only `.exe` programs. On macOS an `.app` bundle requires no arguments.
    /// The path need not exist, and arguments are returned unchanged.
    pub fn validate(mut self) -> Result<Self, VaultError> {
        if self.program.len() > MAX_EDITOR_TEXT || self.program.chars().any(char::is_control) {
            return Err(VaultError::Invalid(
                "external editor program is too long or contains control characters".into(),
            ));
        }
        self.program = self.program.trim().to_owned();
        if self.args.len() > MAX_EDITOR_ARGS
            || self
                .args
                .iter()
                .any(|arg| arg.len() > MAX_EDITOR_TEXT || arg.chars().any(char::is_control))
            || self.args.iter().map(String::len).sum::<usize>() > MAX_EDITOR_ARGS_BYTES
        {
            return Err(VaultError::Invalid(
                "external editor arguments are too many, too long or contain control characters"
                    .into(),
            ));
        }
        if self.program.is_empty() {
            if !self.args.is_empty() {
                return Err(VaultError::Invalid(
                    "external editor arguments require a program".into(),
                ));
            }
            return Ok(self);
        }
        let path = std::path::Path::new(&self.program);
        if !path.is_absolute() || path.file_name().is_none() {
            return Err(VaultError::Invalid(
                "external editor program must be an absolute path".into(),
            ));
        }
        #[cfg(windows)]
        if !path
            .extension()
            .is_some_and(|extension| extension.eq_ignore_ascii_case("exe"))
        {
            return Err(VaultError::Invalid(
                "external editor program must be an .exe file on Windows".into(),
            ));
        }
        #[cfg(target_os = "macos")]
        if path
            .extension()
            .is_some_and(|extension| extension.eq_ignore_ascii_case("app"))
            && !self.args.is_empty()
        {
            return Err(VaultError::Invalid(
                "external editor app bundles do not accept arguments".into(),
            ));
        }
        Ok(self)
    }
}

/// Preferences of this device only; never leave it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct DevicePrefs {
    /// Absolute directory downloads go to; empty uses the system download directory.
    pub download_dir: String,
    /// Transfers copying at the same time (1–8).
    pub max_transfers: u8,
    /// External editor program and arguments on this device only.
    pub external_editor: ExternalEditorPrefs,
    /// The older single agent setting; moved into `agents` when read, never written.
    #[serde(skip_serializing)]
    pub agent: AgentPrefs,
    /// ACP agents on this device.
    pub agents: AgentsPrefs,
    /// SSH connection timing on this device's networks.
    pub connection: ConnectionPrefs,
    /// Embedded tailnet configuration for this device.
    #[serde(default)]
    pub tailscale: TailscalePrefs,
}

/// Device-local tailnet connection settings.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct TailscalePrefs {
    pub configured: bool,
    pub auto_connect: bool,
    pub control_url: String,
    pub hostname: String,
}

impl Default for TailscalePrefs {
    fn default() -> Self {
        Self {
            configured: false,
            auto_connect: true,
            control_url: String::new(),
            hostname: "RoveTerm".to_owned(),
        }
    }
}

impl TailscalePrefs {
    /// Normalizes optional overrides and validates the control URL and hostname.
    pub fn validate(mut self) -> Result<Self, VaultError> {
        let control_url = self.control_url.trim();
        if self.control_url.chars().any(char::is_control)
            || control_url.len() > 512
            || (!control_url.is_empty()
                && !url::Url::parse(control_url).is_ok_and(|url| {
                    matches!(url.scheme(), "http" | "https")
                        && url.host().is_some()
                        && url.username().is_empty()
                        && url.password().is_none()
                }))
        {
            return Err(VaultError::Invalid("invalid Tailscale control URL".into()));
        }
        self.control_url = control_url.to_owned();

        let hostname = self.hostname.trim();
        if self.hostname.chars().any(char::is_control)
            || hostname.chars().count() > 63
            || hostname.chars().any(char::is_whitespace)
        {
            return Err(VaultError::Invalid("invalid Tailscale hostname".into()));
        }
        self.hostname = hostname.to_owned();
        Ok(self)
    }
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
            download_dir: String::new(),
            max_transfers: 3,
            external_editor: ExternalEditorPrefs::default(),
            agent: AgentPrefs::default(),
            agents: AgentsPrefs::default(),
            connection: ConnectionPrefs::default(),
            tailscale: TailscalePrefs::default(),
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
        if self.providers.len() > MAX_PROVIDERS {
            return Err(VaultError::Invalid("too many providers".into()));
        }
        if !(1..=200).contains(&self.max_steps) {
            return Err(VaultError::Invalid(
                "the requests per turn must be 1–200".into(),
            ));
        }
        if self.reviewer == ReviewerKind::Chosen {
            let offered = self.review_model.as_ref().is_some_and(|wanted| {
                self.providers.iter().any(|provider| {
                    provider.id == wanted.provider
                        && (provider.model == wanted.model
                            || provider.models.contains(&wanted.model))
                })
            });
            if !offered {
                return Err(VaultError::Invalid(
                    "the judge's model must be one of a provider's models".into(),
                ));
            }
        }
        if self.schedules.len() > MAX_SCHEDULES {
            return Err(VaultError::Invalid("too many schedules".into()));
        }
        for schedule in &mut self.schedules {
            schedule.validate()?;
        }
        let mut ids: Vec<&str> = self
            .schedules
            .iter()
            .map(|schedule| schedule.id.as_str())
            .collect();
        ids.sort_unstable();
        if ids.windows(2).any(|pair| pair[0] == pair[1]) {
            return Err(VaultError::Invalid("two schedules share an id".into()));
        }
        if !(50..=99).contains(&self.review_confidence) {
            return Err(VaultError::Invalid(
                "the review confidence must be 50–99 percent".into(),
            ));
        }
        if !(30..=1800).contains(&self.request_timeout_secs) {
            return Err(VaultError::Invalid(
                "the request timeout must be 30–1800 seconds".into(),
            ));
        }
        let mut ids = std::collections::HashSet::new();
        for provider in &mut self.providers {
            provider.validate()?;
            if !ids.insert(provider.id.clone()) {
                return Err(VaultError::Invalid(format!(
                    "provider id {:?} is used twice",
                    provider.id
                )));
            }
        }
        if let Some(agent) = &self.default_agent {
            check_agent_ref(agent)?;
        }
        Ok(())
    }
}

/// Most model providers.
const MAX_PROVIDERS: usize = 20;

/// `true` for a short id of lowercase letters, digits and `-`.
fn is_slug(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 40
        && id
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

/// An agent reference is `builtin:<id>` or `acp:<id>`.
fn check_agent_ref(agent: &str) -> Result<(), VaultError> {
    match agent.split_once(':') {
        Some(("builtin" | "acp", id)) if is_slug(id) => Ok(()),
        _ => Err(VaultError::Invalid(format!(
            "{agent:?} is not an agent reference"
        ))),
    }
}

impl ProviderPrefs {
    /// Trims text and checks it: a slug id, a name, an http(s) address without credentials, a
    /// model, and a positive token limit.
    fn validate(&mut self) -> Result<(), VaultError> {
        self.name = self.name.trim().to_owned();
        self.base_url = self.base_url.trim().trim_end_matches('/').to_owned();
        self.model = self.model.trim().to_owned();
        let invalid = |what: &str| {
            Err(VaultError::Invalid(format!(
                "provider {:?}: {what}",
                self.name
            )))
        };
        if !is_slug(&self.id) {
            return invalid("the id is not valid");
        }
        if self.name.is_empty()
            || self.name.chars().count() > 60
            || self.name.chars().any(char::is_control)
        {
            return invalid("the name is not valid");
        }
        let scheme_ok =
            self.base_url.starts_with("https://") || self.base_url.starts_with("http://");
        let host = self
            .base_url
            .split_once("://")
            .map(|(_, rest)| rest)
            .unwrap_or_default();
        if !scheme_ok
            || host.is_empty()
            || host.starts_with('/')
            || host.contains('@')
            || host.contains('?')
            || self.base_url.len() > 500
            || self
                .base_url
                .chars()
                .any(|c| c.is_whitespace() || c.is_control())
        {
            return invalid("the address must be an http or https URL");
        }
        let bad_model = |model: &str| {
            model.is_empty() || model.len() > 200 || model.chars().any(char::is_control)
        };
        if bad_model(&self.model) {
            return invalid("the model is not valid");
        }
        // The default model is not repeated; the rest keep their order without repeats.
        let mut seen = vec![self.model.clone()];
        let mut models = Vec::new();
        for model in &self.models {
            let model = model.trim();
            if model.is_empty() || seen.iter().any(|known| known == model) {
                continue;
            }
            if bad_model(model) {
                return invalid("a model is not valid");
            }
            seen.push(model.to_owned());
            models.push(model.to_owned());
        }
        if models.len() > 32 {
            return invalid("at most 32 more models");
        }
        self.models = models;
        if self
            .max_tokens
            .is_some_and(|max| max == 0 || max > 1_000_000)
        {
            return invalid("the token limit must be 1–1000000");
        }
        // Windows are kept for the provider's models only.
        self.contexts
            .retain(|model, _| seen.iter().any(|known| known == model));
        let bad_window = |tokens: u64| !(1_000..=100_000_000).contains(&tokens);
        if self.contexts.values().any(|tokens| bad_window(*tokens))
            || self.context_window.is_some_and(bad_window)
        {
            return invalid("a context window must be 1000–100000000 tokens");
        }
        Ok(())
    }
}

impl AgentsPrefs {
    /// Trims commands and checks ids, names and sizes.
    fn validate(&mut self) -> Result<(), VaultError> {
        let bad = |text: &str| text.len() > 1024 || text.chars().any(char::is_control);
        if self.custom.len() > 20
            || self.disabled.len() > 50
            || self.disabled.iter().any(|id| !is_slug(id))
        {
            return Err(VaultError::Invalid("too many agents".into()));
        }
        let mut ids = std::collections::HashSet::new();
        for agent in &mut self.custom {
            agent.command = agent.command.trim().to_owned();
            agent.name = agent.name.trim().to_owned();
            let id_ok = agent.id.strip_prefix("custom-").is_some_and(is_slug);
            if !id_ok
                || !ids.insert(agent.id.clone())
                || agent.name.is_empty()
                || agent.name.chars().count() > 60
                || agent.command.is_empty()
                || bad(&agent.command)
                || agent.args.len() > 32
                || agent.args.iter().any(|arg| bad(arg))
            {
                return Err(VaultError::Invalid(format!(
                    "the agent {:?} is not valid",
                    agent.name
                )));
            }
        }
        Ok(())
    }
}

impl DevicePrefs {
    /// Moves the older single agent command into the agent list.
    pub(crate) fn upgrade(mut self) -> Self {
        let legacy = std::mem::take(&mut self.agent);
        if !legacy.command.trim().is_empty() && self.agents.custom.is_empty() {
            self.agents.custom.push(CustomAgent {
                id: "custom-1".into(),
                name: "Agent".into(),
                command: legacy.command.trim().to_owned(),
                args: legacy.args,
            });
            self.agents.auth = legacy.auth;
        }
        self
    }

    /// Trims paths and checks ranges. Returns the normalized preferences. Paths must be
    /// absolute; filesystem access is left to the operations that use them.
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
        self.external_editor = self.external_editor.validate()?;
        self.tailscale = self.tailscale.validate()?;
        self.agents.validate()?;
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

    #[test]
    fn tailscale_hostname_defaults_to_roveterm_and_accepts_legacy_empty_value() {
        let defaults = TailscalePrefs::default();
        assert_eq!(defaults.hostname, "RoveTerm");
        let legacy = TailscalePrefs {
            hostname: String::new(),
            ..defaults
        };
        assert!(legacy.validate().unwrap().hostname.is_empty());
    }

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
    fn legacy_status_preferences_fill_new_fields_with_defaults() {
        let parsed: StatusPrefs = serde_json::from_str(r#"{"interval_secs": 5}"#).unwrap();
        assert_eq!(
            parsed,
            StatusPrefs {
                interval_secs: 5,
                ..StatusPrefs::default()
            }
        );
        let wire = serde_json::to_value(parsed).unwrap();
        for field in [
            "show_system",
            "show_cpu",
            "show_memory",
            "show_swap",
            "show_network",
            "show_disks",
            "terminal_bar",
            "terminal_bar_cpu",
            "terminal_bar_memory",
            "terminal_bar_network",
            "terminal_bar_tailscale_path",
        ] {
            assert_eq!(wire[field], true, "{field}");
        }
        assert_eq!(wire["terminal_bar_latency"], false);
        assert_eq!(wire["cpu_display"], "auto");
        assert_eq!(wire["network_unit"], "bytes");
        assert_eq!(wire["terminal_bar_memory_display"], "full_percent");
        assert_eq!(wire["terminal_bar_network_display"], "split");
    }

    #[test]
    fn status_preferences_round_trip_all_fields() {
        for (cpu_display, serialized_cpu_display, memory_display, serialized_memory_display, network_display, serialized_network_display) in [
            (
                CpuDisplay::PerCore,
                "per_core",
                MemoryDisplay::Full,
                "full",
                NetworkDisplay::Total,
                "total",
            ),
            (
                CpuDisplay::Summary,
                "summary",
                MemoryDisplay::Used,
                "used",
                NetworkDisplay::Split,
                "split",
            ),
            (
                CpuDisplay::Auto,
                "auto",
                MemoryDisplay::Percent,
                "percent",
                NetworkDisplay::Total,
                "total",
            ),
        ] {
            let prefs = StatusPrefs {
                interval_secs: 30,
                show_system: false,
                show_cpu: false,
                show_memory: false,
                show_swap: false,
                show_network: false,
                show_disks: false,
                cpu_display,
                network_unit: NetworkUnit::Bits,
                terminal_bar: false,
                terminal_bar_cpu: false,
                terminal_bar_memory: false,
                terminal_bar_memory_display: memory_display,
                terminal_bar_network: false,
                terminal_bar_network_display: network_display,
                terminal_bar_latency: true,
                terminal_bar_tailscale_path: false,
            };
            let wire = serde_json::to_value(&prefs).unwrap();
            assert_eq!(wire["cpu_display"], serialized_cpu_display);
            assert_eq!(wire["network_unit"], "bits");
            assert_eq!(wire["terminal_bar_memory_display"], serialized_memory_display);
            assert_eq!(wire["terminal_bar_network_display"], serialized_network_display);
            assert_eq!(wire["terminal_bar_latency"], true);
            assert_eq!(wire["terminal_bar_tailscale_path"], false);
            assert_eq!(serde_json::from_value::<StatusPrefs>(wire).unwrap(), prefs);
        }
    }

    fn editor_prefs() -> ExternalEditorPrefs {
        ExternalEditorPrefs {
            program: if cfg!(windows) {
                r"C:\Program Files\Rove Editor\editor.exe"
            } else {
                "/opt/Rove Editor/bin/editor"
            }
            .into(),
            args: vec!["--wait".into(), "{file}".into()],
        }
    }

    #[test]
    fn external_editor_defaults_keep_older_device_records_readable() {
        let old: DevicePrefs = serde_json::from_str(
            r#"{"window_material":false,"download_dir":"","max_transfers":5}"#,
        )
        .unwrap();
        assert_eq!(old.max_transfers, 5);
        assert_eq!(old.external_editor, ExternalEditorPrefs::default());
        assert!(old.validate().is_ok());

        let empty: DevicePrefs = serde_json::from_str(r#"{"external_editor":{}}"#).unwrap();
        assert_eq!(empty.external_editor, ExternalEditorPrefs::default());
        let program_only: ExternalEditorPrefs =
            serde_json::from_value(serde_json::json!({ "program": editor_prefs().program }))
                .unwrap();
        assert!(program_only.args.is_empty());
        assert!(program_only.validate().is_ok());
    }

    #[test]
    fn external_editor_requires_a_native_absolute_program_without_disk_access() {
        let configured = editor_prefs();
        assert!(configured.validate().is_ok());
        for program in [
            "editor",
            "./editor",
            "../bin/editor",
            "~/bin/editor",
            "file:///opt/editor",
            "https://example.com/editor",
            "/",
            "\"/opt/editor\"",
        ] {
            let editor = ExternalEditorPrefs {
                program: program.into(),
                args: Vec::new(),
            };
            assert!(editor.validate().is_err(), "{program}");
        }
        let mut device = DevicePrefs::default();
        device.external_editor.program = "editor".into();
        assert!(matches!(device.validate(), Err(VaultError::Invalid(_))));
    }

    #[test]
    fn external_editor_preserves_arguments_and_file_placeholders_as_data() {
        let mut editor = editor_prefs();
        editor.program = format!("  {}  ", editor.program);
        editor.args = [
            "--wait",
            "  significant spaces  ",
            "",
            "{file}",
            "--path={file}",
            "$(touch /tmp/nope); & | > < `command` \"quoted\" 'single'",
        ]
        .map(str::to_owned)
        .to_vec();
        let args = editor.args.clone();
        let validated = editor.validate().unwrap();
        assert_eq!(validated.program, editor_prefs().program);
        assert_eq!(validated.args, args);
        assert_eq!(
            serde_json::from_str::<ExternalEditorPrefs>(
                &serde_json::to_string(&validated).unwrap()
            )
            .unwrap(),
            validated
        );
    }

    #[test]
    fn external_editor_can_be_cleared_but_arguments_need_a_program() {
        assert_eq!(
            ExternalEditorPrefs {
                program: "   ".into(),
                args: Vec::new(),
            }
            .validate()
            .unwrap(),
            ExternalEditorPrefs::default()
        );
        for args in [vec!["{file}".into()], vec![String::new()]] {
            assert!(
                ExternalEditorPrefs {
                    program: String::new(),
                    args,
                }
                .validate()
                .is_err()
            );
        }
    }

    #[test]
    fn external_editor_rejects_control_characters_in_paths_and_arguments() {
        for control in ['\0', '\n', '\r', '\t', '\u{7f}', '\u{85}'] {
            let mut editor = editor_prefs();
            editor.program.push(control);
            assert!(editor.validate().is_err(), "program: {control:?}");
            let mut editor = editor_prefs();
            editor.args.push(format!("before{control}after"));
            assert!(editor.validate().is_err(), "argument: {control:?}");
        }
    }

    #[test]
    fn external_editor_bounds_program_and_argument_sizes() {
        let prefix = if cfg!(windows) { "C:\\" } else { "/" };
        let program = format!(
            "{prefix}{}.exe",
            "e".repeat(MAX_EDITOR_TEXT - prefix.len() - 4)
        );
        let editor = ExternalEditorPrefs {
            program: program.clone(),
            args: vec!["a".repeat(MAX_EDITOR_TEXT); MAX_EDITOR_ARGS_BYTES / MAX_EDITOR_TEXT],
        };
        assert!(editor.clone().validate().is_ok());
        let mut too_long = editor.clone();
        too_long.program.insert_str(prefix.len(), "e");
        assert!(too_long.validate().is_err());
        let mut too_long = editor.clone();
        too_long.args[0].push('a');
        assert!(too_long.validate().is_err());
        let mut too_many_bytes = editor;
        too_many_bytes.args.push("a".into());
        assert!(too_many_bytes.validate().is_err());
        assert!(
            ExternalEditorPrefs {
                program: program.clone(),
                args: vec![String::new(); MAX_EDITOR_ARGS],
            }
            .validate()
            .is_ok()
        );
        assert!(
            ExternalEditorPrefs {
                program,
                args: vec![String::new(); MAX_EDITOR_ARGS + 1],
            }
            .validate()
            .is_err()
        );
    }

    #[cfg(windows)]
    #[test]
    fn external_editor_windows_requires_exe_and_rejects_relative_drive_paths() {
        for program in [
            r"C:\Tools\editor.cmd",
            r"C:\Tools\editor.bat",
            r"C:\Tools\editor",
            r"C:editor.exe",
            r"\Tools\editor.exe",
        ] {
            let editor = ExternalEditorPrefs {
                program: program.into(),
                args: Vec::new(),
            };
            assert!(editor.validate().is_err(), "{program}");
        }
        assert!(
            ExternalEditorPrefs {
                program: r"C:\Tools\Editor.EXE".into(),
                args: vec!["{file}".into()],
            }
            .validate()
            .is_ok()
        );
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn external_editor_macos_app_bundles_require_no_arguments() {
        let bundle = ExternalEditorPrefs {
            program: "/Applications/Rove Editor.app".into(),
            args: Vec::new(),
        };
        assert!(bundle.clone().validate().is_ok());
        assert!(
            ExternalEditorPrefs {
                args: vec!["{file}".into()],
                ..bundle
            }
            .validate()
            .is_err()
        );
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

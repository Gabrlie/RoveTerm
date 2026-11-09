//! User preferences that follow the user rather than the device: terminal appearance for now.
//! Fields missing from a stored record take their defaults, so older records stay readable.

use serde::{Deserialize, Serialize};

use crate::VaultError;

/// All preferences, stored as one record.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Preferences {
    /// Terminal appearance.
    pub terminal: TerminalPrefs,
}

/// Terminal appearance.
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
}

impl Default for TerminalPrefs {
    fn default() -> Self {
        Self {
            font_family: String::new(),
            font_size: 13,
            line_height: 1.15,
        }
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
            },
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
}

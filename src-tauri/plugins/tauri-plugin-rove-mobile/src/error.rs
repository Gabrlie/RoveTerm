#[derive(Debug, thiserror::Error)]
pub enum MobileError {
    #[error("mobile bridge operation is unsupported on this platform")]
    Unsupported,
    #[error("mobile service is unavailable: {0}")]
    Unavailable(String),
    #[error("authentication was denied")]
    Denied,
    #[error("device authentication is unavailable: {0}")]
    BiometricUnavailable(String),
    #[error("the device key was invalidated")]
    KeyInvalidated,
    #[error("malformed mobile bridge data: {0}")]
    Malformed(String),
    #[error("invalid mobile bridge input: {0}")]
    InvalidInput(String),
    #[error("mobile plugin invocation failed: {0}")]
    Plugin(String),
}

#[cfg(any(mobile, test))]
pub(crate) fn map_invoke_error(code: Option<&str>, message: Option<&str>) -> MobileError {
    let detail = || {
        message
            .or(code)
            .unwrap_or("native command failed")
            .to_owned()
    };
    match code {
        Some("denied") => MobileError::Denied,
        Some("unavailable") => MobileError::Unavailable(detail()),
        Some("biometric_unavailable") => MobileError::BiometricUnavailable(detail()),
        Some("key_invalidated") => MobileError::KeyInvalidated,
        Some("malformed") => MobileError::Malformed(detail()),
        Some("invalid_input") => MobileError::InvalidInput(detail()),
        _ => MobileError::Plugin(detail()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_native_error_codes() {
        let cases = [
            ("denied", MobileError::Denied),
            ("unavailable", MobileError::Unavailable("detail".into())),
            (
                "biometric_unavailable",
                MobileError::BiometricUnavailable("detail".into()),
            ),
            ("key_invalidated", MobileError::KeyInvalidated),
            ("malformed", MobileError::Malformed("detail".into())),
            ("invalid_input", MobileError::InvalidInput("detail".into())),
        ];
        for (code, expected) in cases {
            let error = map_invoke_error(Some(code), Some("detail"));
            assert_eq!(
                std::mem::discriminant(&error),
                std::mem::discriminant(&expected)
            );
            assert_eq!(error.to_string(), expected.to_string());
        }
    }

    #[test]
    fn preserves_diagnostics_for_unknown_or_missing_codes() {
        for (code, message, expected) in [
            (Some("future"), Some("detail"), "detail"),
            (Some("future"), None, "future"),
            (None, Some("detail"), "detail"),
            (None, None, "native command failed"),
        ] {
            assert!(matches!(
                map_invoke_error(code, message),
                MobileError::Plugin(detail) if detail == expected
            ));
        }
    }
}

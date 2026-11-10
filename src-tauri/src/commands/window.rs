//! Window appearance.

/// Turns the translucent window material on or off and returns whether the window is now
/// translucent, so the WebView knows whether to draw its own opaque background.
///
/// - macOS: the sidebar vibrancy material, which blurs what is behind the window beyond
///   recognition (Liquid Glass stays see-through, which reads badly behind a whole window).
/// - Windows 11: Mica. Windows 10 has no suitable material and stays opaque.
/// - Linux and mobile: always opaque.
#[tauri::command]
pub fn window_set_material(window: tauri::WebviewWindow, enabled: bool) -> bool {
    #[cfg(any(target_os = "macos", target_os = "windows"))]
    {
        use tauri::utils::config::WindowEffectsConfig;
        use tauri::window::Effect;

        let effects = if cfg!(target_os = "macos") {
            vec![Effect::Sidebar]
        } else {
            vec![Effect::Mica]
        };
        let supported = cfg!(target_os = "macos") || windows_11();
        let config = (enabled && supported).then(|| WindowEffectsConfig {
            effects,
            ..Default::default()
        });
        let translucent = config.is_some();
        match window.set_effects(config) {
            Ok(()) => translucent,
            Err(error) => {
                tracing::warn!(%error, "setting the window material failed");
                false
            }
        }
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        // No translucent material on this platform: the WebView keeps its opaque background.
        let _ = (window, enabled);
        false
    }
}

/// Mica needs Windows 11 (build 22000 or later).
#[cfg(target_os = "windows")]
fn windows_11() -> bool {
    windows_version::OsVersion::current().build >= 22000
}

#[cfg(target_os = "macos")]
fn windows_11() -> bool {
    false
}

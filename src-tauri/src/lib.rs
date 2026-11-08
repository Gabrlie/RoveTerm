//! RoveTerm application shell: assembles the profile's plugins and exposes them to the WebView.

/// Starts the Tauri application. Shared by the desktop binary and the mobile entry point.
#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();
    tauri::Builder::default()
        .run(tauri::generate_context!())
        .expect("failed to run the Tauri application");
}

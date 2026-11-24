fn main() {
    tauri_plugin::Builder::new(&[])
        .android_path("android")
        .ios_path("ios")
        .build();

    #[cfg(target_os = "macos")]
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("ios") {
        tauri_plugin::mobile::update_info_plist(|plist| {
            for (key, value) in [
                (
                    "NSFaceIDUsageDescription",
                    "RoveTerm uses Face ID to unlock your saved credentials.",
                ),
                (
                    "NSLocalNetworkUsageDescription",
                    "RoveTerm connects to hosts on your local network.",
                ),
            ] {
                if !plist.contains_key(key) {
                    plist.insert(key.into(), value.into());
                }
            }
        })
        .expect("update RoveTerm iOS usage descriptions");
    }
}

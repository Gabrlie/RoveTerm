-keep class dev.roveterm.mobile.RoveMobilePlugin {
    public <init>(android.app.Activity);
    @app.tauri.annotation.Command <methods>;
    @app.tauri.annotation.ActivityCallback <methods>;
}
-keep class dev.roveterm.mobile.RegisterInsetsArgs { *; }

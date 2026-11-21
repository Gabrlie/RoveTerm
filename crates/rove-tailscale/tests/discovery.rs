use std::sync::Arc;

use rove_kernel::{Context, Plugin, Profile, Runtime};
use rove_tailscale::{TailnetService, TailnetState, TailscalePlugin};

#[test]
fn boot_registers_and_disposal_revokes_the_service() {
    let context = Context::new();
    let profile = Profile::from_layers(["[profile]\nname='test'\nplugins=['tailscale']"]).unwrap();
    let plugins: [Arc<dyn Plugin>; 1] = [Arc::new(TailscalePlugin)];
    let runtime = Runtime::boot_in(context.clone(), &profile, &plugins).unwrap();
    let service = context.require::<dyn TailnetService>().unwrap();
    assert_eq!(service.status().state, TailnetState::Unconfigured);
    drop(runtime);
    assert!(context.get::<dyn TailnetService>().is_none());
}

#[test]
fn invalid_plugin_timeout_fails_at_boot() {
    let context = Context::new();
    let profile = Profile::from_layers([
        "[profile]\nname='test'\nplugins=['tailscale']\n[plugin.tailscale]\nconnect_timeout_ms=0",
    ])
    .unwrap();
    let plugins: [Arc<dyn Plugin>; 1] = [Arc::new(TailscalePlugin)];
    assert!(Runtime::boot_in(context.clone(), &profile, &plugins).is_err());
    assert!(context.get::<dyn TailnetService>().is_none());
}

//! Native mobile services for the RoveTerm application shell.

use tauri::{
    Manager, Runtime,
    plugin::{Builder, TauriPlugin},
};

#[cfg(desktop)]
mod desktop;
#[cfg(mobile)]
mod mobile;

mod error;
mod models;

#[cfg(desktop)]
pub use desktop::MobileBridge;
pub use error::MobileError;
#[cfg(mobile)]
pub use mobile::MobileBridge;
pub use models::{
    ApkInstallStatus, BiometricStatus, KeepAliveAction, KeepAliveUpdate, LifecycleEvent,
    LifecycleState, MobileInsets, StagedFile,
};

pub const INSETS_EVENT: &str = "rove-mobile://insets";
pub const LIFECYCLE_EVENT: &str = "rove-mobile://lifecycle";
pub const BACK_EVENT: &str = "rove-mobile://back";

pub trait MobileBridgeExt<R: Runtime> {
    fn rove_mobile(&self) -> &MobileBridge<R>;
}

impl<R: Runtime, T: Manager<R>> MobileBridgeExt<R> for T {
    fn rove_mobile(&self) -> &MobileBridge<R> {
        self.state::<MobileBridge<R>>().inner()
    }
}

pub fn init<R: Runtime>() -> TauriPlugin<R> {
    Builder::new("rove-mobile")
        .setup(|app, _api| {
            #[cfg(mobile)]
            let bridge = mobile::init(app, _api)?;
            #[cfg(desktop)]
            let bridge = desktop::init::<R>();
            app.manage(bridge);
            Ok(())
        })
        .build()
}

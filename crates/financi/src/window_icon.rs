//! The app icon and id (3.63): what the window manager, Dock and taskbar
//! show. The artwork is `assets/icon/financi.svg`; the PNGs rendered from
//! it by `scripts/icons.sh` (`mise run icons`) are committed, so the build
//! needs no SVG renderer.
//!
//! Per platform:
//! - Linux/X11: `WindowOptions::icon`, decoded here from the embedded
//!   256 px PNG.
//! - Linux/Wayland: compositors ignore per-window icons and match the
//!   window's `app_id` to an installed desktop entry — see
//!   `packaging/linux/financi.desktop` and `mise run install-desktop`.
//! - macOS: an app bundle carries an `.icns` (`scripts/bundle-macos.sh`);
//!   under `cargo run` there is no bundle, so [`set_dock_icon`] sets the
//!   Dock tile at startup instead.

use std::sync::Arc;

/// The application id: `WindowOptions::app_id`, the desktop entry's file
/// name, and its `Icon` and `StartupWMClass` values (tested below to stay
/// equal).
pub const APP_ID: &str = "financi";

/// The 256 px PNG, embedded at build time: no asset lookup at runtime and
/// nothing to install for the window icon to work.
const ICON_PNG_256: &[u8] = include_bytes!("../../../assets/icon/png/financi-256.png");

/// Decode [`ICON_PNG_256`] for `WindowOptions::icon` (used on X11; other
/// platforms ignore the field). Panics only if the committed PNG is
/// broken, which [`tests::embedded_icon_decodes_256_rgba`] catches.
pub fn window_icon() -> Arc<image::RgbaImage> {
    let image = image::load_from_memory(ICON_PNG_256)
        .expect("the embedded icon PNG must decode")
        .to_rgba8();
    Arc::new(image)
}

/// macOS only: set the Dock tile's icon for `cargo run`, which has no app
/// bundle to carry one (a bundled build gets its icon from the `.icns`;
/// see `scripts/bundle-macos.sh`). No-op on other platforms, where the
/// window icon and desktop entry cover it.
///
/// Must run on the main thread after GPUI has created `NSApplication`,
/// i.e. from inside the `Application::run` closure.
#[cfg(target_os = "macos")]
pub fn set_dock_icon() {
    use objc2::AnyThread as _; // for `NSImage::alloc`
    use objc2_app_kit::{NSApplication, NSImage};
    use objc2_foundation::NSData;

    // GPUI calls the run closure on the main thread, so the marker is
    // cheap to obtain; bail out rather than panic if that ever changes.
    let Some(main_thread) = objc2::MainThreadMarker::new() else {
        return;
    };
    let data = NSData::with_bytes(ICON_PNG_256);
    let Some(image) = NSImage::initWithData(NSImage::alloc(), &data) else {
        return;
    };
    let app = NSApplication::sharedApplication(main_thread);
    unsafe { app.setApplicationIconImage(Some(&image)) };
}

#[cfg(not(target_os = "macos"))]
pub fn set_dock_icon() {}

#[cfg(test)]
mod tests;

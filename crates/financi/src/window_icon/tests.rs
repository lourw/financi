//! Tests for 3.63: the embedded icon decodes, and the desktop entry stays
//! in step with [`super::APP_ID`] (a Wayland compositor matches the
//! window's `app_id` against the entry's `StartupWMClass`, and loads the
//! icon by its `Icon` name).

use super::APP_ID;

/// `window_icon()` panics if the embedded PNG doesn't decode; also check
/// the size and colour type the window manager expects.
#[test]
fn embedded_icon_decodes_256_rgba() {
    let icon = super::window_icon();
    assert_eq!(icon.width(), 256, "embedded icon width");
    assert_eq!(icon.height(), 256, "embedded icon height");
    // RGBA: four bytes per pixel (to_rgba8() guarantees the layout).
    assert_eq!(icon.as_raw().len(), 256 * 256 * 4);
}

/// The committed desktop entry must carry the same id the window sets, or
/// the taskbar shows the generic icon.
#[test]
fn desktop_entry_ids_match_app_id() {
    let desktop = include_str!("../../../../packaging/linux/financi.desktop");
    let value_of = |key: &str| {
        desktop
            .lines()
            .find_map(|line| line.strip_prefix(&format!("{key}=")))
            .unwrap_or_else(|| panic!("desktop entry has no {key}= line: {desktop}"))
    };
    assert_eq!(value_of("Icon"), APP_ID);
    assert_eq!(value_of("StartupWMClass"), APP_ID);
    assert_eq!(value_of("Exec"), APP_ID);
    assert_eq!(value_of("Name"), APP_ID);
}

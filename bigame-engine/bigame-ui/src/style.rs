//! CSS theme loader + `GResource` registration for BiGame-mode.

use gtk4::{gio, glib};

/// Register compiled `GResource` bundle and load application CSS.
///
/// Call once during `Application::connect_startup`.
///
/// # Panics
/// Panics if the compiled `GResource` cannot be loaded (build.rs failure).
pub fn load_css() {
    // Register compiled GResource from build.rs output
    let bytes = glib::Bytes::from_static(include_bytes!(concat!(
        env!("OUT_DIR"),
        "/resources.gresource"
    )));
    let resource = gio::Resource::from_data(&bytes).expect("load gresource");
    gio::resources_register(&resource);

    // Load CSS from the registered resource
    let provider = gtk4::CssProvider::new();
    provider.load_from_resource("/com/biglinux/BiGameMode/style.css");

    if let Some(display) = gtk4::gdk::Display::default() {
        gtk4::style_context_add_provider_for_display(
            &display,
            &provider,
            gtk4::STYLE_PROVIDER_PRIORITY_APPLICATION,
        );
    }
}

#[cfg(test)]
mod tests {
    /// The application icon, as installed and as bundled in the resource.
    const ICON: &str = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../usr/share/icons/hicolor/scalable/apps/com.biglinux.BiGameMode.svg"
    ));

    /// A value of an attribute on the `<svg>` element.
    fn root_attribute(svg: &str, name: &str) -> Option<String> {
        let root = &svg[svg.find("<svg")?..];
        let root = &root[..root.find('>')?];
        let start = root.find(&format!(" {name}=\""))? + name.len() + 3;
        Some(root[start..][..root[start..].find('"')?].to_owned())
    }

    #[test]
    fn the_app_icon_declares_a_size_the_about_dialog_can_draw_sharp() {
        // GTK draws an SVG at the size it declares and scales that picture:
        // declared at 24 px, the About dialog's 128 px icon came out blurred
        // and smeared. The drawing itself is in the 24-unit viewBox.
        for name in ["width", "height"] {
            let size: f64 = root_attribute(ICON, name)
                .and_then(|v| v.trim_end_matches("px").parse().ok())
                .unwrap_or(0.0);
            assert!(size >= 256.0, "{name} is {size}");
        }
        assert_eq!(
            root_attribute(ICON, "viewBox").as_deref(),
            Some("0 0 24 24")
        );
    }
}

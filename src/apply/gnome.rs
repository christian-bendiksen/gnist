//! Updates GNOME appearance settings with `gsettings`.

use crate::theme::Theme;
use std::process::{Command, Stdio};

const SCHEMA: &str = "org.gnome.desktop.interface";

pub fn run(theme: &Theme, skip_icons: bool) {
    let (color_scheme, gtk_theme) = if theme.is_light {
        ("prefer-light", "adw-gtk3")
    } else {
        ("prefer-dark", "adw-gtk3-dark")
    };

    // `libgnist_gio` notifies Chromium when CSS changes, so cycling through an
    // intermediate GTK theme would only add a white flash to the fade.
    gsettings_set(SCHEMA, "color-scheme", color_scheme);
    gsettings_set(SCHEMA, "gtk-theme", gtk_theme);

    if !skip_icons && let Some(icon) = theme.icon_theme.as_deref() {
        gsettings_set(SCHEMA, "icon-theme", icon);
    }
}

fn gsettings_set(schema: &str, key: &str, value: &str) {
    Command::new("gsettings")
        .args(["set", schema, key, value])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .ok();
}

//! Updates GNOME appearance settings with `gsettings`.
//!
//! A theme may ship a templated `gnome.toml` to override the schema, keys, and
//! theme names, or to set extra keys from theme values:
//!
//! ```toml
//! schema             = "org.gnome.desktop.interface"
//! light_theme        = "adw-gtk3"
//! dark_theme         = "adw-gtk3-dark"
//!
//! [[extra]]
//! key                = "cursor-theme"
//! value              = "{{ cursor_theme }}"
//! ```

use crate::render::engine::render_str;
use crate::render::value::Value;
use crate::theme::Theme;
use std::collections::HashMap;
use std::path::Path;
use std::process::{Command, Stdio};

struct Extra {
    key: String,
    value: String,
}

#[derive(Default)]
struct Config {
    schema: String,
    color_scheme_key: String,
    gtk_theme_key: String,
    icon_theme_key: String,
    light_scheme: String,
    dark_scheme: String,
    light_theme: String,
    dark_theme: String,
    extra: Vec<Extra>,
}

impl Config {
    fn defaults() -> Self {
        Self {
            schema: "org.gnome.desktop.interface".into(),
            color_scheme_key: "color-scheme".into(),
            gtk_theme_key: "gtk-theme".into(),
            icon_theme_key: "icon-theme".into(),
            light_scheme: "prefer-light".into(),
            dark_scheme: "prefer-dark".into(),
            light_theme: "adw-gtk3".into(),
            dark_theme: "adw-gtk3-dark".into(),
            extra: Vec::new(),
        }
    }

    fn load(root: &Path, values: &HashMap<String, Value>) -> Self {
        let mut cfg = Self::defaults();
        let Ok(src) = std::fs::read_to_string(root.join("gnome.toml")) else {
            return cfg;
        };
        let Ok(toml) = render_str(&src, values).parse::<toml::Value>() else {
            return cfg;
        };
        let get = |key: &str| toml.get(key).and_then(|v| v.as_str()).map(str::to_owned);

        cfg.schema = get("schema").unwrap_or(cfg.schema);
        cfg.color_scheme_key = get("color_scheme_key").unwrap_or(cfg.color_scheme_key);
        cfg.gtk_theme_key = get("gtk_theme_key").unwrap_or(cfg.gtk_theme_key);
        cfg.icon_theme_key = get("icon_theme_key").unwrap_or(cfg.icon_theme_key);
        cfg.light_scheme = get("light_scheme").unwrap_or(cfg.light_scheme);
        cfg.dark_scheme = get("dark_scheme").unwrap_or(cfg.dark_scheme);
        cfg.light_theme = get("light_theme").unwrap_or(cfg.light_theme);
        cfg.dark_theme = get("dark_theme").unwrap_or(cfg.dark_theme);

        cfg.extra = toml
            .get("extra")
            .and_then(|v| v.as_array())
            .map(|rows| {
                rows.iter()
                    .filter_map(|row| {
                        Some(Extra {
                            key: row.get("key")?.as_str()?.to_owned(),
                            value: row.get("value")?.as_str()?.to_owned(),
                        })
                    })
                    .collect()
            })
            .unwrap_or_default();

        cfg
    }
}

pub fn run(theme: &Theme, skip_icons: bool) {
    let cfg = Config::load(&theme.root, &theme.vars);

    let (color_scheme, gtk_theme) = if theme.is_light {
        (&cfg.light_scheme, cfg.light_theme.as_str())
    } else {
        (&cfg.dark_scheme, cfg.dark_theme.as_str())
    };

    // `libgnist_gio` notifies Chromium when CSS changes, so cycling through an
    // intermediate GTK theme would only add a white flash to the fade.
    gsettings_set(&cfg.schema, &cfg.color_scheme_key, color_scheme);
    gsettings_set(&cfg.schema, &cfg.gtk_theme_key, gtk_theme);

    if !skip_icons && let Some(icon) = theme.icon_theme.as_deref() {
        gsettings_set(&cfg.schema, &cfg.icon_theme_key, icon);
    }

    for row in &cfg.extra {
        gsettings_set(&cfg.schema, &row.key, &row.value);
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

#[cfg(test)]
mod tests {
    use super::Config;
    use crate::render::value::Value;
    use std::{collections::HashMap, fs};

    fn values() -> HashMap<String, Value> {
        HashMap::from([("accent".to_string(), Value::Str("#89b4fa".into()))])
    }

    #[test]
    fn missing_gnome_toml_uses_defaults() {
        let dir = tempfile::tempdir().unwrap();
        let cfg = Config::load(dir.path(), &values());
        assert_eq!(cfg.schema, "org.gnome.desktop.interface");
        assert_eq!(cfg.light_theme, "adw-gtk3");
        assert_eq!(cfg.dark_theme, "adw-gtk3-dark");
        assert_eq!(cfg.light_scheme, "prefer-light");
        assert_eq!(cfg.dark_scheme, "prefer-dark");
        assert!(cfg.extra.is_empty());
    }

    #[test]
    fn gnome_toml_overrides_are_templated() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(
            dir.path().join("gnome.toml"),
            "light_theme = \"my-light\"\n\n[[extra]]\nkey = \"accent-color\"\nvalue = \"{{ accent }}\"\n",
        )
        .unwrap();

        let cfg = Config::load(dir.path(), &values());

        assert_eq!(cfg.light_theme, "my-light");
        assert_eq!(cfg.dark_theme, "adw-gtk3-dark");
        assert_eq!(cfg.extra[0].key, "accent-color");
        assert_eq!(cfg.extra[0].value, "#89b4fa");
    }
}

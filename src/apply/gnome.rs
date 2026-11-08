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

use crate::ctx::Ctx;
use crate::render::engine::render_str;
use crate::render::value::Value;
use crate::theme::Theme;
use anyhow::{Context, Result, bail};
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

/// Accessibility values rendered from the active Malm profile. No file means
/// this Gnist installation continues to use its existing desktop settings.
#[derive(Debug, PartialEq)]
struct Accessibility {
    enable_animations: bool,
    high_contrast: bool,
    text_scale: f64,
}

impl Accessibility {
    fn load(path: &Path) -> Result<Option<Self>> {
        let src = match std::fs::read_to_string(path) {
            Ok(src) => src,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error).context("read accessibility settings"),
        };
        let doc: toml::Value = src.parse().context("parse accessibility settings")?;
        let enable_animations = doc
            .get("enable-animations")
            .and_then(toml::Value::as_bool)
            .context("accessibility enable-animations must be a boolean")?;
        let high_contrast = doc
            .get("high-contrast")
            .and_then(toml::Value::as_bool)
            .context("accessibility high-contrast must be a boolean")?;
        let text_scale = doc
            .get("text-scaling-factor")
            .and_then(|v| v.as_float().or_else(|| v.as_integer().map(|n| n as f64)))
            .context("accessibility text-scaling-factor must be a number")?;
        if !text_scale.is_finite() || !(0.5..=3.0).contains(&text_scale) {
            bail!("accessibility text-scaling-factor must be between 0.5 and 3.0");
        }
        Ok(Some(Self {
            enable_animations,
            high_contrast,
            text_scale,
        }))
    }
}

pub fn run(ctx: &Ctx, theme: &Theme, skip_icons: bool) -> Result<()> {
    let accessibility = Accessibility::load(&ctx.current_link.join("herda-accessibility.toml"))?;

    let cfg = Config::load(&theme.root, &theme.vars);

    let (color_scheme, gtk_theme) = if theme.is_light {
        (&cfg.light_scheme, cfg.light_theme.as_str())
    } else {
        (&cfg.dark_scheme, cfg.dark_theme.as_str())
    };

    // `libgnist_gio` notifies Chromium when CSS changes, so cycling through an
    // intermediate GTK theme would only add a white flash to the fade.
    gsettings_set(&cfg.schema, &cfg.color_scheme_key, color_scheme);
    if accessibility.is_none()
        || cfg.schema != "org.gnome.desktop.interface"
        || cfg.gtk_theme_key != "gtk-theme"
    {
        // Keep a theme's nonstandard settings target as well; the standard
        // GTK key is applied last below when accessibility is present.
        gsettings_set(&cfg.schema, &cfg.gtk_theme_key, gtk_theme);
    }

    if !skip_icons && let Some(icon) = theme.icon_theme.as_deref() {
        gsettings_set(&cfg.schema, &cfg.icon_theme_key, icon);
    }

    for row in &cfg.extra {
        gsettings_set(&cfg.schema, &row.key, &row.value);
    }

    if let Some(a11y) = &accessibility {
        // GTK3 honors the built-in HighContrast theme; restoring the selected
        // theme when disabled preserves Gnist's normal light/dark selection.
        let selected_theme = if a11y.high_contrast {
            "HighContrast"
        } else {
            gtk_theme
        };
        // Turn motion off before changing the theme so the transition itself
        // does not animate when both preferences change together.
        required_setting(
            "org.gnome.desktop.interface",
            "enable-animations",
            RequiredValue::Bool(a11y.enable_animations),
        )?;
        required_setting(
            "org.gnome.desktop.interface",
            "text-scaling-factor",
            RequiredValue::Float(a11y.text_scale),
        )?;
        required_setting(
            "org.gnome.desktop.a11y.interface",
            "high-contrast",
            RequiredValue::Bool(a11y.high_contrast),
        )?;
        required_setting(
            "org.gnome.desktop.interface",
            "gtk-theme",
            RequiredValue::String(selected_theme),
        )?;
    }
    Ok(())
}

#[derive(Debug)]
enum RequiredValue<'a> {
    Bool(bool),
    Float(f64),
    String(&'a str),
}

impl RequiredValue<'_> {
    fn as_argument(&self) -> String {
        match self {
            Self::Bool(value) => value.to_string(),
            Self::Float(value) => value.to_string(),
            Self::String(value) => (*value).to_owned(),
        }
    }

    fn matches_output(&self, output: &str) -> bool {
        let output = output.trim();
        match self {
            Self::Bool(expected) => output.parse::<bool>().ok() == Some(*expected),
            Self::Float(expected) => output.parse::<f64>().ok() == Some(*expected),
            // gsettings prints strings in GVariant syntax; it uses either
            // quote and escapes backslashes, quotes and control characters.
            Self::String(expected) => gvariant_string(output).as_deref() == Some(*expected),
        }
    }
}

fn gvariant_string(output: &str) -> Option<String> {
    let mut chars = output.chars();
    let quote = chars.next()?;
    if quote != '\'' && quote != '"' {
        return None;
    }
    let mut decoded = String::new();
    loop {
        let c = chars.next()?;
        if c == quote {
            return if chars.next().is_none() {
                Some(decoded)
            } else {
                None
            };
        }
        if c == '\\' {
            decoded.push(match chars.next()? {
                '\\' => '\\',
                '\'' => '\'',
                '"' => '"',
                'n' => '\n',
                'r' => '\r',
                't' => '\t',
                'b' => '\x08',
                'f' => '\x0c',
                _ => return None,
            });
        } else {
            decoded.push(c);
        }
    }
}

fn required_setting(schema: &str, key: &str, value: RequiredValue<'_>) -> Result<()> {
    let argument = value.as_argument();
    let write = Command::new("gsettings")
        .args(["set", schema, key, &argument])
        // Keep dconf's otherwise localized commit diagnostic recognizable.
        .env("LC_ALL", "C")
        .stdin(Stdio::null())
        .output()
        .with_context(|| format!("run gsettings for {schema} {key}"))?;
    let diagnostic = String::from_utf8_lossy(&write.stderr);
    if !write.status.success() || diagnostic.contains("failed to commit changes to dconf") {
        bail!(
            "gsettings failed for {schema} {key}: {}; {}",
            write.status,
            diagnostic.trim()
        );
    }
    // dconf can reject a write while gsettings set exits successfully. Read
    // from a separate process so it cannot return a local optimistic cache.
    let readback = Command::new("gsettings")
        .args(["get", schema, key])
        .stdin(Stdio::null())
        .output()
        .with_context(|| format!("read gsettings for {schema} {key}"))?;
    if !readback.status.success() {
        bail!(
            "gsettings read-back failed for {schema} {key}: {}",
            readback.status
        );
    }
    let actual = std::str::from_utf8(&readback.stdout)
        .with_context(|| format!("invalid gsettings read-back for {schema} {key}"))?;
    if !value.matches_output(actual) {
        bail!(
            "gsettings read-back mismatch for {schema} {key}: expected {value:?}, got {:?}",
            actual.trim()
        );
    }
    Ok(())
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
    use super::{Accessibility, Config, RequiredValue};
    use crate::render::value::Value;
    use std::{collections::HashMap, fs};

    fn values() -> HashMap<String, Value> {
        HashMap::from([("accent".to_string(), Value::Str("#89b4fa".into()))])
    }

    #[test]
    fn required_readback_compares_typed_values_and_rejects_malformed_values() {
        assert!(RequiredValue::Bool(false).matches_output("false\n"));
        assert!(!RequiredValue::Bool(false).matches_output("true"));
        assert!(RequiredValue::Float(1.0).matches_output("1.0"));
        assert!(RequiredValue::Float(1.25).matches_output("1.25"));
        assert!(!RequiredValue::Float(1.25).matches_output("nan"));
        assert!(RequiredValue::String("HighContrast").matches_output("'HighContrast'"));
        assert!(RequiredValue::String("a'b").matches_output("\"a'b\""));
        assert!(!RequiredValue::String("HighContrast").matches_output("'HighContrast' trailing"));
        assert!(!RequiredValue::String("HighContrast").matches_output(""));
    }

    #[test]
    fn accessibility_requires_complete_typed_values_in_schema_range() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("herda-accessibility.toml");
        assert_eq!(Accessibility::load(&file).unwrap(), None);
        fs::write(
            &file,
            "enable-animations = false\nhigh-contrast = true\ntext-scaling-factor = 1.25\n",
        )
        .unwrap();
        assert_eq!(
            Accessibility::load(&file).unwrap(),
            Some(Accessibility {
                enable_animations: false,
                high_contrast: true,
                text_scale: 1.25,
            })
        );
        for invalid in [
            "enable-animations = false\nhigh-contrast = true\ntext-scaling-factor = 3.1\n",
            "enable-animations = false\nhigh-contrast = true\ntext-scaling-factor = 0.49\n",
            "enable-animations = 'false'\nhigh-contrast = true\ntext-scaling-factor = 1.0\n",
            "enable-animations = false\ntext-scaling-factor = 1.0\n",
        ] {
            fs::write(&file, invalid).unwrap();
            assert!(Accessibility::load(&file).is_err(), "accepted {invalid}");
        }
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

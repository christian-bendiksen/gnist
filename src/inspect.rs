//! Side-effect-free palette data for desktop previews and the published
//! `palette.json` manifest. Uses the same source precedence and derived values
//! as rendering, never a second theme parser.
use crate::{Ctx, Theme, render::value::Value, theme};
use anyhow::{Context, Result, bail};
use serde_json::{Value as JsonValue, json};
use std::{collections::BTreeMap, path::Path};

const SCHEMA_VERSION: u64 = 1;

fn extract_colors(theme: &Theme) -> BTreeMap<String, String> {
    theme
        .vars
        .iter()
        .filter(|(key, _)| !key.ends_with("_strip") && !key.ends_with("_rgb"))
        .filter_map(|(key, value)| match value {
            Value::Str(s) => theme::colors::parse_color(s).map(|c| (key.clone(), c.to_hex())),
            Value::Color(c) => Some((key.clone(), c.to_hex())),
            _ => None,
        })
        .collect()
}

/// Extract non-colour scalars (`look`, `light_theme`, `dark_theme`, `*_alpha`, …).
fn extract_values(theme: &Theme) -> BTreeMap<String, JsonValue> {
    theme
        .vars
        .iter()
        .filter(|(key, _)| {
            !key.ends_with("_strip")
                && !key.ends_with("_rgb")
                && !matches!(
                    key.as_str(),
                    "name" | "mode" | "theme_type" | "is_light" | "colors" | "wallpapers"
                )
        })
        .filter_map(|(key, value)| match value {
            Value::Str(s) if theme::colors::parse_color(s).is_none() => {
                Some((key.clone(), JsonValue::String(s.clone())))
            }
            Value::Int(i) => Some((key.clone(), json!(i))),
            Value::Float(f) => {
                serde_json::Number::from_f64(*f).map(|num| (key.clone(), JsonValue::Number(num)))
            }
            Value::Bool(b) => Some((key.clone(), JsonValue::Bool(*b))),
            _ => None,
        })
        .collect()
}

fn extract_wallpapers(theme: &Theme) -> Vec<String> {
    theme
        .vars
        .get("wallpapers")
        .map(|value| match value {
            Value::List(items) => items.iter().map(Value::display).collect(),
            _ => Vec::new(),
        })
        .unwrap_or_default()
}

fn mode_str(theme: &Theme) -> &'static str {
    if theme.is_light { "light" } else { "dark" }
}

fn preview_str(theme: &Theme) -> Option<String> {
    theme
        .preview
        .as_ref()
        .map(|p| p.to_string_lossy().into_owned())
}

/// Build the published `palette.json` manifest (schema_version 1).
pub fn palette_manifest(theme: &Theme, revision: u64) -> JsonValue {
    json!({
        "schema_version": SCHEMA_VERSION,
        "name": theme.name,
        "display_name": theme.display_name,
        "mode": mode_str(theme),
        "revision": revision,
        "colors": extract_colors(theme),
        "values": extract_values(theme),
        "wallpapers": extract_wallpapers(theme),
        "icon_theme": theme.icon_theme,
        "preview": preview_str(theme),
        "generated": theme.generated,
    })
}

/// Write `palette.json` into the staging directory so it becomes visible
/// atomically with the rest of the rendered tree.
pub fn write_palette_manifest(stage: &Path, theme: &Theme, revision: u64) -> Result<()> {
    let manifest = palette_manifest(theme, revision);
    let mut bytes = serde_json::to_vec_pretty(&manifest).context("serialize palette.json")?;
    bytes.push(b'\n');
    std::fs::write(stage.join("palette.json"), bytes).context("write palette.json")
}

fn inspect_entry(theme: &Theme, current: &str) -> JsonValue {
    json!({
        "id": theme.name,
        "name": theme.display_name,
        "current": current == theme.name,
        "light": theme.is_light,
        "mode": mode_str(theme),
        "generated": theme.generated,
        "preview": preview_str(theme),
        "colors": extract_colors(theme),
        "values": extract_values(theme),
        "icon_theme": theme.icon_theme,
        "wallpapers": extract_wallpapers(theme),
    })
}

pub fn run(ctx: &Ctx, requested: Option<&str>) -> Result<()> {
    let current = match std::fs::read_to_string(&ctx.current_theme_file) {
        Ok(current) => current,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(error) => return Err(error).context("read current theme"),
    };
    let current = current.trim();

    let themes = match requested {
        Some(name) => {
            let Some(root) = theme::find_root(ctx, name) else {
                bail!("theme is not installed: {name}");
            };
            vec![match Theme::load_root(root, name) {
                Ok(theme) => inspect_entry(&theme, current),
                Err(error) => json!({"id": name, "error": format!("{error:#}")}),
            }]
        }
        None => {
            let names = theme::list_names(ctx)?;
            names
                .iter()
                .map(|name| {
                    // A broken third-party theme must not hide all other installed themes.
                    match Theme::load(ctx, name) {
                        Ok(theme) => inspect_entry(&theme, current),
                        Err(error) => json!({"id": name, "error": format!("{error:#}")}),
                    }
                })
                .collect()
        }
    };

    println!(
        "{}",
        json!({"schema_version": SCHEMA_VERSION, "current": current, "themes": themes})
    );
    Ok(())
}

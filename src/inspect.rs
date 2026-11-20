//! Side-effect-free palette data for desktop previews. Uses the same source
//! precedence and derived values as rendering, never a second theme parser.
use crate::{Ctx, Theme, render::value::Value, theme};
use anyhow::{Context, Result, bail};
use serde_json::{Value as JsonValue, json};
use std::collections::BTreeMap;

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

fn inspect_entry(theme: &Theme, current: &str) -> JsonValue {
    let mode = if theme.is_light { "light" } else { "dark" };
    let preview = theme
        .preview
        .as_ref()
        .map(|p| p.to_string_lossy().into_owned());
    json!({
        "id": theme.name,
        "name": theme.display_name,
        "current": current == theme.name,
        "light": theme.is_light,
        "mode": mode,
        "generated": theme.generated,
        "preview": preview,
        "colors": extract_colors(theme),
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

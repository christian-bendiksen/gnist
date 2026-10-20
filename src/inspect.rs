//! Side-effect-free palette data for desktop previews. Uses the same source
//! precedence and derived values as rendering, never a second theme parser.
use crate::{Ctx, Theme, theme};
use anyhow::{Context, Result, ensure};
use serde_json::json;
use std::collections::BTreeMap;

pub fn run(ctx: &Ctx, requested: Option<&str>) -> Result<()> {
    let names = theme::list_names(ctx)?;
    if let Some(name) = requested {
        ensure!(
            names.iter().any(|item| item == name),
            "theme is not installed: {name}"
        );
    }
    let current = match std::fs::read_to_string(&ctx.current_theme_file) {
        Ok(current) => current,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(error) => return Err(error).context("read current theme"),
    };
    let mut themes = Vec::new();
    for name in names
        .iter()
        .filter(|name| requested.is_none_or(|wanted| wanted == *name))
    {
        // A broken third-party theme must not hide all other installed themes.
        themes.push(match Theme::load(ctx, name) {
            Ok(theme) => {
                let colors: BTreeMap<_, _> = theme.vars.iter()
                    .filter_map(|(key, value)| theme::colors::parse_color(&value.display())
                        .map(|color| (key.clone(), color.to_hex())))
                    .collect();
                json!({"id": name, "current": current.trim() == name,
                    "light": theme.is_light, "colors": colors, "icon_theme": theme.icon_theme,
                    "accentChoices": theme.accent_choices, "accent": theme.accent,
                    "wallpapers": theme.vars.get("wallpapers").map(|value| match value {
                        crate::render::value::Value::List(items) => items.iter().map(|item| item.display()).collect::<Vec<_>>(),
                        _ => Vec::new(),
                    }).unwrap_or_default()})
            }
            Err(error) => json!({"id": name, "error": format!("{error:#}")}),
        });
    }
    println!(
        "{}",
        json!({"schema_version": 1, "current": current.trim(), "themes": themes})
    );
    Ok(())
}

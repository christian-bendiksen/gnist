//! Opt-in, theme-owned accent roles. Never synthesize or retune a palette.
use super::colors;
use anyhow::{Context, Result, ensure};
use serde_json::{Value, json};
use std::{
    collections::{HashMap, HashSet},
    fs,
    path::Path,
};

const ROLES: &[&str] = &[
    "accent",
    "accent_dim",
    "accent_bg",
    "accent_text",
    "on_accent",
    "focus_ring",
    "selection_background",
    "selection_foreground",
    "cursor",
    "border_active",
    "brand_herda",
    "wallpaper_accent",
];

#[derive(Default)]
pub(super) struct Accent {
    pub roles: HashMap<String, String>,
    pub choices: Vec<Value>,
    pub selected: Option<String>,
}

fn document(path: &Path) -> Result<Option<Value>> {
    let bytes = match fs::read(path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error).with_context(|| format!("read {}", path.display())),
    };
    ensure!(bytes.len() <= 65536, "accent document is too large");
    Ok(Some(
        serde_json::from_slice(&bytes).context("parse accent document")?,
    ))
}

fn text<'a>(value: &'a Value, key: &str) -> Result<&'a str> {
    value
        .get(key)
        .and_then(Value::as_str)
        .with_context(|| format!("missing accent {key}"))
}

fn hex(value: &str) -> bool {
    value.len() == 7 && value.starts_with('#') && value[1..].bytes().all(|c| c.is_ascii_hexdigit())
}

pub(super) fn load(root: &Path, light: bool, selection: Option<&Path>) -> Result<Accent> {
    let Some(data) = document(&root.join("accent-choices.json"))? else {
        // Fixed-accent themes do not even read the user's selection.
        return Ok(Accent::default());
    };
    ensure!(data["schema"] == 1, "unsupported accent schema");
    let default = text(&data, "default")?;
    let choices = data["choices"]
        .as_array()
        .context("missing accent choices")?;
    ensure!(
        !choices.is_empty() && choices.len() <= 32,
        "invalid accent choice count"
    );
    let mut ids = HashSet::new();
    for choice in choices {
        let id = text(choice, "id")?;
        ensure!(
            !id.is_empty()
                && id.len() <= 64
                && id
                    .bytes()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'-'),
            "invalid accent id"
        );
        ensure!(ids.insert(id), "duplicate accent id");
        let label = text(choice, "label")?;
        ensure!(
            !label.is_empty() && label.len() <= 80 && !label.chars().any(char::is_control),
            "invalid accent label"
        );
        for mode in ["dark", "light"] {
            let palette = &choice["modes"][mode];
            ensure!(hex(text(palette, "swatch")?), "invalid accent swatch");
            let roles = palette["roles"]
                .as_object()
                .context("missing accent roles")?;
            ensure!(
                ROLES[..ROLES.len() - 1]
                    .iter()
                    .all(|key| roles.contains_key(*key)),
                "incomplete accent roles"
            );
            for (key, value) in roles {
                ensure!(
                    ROLES.contains(&key.as_str()) && value.as_str().is_some_and(hex),
                    "invalid accent role: {key}"
                );
            }
            let accent = colors::parse_color(text(&palette["roles"], "accent")?)
                .context("invalid accent")?
                .luminance();
            let ink = colors::parse_color(text(&palette["roles"], "on_accent")?)
                .context("invalid on-accent ink")?
                .luminance();
            ensure!(
                (accent.max(ink) + 0.05) / (accent.min(ink) + 0.05) >= 4.5,
                "on-accent contrast is below 4.5:1"
            );
        }
    }
    ensure!(ids.contains(default), "accent default is not a choice");
    let selection = selection.map(document).transpose()?.flatten();
    if let Some(value) = &selection {
        ensure!(
            value.is_object() && (value["accent"].is_null() || value["accent"].is_string()),
            "invalid accent selection"
        );
    }
    let requested = selection
        .as_ref()
        .and_then(|value| value["accent"].as_str())
        .unwrap_or(default);
    // A choice made in another declaring theme is retained, but this theme
    // uses its own default if it has no such choice.
    let selected = if ids.contains(requested) {
        requested
    } else {
        default
    };
    let mode = if light { "light" } else { "dark" };
    let choice = choices
        .iter()
        .find(|choice| choice["id"] == selected)
        .context("missing selected accent")?;
    Ok(Accent {
        selected: Some(selected.to_owned()),
        choices: choices.iter().map(|choice| json!({"id": choice["id"], "label": choice["label"], "color": choice["modes"][mode]["swatch"]})).collect(),
        roles: choice["modes"][mode]["roles"].as_object().unwrap().iter().map(|(key, value)| (key.clone(), value.as_str().unwrap().to_owned())).collect(),
    })
}

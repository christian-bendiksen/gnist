use anyhow::{Context, Result};
use std::{collections::HashMap, fs, path::Path};

pub fn build_vars_from_colors(colors_file: &Path) -> Result<HashMap<String, String>> {
    let src = fs::read_to_string(colors_file)
        .with_context(|| format!("read {}", colors_file.display()))?;

    let table: toml::Value = toml::from_str(&src).context("parse colors.toml")?;

    let mut vars = HashMap::new();
    flatten("", &table, &mut vars);

    // Collect first because the map cannot be extended while it is being iterated.
    let derived: Vec<(String, String)> = vars
        .iter()
        .filter(|(_, v)| v.starts_with('#'))
        .flat_map(|(k, v)| derive_color_keys(k, v))
        .collect();

    vars.extend(derived);
    Ok(vars)
}

/// Flatten nested TOML keys with underscores between path components.
fn flatten(prefix: &str, value: &toml::Value, out: &mut HashMap<String, String>) {
    match value {
        toml::Value::Table(map) => {
            for (k, v) in map {
                let key = if prefix.is_empty() {
                    k.to_owned()
                } else {
                    format!("{prefix}_{k}")
                };
                flatten(&key, v, out);
            }
        }
        toml::Value::String(s) => {
            out.insert(prefix.to_owned(), s.clone());
        }
        toml::Value::Integer(i) => {
            out.insert(prefix.to_owned(), i.to_string());
        }
        toml::Value::Float(f) => {
            out.insert(prefix.to_owned(), f.to_string());
        }
        toml::Value::Boolean(b) => {
            out.insert(prefix.to_owned(), b.to_string());
        }
        // Color files use scalar values, so arrays and datetimes are ignored.
        _ => {}
    }
}

/// Add the stripped and decimal RGB forms used by templates.
fn derive_color_keys(key: &str, hex: &str) -> impl Iterator<Item = (String, String)> {
    let bare = hex.trim_start_matches('#');
    let rgb = hex_to_rgb(bare).map(|r| (format!("{key}_rgb"), r));
    let strip = (format!("{key}_strip"), bare.to_owned());
    std::iter::once(strip).chain(rgb)
}

fn hex_to_rgb(hex: &str) -> Option<String> {
    if hex.len() != 6 || !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    let r = u8::from_str_radix(&hex[0..2], 16).ok()?;
    let g = u8::from_str_radix(&hex[2..4], 16).ok()?;
    let b = u8::from_str_radix(&hex[4..6], 16).ok()?;
    Some(format!("{r},{g},{b}"))
}

pub fn insert_border(vars: &mut HashMap<String, String>, kind: &str, color: &str) {
    let key = format!("border_{kind}");
    vars.extend(derive_color_keys(&key, color));
    vars.insert(key, color.to_owned());
}

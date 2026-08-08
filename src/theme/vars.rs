use super::colors;
use crate::render::value::Value;
use anyhow::{Context, Result};
use std::{collections::HashMap, fs, path::Path};

/// Build the typed template context from a theme's `colors.toml`.
///
/// Scalars become typed values (integers, floats, booleans, lists) while
/// strings keep their raw spelling so plain `{{ key }}` output is unchanged.
pub fn build_vars_from_colors(colors_file: &Path) -> Result<HashMap<String, Value>> {
    let src = fs::read_to_string(colors_file)
        .with_context(|| format!("read {}", colors_file.display()))?;

    let table: toml::Value = toml::from_str(&src).context("parse colors.toml")?;

    let mut vars = HashMap::new();
    flatten("", &table, &mut vars);

    // Collect first because the map cannot be extended while it is being iterated.
    let derived: Vec<(String, Value)> = vars
        .iter()
        .filter_map(|(k, v)| match v {
            Value::Str(s) if s.trim_start().starts_with('#') => {
                Some(derive_color_keys(k, s).collect::<Vec<_>>())
            }
            _ => None,
        })
        .flatten()
        .collect();

    vars.extend(derived);
    Ok(vars)
}

/// Inject the render context that is not part of `colors.toml`.
pub fn inject_context(
    vars: &mut HashMap<String, Value>,
    name: &str,
    is_light: bool,
    wallpapers: &[String],
) {
    vars.insert("name".to_owned(), Value::Str(name.to_owned()));
    vars.insert(
        "mode".to_owned(),
        Value::Str(if is_light { "light" } else { "dark" }.to_owned()),
    );
    vars.insert("is_light".to_owned(), Value::Bool(is_light));
    vars.insert(
        "colors".to_owned(),
        Value::List(
            (0..16)
                .map(|i| {
                    vars.get(&format!("color{i}"))
                        .cloned()
                        .unwrap_or(Value::Missing)
                })
                .collect(),
        ),
    );
    vars.insert(
        "wallpapers".to_owned(),
        Value::List(wallpapers.iter().cloned().map(Value::Str).collect()),
    );
}

/// Flatten nested TOML keys with underscores between path components.
fn flatten(prefix: &str, value: &toml::Value, out: &mut HashMap<String, Value>) {
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
            out.insert(prefix.to_owned(), Value::Str(s.clone()));
        }
        toml::Value::Integer(i) => {
            out.insert(prefix.to_owned(), Value::Int(*i));
        }
        toml::Value::Float(f) => {
            out.insert(prefix.to_owned(), Value::Float(*f));
        }
        toml::Value::Boolean(b) => {
            out.insert(prefix.to_owned(), Value::Bool(*b));
        }
        toml::Value::Array(items) => {
            out.insert(
                prefix.to_owned(),
                Value::List(
                    items
                        .iter()
                        .map(|item| match item {
                            toml::Value::String(s) => Value::Str(s.clone()),
                            toml::Value::Integer(i) => Value::Int(*i),
                            toml::Value::Float(f) => Value::Float(*f),
                            toml::Value::Boolean(b) => Value::Bool(*b),
                            _ => Value::Missing,
                        })
                        .collect(),
                ),
            );
        }
        // Datetimes are not modelled.
        _ => {}
    }
}

/// Add the stripped and decimal RGB forms used by templates.
///
/// Shorthand and 8-digit hex are expanded/normalized through the shared parser,
/// so `#abc` yields `aabbcc` and `#rrggbbaa` keeps only its RGB channels.
fn derive_color_keys(key: &str, hex: &str) -> impl Iterator<Item = (String, Value)> {
    let mut keys = Vec::new();

    let strip = match colors::parse_hex(hex) {
        Some([r, g, b]) => format!("{r:02x}{g:02x}{b:02x}"),
        None => hex.trim_start_matches('#').to_owned(),
    };
    keys.push((format!("{key}_strip"), Value::Str(strip)));

    if let Some([r, g, b]) = colors::parse_hex(hex) {
        keys.push((format!("{key}_rgb"), Value::Str(format!("{r},{g},{b}"))));
    }

    keys.into_iter()
}

pub fn insert_border(vars: &mut HashMap<String, Value>, kind: &str, color: &str) {
    let key = format!("border_{kind}");
    vars.extend(derive_color_keys(&key, color));
    vars.insert(key, Value::Str(color.to_owned()));
}

#[cfg(test)]
mod tests {
    use super::{build_vars_from_colors, derive_color_keys, inject_context};
    use crate::render::value::Value;
    use std::collections::HashMap;

    fn derived(color: &str) -> HashMap<String, Value> {
        derive_color_keys("accent", color).collect()
    }

    fn str_value(vars: &HashMap<String, Value>, key: &str) -> String {
        match &vars[key] {
            Value::Str(s) => s.clone(),
            other => other.display(),
        }
    }

    #[test]
    fn six_digit_hex_derives_strip_and_rgb() {
        let vars = derived("#cdd6f4");
        assert_eq!(str_value(&vars, "accent_strip"), "cdd6f4");
        assert_eq!(str_value(&vars, "accent_rgb"), "205,214,244");
    }

    #[test]
    fn shorthand_is_expanded_before_deriving() {
        let vars = derived("#abc");
        assert_eq!(str_value(&vars, "accent_strip"), "aabbcc");
        assert_eq!(str_value(&vars, "accent_rgb"), "170,187,204");
    }

    #[test]
    fn alpha_is_dropped_before_deriving() {
        let vars = derived("#89b4faff");
        assert_eq!(str_value(&vars, "accent_strip"), "89b4fa");
        assert_eq!(str_value(&vars, "accent_rgb"), "137,180,250");
    }

    #[test]
    fn non_hex_values_only_get_a_strip_form() {
        let vars = derived("#dusk");
        assert_eq!(str_value(&vars, "accent_strip"), "dusk");
        assert!(!vars.contains_key("accent_rgb"));
    }

    #[test]
    fn scalars_are_typed_and_strings_keep_their_spelling() {
        let dir = tempfile::tempdir().unwrap();
        let colors = dir.path().join("colors.toml");
        std::fs::write(
            &colors,
            "accent = \"#CDD6F4\"\nradius = 8\nratio = 0.5\nenabled = true\nitems = [\"a\", \"b\"]\n",
        )
        .unwrap();

        let vars = build_vars_from_colors(&colors).unwrap();

        assert!(matches!(&vars["accent"], Value::Str(s) if s == "#CDD6F4"));
        assert!(matches!(&vars["radius"], Value::Int(8)));
        assert!(matches!(&vars["ratio"], Value::Float(f) if *f == 0.5));
        assert!(matches!(&vars["enabled"], Value::Bool(true)));
        assert!(matches!(&vars["items"], Value::List(_)));
    }

    #[test]
    fn context_injects_mode_name_and_lists() {
        let mut vars = HashMap::new();
        vars.insert("color0".to_owned(), Value::Str("#000000".into()));
        inject_context(&mut vars, "dusk", true, &["/a.png".into(), "/b.png".into()]);

        assert_eq!(str_value(&vars, "name"), "dusk");
        assert_eq!(str_value(&vars, "mode"), "light");
        assert!(matches!(&vars["is_light"], Value::Bool(true)));
        assert!(matches!(&vars["colors"], Value::List(_)));
        assert!(matches!(&vars["wallpapers"], Value::List(_)));
    }
}

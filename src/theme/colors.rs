//! Builds `colors.toml` from Kitty, Ghostty, or Alacritty, in that order.

use std::collections::HashMap;
use std::fmt::Write as _;
use std::path::Path;

const REQUIRED: &[&str] = &[
    "foreground",
    "background",
    "cursor",
    "selection_foreground",
    "selection_background",
    "color0",
    "color1",
    "color2",
    "color3",
    "color4",
    "color5",
    "color6",
    "color7",
    "color8",
    "color9",
    "color10",
    "color11",
    "color12",
    "color13",
    "color14",
    "color15",
];

pub fn generate(theme_dir: &Path) -> Option<String> {
    generate_from_kitty(theme_dir)
        .or_else(|| generate_from_ghostty(theme_dir))
        .or_else(|| generate_from_alacritty(theme_dir))
}

fn build_output(map: &HashMap<String, String>) -> Option<String> {
    if REQUIRED.iter().any(|k| !map.contains_key(*k)) {
        return None;
    }

    let mut out = String::new();
    writeln!(out, "accent = \"{}\"", map["color4"]).unwrap();
    for key in [
        "cursor",
        "foreground",
        "background",
        "selection_foreground",
        "selection_background",
    ] {
        writeln!(out, "{key} = \"{}\"", map[key]).unwrap();
    }
    writeln!(out).unwrap();
    for i in 0..16u8 {
        writeln!(out, "color{i} = \"{}\"", map[&format!("color{i}")]).unwrap();
    }
    Some(out)
}

fn generate_from_kitty(theme_dir: &Path) -> Option<String> {
    let conf = std::fs::read_to_string(theme_dir.join("kitty.conf")).ok()?;
    build_output(&parse_kitty(&conf))
}

fn generate_from_ghostty(theme_dir: &Path) -> Option<String> {
    let conf = std::fs::read_to_string(theme_dir.join("ghostty.conf")).ok()?;
    build_output(&parse_ghostty(&conf))
}

fn generate_from_alacritty(theme_dir: &Path) -> Option<String> {
    let conf = std::fs::read_to_string(theme_dir.join("alacritty.toml")).ok()?;
    build_output(&parse_alacritty(&conf)?)
}

fn fallback(map: &mut HashMap<String, String>, key: &str, source: &str) {
    if !map.contains_key(key)
        && let Some(val) = map.get(source).cloned()
    {
        map.insert(key.to_owned(), val);
    }
}

fn parse_kitty(conf: &str) -> HashMap<String, String> {
    conf.lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .filter_map(|l| {
            let mut parts = l.splitn(2, |c: char| c.is_ascii_whitespace());
            Some((parts.next()?.to_owned(), parts.next()?.trim().to_owned()))
        })
        .collect()
}

fn parse_ghostty(conf: &str) -> HashMap<String, String> {
    let mut map = HashMap::new();

    for line in conf.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let Some((key, val)) = line.split_once('=') else {
            continue;
        };
        let (key, val) = (key.trim(), val.trim());

        if key == "palette" {
            if let Some((idx, color)) = val.split_once('=')
                && let Ok(n) = idx.trim().parse::<u8>()
            {
                map.insert(format!("color{n}"), normalize_color(color.trim()));
            }
            continue;
        }

        let canonical = match key {
            "background" => "background",
            "foreground" => "foreground",
            "cursor-color" => "cursor",
            "selection-background" => "selection_background",
            "selection-foreground" => "selection_foreground",
            _ => continue,
        };
        map.insert(canonical.to_owned(), normalize_color(val));
    }
    fallback(&mut map, "cursor", "foreground");

    map
}

fn parse_alacritty(conf: &str) -> Option<HashMap<String, String>> {
    let val: toml::Value = conf.parse().ok()?;
    let colors = val.get("colors")?;
    let mut map = HashMap::new();

    if let Some(primary) = colors.get("primary") {
        if let Some(v) = toml_str(primary, "background") {
            map.insert("background".into(), v);
        }
        if let Some(v) = toml_str(primary, "foreground") {
            map.insert("foreground".into(), v);
        }
    }

    // Alacritty nests the cursor color under `[colors.cursor]`.
    if let Some(cursor_section) = colors.get("cursor")
        && let Some(v) = toml_str(cursor_section, "cursor")
    {
        map.insert("cursor".into(), v);
    }
    fallback(&mut map, "cursor", "foreground");

    // Alacritty calls the selection foreground `text`.
    if let Some(sel) = colors.get("selection") {
        if let Some(v) = toml_str(sel, "background") {
            map.insert("selection_background".into(), v);
        }
        if let Some(v) = toml_str(sel, "text").or_else(|| toml_str(sel, "foreground")) {
            map.insert("selection_foreground".into(), v);
        }
    }
    fallback(&mut map, "selection_background", "background");
    fallback(&mut map, "selection_foreground", "foreground");

    const ORDER: &[&str] = &[
        "black", "red", "green", "yellow", "blue", "magenta", "cyan", "white",
    ];
    if let Some(normal) = colors.get("normal") {
        for (i, name) in ORDER.iter().enumerate() {
            if let Some(v) = toml_str(normal, name) {
                map.insert(format!("color{i}"), v);
            }
        }
    }
    if let Some(bright) = colors.get("bright") {
        for (i, name) in ORDER.iter().enumerate() {
            if let Some(v) = toml_str(bright, name) {
                map.insert(format!("color{}", i + 8), v);
            }
        }
    }

    Some(map)
}

fn toml_str(val: &toml::Value, key: &str) -> Option<String> {
    val.get(key)?.as_str().map(normalize_color)
}

pub fn normalize_color(s: &str) -> String {
    let s = s.trim_matches('"').trim_matches('\'');
    if s.starts_with('#') {
        s.to_owned()
    } else if let Some(inner) = s.strip_prefix("rgba(").and_then(|s| s.strip_suffix(')')) {
        // Gnist does not model alpha, so keep only the RGB channels.
        format!("#{}", &inner.trim()[..6])
    } else if let Some(inner) = s.strip_prefix("rgb(").and_then(|s| s.strip_suffix(')')) {
        format!("#{}", inner.trim())
    } else {
        format!("#{s}")
    }
}

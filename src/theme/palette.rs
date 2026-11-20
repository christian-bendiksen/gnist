//! Omarchy 4 semantic <-> ANSI palette alias cascade and colour mixing.
//!
//! Resolution is strictly **fill-only**: it never overwrites a key the theme
//! explicitly wrote. Upstream Omarchy force-sets `color0 = background`,
//! `color7 = foreground`, and `cursor = bright_foreground`, whereas Herda's
//! bundled themes intentionally define those with distinct values (for example
//! `herda-lysrom` sets `color0 = "#2C2F35"`, `background = "#0A0D14"`, and
//! `cursor = "#4DA3FF"`). A pure Omarchy 4 theme defines none of those keys,
//! so fill-only produces byte-identical output for upstream Omarchy themes
//! while keeping every existing Herda render unchanged.

use super::colors;
use crate::render::value::Value;
use std::collections::HashMap;

/// Fill missing semantic, ANSI, shade, and short-name palette keys in `vars`.
pub fn resolve(vars: &mut HashMap<String, Value>) {
    // 1. Short names to canonical.
    const SHORT_TO_CANONICAL: &[(&str, &str)] = &[
        ("bg", "background"),
        ("dark_bg", "dark_background"),
        ("darker_bg", "darker_background"),
        ("lighter_bg", "lighter_background"),
        ("fg", "foreground"),
        ("dark_fg", "dark_foreground"),
        ("light_fg", "light_foreground"),
        ("bright_fg", "bright_foreground"),
    ];
    for &(short, canonical) in SHORT_TO_CANONICAL {
        fill_from(vars, canonical, &[short]);
    }

    // 2. Background/foreground <-> color0/color7.
    fill_from(vars, "background", &["color0"]);
    fill_from(vars, "foreground", &["color7"]);
    fill_from(vars, "color0", &["background"]);
    fill_from(vars, "color7", &["foreground"]);

    // 3. Semantic from ANSI when missing.
    const SEMANTIC_FROM_ANSI: &[(&str, &str)] = &[
        ("red", "color1"),
        ("green", "color2"),
        ("yellow", "color3"),
        ("blue", "color4"),
        ("magenta", "color5"),
        ("cyan", "color6"),
        ("bright_red", "color9"),
        ("bright_green", "color10"),
        ("bright_yellow", "color11"),
        ("bright_blue", "color12"),
        ("bright_magenta", "color13"),
        ("bright_cyan", "color14"),
    ];
    for &(semantic, ansi) in SEMANTIC_FROM_ANSI {
        fill_from(vars, semantic, &[ansi]);
    }
    fill_from(vars, "magenta", &["purple"]);
    fill_from(vars, "bright_magenta", &["bright_purple"]);

    // 4. Derived semantic, cursor, muted, and selection keys.
    fill_from(vars, "light_foreground", &["color7", "foreground"]);
    fill_from(vars, "bright_foreground", &["color15", "foreground"]);
    fill_from(vars, "cursor", &["bright_foreground"]);
    fill_from(vars, "lighter_background", &["color0", "background"]);
    fill_from(vars, "dark_foreground", &["color8", "foreground"]);
    fill_from(vars, "muted", &["color8", "dark_foreground"]);
    fill_from(
        vars,
        "selection",
        &["selection_background", "color8", "color0", "background"],
    );
    fill_from(vars, "selection_background", &["selection"]);
    fill_from(vars, "selection_foreground", &["bright_foreground"]);
    fill_from(vars, "orange", &["yellow"]);
    fill_mixed(vars, "brown", "orange", "#000000", "50%");

    // 5. Shades.
    fill_mixed(vars, "dark_background", "background", "#000000", "25%");
    fill_mixed(vars, "darker_background", "background", "#000000", "50%");
    for (bright, base) in [
        ("bright_red", "red"),
        ("bright_yellow", "yellow"),
        ("bright_green", "green"),
        ("bright_cyan", "cyan"),
        ("bright_blue", "blue"),
        ("bright_magenta", "magenta"),
    ] {
        fill_mixed(vars, bright, base, "#ffffff", "20%");
    }
    fill_from(vars, "purple", &["magenta"]);
    fill_from(vars, "bright_purple", &["bright_magenta"]);

    // 6. ANSI from semantic when missing.
    const ANSI_FROM_SEMANTIC: &[(&str, &str)] = &[
        ("color0", "background"),
        ("color1", "red"),
        ("color2", "green"),
        ("color3", "yellow"),
        ("color4", "blue"),
        ("color5", "magenta"),
        ("color6", "cyan"),
        ("color7", "foreground"),
        ("color8", "muted"),
        ("color9", "bright_red"),
        ("color10", "bright_green"),
        ("color11", "bright_yellow"),
        ("color12", "bright_blue"),
        ("color13", "bright_magenta"),
        ("color14", "bright_cyan"),
        ("color15", "bright_foreground"),
    ];
    for &(ansi, semantic) in ANSI_FROM_SEMANTIC {
        fill_from(vars, ansi, &[semantic]);
    }
    fill_from(vars, "accent", &["color4"]);

    // 7. Canonical back to short names so old templates still resolve.
    for &(short, canonical) in SHORT_TO_CANONICAL {
        fill_from(vars, short, &[canonical]);
    }
}

/// Parse an Omarchy mix amount: `N%` -> `N / 100`, a bare number > 1 -> `N / 100`,
/// clamped to `0.0..=1.0`.
pub fn parse_mix_amount(raw: &str) -> Option<f64> {
    let trimmed = raw.trim();
    let (num_str, is_percent) = match trimmed.strip_suffix('%') {
        Some(rest) => (rest.trim(), true),
        None => (trimmed, false),
    };
    if num_str.is_empty() {
        return None;
    }
    let val: f64 = num_str.parse().ok()?;
    if !val.is_finite() {
        return None;
    }
    let ratio = if is_percent || val > 1.0 {
        val / 100.0
    } else {
        val
    };
    Some(ratio.clamp(0.0, 1.0))
}

/// Mix two RGB channels using Omarchy's rounding formula:
/// `int(start * (1 - a) + end * a + 0.5)`.
pub fn mix_rgb8(start: [u8; 3], end: [u8; 3], amount: f64) -> [u8; 3] {
    let a = amount.clamp(0.0, 1.0);
    let mix_ch = |s: u8, e: u8| -> u8 {
        ((f64::from(s) * (1.0 - a) + f64::from(e) * a + 0.5).floor()).clamp(0.0, 255.0) as u8
    };
    [
        mix_ch(start[0], end[0]),
        mix_ch(start[1], end[1]),
        mix_ch(start[2], end[2]),
    ]
}

/// Mix two hex colours with Omarchy's formula and amount syntax, returning `#rrggbb`.
pub fn mix_hex(start: &str, end: &str, amount: &str) -> Option<String> {
    let s = colors::parse_hex(start)?;
    let e = colors::parse_hex(end)?;
    let a = parse_mix_amount(amount)?;
    let [r, g, b] = mix_rgb8(s, e, a);
    Some(format!("#{r:02x}{g:02x}{b:02x}"))
}

fn is_present(vars: &HashMap<String, Value>, key: &str) -> bool {
    match vars.get(key) {
        None | Some(Value::Missing) => false,
        Some(Value::Str(s)) => !s.trim().is_empty(),
        Some(_) => true,
    }
}

fn get_str(vars: &HashMap<String, Value>, key: &str) -> Option<String> {
    match vars.get(key) {
        Some(Value::Str(s)) if !s.trim().is_empty() => Some(s.clone()),
        Some(Value::Color(c)) => Some(c.to_hex()),
        _ => None,
    }
}

fn fill_from(vars: &mut HashMap<String, Value>, target: &str, sources: &[&str]) {
    if is_present(vars, target) {
        return;
    }
    for src in sources {
        if let Some(val) = get_str(vars, src) {
            vars.insert(target.to_owned(), Value::Str(val));
            return;
        }
    }
}

fn fill_mixed(
    vars: &mut HashMap<String, Value>,
    target: &str,
    source: &str,
    end: &str,
    amount: &str,
) {
    if is_present(vars, target) {
        return;
    }
    if let Some(start) = get_str(vars, source)
        && let Some(mixed) = mix_hex(&start, end, amount)
    {
        vars.insert(target.to_owned(), Value::Str(mixed));
    }
}

#[cfg(test)]
mod tests {
    use super::{mix_hex, parse_mix_amount, resolve};
    use crate::render::value::Value;
    use crate::theme::vars::build_vars_from_str;
    use std::collections::HashMap;

    fn val<'a>(vars: &'a HashMap<String, Value>, key: &str) -> &'a str {
        match vars.get(key) {
            Some(Value::Str(s)) => s.as_str(),
            other => panic!("expected string for {key}, got {other:?}"),
        }
    }

    #[test]
    fn omarchy4_tokyo_night_resolves_to_expected_palette() {
        // Upstream Omarchy 4 tokyo-night semantic colors.toml (no colorN, cursor,
        // selection_foreground, selection_background, brown, or dark_background).
        let tokyo_night = r##"
mode = "dark"
accent = "#7aa2f7"
selection = "#292e42"
muted = "#414868"
background = "#1a1b26"
lighter_background = "#24283b"
foreground = "#a9b1d6"
dark_foreground = "#565f89"
light_foreground = "#b4bee6"
bright_foreground = "#c0caf5"
red = "#f7768e"
yellow = "#e0af68"
orange = "#ff9e64"
green = "#9ece6a"
cyan = "#7dcfff"
blue = "#7aa2f7"
magenta = "#bb9af7"
bright_red = "#ff7a93"
bright_yellow = "#ff9e64"
bright_green = "#b9f27c"
bright_cyan = "#0db9d7"
bright_blue = "#7da6ff"
bright_magenta = "#ad8ee6"
"##;

        let mut vars = build_vars_from_str(tokyo_night).unwrap();
        resolve(&mut vars);

        let expected = [
            ("color0", "#1a1b26"),
            ("color1", "#f7768e"),
            ("color2", "#9ece6a"),
            ("color3", "#e0af68"),
            ("color4", "#7aa2f7"),
            ("color5", "#bb9af7"),
            ("color6", "#7dcfff"),
            ("color7", "#a9b1d6"),
            ("color8", "#414868"),
            ("color9", "#ff7a93"),
            ("color10", "#b9f27c"),
            ("color11", "#ff9e64"),
            ("color12", "#7da6ff"),
            ("color13", "#ad8ee6"),
            ("color14", "#0db9d7"),
            ("color15", "#c0caf5"),
            ("cursor", "#c0caf5"),
            ("selection_background", "#292e42"),
            ("selection_foreground", "#c0caf5"),
            ("brown", "#804f32"),
            ("dark_background", "#14141d"),
            ("darker_background", "#0d0e13"),
            ("purple", "#bb9af7"),
            ("bright_purple", "#ad8ee6"),
            ("bg", "#1a1b26"),
            ("fg", "#a9b1d6"),
            ("dark_bg", "#14141d"),
            ("darker_bg", "#0d0e13"),
            ("lighter_bg", "#24283b"),
            ("dark_fg", "#565f89"),
            ("light_fg", "#b4bee6"),
            ("bright_fg", "#c0caf5"),
        ];

        for (key, want) in expected {
            assert_eq!(val(&vars, key), want, "mismatch for {key}");
        }
    }

    #[test]
    fn fill_only_preserves_every_herda_lysrom_key_and_derives_semantic_aliases() {
        let lysrom = r##"
look = "lysrom"
accent = "#4DA3FF"
cursor = "#4DA3FF"
foreground = "#E8ECF4"
background = "#0A0D14"
selection_foreground = "#E8ECF4"
selection_background = "#1E3A5F"
color0 = "#2C2F35"
color1 = "#F28B82"
color2 = "#7CD595"
color3 = "#E2C27A"
color4 = "#8DB8F0"
color5 = "#C99BE8"
color6 = "#7FCFE0"
color7 = "#C9CDD1"
color8 = "#979CA1"
color9 = "#FFA8A1"
color10 = "#9FE3B1"
color11 = "#F0D59B"
color12 = "#ABCBF6"
color13 = "#DBB7F1"
color14 = "#A5E2EC"
color15 = "#ECECE9"
muted = "#ADACAC"
"##;

        let original = build_vars_from_str(lysrom).unwrap();
        let mut vars = original.clone();
        resolve(&mut vars);

        // Every key explicitly written by herda-lysrom must be byte-identical.
        for (key, orig_val) in &original {
            assert_eq!(
                vars.get(key),
                Some(orig_val),
                "fill-only violated for key {key}"
            );
        }

        // Semantic aliases are filled from ANSI.
        assert_eq!(val(&vars, "red"), val(&vars, "color1"));
        assert_eq!(val(&vars, "green"), val(&vars, "color2"));
        assert_eq!(val(&vars, "yellow"), val(&vars, "color3"));
        assert_eq!(val(&vars, "blue"), val(&vars, "color4"));
        assert_eq!(val(&vars, "magenta"), val(&vars, "color5"));
        assert_eq!(val(&vars, "cyan"), val(&vars, "color6"));
        assert_eq!(val(&vars, "bright_red"), val(&vars, "color9"));
        assert_eq!(val(&vars, "bg"), "#0A0D14");
        assert_eq!(val(&vars, "fg"), "#E8ECF4");
    }

    #[test]
    fn mix_hex_matches_omarchy_half_up_rounding_and_amount_formats() {
        // Half-up rounding: 1 * 0.5 + 0.5 = 1.0 -> 0x01 (unlike round_ties_even -> 0x00).
        assert_eq!(
            mix_hex("#000000", "#010101", "50%").as_deref(),
            Some("#010101")
        );
        assert_eq!(
            mix_hex("#000000", "#ffffff", "50").as_deref(),
            Some("#808080")
        );
        assert_eq!(
            mix_hex("#000000", "#ffffff", "0.5").as_deref(),
            Some("#808080")
        );
        assert_eq!(
            mix_hex("#1a1b26", "#000000", "25%").as_deref(),
            Some("#14141d")
        );
        assert_eq!(
            mix_hex("#ff9e64", "#000000", "50%").as_deref(),
            Some("#804f32")
        );
        assert_eq!(parse_mix_amount("15%"), Some(0.15));
        assert_eq!(parse_mix_amount("0.35"), Some(0.35));
        assert_eq!(parse_mix_amount("50"), Some(0.5));
        assert_eq!(parse_mix_amount("150%"), Some(1.0));
        assert_eq!(parse_mix_amount("-5%"), Some(0.0));
        assert_eq!(parse_mix_amount("abc"), None);
    }
}

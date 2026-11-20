//! Omarchy 4 bare-word template helpers.
//!
//! Supported forms (matched on the raw expression string before the normal
//! expression lexer runs, because `%` and `(` lex as operators):
//!
//! - `mix KEY KEY NUMBER[%]` -> `#rrggbb`
//! - `mix_strip KEY KEY NUMBER[%]` -> `rrggbb`
//! - `mix_rgb KEY KEY NUMBER[%]` -> `r,g,b`
//! - `hypr_gradient KEY [FALLBACK]` -> Lua/Hyprland gradient string or table
//! - `shell_gradient KEY [FALLBACK]` -> space-separated colours + optional angle
//! - `gradient_start KEY [FALLBACK]` -> first colour as `#rrggbb`

use std::collections::HashMap;

use super::value::Value;
use crate::theme::{colors, palette};

/// Evaluate an Omarchy bare-word helper if `expr` matches one.
///
/// Returns `None` when `expr` is not an Omarchy helper or when required inputs
/// cannot be resolved (letting the caller keep the raw placeholder visible).
pub fn eval(
    expr: &str,
    values: &HashMap<String, Value>,
    overlay: &HashMap<String, Value>,
) -> Option<Value> {
    let trimmed = expr.trim();
    let (helper, rest) = split_first_word(trimmed)?;
    let rest = rest.trim();
    if rest.is_empty() {
        return None;
    }

    match helper {
        "mix" => eval_mix(rest, values, overlay, MixFormat::Hex),
        "mix_strip" => eval_mix(rest, values, overlay, MixFormat::Strip),
        "mix_rgb" => eval_mix(rest, values, overlay, MixFormat::Rgb),
        "hypr_gradient" => eval_gradient(rest, values, overlay, GradientFormat::Hypr),
        "shell_gradient" => eval_gradient(rest, values, overlay, GradientFormat::Shell),
        "gradient_start" => eval_gradient(rest, values, overlay, GradientFormat::Start),
        _ => None,
    }
}

#[derive(Clone, Copy)]
enum MixFormat {
    Hex,
    Strip,
    Rgb,
}

#[derive(Clone, Copy)]
enum GradientFormat {
    Hypr,
    Shell,
    Start,
}

fn eval_mix(
    rest: &str,
    values: &HashMap<String, Value>,
    overlay: &HashMap<String, Value>,
    format: MixFormat,
) -> Option<Value> {
    let parts: Vec<&str> = rest.split_whitespace().collect();
    if parts.len() != 3 {
        return None;
    }
    let start = resolve_hex6(parts[0], values, overlay)?;
    let end = resolve_hex6(parts[1], values, overlay)?;
    let amount = palette::parse_mix_amount(parts[2])?;
    let [r, g, b] = palette::mix_rgb8(start, end, amount);

    let rendered = match format {
        MixFormat::Hex => format!("#{r:02x}{g:02x}{b:02x}"),
        MixFormat::Strip => format!("{r:02x}{g:02x}{b:02x}"),
        MixFormat::Rgb => format!("{r},{g},{b}"),
    };
    Some(Value::Str(rendered))
}

fn resolve_hex6(
    key: &str,
    values: &HashMap<String, Value>,
    overlay: &HashMap<String, Value>,
) -> Option<[u8; 3]> {
    if let Some(raw) = lookup_nonempty(values, overlay, key) {
        return parse_strict_hex6(&raw);
    }
    if key.starts_with('#') {
        return parse_strict_hex6(key);
    }
    None
}

fn parse_strict_hex6(s: &str) -> Option<[u8; 3]> {
    let hex = s.trim().strip_prefix('#')?;
    if hex.len() != 6 || !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    colors::parse_hex(s)
}

fn eval_gradient(
    rest: &str,
    values: &HashMap<String, Value>,
    overlay: &HashMap<String, Value>,
    format: GradientFormat,
) -> Option<Value> {
    let (key, fallback) = match split_first_word(rest) {
        Some((k, fb)) => {
            let fb = fb.trim();
            (k, (!fb.is_empty()).then_some(fb))
        }
        None => (rest, None),
    };
    if is_operator_token(key) {
        return None;
    }

    let spec = lookup_nonempty(values, overlay, key)
        .or_else(|| {
            fallback.map(|fb| lookup_nonempty(values, overlay, fb).unwrap_or_else(|| fb.to_owned()))
        })
        .unwrap_or_else(|| key.to_owned());

    let mut colors_out: Vec<String> = Vec::new();
    let mut angle: Option<String> = None;

    for part in split_spec_parts(&spec) {
        if let Some(deg) = parse_angle_deg(part) {
            angle = Some(deg.to_owned());
        } else {
            let resolved =
                lookup_nonempty(values, overlay, part).unwrap_or_else(|| part.to_owned());
            colors_out.push(resolved);
        }
    }

    let rendered = match format {
        GradientFormat::Hypr => match colors_out.len() {
            0 => format!("\"{spec}\""),
            1 => format!("\"{}\"", colors_out[0]),
            _ => {
                let items = colors_out
                    .iter()
                    .map(|c| format!("\"{c}\""))
                    .collect::<Vec<_>>()
                    .join(", ");
                match angle {
                    Some(a) => format!("{{ colors = {{ {items} }}, angle = {a} }}"),
                    None => format!("{{ colors = {{ {items} }} }}"),
                }
            }
        },
        GradientFormat::Shell => {
            if colors_out.is_empty() {
                match angle {
                    Some(a) => format!("{a}deg"),
                    None => spec,
                }
            } else {
                let mut out = colors_out.join(" ");
                if let Some(a) = angle {
                    out.push(' ');
                    out.push_str(&a);
                    out.push_str("deg");
                }
                out
            }
        }
        GradientFormat::Start => {
            let first = colors_out.first()?;
            parse_gradient_color(first)?
        }
    };

    Some(Value::Str(rendered))
}

fn split_first_word(s: &str) -> Option<(&str, &str)> {
    let idx = s.find(char::is_whitespace)?;
    Some((&s[..idx], &s[idx..]))
}

fn is_operator_token(token: &str) -> bool {
    matches!(
        token,
        "|" | "==" | "!=" | ">=" | "<=" | ">" | "<" | "+" | "-" | "*" | "/" | "%"
    )
}

fn lookup_nonempty(
    values: &HashMap<String, Value>,
    overlay: &HashMap<String, Value>,
    key: &str,
) -> Option<String> {
    let v = overlay.get(key).or_else(|| values.get(key))?;
    if v.is_missing() {
        return None;
    }
    let s = v.display();
    let trimmed = s.trim();
    (!trimmed.is_empty()).then(|| trimmed.to_owned())
}

/// Split a gradient spec on whitespace, keeping whitespace inside `(...)` intact.
fn split_spec_parts(spec: &str) -> Vec<&str> {
    let mut parts = Vec::new();
    let mut depth: usize = 0;
    let mut start: Option<usize> = None;

    for (i, ch) in spec.char_indices() {
        match ch {
            '(' => {
                if start.is_none() {
                    start = Some(i);
                }
                depth += 1;
            }
            ')' => {
                depth = depth.saturating_sub(1);
            }
            c if c.is_whitespace() && depth == 0 => {
                if let Some(s) = start.take() {
                    parts.push(&spec[s..i]);
                }
            }
            _ => {
                if start.is_none() {
                    start = Some(i);
                }
            }
        }
    }
    if let Some(s) = start {
        parts.push(&spec[s..]);
    }
    parts
}

/// Match `-?N(.N)?deg` and return the numeric prefix without `deg`.
fn parse_angle_deg(part: &str) -> Option<&str> {
    let num = part.strip_suffix("deg")?;
    let body = num.strip_prefix('-').unwrap_or(num);
    if body.is_empty() {
        return None;
    }
    match body.split_once('.') {
        Some((int_part, frac_part)) => {
            if !int_part.is_empty()
                && !frac_part.is_empty()
                && int_part.bytes().all(|b| b.is_ascii_digit())
                && frac_part.bytes().all(|b| b.is_ascii_digit())
            {
                Some(num)
            } else {
                None
            }
        }
        None => body.bytes().all(|b| b.is_ascii_digit()).then_some(num),
    }
}

/// Convert a gradient stop colour to `#rrggbb`.
///
/// Accepts `#rrggbb[aa]`, `rgb[a](rrggbb[aa])`, `rgb[a](r,g,b[,a])`, and `0xAARRGGBB`.
fn parse_gradient_color(raw: &str) -> Option<String> {
    let s = raw.trim().trim_matches('"').trim_matches('\'').trim();

    if s.starts_with('#') {
        let [r, g, b] = colors::parse_hex(s)?;
        return Some(format!("#{r:02x}{g:02x}{b:02x}"));
    }

    if let Some(hex) = s.strip_prefix("0x").or_else(|| s.strip_prefix("0X")) {
        if hex.bytes().all(|b| b.is_ascii_hexdigit()) {
            return match hex.len() {
                8 => Some(format!("#{}", hex[2..8].to_ascii_lowercase())),
                6 => Some(format!("#{}", hex.to_ascii_lowercase())),
                _ => None,
            };
        }
        return None;
    }

    let lower = s.to_ascii_lowercase();
    if (lower.starts_with("rgb(") || lower.starts_with("rgba(")) && s.ends_with(')') {
        let open = s.find('(')?;
        let inner = s[open + 1..s.len() - 1].trim();
        if !inner.contains(',') {
            let hex = inner.trim_start_matches('#');
            if matches!(hex.len(), 6 | 8) && hex.bytes().all(|b| b.is_ascii_hexdigit()) {
                return Some(format!("#{}", hex[..6].to_ascii_lowercase()));
            }
            return None;
        }
        return colors::parse_color(s).map(colors::Rgb::to_hex);
    }

    None
}

#[cfg(test)]
mod tests {
    use crate::render::engine::render_str;
    use crate::render::value::Value;
    use std::collections::HashMap;

    fn str_map(pairs: &[(&str, &str)]) -> HashMap<String, Value> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), Value::Str(v.to_string())))
            .collect()
    }

    #[test]
    fn mix_helpers_format_hex_strip_and_rgb() {
        let vars = str_map(&[
            ("background", "#1a1b26"),
            ("foreground", "#a9b1d6"),
            ("accent", "#7aa2f7"),
        ]);

        assert_eq!(
            render_str("{{ mix background foreground 15% }}", &vars),
            "#2f3240"
        );
        assert_eq!(
            render_str("{{ mix_strip background accent 0.35 }}", &vars),
            "3c4a6f"
        );
        assert_eq!(
            render_str("{{ mix_rgb background accent 50 }}", &vars),
            "74,95,143"
        );
    }

    #[test]
    fn mix_with_missing_or_non_hex_key_stays_raw() {
        let vars = str_map(&[("background", "#1a1b26"), ("bad", "not-a-color")]);

        assert_eq!(
            render_str("{{ mix background missing 15% }}", &vars),
            "{{ mix background missing 15% }}"
        );
        assert_eq!(
            render_str("{{ mix background bad 15% }}", &vars),
            "{{ mix background bad 15% }}"
        );
        assert_eq!(
            render_str("{{ mix_strip missing background 0.5 }}", &vars),
            "{{ mix_strip missing background 0.5 }}"
        );
    }

    #[test]
    fn gradient_helpers_handle_multi_stop_and_solid_fallbacks() {
        let vars = str_map(&[
            ("accent", "#7aa2f7"),
            ("blue", "#33ccff"),
            ("green", "#00ff99"),
            (
                "hyprland_active_border",
                "rgba(33ccffee) rgba(00ff99ee) 45deg",
            ),
            ("named_grad", "blue green -30.5deg"),
            ("no_angle_grad", "rgba(33ccffee) rgba(00ff99ee)"),
        ]);

        assert_eq!(
            render_str("{{ hypr_gradient hyprland_active_border accent }}", &vars),
            r#"{ colors = { "rgba(33ccffee)", "rgba(00ff99ee)" }, angle = 45 }"#
        );
        assert_eq!(
            render_str("{{ hypr_gradient no_angle_grad accent }}", &vars),
            r#"{ colors = { "rgba(33ccffee)", "rgba(00ff99ee)" } }"#
        );
        assert_eq!(
            render_str("{{ hypr_gradient missing_border accent }}", &vars),
            "\"#7aa2f7\""
        );
        assert_eq!(
            render_str("{{ hypr_gradient missing_border #ff00aa }}", &vars),
            "\"#ff00aa\""
        );

        assert_eq!(
            render_str("{{ shell_gradient hyprland_active_border accent }}", &vars),
            "rgba(33ccffee) rgba(00ff99ee) 45deg"
        );
        assert_eq!(
            render_str("{{ shell_gradient named_grad accent }}", &vars),
            "#33ccff #00ff99 -30.5deg"
        );
        assert_eq!(
            render_str("{{ shell_gradient missing_border accent }}", &vars),
            "#7aa2f7"
        );

        assert_eq!(
            render_str("{{ gradient_start hyprland_active_border accent }}", &vars),
            "#33ccff"
        );
        assert_eq!(
            render_str("{{ gradient_start missing_border accent }}", &vars),
            "#7aa2f7"
        );
    }

    #[test]
    fn gradient_start_accepts_all_omarchy_color_spellings() {
        let vars = str_map(&[
            ("hex6", "#33CCFF"),
            ("hex8", "#33ccffee #00ff99ee 45deg"),
            ("rgb_hex", "rgb(33ccff) 90deg"),
            ("rgba_hex", "rgba(33ccffee) rgba(00ff99ee) 45deg"),
            ("rgb_dec", "rgb(51, 204, 255) 45deg"),
            ("rgba_dec", "rgba(51,204,255,0.9) 45deg"),
            ("argb_hex", "0xee33ccff 0xee00ff99 45deg"),
        ]);

        for key in [
            "hex6", "hex8", "rgb_hex", "rgba_hex", "rgb_dec", "rgba_dec", "argb_hex",
        ] {
            assert_eq!(
                render_str(&format!("{{{{ gradient_start {key} }}}}"), &vars),
                "#33ccff",
                "failed for {key}"
            );
        }

        // Unparseable missing key without a colour fallback stays raw.
        assert_eq!(
            render_str("{{ gradient_start missing }}", &vars),
            "{{ gradient_start missing }}"
        );
    }

    #[test]
    fn gnist_mix_filter_still_works_alongside_bare_word_mix() {
        let vars = str_map(&[("accent", "#89b4fa"), ("background", "#1e1e2e")]);
        assert_eq!(
            render_str("{{ accent | mix background 0.5 }}", &vars),
            "#546994"
        );
    }
}

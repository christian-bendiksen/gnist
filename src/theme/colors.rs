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
            // Ghostty lists several `idx=color` entries on one line.
            for entry in val.split(',') {
                let Some((idx, color)) = entry.trim().split_once('=') else {
                    continue;
                };
                if let Ok(n) = idx.trim().parse::<u8>() {
                    map.insert(format!("color{n}"), normalize_color(color.trim()));
                }
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

/// A color with 0..1 component values. Alpha is modelled but Gnist's canonical
/// output is opaque `#rrggbb`, so formatters ignore it unless asked for rgba.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Rgb {
    pub r: f32,
    pub g: f32,
    pub b: f32,
    pub a: f32,
}

impl Rgb {
    pub const BLACK: Self = Self::new(0.0, 0.0, 0.0, 1.0);
    pub const WHITE: Self = Self::new(1.0, 1.0, 1.0, 1.0);

    const fn new(r: f32, g: f32, b: f32, a: f32) -> Self {
        Self { r, g, b, a }
    }

    pub fn opaque(r: f32, g: f32, b: f32) -> Self {
        Self::new(r, g, b, 1.0)
    }

    pub fn from_rgb8(r: u8, g: u8, b: u8) -> Self {
        Self::opaque(r as f32 / 255.0, g as f32 / 255.0, b as f32 / 255.0)
    }

    pub fn from_hex(hex: &str) -> Option<Self> {
        let rgb = parse_hex(hex)?;
        Some(Self::from_rgb8(rgb[0], rgb[1], rgb[2]))
    }

    pub fn to_rgb8(self) -> (u8, u8, u8) {
        let to = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
        (to(self.r), to(self.g), to(self.b))
    }

    pub fn to_hex(self) -> String {
        let (r, g, b) = self.to_rgb8();
        format!("#{r:02x}{g:02x}{b:02x}")
    }

    /// `r,g,b` comma tuple, matching the `_rgb` template form.
    pub fn to_rgb_string(self) -> String {
        let (r, g, b) = self.to_rgb8();
        format!("{r},{g},{b}")
    }

    /// CSS `rgba(r,g,b,a)` including the stored alpha.
    pub fn to_rgba_string(self) -> String {
        let (r, g, b) = self.to_rgb8();
        let a = (self.a * 10_000.0).round() / 10_000.0;
        format!("rgba({r},{g},{b},{a})")
    }

    pub fn with_alpha(mut self, a: f32) -> Self {
        self.a = a.clamp(0.0, 1.0);
        self
    }

    pub fn mix(self, other: Self, t: f32) -> Self {
        let t = t.clamp(0.0, 1.0);
        let lerp = |a: f32, b: f32| a + (b - a) * t;
        Self::new(
            lerp(self.r, other.r),
            lerp(self.g, other.g),
            lerp(self.b, other.b),
            self.a,
        )
    }

    pub fn lighten(self, amount: f32) -> Self {
        self.mix(Self::WHITE, amount)
    }

    pub fn darken(self, amount: f32) -> Self {
        self.mix(Self::BLACK, amount)
    }

    pub fn invert(self) -> Self {
        Self::new(1.0 - self.r, 1.0 - self.g, 1.0 - self.b, self.a)
    }

    /// WCAG relative luminance over linearised components.
    pub fn luminance(self) -> f32 {
        let lin = |c: f32| {
            if c <= 0.040_45 {
                c / 12.92
            } else {
                ((c + 0.055) / 1.055).powf(2.4)
            }
        };
        0.2126 * lin(self.r) + 0.7152 * lin(self.g) + 0.0722 * lin(self.b)
    }

    pub fn grayscale(self) -> Self {
        let l = self.luminance();
        Self::new(l, l, l, self.a)
    }

    /// Return black or white text chosen for the highest contrast ratio.
    pub fn contrast(self) -> Self {
        let l = self.luminance();
        let black = (l + 0.05) / 0.05;
        let white = 1.05 / (l + 0.05);
        if black >= white { Self::BLACK } else { Self::WHITE }
    }

    pub fn saturate(self, amount: f32) -> Self {
        let (h, s, l) = self.to_hsl();
        Self::from_hsl(h, (s + amount).clamp(0.0, 1.0), l).with_alpha(self.a)
    }

    pub fn adjust_hue(self, deg: f32) -> Self {
        let (h, s, l) = self.to_hsl();
        Self::from_hsl((h + deg).rem_euclid(360.0), s, l).with_alpha(self.a)
    }

    pub fn complement(self) -> Self {
        self.adjust_hue(180.0)
    }

    pub fn to_hsl(self) -> (f32, f32, f32) {
        let max = self.r.max(self.g).max(self.b);
        let min = self.r.min(self.g).min(self.b);
        let l = (max + min) / 2.0;
        let d = max - min;
        let s = if d == 0.0 {
            0.0
        } else {
            d / (1.0 - (2.0 * l - 1.0).abs())
        };
        let h = if d == 0.0 {
            0.0
        } else if max == self.r {
            let mut h = (self.g - self.b) / d;
            if self.g < self.b {
                h += 6.0;
            }
            h * 60.0
        } else if max == self.g {
            ((self.b - self.r) / d + 2.0) * 60.0
        } else {
            ((self.r - self.g) / d + 4.0) * 60.0
        };
        (h, s, l)
    }

    pub fn from_hsl(h: f32, s: f32, l: f32) -> Self {
        let h = (h.rem_euclid(360.0)) / 360.0;
        let (s, l) = (s.clamp(0.0, 1.0), l.clamp(0.0, 1.0));
        let f = |n: f32| {
            let k = (n + h * 12.0) % 12.0;
            let a = s * l.min(1.0 - l);
            l - a * (-1.0_f32).max((k - 3.0).min(9.0 - k).min(1.0))
        };
        Self::opaque(f(0.0), f(8.0), f(4.0))
    }

    /// CIE OKLab components in the 0..1 lightness range.
    pub fn to_oklab(self) -> (f32, f32, f32) {
        let lin = |c: f32| {
            if c <= 0.040_45 {
                c / 12.92
            } else {
                ((c + 0.055) / 1.055).powf(2.4)
            }
        };
        let (r, g, b) = (lin(self.r), lin(self.g), lin(self.b));
        let l = 0.412_221_470_8 * r + 0.536_332_536_3 * g + 0.051_445_992_9 * b;
        let m = 0.211_903_498_2 * r + 0.680_699_545_1 * g + 0.107_396_956_6 * b;
        let s = 0.088_302_461_9 * r + 0.281_718_837_6 * g + 0.629_978_700_5 * b;
        let (l, m, s) = (l.cbrt(), m.cbrt(), s.cbrt());
        (
            0.210_454_255_3 * l + 0.793_617_785_0 * m - 0.004_072_046_8 * s,
            1.977_998_495_1 * l - 2.428_592_205_0 * m + 0.450_593_709_9 * s,
            0.025_904_037_1 * l + 0.782_771_766_2 * m - 0.808_675_766_0 * s,
        )
    }
}

/// Parse any supported color spelling into a structured color.
///
/// Accepted forms:
/// - `#rgb`, `#rgba`, `#rrggbb`, `#rrggbbaa` (shorthand is expanded)
/// - `rgb(r,g,b)` / `rgba(r,g,b,a)`, with channels as 0-255 integers or 0..1 floats
/// - COSMIC namespaced forms: `rgba.hex(...)`, `rgba.rgb(...)`, `rgba.linear_rgb(...)`,
///   `rgba.hsl(...)`, `rgba.addressable(...)`
/// - bare hex digits, e.g. the `cdd6f4` values Glow/Kitties-style files use
pub fn parse_color(s: &str) -> Option<Rgb> {
    let s = s.trim().trim_matches('"').trim_matches('\'').trim();
    if s.starts_with('#') {
        return Rgb::from_hex(s);
    }
    if let Some((kind, args)) = split_function(s)
        && let Some(c) = functional_color(kind, args)
    {
        return Some(c);
    }
    Rgb::from_hex(s)
}

/// Parse a color into canonical opaque `#rrggbb`. Alpha is dropped.
pub fn normalize_color(s: &str) -> String {
    let s = s.trim().trim_matches('"').trim_matches('\'').trim();
    if s.is_empty() {
        return s.to_owned();
    }
    match parse_color(s) {
        Some(c) => c.to_hex(),
        None => format!("#{s}"),
    }
}

/// Parse `name(args)` into its function name and inner argument list.
fn split_function(s: &str) -> Option<(&str, &str)> {
    let open = s.find('(')?;
    let name = &s[..open];
    if name.is_empty() || name.contains([' ', '\t']) {
        return None;
    }
    let inner = s.get(open + 1..)?.strip_suffix(')')?;
    Some((name, inner))
}

/// Convert the argument list of a functional color form to a color.
fn functional_color(kind: &str, args: &str) -> Option<Rgb> {
    let args = args.trim();
    match kind.strip_prefix("rgba.").unwrap_or(kind) {
        "hex" => Rgb::from_hex(args),
        "rgb" | "rgba" => {
            let c = parse_channels(args)?;
            let unit = args.contains('.') && c.iter().all(|&v| v <= 1.0);
            Some(channels_to_rgb(&c, unit, parse_alpha(args)))
        }
        "linear_rgb" => {
            let c = parse_channels(args)?;
            let srgb = c.map(linear_to_srgb);
            Some(channels_to_rgb(&srgb, true, 1.0))
        }
        "hsl" => {
            let mut channels = args.split(',').map(|p| p.trim().parse::<f64>().ok());
            let h = channels.next()??;
            let s = channels.next()??;
            let l = channels.next()??;
            Some(Rgb::from_hsl(h as f32, s as f32, l as f32))
        }
        "addressable" => {
            let c = parse_channels(args)?;
            Some(channels_to_rgb(&c, false, parse_alpha(args)))
        }
        _ => None,
    }
}

/// Split a comma-separated channel list into three floats.
fn parse_channels(s: &str) -> Option<[f64; 3]> {
    let mut channels = s.split(',').map(|p| p.trim().parse::<f64>().ok());
    let c = [channels.next()??, channels.next()??, channels.next()??];
    Some(c)
}

/// Fourth channel of `rgba(...)` as a 0..1 alpha, if present.
fn parse_alpha(args: &str) -> f32 {
    args.split(',')
        .nth(3)
        .and_then(|p| p.trim().parse::<f64>().ok())
        .map(|a| if a <= 1.0 { a as f32 } else { a as f32 / 255.0 })
        .unwrap_or(1.0)
        .clamp(0.0, 1.0)
}

/// Convert three channels that are either 0-255 integers or 0..1 unit floats.
fn channels_to_rgb(c: &[f64; 3], unit: bool, alpha: f32) -> Rgb {
    let to = |v: f64| -> f32 {
        let v = if unit { v * 255.0 } else { v };
        (v.round().clamp(0.0, 255.0) / 255.0) as f32
    };
    Rgb {
        r: to(c[0]),
        g: to(c[1]),
        b: to(c[2]),
        a: alpha,
    }
}

fn linear_to_srgb(v: f64) -> f64 {
    if v <= 0.003_130_8 {
        12.92 * v
    } else {
        1.055 * v.powf(1.0 / 2.4) - 0.055
    }
}

/// Parse `#rgb`, `#rgba`, `#rrggbb`, or `#rrggbbaa` (with or without `#`) and
/// return the RGB channels. Shorthand is expanded and alpha is dropped.
pub fn parse_hex(hex: &str) -> Option<[u8; 3]> {
    let hex = hex.trim().trim_matches('"').trim_matches('\'').trim();
    let hex = hex.trim_start_matches('#');
    if hex.is_empty() || !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    let digits = hex.as_bytes();

    let nibble = |i: usize| -> Option<u8> {
        let c = *digits.get(i)?;
        Some(match c {
            b'0'..=b'9' => c - b'0',
            b'a'..=b'f' => c - b'a' + 10,
            b'A'..=b'F' => c - b'A' + 10,
            _ => return None,
        })
    };
    // `#a` shorthand: each nibble doubles into a full byte.
    let doubled = |i: usize| -> Option<u8> {
        let n = nibble(i)?;
        Some(n << 4 | n)
    };
    let pair = |i: usize| -> Option<u8> {
        let hi = nibble(i)?;
        let lo = nibble(i + 1)?;
        Some(hi << 4 | lo)
    };

    match hex.len() {
        3 | 4 => Some([doubled(0)?, doubled(1)?, doubled(2)?]),
        6 | 8 => Some([pair(0)?, pair(2)?, pair(4)?]),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::{normalize_color, parse_ghostty, parse_hex};

    #[test]
    fn hex_is_preserved_and_lowercased() {
        assert_eq!(normalize_color("#CDD6F4"), "#cdd6f4");
        assert_eq!(normalize_color("#cdd6f4"), "#cdd6f4");
    }

    #[test]
    fn hex_shorthand_is_expanded() {
        assert_eq!(normalize_color("#abc"), "#aabbcc");
        assert_eq!(normalize_color("#ABCD"), "#aabbcc");
    }

    #[test]
    fn hex_alpha_is_dropped() {
        assert_eq!(normalize_color("#89b4faff"), "#89b4fa");
        assert_eq!(normalize_color("#abcd"), "#aabbcc");
    }

    #[test]
    fn bare_hex_is_prefixed() {
        assert_eq!(normalize_color("cdd6f4"), "#cdd6f4");
        assert_eq!(parse_hex("89B4FA"), Some([0x89, 0xb4, 0xfa]));
    }

    #[test]
    fn rgb_and_rgba_integers() {
        assert_eq!(normalize_color("rgb(206, 215, 216)"), "#ced7d8");
        assert_eq!(normalize_color("rgba(206, 215, 216, 0.5)"), "#ced7d8");
    }

    #[test]
    fn rgb_unit_floats() {
        assert_eq!(normalize_color("rgb(0.349, 0.706, 0.961)"), "#59b4f5");
        assert_eq!(normalize_color("rgba(0.5, 0.5, 0.5, 0.5)"), "#808080");
    }

    #[test]
    fn cosmic_namespaced_forms() {
        assert_eq!(normalize_color("rgba.hex(89b4fa)"), "#89b4fa");
        assert_eq!(normalize_color("rgba.hex('89B4FA')"), "#89b4fa");
        assert_eq!(normalize_color("rgba.rgb(137, 180, 250)"), "#89b4fa");
        assert_eq!(normalize_color("rgba.addressable(137, 180, 250, 255)"), "#89b4fa");
    }

    #[test]
    fn cosmic_linear_rgb_applies_srgb_decode() {
        assert_eq!(normalize_color("rgba.linear_rgb(1.0, 1.0, 1.0)"), "#ffffff");
        assert_eq!(normalize_color("rgba.linear_rgb(0.0, 0.0, 0.0)"), "#000000");
        assert_eq!(normalize_color("rgba.linear_rgb(0.5, 0.5, 0.5)"), "#bcbcbc");
    }

    #[test]
    fn cosmic_hsl_forms() {
        assert_eq!(normalize_color("rgba.hsl(0, 1, 0.5)"), "#ff0000");
        assert_eq!(normalize_color("rgba.hsl(120, 1, 0.5)"), "#00ff00");
        assert_eq!(normalize_color("rgba.hsl(0, 0, 0)"), "#000000");
        assert_eq!(normalize_color("rgba.hsl(0, 0, 1)"), "#ffffff");
    }

    #[test]
    fn named_values_pass_through() {
        assert_eq!(normalize_color("orange"), "#orange");
    }

    #[test]
    fn ghostty_palette_entries_on_one_line() {
        let conf = "palette = 0=#45475a,1=#f38ba8,2=#a6e3a1\n";
        let map = parse_ghostty(conf);
        assert_eq!(map["color0"], "#45475a");
        assert_eq!(map["color1"], "#f38ba8");
        assert_eq!(map["color2"], "#a6e3a1");
    }

    #[test]
    fn ghostty_palette_entries_on_separate_lines() {
        let conf = "palette = 0=#45475a\npalette = 2=#a6e3a1\n";
        let map = parse_ghostty(conf);
        assert_eq!(map["color0"], "#45475a");
        assert_eq!(map["color2"], "#a6e3a1");
    }
}

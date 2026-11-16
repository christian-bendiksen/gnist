//! Active Gnist theme resolution and `colors.toml` palette derivation.
//!
//! Resolves the active Gnist theme directory (honouring `$GNIST_THEME_DIR`,
//! `$XDG_CONFIG_HOME/gnist/themes/current.theme`,
//! `$XDG_DATA_HOME/gnist/themes/current/colors.toml`, `$HOME/.local/share/...`,
//! `/etc/gnist/themes/current/colors.toml`, and `SUDO_USER` / `PKEXEC_UID`
//! privilege-escalation fallback so `sudo pkg` / `sudo malm` still follow the
//! logged-in person's theme), plus `$XDG_RUNTIME_DIR/herda/room` when
//! `look = "lysrom"`.

use std::collections::BTreeMap;
use std::ffi::OsStr;
use std::path::{Path, PathBuf};

use crate::ink::{Palette, Rgb};

/// Resolved palette for the active Gnist theme, falling back to the built-in
/// `herda-lysrom` Night palette when no theme file is found.
#[must_use]
pub fn detect_palette() -> Palette {
    let mut is_lysrom = true;
    let parsed = find_colors_toml().map(|text| {
        let doc = parse_toml_doc(&text);
        is_lysrom = doc.look.as_deref() == Some("lysrom");
        doc
    });
    let room = if is_lysrom { detect_room() } else { None };
    match parsed {
        Some(doc) => palette_from_doc(&doc, room).unwrap_or(Palette::LYSROM_NIGHT),
        None => Palette::lysrom_night_with_room(room),
    }
}

/// Parse a `colors.toml` document (plus optional Lysrom room `1..=8`) into a
/// [`Palette`]. Returns `None` when the document carries no colour entries.
#[must_use]
pub fn parse_colors_toml(text: &str, room: Option<u8>) -> Option<Palette> {
    let doc = parse_toml_doc(text);
    palette_from_doc(&doc, room)
}

struct ParsedToml {
    look: Option<String>,
    colors: BTreeMap<String, Rgb>,
}

fn parse_toml_doc(text: &str) -> ParsedToml {
    let mut look = None;
    let mut colors = BTreeMap::new();
    let mut section = String::new();

    for raw_line in text.lines() {
        let line = strip_comment(raw_line).trim();
        if line.is_empty() {
            continue;
        }
        if let Some(header) = line.strip_prefix('[').and_then(|s| s.strip_suffix(']')) {
            let header = header.trim();
            if !header.is_empty() && !header.starts_with('[') {
                section = header.replace('.', "_");
            }
            continue;
        }
        let Some((raw_key, raw_value)) = line.split_once('=') else {
            continue;
        };
        let key = raw_key.trim().trim_matches('"').trim_matches('\'');
        if key.is_empty() {
            continue;
        }
        let full_key = if section.is_empty() {
            key.to_owned()
        } else {
            format!("{section}_{key}")
        };
        let value = raw_value.trim();
        let Some(unquoted) = unquote(value) else {
            continue;
        };
        if full_key == "look" {
            look = Some(unquoted.to_owned());
        } else if let Some(rgb) = Rgb::parse_hex(unquoted) {
            colors.insert(full_key, rgb);
        }
    }

    ParsedToml { look, colors }
}

fn strip_comment(line: &str) -> &str {
    let mut in_double = false;
    let mut in_single = false;
    let mut escape = false;
    for (idx, ch) in line.char_indices() {
        if escape {
            escape = false;
            continue;
        }
        if in_double {
            match ch {
                '\\' => escape = true,
                '"' => in_double = false,
                _ => {}
            }
        } else if in_single {
            if ch == '\'' {
                in_single = false;
            }
        } else {
            match ch {
                '"' => in_double = true,
                '\'' => in_single = true,
                '#' => return &line[..idx],
                _ => {}
            }
        }
    }
    line
}

fn unquote(value: &str) -> Option<&str> {
    if value.len() >= 2
        && ((value.starts_with('"') && value.ends_with('"'))
            || (value.starts_with('\'') && value.ends_with('\'')))
    {
        Some(&value[1..value.len() - 1])
    } else {
        None
    }
}

fn palette_from_doc(doc: &ParsedToml, room: Option<u8>) -> Option<Palette> {
    if doc.colors.is_empty() {
        return None;
    }
    let get = |keys: &[&str]| keys.iter().find_map(|k| doc.colors.get(*k).copied());

    let is_lysrom = doc.look.as_deref() == Some("lysrom");
    let background = get(&["background", "lysrom_wall"]).unwrap_or(Rgb::new(5, 5, 10));
    let light = background.is_light();
    let fallback = if light {
        Palette::LYSROM_DAY
    } else {
        Palette::LYSROM_NIGHT
    };

    let text = get(&["foreground", "lysrom_ink"]).unwrap_or(fallback.text);
    let muted = get(&["muted", "comment"]).unwrap_or(fallback.muted);
    let faint = get(&["faint", "color8"]).unwrap_or(fallback.faint);
    let rule = if is_lysrom {
        get(&["border_strong", "hair", "border_inactive"]).unwrap_or(fallback.rule)
    } else {
        get(&["border_inactive", "border_strong", "hair"]).unwrap_or(fallback.rule)
    };

    let base_accent = get(&["accent", "border_active", "brand_herda"]).unwrap_or(fallback.gold);
    let room_role = if is_lysrom {
        room.filter(|r| (1..=8).contains(r)).and_then(|r| {
            let k1 = format!("role{r}");
            let k2 = format!("role{r}_ink");
            get(&[&k1, &k2])
        })
    } else {
        None
    };
    let gold = room_role.unwrap_or(base_accent);
    let room_overrode_accent = room_role.is_some_and(|r| r != base_accent);

    let green = get(&["success", "color2"]).unwrap_or(fallback.green);
    let danger = if is_lysrom {
        get(&["lysrom_danger", "error", "color1", "color9"]).unwrap_or(fallback.danger)
    } else {
        get(&["error", "color9", "color1"]).unwrap_or(fallback.danger)
    };
    let blue = get(&["color4", "link"]).unwrap_or(fallback.blue);
    let magenta = get(&["color5"]).unwrap_or(fallback.magenta);
    let cyan = get(&["color6", "info"]).unwrap_or(fallback.cyan);

    let plate = get(&["plate", "surface", "surface_low"])
        .unwrap_or_else(|| background.blend(text, if light { 0.05 } else { 0.07 }));
    let selected_fg = get(&["selection_foreground"]).unwrap_or(text);
    let selected_bg = if room_overrode_accent {
        plate.blend(gold, if light { 0.20 } else { 0.26 })
    } else {
        get(&["selection_background"])
            .unwrap_or_else(|| plate.blend(gold, if light { 0.20 } else { 0.26 }))
    };

    let wash_plate = plate;
    let wash_key = plate.blend(text, if light { 0.08 } else { 0.09 });
    let wash_accent = plate.blend(gold, if light { 0.16 } else { 0.18 });
    let wash_cursor = plate.blend(gold, if light { 0.08 } else { 0.09 });
    let wash_alarm = plate.blend(danger, if light { 0.12 } else { 0.14 });

    Some(Palette {
        text,
        muted,
        faint,
        rule,
        gold,
        green,
        danger,
        blue,
        magenta,
        cyan,
        selected_fg,
        selected_bg,
        wash_plate,
        wash_key,
        wash_accent,
        wash_cursor,
        wash_alarm,
    })
}

fn find_colors_toml() -> Option<String> {
    if let Some(dir) = non_empty_path(std::env::var_os("GNIST_THEME_DIR").as_deref()) {
        let candidate = dir.join("colors.toml");
        if let Ok(text) = std::fs::read_to_string(&candidate) {
            return Some(text);
        }
    }

    let data_dirs = xdg_data_dirs();
    let user_dirs = collect_user_xdg_dirs();

    for (config_home, data_home) in &user_dirs {
        if let Some(name) = read_theme_name(&config_home.join("gnist/themes/current.theme"))
            && let Some(text) = find_named_theme(&name, config_home, data_home, &data_dirs)
        {
            return Some(text);
        }
        for current in [
            config_home.join("gnist/themes/current/colors.toml"),
            data_home.join("gnist/themes/current/colors.toml"),
        ] {
            if let Ok(text) = std::fs::read_to_string(&current) {
                return Some(text);
            }
        }
    }

    for fallback_path in [
        Path::new("/etc/gnist/themes/current/colors.toml"),
        Path::new("/usr/share/gnist/themes/herda-lysrom/colors.toml"),
        Path::new("/usr/share/gnist/themes/data/herda-lysrom/colors.toml"),
    ] {
        if let Ok(text) = std::fs::read_to_string(fallback_path) {
            return Some(text);
        }
    }

    None
}

fn find_named_theme(
    name: &str,
    config_home: &Path,
    data_home: &Path,
    data_dirs: &[PathBuf],
) -> Option<String> {
    let mut roots = vec![
        config_home.join("gnist/themes/data"),
        data_home.join("gnist/themes/data"),
        data_home.join("gnist/themes"),
    ];
    for dir in data_dirs {
        roots.push(dir.join("gnist/themes/data"));
        roots.push(dir.join("gnist/themes"));
    }
    roots.push(PathBuf::from("/etc/gnist/themes/data"));
    roots.push(PathBuf::from("/etc/gnist/themes"));

    for root in roots {
        let candidate = root.join(name).join("colors.toml");
        if let Ok(text) = std::fs::read_to_string(&candidate) {
            return Some(text);
        }
    }
    None
}

fn read_theme_name(path: &Path) -> Option<String> {
    let text = std::fs::read_to_string(path).ok()?;
    let name = text.trim();
    if name.is_empty() || name == "." || name == ".." || name.contains(['/', '\0', '\n']) {
        return None;
    }
    Some(name.to_owned())
}

fn detect_room() -> Option<u8> {
    if let Some(runtime_dir) = non_empty_path(std::env::var_os("XDG_RUNTIME_DIR").as_deref())
        .filter(|path| path.is_absolute())
        && let Some(room) = read_room_file(&runtime_dir.join("herda/room"))
    {
        return Some(room);
    }
    if let Some((_, Some(uid))) = escalated_user_home_and_uid() {
        let path = PathBuf::from(format!("/run/user/{uid}/herda/room"));
        if let Some(room) = read_room_file(&path) {
            return Some(room);
        }
    }
    None
}

fn read_room_file(path: &Path) -> Option<u8> {
    let bytes = std::fs::read(path).ok()?;
    match bytes.trim_ascii() {
        [digit @ b'1'..=b'8'] => Some(digit - b'0'),
        _ => None,
    }
}

fn non_empty_path(value: Option<&OsStr>) -> Option<PathBuf> {
    value
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
}

fn xdg_data_dirs() -> Vec<PathBuf> {
    let raw = std::env::var("XDG_DATA_DIRS")
        .ok()
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| "/usr/local/share:/usr/share".to_owned());
    raw.split(':')
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .collect()
}

fn collect_user_xdg_dirs() -> Vec<(PathBuf, PathBuf)> {
    let mut dirs = Vec::new();
    let home = non_empty_path(std::env::var_os("HOME").as_deref());
    let config_home = non_empty_path(std::env::var_os("XDG_CONFIG_HOME").as_deref())
        .or_else(|| home.as_ref().map(|h| h.join(".config")));
    let data_home = non_empty_path(std::env::var_os("XDG_DATA_HOME").as_deref())
        .or_else(|| home.as_ref().map(|h| h.join(".local/share")));
    if let (Some(c), Some(d)) = (config_home, data_home) {
        dirs.push((c, d));
    }
    if let Some((user_home, _)) = escalated_user_home_and_uid() {
        let pair = (user_home.join(".config"), user_home.join(".local/share"));
        if !dirs.contains(&pair) {
            dirs.push(pair);
        }
    }
    dirs
}

fn escalated_user_home_and_uid() -> Option<(PathBuf, Option<u32>)> {
    let pkexec_uid = std::env::var("PKEXEC_UID")
        .ok()
        .and_then(|s| s.parse::<u32>().ok());
    let sudo_uid = std::env::var("SUDO_UID")
        .ok()
        .and_then(|s| s.parse::<u32>().ok());
    let sudo_user = std::env::var("SUDO_USER").ok().filter(|s| !s.is_empty());
    let target_uid = pkexec_uid.or(sudo_uid);
    if target_uid.is_none() && sudo_user.is_none() {
        return None;
    }
    let passwd = std::fs::read_to_string("/etc/passwd").ok()?;
    parse_passwd_user(&passwd, target_uid, sudo_user.as_deref())
}

pub(crate) fn parse_passwd_user(
    passwd: &str,
    target_uid: Option<u32>,
    target_name: Option<&str>,
) -> Option<(PathBuf, Option<u32>)> {
    for line in passwd.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let mut fields = line.split(':');
        let Some(name) = fields.next() else {
            continue;
        };
        let _pw = fields.next()?;
        let Some(uid) = fields.next().and_then(|s| s.parse::<u32>().ok()) else {
            continue;
        };
        let _gid = fields.next()?;
        let _gecos = fields.next()?;
        let Some(home) = fields.next() else {
            continue;
        };
        let matches =
            target_uid.is_some_and(|u| u == uid) || target_name.is_some_and(|n| n == name);
        if matches && Path::new(home).is_absolute() {
            return Some((PathBuf::from(home), Some(uid)));
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cells::{Buffer, Style};
    use crate::document::Document;
    use crate::ink::{Ink, Wash};
    use crate::terminal::Theme;

    const LYSROM_NIGHT_TOML: &str = r##"
look = "lysrom"
accent = "#FF774D"
foreground = "#F3F2EF"
background = "#05050A"
plate = "#17171D"
selection_foreground = "#F3F2EF"
selection_background = "#5D302B"
color4 = "#8DB8F0"
color5 = "#C99BE8"
color6 = "#7FCFE0"
border_strong = "#3E3E43"
faint = "#7A7A7C"
muted = "#ADACAC"
success = "#7CD595"
error = "#F2365A"
lysrom_danger = "#F2365A"
role1 = "#4AA8FF"
role2 = "#3FD69A"
role3 = "#8F7BFF"
role4 = "#FF774D"
role5 = "#FFC24D"
role6 = "#C9E04A"
role7 = "#3FD0E0"
role8 = "#FF7ACC"
"##;

    const LYSROM_DAY_TOML: &str = r##"
look = "lysrom"
accent = "#E36A47"
foreground = "#18171B"
background = "#F4F2ED"
plate = "#FFFFFF"
selection_foreground = "#18171B"
selection_background = "#FFD3CA"
color4 = "#1F5FB8"
color5 = "#7B3FA6"
color6 = "#00697A"
border_strong = "#D2D1CF"
faint = "#8C8B8D"
muted = "#5D5D5F"
success = "#166236"
error = "#CF1F45"
lysrom_danger = "#CF1F45"
role1 = "#3B90EC"
role2 = "#22A16F"
role3 = "#8C7DEB"
role4 = "#E36A47"
role5 = "#C38000"
role6 = "#83961A"
role7 = "#1B9CAA"
role8 = "#D468B1"
"##;

    const HERDA_DARK_TOML: &str = r##"
accent = "#FFBC50"
foreground = "#ECECE9"
background = "#1E2024"
plate = "#16181B"
selection_foreground = "#ECECE9"
selection_background = "#33465F"
color4 = "#8DB8F0"
color5 = "#C99BE8"
color6 = "#7FCFE0"
border_strong = "#888D8F"
border_inactive = "#3D4044"
faint = "#979CA1"
muted = "#B8BCC0"
success = "#7CD595"
error = "#FF9F98"
"##;

    const HERDA_LIGHT_TOML: &str = r##"
accent = "#785000"
foreground = "#1A1C1E"
background = "#DCE1E6"
plate = "#FBFBFA"
selection_foreground = "#1A1C1E"
selection_background = "#CCDCF3"
color4 = "#1F5FB8"
color5 = "#7B3FA6"
color6 = "#00697A"
border_strong = "#7C8183"
border_inactive = "#C8CDCF"
faint = "#62676C"
muted = "#3C4045"
success = "#166236"
error = "#A3221B"
"##;

    fn assert_distinct_palette(palette: Palette, label: &str) {
        let inks = [
            Ink::Text,
            Ink::Muted,
            Ink::Faint,
            Ink::Rule,
            Ink::Gold,
            Ink::Green,
            Ink::Danger,
            Ink::Blue,
            Ink::Magenta,
            Ink::Cyan,
            Ink::Selected,
        ];
        let washes = [
            Wash::Plate,
            Wash::Key,
            Wash::Accent,
            Wash::Cursor,
            Wash::Alarm,
        ];
        for truecolor in [true, false] {
            let mut seen_inks = BTreeMap::new();
            for ink in inks {
                let code = palette.ink_sgr(ink, truecolor);
                if let Some(other) = seen_inks.insert(code.clone(), ink) {
                    panic!(
                        "{label}: {ink:?} and {other:?} both emit {code} (truecolor={truecolor})"
                    );
                }
            }
            let mut seen_washes = BTreeMap::new();
            for wash in washes {
                let code = palette.wash_sgr(wash, truecolor);
                assert!(code.starts_with("48;"), "{label}: {wash:?} -> {code}");
                if let Some(other) = seen_washes.insert(code.clone(), wash) {
                    panic!(
                        "{label}: {wash:?} and {other:?} both emit {code} (truecolor={truecolor})"
                    );
                }
            }
        }
    }

    #[test]
    fn parses_lysrom_night_day_and_herda_themes_with_exact_sgr_codes() {
        let night = Theme::from_colors_toml(LYSROM_NIGHT_TOML, None, true).expect("lysrom night");
        assert!(
            night
                .paint("pkg", Ink::Gold, true)
                .contains("38;2;255;119;77")
        );
        assert!(
            night
                .paint("txt", Ink::Text, false)
                .contains("38;2;243;242;239")
        );
        assert!(
            night
                .paint("err", Ink::Danger, false)
                .contains("38;2;242;54;90")
        );
        assert!(
            night
                .paint("ok", Ink::Green, false)
                .contains("38;2;124;213;149")
        );
        assert!(night.rule(4).contains("38;2;62;62;67"));
        assert_eq!(night.palette.wash_sgr(Wash::Plate, true), "48;2;23;23;29");
        assert_distinct_palette(night.palette, "lysrom-night");

        let night_room1 =
            Theme::from_colors_toml(LYSROM_NIGHT_TOML, Some(1), true).expect("room 1");
        assert!(
            night_room1
                .paint("pkg", Ink::Gold, true)
                .contains("38;2;74;168;255")
        );
        assert_distinct_palette(night_room1.palette, "lysrom-night-room1");

        let day = Theme::from_colors_toml(LYSROM_DAY_TOML, None, true).expect("lysrom day");
        assert!(
            day.paint("pkg", Ink::Gold, true)
                .contains("38;2;227;106;71")
        );
        assert!(day.paint("txt", Ink::Text, false).contains("38;2;24;23;27"));
        assert!(
            day.paint("err", Ink::Danger, false)
                .contains("38;2;207;31;69")
        );
        assert!(day.paint("ok", Ink::Green, false).contains("38;2;22;98;54"));
        assert!(day.rule(4).contains("38;2;210;209;207"));
        assert_eq!(day.palette.wash_sgr(Wash::Plate, true), "48;2;255;255;255");
        assert_distinct_palette(day.palette, "lysrom-day");

        let day_room3 =
            Theme::from_colors_toml(LYSROM_DAY_TOML, Some(3), true).expect("day room 3");
        assert!(
            day_room3
                .paint("pkg", Ink::Gold, true)
                .contains("38;2;140;125;235")
        );
        assert_distinct_palette(day_room3.palette, "lysrom-day-room3");

        let herda = Theme::from_colors_toml(HERDA_DARK_TOML, Some(1), true).expect("herda dark");
        // Non-lysrom themes ignore room override and keep their own accent.
        assert!(
            herda
                .paint("pkg", Ink::Gold, true)
                .contains("38;2;255;188;80")
        );
        assert_distinct_palette(herda.palette, "herda-dark");

        let herda_light =
            Theme::from_colors_toml(HERDA_LIGHT_TOML, None, true).expect("herda light");
        assert!(
            herda_light
                .paint("pkg", Ink::Gold, true)
                .contains("38;2;120;80;0")
        );
        assert_distinct_palette(herda_light.palette, "herda-light");
    }

    #[test]
    fn buffer_and_document_emit_lysrom_colors_from_theme() {
        let day = Theme::from_colors_toml(LYSROM_DAY_TOML, None, true).expect("lysrom day");
        let mut doc = Document::new("pkg", "Update", "REVIEW", 80, day);
        doc.text("ok", Ink::Green);
        let rendered = doc.render();
        assert!(rendered.contains("38;2;227;106;71"), "{rendered}");
        assert!(rendered.contains("38;2;24;23;27"), "{rendered}");
        assert!(rendered.contains("38;2;22;98;54"), "{rendered}");

        let mut buf = Buffer::new(10, 1);
        buf.wash(0, 0, 10, 1, Wash::Plate);
        buf.put(0, 0, "hi", Style::new(Ink::Gold).bold());
        let painted = buf.paint(None, day);
        assert!(
            painted.contains("38;2;227;106;71;48;2;255;255;255"),
            "{painted}"
        );
    }

    #[test]
    fn detects_gnist_theme_from_env_and_room() {
        fn with(vars: &[(&str, Option<&str>)], body: impl FnOnce()) {
            let saved: Vec<_> = vars
                .iter()
                .map(|(k, _)| (*k, std::env::var_os(k)))
                .collect();
            for (key, value) in vars {
                match value {
                    Some(value) => unsafe { std::env::set_var(key, value) },
                    None => unsafe { std::env::remove_var(key) },
                }
            }
            body();
            for (key, value) in saved {
                match value {
                    Some(value) => unsafe { std::env::set_var(key, value) },
                    None => unsafe { std::env::remove_var(key) },
                }
            }
        }

        let _guard = crate::terminal::ENV
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let tmp = std::env::temp_dir().join(format!("herda-ui-gnist-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        let theme_dir = tmp.join("direct-theme");
        let xdg_config = tmp.join("config");
        let xdg_data = tmp.join("data");
        let xdg_runtime = tmp.join("runtime");
        std::fs::create_dir_all(&theme_dir).unwrap();
        std::fs::create_dir_all(xdg_config.join("gnist/themes/data/herda-lysrom-light")).unwrap();
        std::fs::create_dir_all(xdg_data.join("gnist/themes/current")).unwrap();
        std::fs::create_dir_all(xdg_runtime.join("herda")).unwrap();

        std::fs::write(theme_dir.join("colors.toml"), HERDA_DARK_TOML).unwrap();
        std::fs::write(
            xdg_config.join("gnist/themes/current.theme"),
            "herda-lysrom-light\n",
        )
        .unwrap();
        std::fs::write(
            xdg_config.join("gnist/themes/data/herda-lysrom-light/colors.toml"),
            LYSROM_DAY_TOML,
        )
        .unwrap();
        std::fs::write(
            xdg_data.join("gnist/themes/current/colors.toml"),
            LYSROM_NIGHT_TOML,
        )
        .unwrap();
        std::fs::write(xdg_runtime.join("herda/room"), "2\n").unwrap();

        // 1. GNIST_THEME_DIR wins.
        with(
            &[
                ("NO_COLOR", None),
                ("FORCE_COLOR", Some("1")),
                ("COLORTERM", Some("truecolor")),
                ("GNIST_THEME_DIR", Some(theme_dir.to_str().unwrap())),
                ("XDG_CONFIG_HOME", Some(xdg_config.to_str().unwrap())),
                ("XDG_DATA_HOME", Some(xdg_data.to_str().unwrap())),
                ("XDG_RUNTIME_DIR", Some(xdg_runtime.to_str().unwrap())),
            ],
            || {
                let t = Theme::detect();
                assert_eq!(t.palette.gold, Rgb::new(0xFF, 0xBC, 0x50));
            },
        );

        // 2. XDG_CONFIG_HOME/gnist/themes/current.theme + room 2 wins when GNIST_THEME_DIR is unset.
        with(
            &[
                ("NO_COLOR", None),
                ("FORCE_COLOR", Some("1")),
                ("COLORTERM", Some("truecolor")),
                ("GNIST_THEME_DIR", None),
                ("XDG_CONFIG_HOME", Some(xdg_config.to_str().unwrap())),
                ("XDG_DATA_HOME", Some(xdg_data.to_str().unwrap())),
                ("XDG_RUNTIME_DIR", Some(xdg_runtime.to_str().unwrap())),
            ],
            || {
                let t = Theme::detect();
                assert_eq!(t.palette.text, Rgb::new(0x18, 0x17, 0x1B));
                assert_eq!(t.palette.gold, Rgb::new(0x22, 0xA1, 0x6F));
            },
        );

        // 3. XDG_DATA_HOME/gnist/themes/current/colors.toml works when current.theme is absent.
        std::fs::remove_file(xdg_config.join("gnist/themes/current.theme")).unwrap();
        with(
            &[
                ("NO_COLOR", None),
                ("FORCE_COLOR", Some("1")),
                ("COLORTERM", Some("truecolor")),
                ("GNIST_THEME_DIR", None),
                ("XDG_CONFIG_HOME", Some(xdg_config.to_str().unwrap())),
                ("XDG_DATA_HOME", Some(xdg_data.to_str().unwrap())),
                ("XDG_RUNTIME_DIR", Some(xdg_runtime.to_str().unwrap())),
            ],
            || {
                let t = Theme::detect();
                assert_eq!(t.palette.text, Rgb::new(0xF3, 0xF2, 0xEF));
                assert_eq!(t.palette.gold, Rgb::new(0x3F, 0xD6, 0x9A));
            },
        );

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn parses_passwd_for_sudo_and_pkexec_users() {
        let passwd = "root:x:0:0:root:/root:/bin/bash\nchristian:x:1000:1000:Christian:/home/christian:/bin/zsh\n";
        assert_eq!(
            parse_passwd_user(passwd, Some(1000), None),
            Some((PathBuf::from("/home/christian"), Some(1000)))
        );
        assert_eq!(
            parse_passwd_user(passwd, None, Some("christian")),
            Some((PathBuf::from("/home/christian"), Some(1000)))
        );
        assert_eq!(parse_passwd_user(passwd, Some(9999), None), None);
    }
}

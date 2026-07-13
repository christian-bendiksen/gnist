use std::{fs, path::Path};

#[derive(Clone, Copy)]
enum Wm {
    Hyprland,
    Niri,
    Mango,
}

const ALL: &[Wm] = &[Wm::Hyprland, Wm::Niri, Wm::Mango];

#[derive(Default)]
pub struct BorderColors {
    pub active: Option<String>,
    pub inactive: Option<String>,
}

impl BorderColors {
    fn is_empty(&self) -> bool {
        self.active.is_none() && self.inactive.is_none()
    }
}

impl Wm {
    fn filename(self) -> &'static str {
        match self {
            Self::Hyprland => "hyprland.conf",
            Self::Niri => "niri-colors.kdl",
            Self::Mango => "mango.conf",
        }
    }

    fn parse_border(self, conf: &str) -> BorderColors {
        match self {
            Self::Hyprland => parse_hyprland(conf),
            Self::Niri => parse_niri(conf),
            Self::Mango => parse_mango(conf),
        }
    }
}

pub fn resolve(theme_root: &Path) -> BorderColors {
    for &wm in ALL {
        if let Ok(s) = fs::read_to_string(theme_root.join(wm.filename())) {
            let colors = wm.parse_border(&s);
            if !colors.is_empty() {
                return colors;
            }
        }
    }
    BorderColors::default()
}

fn parse_hyprland(conf: &str) -> BorderColors {
    let find = |prefixes: &[&str]| -> Option<String> {
        conf.lines()
            .map(str::trim)
            .filter(|l| !l.starts_with('#'))
            .find_map(|line| {
                let rest = prefixes.iter().find_map(|p| line.strip_prefix(p))?;
                let val = rest
                    .trim_start_matches(|c: char| c.is_ascii_whitespace() || c == '=')
                    .trim();
                (!val.starts_with('$')).then(|| super::colors::normalize_color(val))
            })
    };

    BorderColors {
        active: find(&["$activeBorderColor", "col.active_border"]),
        inactive: find(&["$inactiveBorderColor", "col.inactive_border"]),
    }
}

fn parse_niri(conf: &str) -> BorderColors {
    let find = |key: &str| -> Option<String> {
        conf.lines().map(str::trim).find_map(|line| {
            let color = line
                .strip_prefix(key)?
                .trim()
                .trim_matches('"')
                .trim_matches('\'');
            (!color.is_empty()).then(|| super::colors::normalize_color(color))
        })
    };

    BorderColors {
        active: find("active-color"),
        inactive: find("inactive-color"),
    }
}

fn parse_mango(conf: &str) -> BorderColors {
    let find = |key: &str| -> Option<String> {
        conf.lines().map(str::trim).find_map(|line| {
            let rest = line
                .strip_prefix(key)?
                .trim_start_matches(|c: char| c.is_ascii_whitespace() || c == '=')
                .trim();
            let hex8 = rest.strip_prefix("0x")?;
            (hex8.len() == 8).then(|| format!("#{}", &hex8[..6]))
        })
    };

    BorderColors {
        active: find("focuscolor"),
        inactive: find("unfocuscolor"),
    }
}

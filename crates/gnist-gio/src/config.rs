use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

pub const VERSION: &str = concat!("2026-rust-", env!("CARGO_PKG_VERSION"));
pub const PRIORITY_APPLICATION: u32 = 600;
pub const PRIORITY_USER: u32 = 800;
pub const DEFAULT_TRANSITION_MS: u32 = 500;
pub const MAX_TRANSITION_MS: u32 = 5_000;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Mode {
    Light,
    Dark,
    Unknown,
}

impl Mode {
    pub fn parse(value: &str) -> Self {
        match value.trim().to_ascii_lowercase().as_str() {
            "light" => Self::Light,
            "dark" => Self::Dark,
            _ => Self::Unknown,
        }
    }

    pub fn is_known(self) -> bool {
        !matches!(self, Self::Unknown)
    }

    pub fn prefer_dark(self) -> Option<bool> {
        match self {
            Self::Light => Some(false),
            Self::Dark => Some(true),
            Self::Unknown => None,
        }
    }
}

impl fmt::Display for Mode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Light => f.write_str("light"),
            Self::Dark => f.write_str("dark"),
            Self::Unknown => f.write_str("unknown"),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum GtkApi {
    Gtk3,
    Gtk4,
}

impl GtkApi {
    pub fn from_major(version: u32) -> Option<Self> {
        match version {
            3 => Some(Self::Gtk3),
            4 => Some(Self::Gtk4),
            _ => None,
        }
    }

    pub fn major(self) -> u32 {
        match self {
            Self::Gtk3 => 3,
            Self::Gtk4 => 4,
        }
    }

    pub fn is_gtk3(self) -> bool {
        matches!(self, Self::Gtk3)
    }
}

impl fmt::Display for GtkApi {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "GTK{}", self.major())
    }
}

static DEBUG: OnceLock<bool> = OnceLock::new();
static DISABLED: OnceLock<bool> = OnceLock::new();
static PRIORITY: OnceLock<u32> = OnceLock::new();
static TRANSITION_MS: OnceLock<u32> = OnceLock::new();
static SET_GTK3_THEME: OnceLock<bool> = OnceLock::new();
static GTK3_LIGHT_THEME: OnceLock<String> = OnceLock::new();
static GTK3_DARK_THEME: OnceLock<String> = OnceLock::new();

pub fn debug_enabled() -> bool {
    *DEBUG.get_or_init(|| std::env::var_os("GTK_GNIST_DEBUG").is_some())
}

pub fn disabled() -> bool {
    *DISABLED.get_or_init(|| std::env::var_os("GTK_DISABLE_GNIST_STYLE").is_some())
}

pub fn css_priority() -> u32 {
    *PRIORITY.get_or_init(|| match std::env::var("GNIST_GTK_PRIORITY").as_deref() {
        Ok("application") => PRIORITY_APPLICATION,
        _ => PRIORITY_USER,
    })
}

pub fn transition_ms() -> u32 {
    *TRANSITION_MS.get_or_init(|| {
        std::env::var("GNIST_TRANSITION_MS")
            .ok()
            .and_then(|value| value.parse::<u32>().ok())
            .map(|value| value.min(MAX_TRANSITION_MS))
            .unwrap_or(DEFAULT_TRANSITION_MS)
    })
}

pub fn set_gtk3_theme_enabled() -> bool {
    *SET_GTK3_THEME.get_or_init(|| {
        !matches!(
            std::env::var("GNIST_SET_GTK3_THEME")
                .unwrap_or_default()
                .trim()
                .to_ascii_lowercase()
                .as_str(),
            "0" | "false" | "no" | "off"
        )
    })
}

pub fn gtk3_theme_name(mode: Mode) -> Option<&'static str> {
    match mode {
        Mode::Light => Some(
            GTK3_LIGHT_THEME
                .get_or_init(|| env_or("GNIST_GTK3_LIGHT_THEME", "adw-gtk3"))
                .as_str(),
        ),
        Mode::Dark => Some(
            GTK3_DARK_THEME
                .get_or_init(|| env_or("GNIST_GTK3_DARK_THEME", "adw-gtk3-dark"))
                .as_str(),
        ),
        Mode::Unknown => None,
    }
}

pub fn default_css_path(gtk: GtkApi) -> PathBuf {
    let config_home = config_home();
    let current = config_home.join("gnist/themes/current/gtk.css");
    if css_path_loadable(&current) {
        return current;
    }

    let legacy = config_home.join(format!("gtk-{}.0/gtk.css", gtk.major()));
    if css_path_loadable(&legacy) {
        return legacy;
    }

    current
}

pub fn css_path_loadable(path: impl AsRef<Path>) -> bool {
    let path = path.as_ref();
    !path.as_os_str().is_empty() && path.is_file()
}

fn config_home() -> PathBuf {
    let home = std::env::var_os("HOME").map(PathBuf::from).unwrap_or_default();

    // Flatpak may redirect XDG_CONFIG_HOME into its sandbox. The configured
    // filesystem override exposes the host theme directory at ~/.config.
    if Path::new("/.flatpak-info").is_file() {
        return home.join(".config");
    }

    std::env::var_os("XDG_CONFIG_HOME")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| home.join(".config"))
}

fn env_or(name: &str, default: &str) -> String {
    std::env::var(name)
        .ok()
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| default.to_owned())
}

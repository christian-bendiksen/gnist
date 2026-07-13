use anyhow::{Context, Result};
use std::path::PathBuf;

/// Resolved paths for theme sources, generated output, and active links.
#[derive(Clone, Debug)]
pub struct Ctx {
    pub config_home: PathBuf,
    pub config_dir: PathBuf,
    pub data_dir: PathBuf,
    pub templates_dir: PathBuf,
    pub user_templates_dir: PathBuf,
    pub generated_dir: PathBuf,
    pub live_dir: PathBuf,
    pub current_link: PathBuf,
    pub current_theme_file: PathBuf,
    pub background_link: PathBuf,
}

impl Ctx {
    /// Resolve Gnist's paths from `HOME` and `XDG_CONFIG_HOME`.
    pub fn new() -> Result<Self> {
        let home = std::env::var("HOME").context("$HOME is not set")?;

        let xdg = std::env::var("XDG_CONFIG_HOME")
            .ok()
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| format!("{home}/.config"));

        let config_home = PathBuf::from(&home).join(".config");
        let config_dir = PathBuf::from(xdg).join("gnist");
        let themes = config_dir.join("themes");
        let generated_dir = themes.join("generated");

        Ok(Self {
            config_home,
            data_dir: themes.join("data"),
            templates_dir: themes.join("templates"),
            user_templates_dir: themes.join("user-templates"),
            live_dir: generated_dir.join("live"),
            current_link: themes.join("current"),
            current_theme_file: themes.join("current.theme"),
            background_link: themes.join("background"),
            generated_dir,
            config_dir,
        })
    }
}

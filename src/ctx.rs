use anyhow::{Context, Result};
use std::{
    ffi::OsStr,
    path::{Path, PathBuf},
};

/// Resolved paths for theme sources, generated output, and active links.
#[derive(Clone, Debug)]
pub struct Ctx {
    pub config_home: PathBuf,
    pub config_dir: PathBuf,
    /// Theme source roots in descending precedence order.
    pub source_roots: Vec<PathBuf>,
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
    /// Resolve Gnist's paths from the XDG base directory environment.
    pub fn new() -> Result<Self> {
        let home = std::env::var_os("HOME").context("$HOME is not set")?;
        anyhow::ensure!(
            Path::new(&home).is_absolute(),
            "$HOME must be an absolute path"
        );
        let xdg_config_home = std::env::var_os("XDG_CONFIG_HOME");
        let xdg_data_home = std::env::var_os("XDG_DATA_HOME");
        let xdg_data_dirs = std::env::var_os("XDG_DATA_DIRS");

        Ok(Self::from_xdg(
            Path::new(&home),
            xdg_config_home.as_deref(),
            xdg_data_home.as_deref(),
            xdg_data_dirs.as_deref(),
        ))
    }

    pub(crate) fn from_xdg(
        home: &Path,
        xdg_config_home: Option<&OsStr>,
        xdg_data_home: Option<&OsStr>,
        xdg_data_dirs: Option<&OsStr>,
    ) -> Self {
        let xdg_config_home = absolute(xdg_config_home).unwrap_or_else(|| home.join(".config"));
        let xdg_data_home = absolute(xdg_data_home).unwrap_or_else(|| home.join(".local/share"));
        let xdg_data_dirs: Vec<PathBuf> = nonempty(xdg_data_dirs)
            .map(|value| {
                std::env::split_paths(value)
                    .filter(|path| !path.as_os_str().is_empty())
                    .filter(|path| path.is_absolute())
                    .collect()
            })
            .unwrap_or_else(default_data_dirs);

        let config_home = xdg_config_home;
        let config_dir = config_home.join("gnist");
        let themes = config_dir.join("themes");
        let generated_dir = themes.join("generated");

        let mut source_roots = vec![themes.clone()];
        for data_root in std::iter::once(xdg_data_home).chain(xdg_data_dirs) {
            let root = data_root.join("gnist/themes");
            if !source_roots.contains(&root) {
                source_roots.push(root);
            }
        }

        Self {
            config_home,
            source_roots,
            data_dir: themes.join("data"),
            templates_dir: themes.join("templates"),
            user_templates_dir: themes.join("user-templates"),
            live_dir: generated_dir.join("live"),
            current_link: themes.join("current"),
            current_theme_file: themes.join("current.theme"),
            background_link: themes.join("background"),
            generated_dir,
            config_dir,
        }
    }
}

fn nonempty(value: Option<&OsStr>) -> Option<&OsStr> {
    value.filter(|value| !value.is_empty())
}

fn absolute(value: Option<&OsStr>) -> Option<PathBuf> {
    nonempty(value)
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
}

fn default_data_dirs() -> Vec<PathBuf> {
    vec![
        PathBuf::from("/usr/local/share"),
        PathBuf::from("/usr/share"),
    ]
}

#[cfg(test)]
mod tests {
    use super::Ctx;
    use std::{ffi::OsStr, path::Path};

    #[test]
    fn resolves_config_user_and_system_source_roots_in_precedence_order() {
        let data_dirs = std::env::join_paths(["/opt/share", "/usr/share"]).unwrap();
        let ctx = Ctx::from_xdg(
            Path::new("/home/test"),
            Some(OsStr::new("/config")),
            Some(OsStr::new("/user-data")),
            Some(&data_dirs),
        );

        assert_eq!(
            ctx.source_roots,
            [
                "/config/gnist/themes",
                "/user-data/gnist/themes",
                "/opt/share/gnist/themes",
                "/usr/share/gnist/themes",
            ]
            .map(std::path::PathBuf::from)
        );
        assert_eq!(ctx.data_dir, Path::new("/config/gnist/themes/data"));
        assert_eq!(ctx.config_home, Path::new("/config"));
        assert_eq!(
            ctx.user_templates_dir,
            Path::new("/config/gnist/themes/user-templates")
        );
        assert_eq!(
            ctx.current_theme_file,
            Path::new("/config/gnist/themes/current.theme")
        );
        assert_eq!(
            ctx.generated_dir,
            Path::new("/config/gnist/themes/generated")
        );
    }

    #[test]
    fn deduplicates_xdg_data_roots() {
        let data_dirs = std::env::join_paths(["/home/test/.local/share", "", "/system"]).unwrap();
        let ctx = Ctx::from_xdg(Path::new("/home/test"), None, None, Some(&data_dirs));

        assert_eq!(
            ctx.source_roots,
            [
                "/home/test/.config/gnist/themes",
                "/home/test/.local/share/gnist/themes",
                "/system/gnist/themes",
            ]
            .map(std::path::PathBuf::from)
        );
    }

    #[test]
    fn relative_xdg_config_and_data_home_use_defaults() {
        let ctx = Ctx::from_xdg(
            Path::new("/home/test"),
            Some(OsStr::new("relative-config")),
            Some(OsStr::new("relative-data")),
            None,
        );

        assert_eq!(ctx.config_home, Path::new("/home/test/.config"));
        assert_eq!(
            ctx.source_roots,
            [
                "/home/test/.config/gnist/themes",
                "/home/test/.local/share/gnist/themes",
                "/usr/local/share/gnist/themes",
                "/usr/share/gnist/themes",
            ]
            .map(std::path::PathBuf::from)
        );
        assert!(ctx.source_roots.iter().all(|root| root.is_absolute()));
    }

    #[test]
    fn ignores_relative_entries_in_xdg_data_dirs() {
        let data_dirs = std::env::join_paths(["relative", "/opt/share"]).unwrap();
        let ctx = Ctx::from_xdg(Path::new("/home/test"), None, None, Some(&data_dirs));

        assert_eq!(
            ctx.source_roots,
            [
                "/home/test/.config/gnist/themes",
                "/home/test/.local/share/gnist/themes",
                "/opt/share/gnist/themes",
            ]
            .map(std::path::PathBuf::from)
        );
    }

    #[test]
    fn ignores_all_relative_xdg_data_dirs_without_substituting_defaults() {
        let data_dirs = std::env::join_paths(["relative-one", "relative-two"]).unwrap();
        let ctx = Ctx::from_xdg(Path::new("/home/test"), None, None, Some(&data_dirs));

        assert_eq!(
            ctx.source_roots,
            [
                "/home/test/.config/gnist/themes",
                "/home/test/.local/share/gnist/themes",
            ]
            .map(std::path::PathBuf::from)
        );
        assert!(ctx.source_roots.iter().all(|root| root.is_absolute()));
    }
}

mod border;
pub mod colors;
mod vars;

use crate::ctx::Ctx;
use anyhow::{Context, Result, bail};
use std::{
    collections::{BTreeSet, HashMap},
    fs,
    path::{Path, PathBuf},
};
use vars::build_vars_from_colors;

#[derive(Clone, Debug)]
pub struct Theme {
    pub name: String,
    pub root: PathBuf,
    pub vars: HashMap<String, String>,
    pub is_light: bool,
    pub icon_theme: Option<String>,
    pub backgrounds_dir: Option<PathBuf>,
}

impl Theme {
    pub fn load(ctx: &Ctx, name: &str) -> Result<Self> {
        Self::load_from_roots(&ctx.source_roots, name)
    }

    fn load_from_roots(source_roots: &[PathBuf], name: &str) -> Result<Self> {
        let first = source_roots
            .first()
            .map(|root| root.join("data").join(name))
            .unwrap_or_else(|| PathBuf::from(name));
        let Some(root) = source_roots
            .iter()
            .map(|source| source.join("data").join(name))
            .find(|root| root.is_dir())
        else {
            bail!("theme not found: {}", first.display());
        };

        Self::load_root(root, name)
    }

    pub(crate) fn load_root(root: PathBuf, name: &str) -> Result<Self> {
        if !root.is_dir() {
            bail!("theme not found: {}", root.display());
        }

        let colors_file = root.join("colors.toml");
        if !colors_file.is_file() {
            bail!(
                "missing colors.toml in theme '{name}': {}",
                colors_file.display()
            );
        }

        let mut vars = build_vars_from_colors(&colors_file)
            .with_context(|| format!("build vars for theme '{name}'"))?;

        let borders = border::resolve(&root);
        let active = borders.active.or_else(|| vars.get("accent").cloned());
        let inactive = borders
            .inactive
            .or_else(|| vars.get("color8").cloned())
            .or_else(|| active.clone());
        if let Some(c) = &active {
            vars::insert_border(&mut vars, "active", c);
        }
        if let Some(c) = &inactive {
            vars::insert_border(&mut vars, "inactive", c);
        }

        let bg_dir = root.join("backgrounds");

        Ok(Self {
            name: name.to_owned(),
            is_light: root.join("light.mode").is_file(),
            icon_theme: read_trimmed(&root.join("icons.theme"))?,
            backgrounds_dir: bg_dir.is_dir().then_some(bg_dir),
            root,
            vars,
        })
    }

    pub fn load_current(ctx: &Ctx) -> Result<Self> {
        let raw = match std::fs::read_to_string(&ctx.current_theme_file) {
            Ok(s) => s,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
            Err(e) => {
                return Err(e)
                    .with_context(|| format!("read {}", ctx.current_theme_file.display()));
            }
        };
        let name = raw.trim();
        anyhow::ensure!(
            !name.is_empty(),
            "current theme is not set ({})",
            ctx.current_theme_file.display()
        );
        Self::load(ctx, name).context("load current theme")
    }

    pub fn stage(&self, stage: &Path) -> Result<()> {
        for name in ["light.mode", "icons.theme"] {
            let src = self.root.join(name);
            if src.is_file() {
                crate::util::symlink_force(&src, &stage.join(name))
                    .with_context(|| format!("symlink {name}"))?;
            }
        }
        if let Some(bg) = &self.backgrounds_dir {
            crate::util::symlink_force(bg, &stage.join("backgrounds"))
                .context("symlink backgrounds")?;
        }
        Ok(())
    }
}

pub fn list_names(ctx: &Ctx) -> Result<Vec<String>> {
    list_names_from_roots(&ctx.source_roots)
}

fn list_names_from_roots(source_roots: &[PathBuf]) -> Result<Vec<String>> {
    let mut names = BTreeSet::new();

    for source_root in source_roots {
        let data_dir = source_root.join("data");
        let entries = match fs::read_dir(&data_dir) {
            Ok(entries) => entries,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
            Err(e) => return Err(e).with_context(|| format!("read {}", data_dir.display())),
        };

        names.extend(
            entries
                .flatten()
                .filter(|entry| entry.path().is_dir())
                .filter_map(|entry| entry.file_name().into_string().ok()),
        );
    }

    Ok(names.into_iter().collect())
}

fn read_trimmed(path: &Path) -> Result<Option<String>> {
    match fs::read_to_string(path) {
        Ok(s) => {
            let t = s.trim();
            Ok((!t.is_empty()).then(|| t.to_owned()))
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e).with_context(|| format!("read {}", path.display())),
    }
}

#[cfg(test)]
mod tests {
    use super::{Theme, list_names_from_roots};
    use std::{fs, path::Path};

    fn write_theme(source_root: &Path, name: &str, accent: &str) {
        let root = source_root.join("data").join(name);
        fs::create_dir_all(&root).unwrap();
        fs::write(root.join("colors.toml"), format!("accent = \"{accent}\"\n")).unwrap();
    }

    #[test]
    fn highest_matching_theme_source_wins() {
        let dir = tempfile::tempdir().unwrap();
        let config = dir.path().join("config");
        let user_data = dir.path().join("user-data");
        write_theme(&config, "shared", "#111111");
        write_theme(&user_data, "shared", "#222222");

        let theme = Theme::load_from_roots(&[config.clone(), user_data], "shared").unwrap();

        assert_eq!(theme.root, config.join("data/shared"));
        assert_eq!(theme.vars["accent"], "#111111");
    }

    #[test]
    fn invalid_higher_precedence_theme_does_not_fall_back() {
        let dir = tempfile::tempdir().unwrap();
        let config = dir.path().join("config");
        let system = dir.path().join("system");
        fs::create_dir_all(config.join("data/shared")).unwrap();
        write_theme(&system, "shared", "#222222");

        let error = Theme::load_from_roots(&[config.clone(), system], "shared").unwrap_err();

        assert!(error.to_string().contains("missing colors.toml"));
        assert!(error.to_string().contains(&config.display().to_string()));
    }

    #[test]
    fn theme_names_are_a_sorted_union_of_all_sources() {
        let dir = tempfile::tempdir().unwrap();
        let config = dir.path().join("config");
        let user_data = dir.path().join("user-data");
        let system = dir.path().join("system");
        write_theme(&config, "zeta", "#111111");
        write_theme(&config, "shared", "#111111");
        write_theme(&user_data, "alpha", "#222222");
        write_theme(&user_data, "shared", "#222222");
        write_theme(&system, "middle", "#333333");

        let names = list_names_from_roots(&[config, user_data, system]).unwrap();

        assert_eq!(names, ["alpha", "middle", "shared", "zeta"]);
    }
}

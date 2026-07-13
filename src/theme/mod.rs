mod border;
pub mod colors;
mod vars;

use crate::ctx::Ctx;
use anyhow::{Context, Result, bail};
use std::{
    collections::HashMap,
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
    pub fn load(data_dir: &Path, name: &str) -> Result<Self> {
        let root = data_dir.join(name);
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
        Self::load(&ctx.data_dir, name).context("load current theme")
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

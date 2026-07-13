use anyhow::{Context, Result, bail};
use std::path::Path;
use std::process::Command;

use crate::ctx::Ctx;
use crate::theme;

/// Turn a repository URL into a theme name, dropping common theme affixes.
fn theme_name_from_url(url: &str) -> Result<String> {
    let base = url
        .trim_end_matches('/')
        .rsplit('/')
        .next()
        .filter(|s| !s.is_empty())
        .with_context(|| format!("cannot derive theme name from URL: {url}"))?;

    let name = base.strip_suffix(".git").unwrap_or(base);
    let name = name.strip_prefix("gnist-").unwrap_or(name);
    let name = name.strip_prefix("oxidize-").unwrap_or(name);
    let name = name.strip_prefix("omarchy-").unwrap_or(name);
    let name = name.strip_suffix("-theme").unwrap_or(name);

    if matches!(name, "" | "." | "..") {
        bail!("invalid theme name derived from URL: {url}");
    }

    Ok(name.to_owned())
}

fn git_clone(url: &str, dest: &Path) -> Result<()> {
    if dest.exists() {
        std::fs::remove_dir_all(dest)
            .with_context(|| format!("remove existing theme at {}", dest.display()))?;
    }

    let status = Command::new("git")
        .args(["clone", "--", url])
        .arg(dest)
        .status()
        .context("spawn git")?;

    if !status.success() {
        bail!("git clone failed for {url}");
    }

    Ok(())
}

pub fn git_pull(path: &Path) -> Result<()> {
    let status = Command::new("git")
        .arg("-C")
        .arg(path)
        .args(["pull", "--ff-only"])
        .status()
        .context("spawn git")?;

    if !status.success() {
        bail!("git pull failed in {}", path.display());
    }
    Ok(())
}

pub fn run(ctx: &Ctx, url: &str) -> Result<String> {
    let name = theme_name_from_url(url)?;
    let dest = ctx.data_dir.join(&name);
    git_clone(url, &dest).with_context(|| format!("install theme '{name}' from {url}"))?;

    let colors_toml = dest.join("colors.toml");
    if !colors_toml.exists()
        && let Some(generated) = theme::colors::generate(&dest)
    {
        std::fs::write(&colors_toml, generated).context("write generated colors.toml")?;
    }

    Ok(name)
}

#[cfg(test)]
mod tests {
    use super::theme_name_from_url;

    #[test]
    fn rejects_path_components_as_theme_names() {
        assert!(theme_name_from_url("https://example.com/.").is_err());
        assert!(theme_name_from_url("https://example.com/..").is_err());
    }
}

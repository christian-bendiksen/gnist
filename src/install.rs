use anyhow::{Context, Result, bail};
use std::path::Path;
use std::process::Command;

use crate::ctx::Ctx;

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
    let name = name.strip_prefix("omarchy-").unwrap_or(name);
    let name = name.strip_suffix("-theme").unwrap_or(name);

    if matches!(name, "" | "." | "..") {
        bail!("invalid theme name derived from URL: {url}");
    }

    Ok(name.to_owned())
}

/// Clone into a sibling staging directory and swap into `dest` only on success,
/// so a failed clone leaves any existing theme directory untouched.
fn git_clone_atomic(url: &str, data_dir: &Path, name: &str) -> Result<()> {
    std::fs::create_dir_all(data_dir).with_context(|| format!("create {}", data_dir.display()))?;

    let pid = std::process::id();
    let staging = data_dir.join(format!(".{name}.cloning-{pid}"));
    if staging.exists() {
        std::fs::remove_dir_all(&staging)
            .with_context(|| format!("remove stale staging dir {}", staging.display()))?;
    }

    let status = Command::new("git")
        .args(["clone", "--", url])
        .arg(&staging)
        .status()
        .context("spawn git");

    match status {
        Ok(s) if s.success() => {}
        Ok(_) => {
            let _ = std::fs::remove_dir_all(&staging);
            bail!("git clone failed for {url}");
        }
        Err(err) => {
            let _ = std::fs::remove_dir_all(&staging);
            return Err(err);
        }
    }

    let dest = data_dir.join(name);
    if std::fs::symlink_metadata(&dest).is_ok() {
        let backup = data_dir.join(format!(".{name}.old-{pid}"));
        if backup.exists() {
            let _ = std::fs::remove_dir_all(&backup);
        }
        std::fs::rename(&dest, &backup)
            .with_context(|| format!("move existing theme {} aside", dest.display()))?;
        if let Err(err) = std::fs::rename(&staging, &dest) {
            let _ = std::fs::rename(&backup, &dest);
            let _ = std::fs::remove_dir_all(&staging);
            return Err(err).with_context(|| format!("move cloned theme into {}", dest.display()));
        }
        if let Err(err) = std::fs::remove_dir_all(&backup) {
            eprintln!(
                "warn: could not remove previous theme backup {}: {err}",
                backup.display()
            );
        }
    } else if let Err(err) = std::fs::rename(&staging, &dest) {
        let _ = std::fs::remove_dir_all(&staging);
        return Err(err).with_context(|| format!("move cloned theme into {}", dest.display()));
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
    git_clone_atomic(url, &ctx.data_dir, &name)
        .with_context(|| format!("install theme '{name}' from {url}"))?;
    Ok(name)
}

#[cfg(test)]
mod tests {
    use super::{git_clone_atomic, theme_name_from_url};
    use std::fs;

    #[test]
    fn rejects_path_components_as_theme_names() {
        assert!(theme_name_from_url("https://example.com/.").is_err());
        assert!(theme_name_from_url("https://example.com/..").is_err());
    }

    #[test]
    fn strips_gnist_and_omarchy_affixes_only() {
        assert_eq!(
            theme_name_from_url("https://example.com/gnist-tokyo-night-theme.git").unwrap(),
            "tokyo-night"
        );
        assert_eq!(
            theme_name_from_url("https://example.com/omarchy-nord-theme.git").unwrap(),
            "nord"
        );
        assert_eq!(
            theme_name_from_url("https://example.com/oxidize-dusk.git").unwrap(),
            "oxidize-dusk"
        );
    }

    #[test]
    fn failed_clone_leaves_existing_theme_directory_byte_identical() {
        let dir = tempfile::tempdir().unwrap();
        let data_dir = dir.path().join("data");
        let existing = data_dir.join("nord");
        fs::create_dir_all(&existing).unwrap();
        let original_colors = b"accent = \"#88c0d0\"\n";
        let original_extra = b"keep me untouched\n";
        fs::write(existing.join("colors.toml"), original_colors).unwrap();
        fs::write(existing.join("custom.conf"), original_extra).unwrap();

        // Point at a guaranteed-missing local path so git clone fails offline.
        let missing_repo = dir.path().join("does-not-exist/omarchy-nord-theme.git");
        let err = git_clone_atomic(missing_repo.to_str().unwrap(), &data_dir, "nord").unwrap_err();
        assert!(err.to_string().contains("git clone failed"), "{err:#}");

        // Existing theme directory is untouched and no staging directory remains.
        assert_eq!(
            fs::read(existing.join("colors.toml")).unwrap(),
            original_colors
        );
        assert_eq!(
            fs::read(existing.join("custom.conf")).unwrap(),
            original_extra
        );
        let entries: Vec<String> = fs::read_dir(&data_dir)
            .unwrap()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect();
        assert_eq!(entries, ["nord"]);
    }
}

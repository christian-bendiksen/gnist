use anyhow::{Context, Result};
use std::{fs, path::Path};

/// Put a Unix symlink in place with an atomic rename.
///
/// Parent directories are created as needed. The final rename can replace a
/// file or symlink at `link`, but not a directory.
#[cfg(unix)]
pub fn symlink_force(target: &Path, link: &Path) -> Result<()> {
    use std::os::unix::fs::symlink;

    if let Some(parent) = link.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("create parent dir {}", parent.display()))?;
    }

    let tmp = link.with_extension("symlink-tmp");
    symlink(target, &tmp).with_context(|| format!("create temp symlink {}", tmp.display()))?;

    fs::rename(&tmp, link)
        .with_context(|| format!("atomic rename symlink into place {}", link.display()))
}

#[cfg(not(unix))]
pub fn symlink_force(_target: &Path, _link: &Path) -> Result<()> {
    anyhow::bail!("symlinks are not supported on this platform")
}

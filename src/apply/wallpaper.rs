//! Selects and cycles wallpapers with `awww`.
use crate::{ctx::Ctx, theme::Theme, util};
use anyhow::{Context, Result, bail, ensure};
use std::{
    fs,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    thread,
    time::Duration,
};

fn canonical_str(path: &Path) -> Option<String> {
    std::fs::canonicalize(path)
        .ok()
        .map(|p| p.to_string_lossy().into_owned())
}

/// Advance to the next wallpaper and ask `awww` to display it.
pub fn run(ctx: &Ctx, theme: &Theme) -> Result<()> {
    let candidates = collect_candidates(ctx, theme);

    if candidates.is_empty() {
        notify(&format!("No wallpaper found for theme '{}'", theme.name));
        return Ok(());
    }

    // Resolve the link from its own directory, not the process's working directory.
    let current = canonical_str(&ctx.background_link);

    let next = pick_next(&candidates, current.as_deref());
    change_wallpaper(next)?;
    util::symlink_force(next, &ctx.background_link)?;
    Ok(())
}

/// Select an exact image independently of the active theme's wallpaper cycle.
pub fn set(ctx: &Ctx, image: &Path) -> Result<()> {
    let image = resolve_image(image)?;
    change_wallpaper(&image)?;
    util::symlink_force(&image, &ctx.background_link)?;
    Ok(())
}

fn resolve_image(path: &Path) -> Result<PathBuf> {
    let image =
        fs::canonicalize(path).with_context(|| format!("resolve wallpaper {}", path.display()))?;
    ensure!(
        image.is_file(),
        "wallpaper is not a regular file: {}",
        path.display()
    );
    Ok(image)
}

struct Candidate {
    path: PathBuf,
    canonical: Option<String>,
}

/// Merge user and theme wallpapers into a stable cycling order.
fn collect_candidates(ctx: &Ctx, theme: &Theme) -> Vec<Candidate> {
    let mut paths: Vec<PathBuf> = Vec::new();

    let user_bg = ctx.config_dir.join("backgrounds").join(&theme.name);
    if user_bg.is_dir() {
        paths.extend(list_files(&user_bg));
    }

    let theme_bg = ctx.current_link.join("backgrounds");
    if theme_bg.is_dir() {
        paths.extend(list_files(&theme_bg));
    }

    paths.sort();
    paths.dedup();
    paths
        .into_iter()
        .map(|path| {
            let canonical = canonical_str(&path);
            Candidate { path, canonical }
        })
        .collect()
}

fn list_files(dir: &Path) -> Vec<PathBuf> {
    fs::read_dir(dir)
        .map(|rd| {
            rd.flatten()
                .map(|e| e.path())
                .filter(|p| p.is_file())
                .collect()
        })
        .unwrap_or_default()
}

fn pick_next<'a>(candidates: &'a [Candidate], current: Option<&str>) -> &'a PathBuf {
    let idx = current
        .and_then(|cur| {
            candidates
                .iter()
                .position(|c| c.canonical.as_deref() == Some(cur))
        })
        .map_or(0, |i| (i + 1) % candidates.len());

    &candidates[idx].path
}

fn change_wallpaper(path: &Path) -> Result<()> {
    wait_for_awww()?;

    let status = Command::new("awww")
        .arg("img")
        .arg(path)
        .args(["--transition-type=fade", "--transition-duration=0.6"])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .context("apply wallpaper with awww")?;
    ensure!(status.success(), "awww failed to apply wallpaper");
    Ok(())
}

fn wait_for_awww() -> Result<()> {
    for attempt in 0..4 {
        let status = Command::new("awww")
            .arg("query")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .context("query awww daemon")?;
        if status.success() {
            return Ok(());
        }

        if attempt < 3 {
            thread::sleep(Duration::from_millis(100));
        }
    }

    bail!("awww daemon did not become ready")
}

fn notify(msg: &str) {
    Command::new("notify-send")
        .args([msg, "-t", "2000"])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .ok();
}

#[cfg(test)]
mod tests {
    use super::resolve_image;
    use std::fs;

    #[test]
    fn resolve_image_canonicalizes_regular_files() {
        let dir = tempfile::tempdir().unwrap();
        let image = dir.path().join("wallpaper.png");
        fs::write(&image, b"image").unwrap();

        assert_eq!(
            resolve_image(&image).unwrap(),
            fs::canonicalize(image).unwrap()
        );
    }

    #[test]
    fn resolve_image_rejects_missing_paths_and_directories() {
        let dir = tempfile::tempdir().unwrap();

        assert!(resolve_image(&dir.path().join("missing.png")).is_err());
        assert!(resolve_image(dir.path()).is_err());
    }
}

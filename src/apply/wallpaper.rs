//! Cycles theme wallpapers with `awww`.
use crate::{ctx::Ctx, theme::Theme, util};
use anyhow::Result;
use std::{
    fs,
    path::{Path, PathBuf},
    process::{Command, Stdio},
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
    util::symlink_force(next, &ctx.background_link)?;

    change_wallpaper(&ctx.background_link);
    Ok(())
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

fn change_wallpaper(path: &Path) {
    Command::new("awww")
        .args([
            "img",
            &path.to_string_lossy(),
            "--transition-type=fade",
            "--transition-duration=0.6",
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .ok();
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

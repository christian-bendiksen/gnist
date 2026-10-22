//! Selects and cycles wallpapers with `awww`.
//!
//! A theme may ship a templated `wallpaper.toml` to override the awww options:
//!
//! ```toml
//! transition_type     = "fade"
//! transition_duration = "0.6"
//! ```
use crate::render::engine::render_str;
use crate::render::value::Value;
use crate::{ctx::Ctx, theme::Theme, util};
use anyhow::{Context, Result, bail, ensure};
use std::{
    collections::HashMap,
    fs,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    thread,
    time::Duration,
};

struct WallpaperConfig {
    transition_type: String,
    transition_duration: String,
}

impl Default for WallpaperConfig {
    fn default() -> Self {
        Self {
            transition_type: "fade".into(),
            transition_duration: "0.6".into(),
        }
    }
}

impl WallpaperConfig {
    fn load(root: &Path, values: &HashMap<String, Value>) -> Self {
        let mut cfg = Self::default();
        let Ok(src) = fs::read_to_string(root.join("wallpaper.toml")) else {
            return cfg;
        };
        let Ok(toml) = render_str(&src, values).parse::<toml::Value>() else {
            return cfg;
        };
        let get = |key: &str| toml.get(key).and_then(|v| v.as_str()).map(str::to_owned);
        cfg.transition_type = get("transition_type").unwrap_or(cfg.transition_type);
        cfg.transition_duration = get("transition_duration").unwrap_or(cfg.transition_duration);
        cfg
    }
}

fn canonical_str(path: &Path) -> Option<String> {
    std::fs::canonicalize(path)
        .ok()
        .map(|p| p.to_string_lossy().into_owned())
}

/// Advance to the next wallpaper and ask `awww` to display it.
pub fn run(ctx: &Ctx, theme: &Theme) -> Result<()> {
    let cfg = WallpaperConfig::load(&theme.root, &theme.vars);
    let candidates = collect_candidates(ctx, theme);

    if candidates.is_empty() {
        notify(&format!("No wallpaper found for theme '{}'", theme.name));
        return Ok(());
    }

    // Resolve the link from its own directory, not the process's working directory.
    let current = canonical_str(&ctx.background_link);

    let next = pick_next(&candidates, current.as_deref());
    change_wallpaper(next, &cfg)?;
    util::symlink_force(next, &ctx.background_link)?;
    Ok(())
}

/// Show the wallpaper for a theme that was just set.
///
/// The background link names `current/backgrounds/<picture>`, so once the new
/// theme is published it already resolves into the new theme's picture of the
/// same name. A dark/light pair that ships the same pictures (Kvarts and
/// Kvarts Light both lead with Facet) must keep the one the person was looking
/// at; [`run`] advanced from it instead, and Kvarts Light came up on the next
/// picture in its list, Herda Drawn. So: the current picture when the theme has
/// it; else its counterpart in the other mode, by name (Tang and Tang Light,
/// `2-herda-tang.jpg` and `2-herda-tang-light.jpg`); else the theme's first.
/// Cycling stays [`run`]'s, for `gnist wallpaper`.
pub fn show_for_theme(ctx: &Ctx, theme: &Theme) -> Result<()> {
    let cfg = WallpaperConfig::load(&theme.root, &theme.vars);
    let candidates = collect_candidates(ctx, theme);

    if candidates.is_empty() {
        notify(&format!("No wallpaper found for theme '{}'", theme.name));
        return Ok(());
    }

    let current = canonical_str(&ctx.background_link);
    // The picture's own name, even when the link no longer leads anywhere: the
    // previous theme's picture of that name is what the counterpart matches.
    let current_name = fs::read_link(&ctx.background_link).ok().and_then(|target| {
        target
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
    });
    let chosen = pick_for_theme(
        &candidates,
        current.as_deref(),
        current_name.as_deref(),
        theme.is_light,
    );
    change_wallpaper(chosen, &cfg)?;
    util::symlink_force(chosen, &ctx.background_link)?;
    Ok(())
}

/// Select an exact image independently of the active theme's wallpaper cycle.
pub fn set(ctx: &Ctx, image: &Path) -> Result<()> {
    let image = resolve_image(image)?;
    change_wallpaper(&image, &WallpaperConfig::default())?;
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

fn pick_for_theme<'a>(
    candidates: &'a [Candidate],
    current: Option<&str>,
    current_name: Option<&str>,
    light: bool,
) -> &'a PathBuf {
    let same = current.and_then(|cur| {
        candidates
            .iter()
            .position(|c| c.canonical.as_deref() == Some(cur))
    });
    let counterpart = || {
        let (base, _) = picture_key(current_name?);
        let matching = |c: &&Candidate| {
            c.path
                .file_name()
                .is_some_and(|name| picture_key(&name.to_string_lossy()).0 == base)
        };
        // The theme's own mode first: a Light theme also carries the dark pictures.
        candidates
            .iter()
            .filter(matching)
            .find(|c| {
                c.path
                    .file_name()
                    .is_some_and(|name| picture_key(&name.to_string_lossy()).1 == light)
            })
            .or_else(|| candidates.iter().find(matching))
            .map(|c| &c.path)
    };
    same.map(|idx| &candidates[idx].path)
        .or_else(counterpart)
        .unwrap_or(&candidates[0].path)
}

/// A picture's name without its order (`2-`), its extension and its mode
/// (`-light`), and whether it is the Light one: `2-herda-tang-light.jpg` is
/// (`herda-tang`, true).
fn picture_key(name: &str) -> (String, bool) {
    let stem = name.rsplit_once('.').map_or(name, |(stem, _)| stem);
    let stem = match stem.split_once('-') {
        Some((order, rest)) if !order.is_empty() && order.bytes().all(|b| b.is_ascii_digit()) => {
            rest
        }
        _ => stem,
    };
    match stem.strip_suffix("-light") {
        Some(base) => (base.to_owned(), true),
        None => (stem.to_owned(), false),
    }
}

fn change_wallpaper(path: &Path, cfg: &WallpaperConfig) -> Result<()> {
    wait_for_awww()?;

    let status = Command::new("awww")
        .arg("img")
        .arg(path)
        .arg(format!("--transition-type={}", cfg.transition_type))
        .arg(format!("--transition-duration={}", cfg.transition_duration))
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
    use super::{Candidate, pick_for_theme, pick_next, picture_key, resolve_image};
    use std::fs;
    use std::path::PathBuf;

    fn candidates(names: &[&str]) -> Vec<Candidate> {
        names
            .iter()
            .map(|name| Candidate {
                path: PathBuf::from(format!("current/backgrounds/{name}")),
                canonical: Some(format!("/themes/light/backgrounds/{name}")),
            })
            .collect()
    }

    #[test]
    fn setting_a_theme_keeps_a_picture_it_shares_and_never_advances() {
        // Kvarts -> Kvarts Light: the link already resolves to Light's Facet.
        let list = candidates(&["0-kvarts-facet.jpg", "1-herda-drawn.jpg"]);
        let facet = Some("/themes/light/backgrounds/0-kvarts-facet.jpg");
        assert_eq!(
            pick_for_theme(&list, facet, Some("0-kvarts-facet.jpg"), true),
            &PathBuf::from("current/backgrounds/0-kvarts-facet.jpg")
        );
        // What `gnist wallpaper` does from the same place: the next one.
        assert_eq!(
            pick_next(&list, facet),
            &PathBuf::from("current/backgrounds/1-herda-drawn.jpg")
        );
        // A picture the theme doesn't have (a dangling link, another theme's
        // file, nothing yet): the theme's first.
        for current in [None, Some("/themes/dark/backgrounds/x.jpg")] {
            assert_eq!(
                pick_for_theme(&list, current, Some("x.jpg"), true),
                &PathBuf::from("current/backgrounds/0-kvarts-facet.jpg")
            );
        }
        // A later picture the person chose in the old theme stays too.
        assert_eq!(
            pick_for_theme(
                &list,
                Some("/themes/light/backgrounds/1-herda-drawn.jpg"),
                Some("1-herda-drawn.jpg"),
                true
            ),
            &PathBuf::from("current/backgrounds/1-herda-drawn.jpg")
        );
    }

    #[test]
    fn switching_dark_and_light_keeps_the_counterpart_picture_by_name() {
        // Herda Desktop Light carries its own pictures and the dark ones too.
        let light = candidates(&[
            "0-herda-drawn-light.jpg",
            "2-herda-tang-light.jpg",
            "6-herda-tang.jpg",
        ]);
        // From the dark theme's Tang (the link now leads nowhere): Tang Light.
        assert_eq!(
            pick_for_theme(&light, None, Some("2-herda-tang.jpg"), true),
            &PathBuf::from("current/backgrounds/2-herda-tang-light.jpg")
        );
        // And back: Tang.
        let dark = candidates(&["0-herda-drawn.jpg", "2-herda-tang.jpg"]);
        assert_eq!(
            pick_for_theme(&dark, None, Some("2-herda-tang-light.jpg"), false),
            &PathBuf::from("current/backgrounds/2-herda-tang.jpg")
        );
        // No counterpart: the theme's first.
        assert_eq!(
            pick_for_theme(&dark, None, Some("9-other.jpg"), false),
            &PathBuf::from("current/backgrounds/0-herda-drawn.jpg")
        );
        assert_eq!(
            picture_key("2-herda-tang-light.jpg"),
            ("herda-tang".to_owned(), true)
        );
        assert_eq!(picture_key("facet.png"), ("facet".to_owned(), false));
    }

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

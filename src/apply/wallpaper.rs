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
    ffi::OsStr,
    fs,
    os::unix::fs::FileTypeExt,
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
    wait_for_awww_in(
        std::env::var_os("XDG_RUNTIME_DIR").as_deref(),
        std::env::var_os("WAYLAND_DISPLAY").as_deref(),
        query_awww,
    )
}

fn query_awww() -> Result<bool> {
    let status = Command::new("awww")
        .arg("query")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .context("query awww daemon")?;
    Ok(status.success())
}

fn wait_for_awww_in(
    xdg_runtime_dir: Option<&OsStr>,
    wayland_display: Option<&OsStr>,
    mut query: impl FnMut() -> Result<bool>,
) -> Result<()> {
    if !has_wallpaper_daemon_socket(xdg_runtime_dir, wayland_display) {
        bail!("awww daemon socket is not present");
    }

    for attempt in 0..4 {
        if query()? {
            return Ok(());
        }

        if attempt < 3 {
            thread::sleep(Duration::from_millis(100));
        }
    }

    bail!("awww daemon did not become ready")
}

/// Fast socket-presence check before spawning `awww query`.
///
/// Each `awww query` call waits ~500 ms internally when no daemon is listening,
/// so four attempts plus three 100 ms sleeps stall theme switches for ~2.4 s
/// whenever no `awww` or `swww` daemon is running in the session.
fn has_wallpaper_daemon_socket(
    xdg_runtime_dir: Option<&OsStr>,
    wayland_display: Option<&OsStr>,
) -> bool {
    let Some(runtime_dir) = xdg_runtime_dir
        .filter(|value| !value.is_empty())
        .map(Path::new)
    else {
        return false;
    };
    if !runtime_dir.is_dir() {
        return false;
    }

    let explicit_display = wayland_display
        .filter(|value| !value.is_empty())
        .and_then(|value| Path::new(value).file_name())
        .and_then(OsStr::to_str)
        .filter(|value| !value.is_empty());
    let display = explicit_display.unwrap_or("wayland-0");

    let direct_candidates = [
        runtime_dir.join(format!("{display}-awww-daemon.sock")),
        runtime_dir.join(format!("{display}-swww-daemon.sock")),
        runtime_dir.join(format!("awww-{display}.sock")),
        runtime_dir.join(format!("swww-{display}.sock")),
        runtime_dir.join("awww.sock"),
        runtime_dir.join("swww.sock"),
        runtime_dir
            .join("awww-ipc")
            .join(format!("{display}-awww-daemon.sock")),
        runtime_dir.join("awww-ipc").join(format!("{display}.sock")),
        runtime_dir.join("swww").join(format!("{display}.sock")),
    ];
    if direct_candidates.iter().any(|path| is_unix_socket(path)) {
        return true;
    }

    has_matching_socket_in_dir(runtime_dir, |name| {
        is_daemon_socket_name(name, display, explicit_display.is_none())
    }) || has_matching_socket_in_dir(&runtime_dir.join("awww-ipc"), |name| {
        is_subdir_daemon_socket_name(name, display, explicit_display.is_none())
    }) || has_matching_socket_in_dir(&runtime_dir.join("swww"), |name| {
        is_subdir_daemon_socket_name(name, display, explicit_display.is_none())
    })
}

fn is_unix_socket(path: &Path) -> bool {
    fs::metadata(path).is_ok_and(|meta| meta.file_type().is_socket())
}

fn has_matching_socket_in_dir(dir: &Path, matches_name: impl Fn(&str) -> bool) -> bool {
    let Ok(entries) = fs::read_dir(dir) else {
        return false;
    };
    entries.flatten().any(|entry| {
        let name = entry.file_name();
        let Some(name) = name.to_str() else {
            return false;
        };
        matches_name(name) && is_unix_socket(&entry.path())
    })
}

fn is_daemon_socket_name(name: &str, display: &str, allow_any_display: bool) -> bool {
    let awww_prefix = format!("{display}-awww-daemon");
    let swww_prefix = format!("{display}-swww-daemon");
    let alt_awww_prefix = format!("awww-{display}");
    let alt_swww_prefix = format!("swww-{display}");

    if matches!(
        name,
        "awww.sock" | "swww.sock" | "awww-daemon.sock" | "swww-daemon.sock"
    ) {
        return true;
    }

    if !name.ends_with(".sock") {
        return false;
    }

    if name.starts_with(&awww_prefix)
        || name.starts_with(&swww_prefix)
        || name.starts_with(&alt_awww_prefix)
        || name.starts_with(&alt_swww_prefix)
    {
        return true;
    }

    allow_any_display
        && (name.starts_with("awww")
            || name.starts_with("swww")
            || name.contains("-awww-daemon")
            || name.contains("-swww-daemon"))
}

fn is_subdir_daemon_socket_name(name: &str, display: &str, allow_any_display: bool) -> bool {
    name.ends_with(".sock")
        && (name.starts_with(display)
            || (allow_any_display && (name.starts_with("awww") || name.starts_with("swww"))))
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
    use super::{
        Candidate, has_wallpaper_daemon_socket, pick_for_theme, pick_next, picture_key,
        resolve_image, wait_for_awww_in,
    };
    use std::ffi::OsStr;
    use std::fs;
    use std::os::unix::net::UnixListener;
    use std::path::PathBuf;
    use std::time::{Duration, Instant};

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

    #[test]
    fn wait_for_awww_fast_fails_when_runtime_dir_is_unset_or_empty() {
        let mut query_calls = 0usize;
        let started = Instant::now();
        let err = wait_for_awww_in(None, Some(OsStr::new("wayland-0")), || {
            query_calls += 1;
            Ok(true)
        })
        .unwrap_err();
        assert!(started.elapsed() < Duration::from_millis(50));
        assert_eq!(query_calls, 0);
        assert!(err.to_string().contains("socket is not present"));

        let empty_dir = tempfile::tempdir().unwrap();
        let started = Instant::now();
        let err = wait_for_awww_in(
            Some(empty_dir.path().as_os_str()),
            Some(OsStr::new("wayland-0")),
            || {
                query_calls += 1;
                Ok(true)
            },
        )
        .unwrap_err();
        assert!(started.elapsed() < Duration::from_millis(50));
        assert_eq!(query_calls, 0);
        assert!(err.to_string().contains("socket is not present"));

        let missing = empty_dir.path().join("does-not-exist");
        assert!(!has_wallpaper_daemon_socket(
            Some(missing.as_os_str()),
            Some(OsStr::new("wayland-0"))
        ));
        assert!(!has_wallpaper_daemon_socket(Some(OsStr::new("")), None));
    }

    #[test]
    fn wallpaper_socket_check_detects_bound_awww_and_swww_sockets() {
        for (socket_name, display) in [
            ("wayland-0-awww-daemon.sock", Some("wayland-0")),
            ("wayland-0-awww-daemon.sock", None),
            ("wayland-2-awww-daemon.sock", Some("wayland-2")),
            ("wayland-2-awww-daemon.sock", Some("/tmp/rt/wayland-2")),
            ("wayland-0-awww-daemon-bg.sock", Some("wayland-0")),
            ("wayland-0-swww-daemon.sock", Some("wayland-0")),
            ("awww-wayland-0.sock", Some("wayland-0")),
            ("swww-wayland-0.sock", Some("wayland-0")),
        ] {
            let dir = tempfile::tempdir().unwrap();
            let _listener = UnixListener::bind(dir.path().join(socket_name)).unwrap();
            assert!(
                has_wallpaper_daemon_socket(Some(dir.path().as_os_str()), display.map(OsStr::new)),
                "expected {socket_name} with display {display:?} to be detected"
            );

            let mut query_calls = 0usize;
            wait_for_awww_in(
                Some(dir.path().as_os_str()),
                display.map(OsStr::new),
                || {
                    query_calls += 1;
                    Ok(true)
                },
            )
            .unwrap();
            assert_eq!(query_calls, 1);
        }
    }

    #[test]
    fn wallpaper_socket_check_ignores_regular_files_and_unrelated_sockets() {
        let dir = tempfile::tempdir().unwrap();
        // Regular files and directories with daemon-like names are not Unix sockets.
        fs::write(
            dir.path().join("wayland-0-awww-daemon.sock"),
            b"not a socket",
        )
        .unwrap();
        fs::write(dir.path().join("awww-wayland-0.sock"), b"not a socket").unwrap();
        fs::create_dir(dir.path().join("swww-wayland-0.sock")).unwrap();

        // Real Unix sockets belonging to the compositor, audio, bus, or another display.
        let _wayland = UnixListener::bind(dir.path().join("wayland-0")).unwrap();
        let _pipewire = UnixListener::bind(dir.path().join("pipewire-0")).unwrap();
        let _bus = UnixListener::bind(dir.path().join("bus")).unwrap();
        let _other = UnixListener::bind(dir.path().join("unrelated.sock")).unwrap();
        let _other_display =
            UnixListener::bind(dir.path().join("wayland-1-awww-daemon.sock")).unwrap();

        assert!(!has_wallpaper_daemon_socket(
            Some(dir.path().as_os_str()),
            Some(OsStr::new("wayland-0"))
        ));

        let mut query_calls = 0usize;
        let err = wait_for_awww_in(
            Some(dir.path().as_os_str()),
            Some(OsStr::new("wayland-0")),
            || {
                query_calls += 1;
                Ok(true)
            },
        )
        .unwrap_err();
        assert_eq!(query_calls, 0);
        assert!(err.to_string().contains("socket is not present"));
    }
}

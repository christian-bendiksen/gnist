//! Connects CLI theme updates to the CSS provider installed by `gnist-gio`.
//!
//! The module adds a display-wide `GtkCssProvider` to GTK applications loaded
//! through `GIO_EXTRA_MODULES`. The D-Bus signal sent here tells that provider
//! which CSS file to reload.

use crate::{ctx::Ctx, theme::Theme};
use anyhow::{Context, Result, anyhow, bail};
use std::{
    fs,
    path::{Path, PathBuf},
    process::{Command, Stdio},
};

const DBUS_OBJECT_PATH: &str = "/org/gnist/Appearance1";
const DBUS_SIGNAL: &str = "org.gnist.Appearance1.Changed";

/// Replace the GTK 3 and GTK 4 user CSS symlinks to wake their file monitors.
///
/// GTK watches the directory entry rather than the symlink target. Renaming a
/// temporary symlink over `gtk.css` produces `IN_MOVED_TO` even when both links
/// point to the same file. This also reaches apps that do not load `gnist-gio`.
pub fn refresh_user_css_links(ctx: &Ctx) {
    use std::os::unix::fs::symlink;

    let target = ctx.current_link.join("gtk.css");
    let dirs = [
        ctx.config_home.join("gtk-3.0"),
        ctx.config_home.join("gtk-4.0"),
    ];

    for dir in &dirs {
        let dst = dir.join("gtk.css");
        let tmp = dir.join(".gtk.css.gnist.tmp");
        let _ = fs::remove_file(&tmp);
        if symlink(&target, &tmp).is_ok() {
            let _ = fs::rename(&tmp, &dst);
        }
    }
}

pub fn emit_current(ctx: &Ctx, theme: Option<&Theme>) -> Result<()> {
    let owned_theme;
    let theme = match theme {
        Some(theme) => theme,
        None => {
            owned_theme = Theme::load_current(ctx).context("load current theme")?;
            &owned_theme
        }
    };

    let css_path = current_gtk_css_path(ctx);
    if !css_path.is_file() {
        bail!("GTK CSS file not found: {}", css_path.display());
    }

    let revision = bump_revision(ctx)?;
    let mode = if theme.is_light { "light" } else { "dark" };

    emit_changed_signal(revision, &css_path, mode)
}

pub fn current_gtk_css_path(ctx: &Ctx) -> PathBuf {
    ctx.current_link.join("gtk.css")
}

fn bump_revision(ctx: &Ctx) -> Result<u64> {
    let runtime_dir = ctx.config_dir.join("runtime");
    fs::create_dir_all(&runtime_dir)
        .with_context(|| format!("create {}", runtime_dir.display()))?;

    let revision_file = runtime_dir.join("gtk-css.revision");

    let previous = fs::read_to_string(&revision_file)
        .ok()
        .and_then(|raw| raw.trim().parse::<u64>().ok())
        .unwrap_or(0);

    let next = previous.saturating_add(1);
    fs::write(&revision_file, format!("{next}\n"))
        .with_context(|| format!("write {}", revision_file.display()))?;

    Ok(next)
}

fn emit_changed_signal(revision: u64, css_path: &Path, mode: &str) -> Result<()> {
    let css_path = css_path
        .to_str()
        .ok_or_else(|| anyhow!("GTK CSS path is not valid UTF-8: {}", css_path.display()))?;

    let status = Command::new("dbus-send")
        .args(["--session", "--type=signal", DBUS_OBJECT_PATH, DBUS_SIGNAL])
        .arg(format!("uint64:{revision}"))
        .arg(format!("string:{css_path}"))
        .arg(format!("string:{mode}"))
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .context("spawn dbus-send")?;

    if !status.success() {
        bail!("dbus-send failed with status {status}");
    }

    Ok(())
}

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

    for (dir, target) in user_css_links(&ctx.config_home, &ctx.current_link) {
        let dst = dir.join("gtk.css");
        let tmp = dir.join(".gtk.css.gnist.tmp");
        let _ = fs::remove_file(&tmp);
        if symlink(&target, &tmp).is_ok() {
            let _ = fs::rename(&tmp, &dst);
        }
    }
}

/// Each GTK version's user CSS directory and the published file it links to.
///
/// GTK 4 reads `gtk.css`. GTK 3 cannot parse what GTK 4 and libadwaita do
/// (`color-mix()`, `@media`, `:root`), so it always links to `gtk3.css`.
/// When that sheet is absent the link is inactive. Refresh still replaces old
/// links to GTK 4 CSS, and a later publication can activate GTK 3's own sheet.
pub fn user_css_links(config_home: &Path, current: &Path) -> [(PathBuf, PathBuf); 2] {
    [
        (config_home.join("gtk-3.0"), current.join("gtk3.css")),
        (config_home.join("gtk-4.0"), current.join("gtk.css")),
    ]
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gtk3_reads_the_gtk3_file_when_the_theme_publishes_one() {
        let dir = tempfile::tempdir().unwrap();
        let current = dir.path().join("current");
        fs::create_dir_all(&current).unwrap();
        fs::write(current.join("gtk.css"), "").unwrap();
        let home = dir.path().join("config");
        let links = user_css_links(&home, &current);
        assert_eq!(links[0], (home.join("gtk-3.0"), current.join("gtk3.css")));
        assert!(!links[0].1.is_file());
        fs::write(current.join("gtk3.css"), "").unwrap();
        let links = user_css_links(&home, &current);
        assert_eq!(links[0], (home.join("gtk-3.0"), current.join("gtk3.css")));
        assert_eq!(links[1], (home.join("gtk-4.0"), current.join("gtk.css")));
    }

    #[test]
    fn refresh_replaces_old_gtk4_links_and_tracks_missing_gtk3_css() {
        use std::os::unix::fs::symlink;

        let dir = tempfile::tempdir().unwrap();
        let ctx = Ctx::from_xdg(dir.path(), None, None, None);
        fs::create_dir_all(&ctx.current_link).unwrap();
        fs::create_dir_all(ctx.config_home.join("gtk-3.0")).unwrap();
        fs::create_dir_all(ctx.config_home.join("gtk-4.0")).unwrap();
        let gtk3 = ctx.current_link.join("gtk3.css");
        let gtk4 = ctx.current_link.join("gtk.css");
        let legacy = ctx.config_home.join("gtk-3.0/gtk.css");
        fs::write(&gtk4, ":root { --x: 1px; }").unwrap();
        symlink(&gtk4, &legacy).unwrap();

        refresh_user_css_links(&ctx);
        assert_eq!(fs::read_link(&legacy).unwrap(), gtk3);
        assert!(!legacy.is_file());
        let gtk4_user = ctx.config_home.join("gtk-4.0/gtk.css");
        assert_eq!(fs::read_link(&gtk4_user).unwrap(), gtk4);
        assert!(gtk4_user.is_file());

        fs::write(&gtk3, "@define-color x #000;").unwrap();
        refresh_user_css_links(&ctx);
        assert_eq!(
            fs::read_to_string(&legacy).unwrap(),
            "@define-color x #000;"
        );

        fs::remove_file(&gtk3).unwrap();
        refresh_user_css_links(&ctx);
        assert_eq!(fs::read_link(&legacy).unwrap(), gtk3);
        assert!(!legacy.is_file());
        assert_eq!(
            fs::read_to_string(gtk4_user).unwrap(),
            ":root { --x: 1px; }"
        );
    }
}

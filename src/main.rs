use anyhow::{Context, Result, bail};
use clap::{Args, Parser, Subcommand};
use herda_ui::document::Row;
use herda_ui::ink::Ink;
use std::path::{Path, PathBuf};

mod apply;
mod ctx;
mod inspect;
mod install;
mod output;
mod render;
mod theme;
mod transaction;
mod util;

use ctx::Ctx;
use theme::Theme;
use transaction::Transaction;

use crate::apply::ApplyFlags;

#[derive(Args, Debug, Clone, Copy)]
struct IntegrationOptions {
    #[arg(long)]
    skip_apply: bool,
    #[arg(long)]
    skip_gnome: bool,
    #[arg(long)]
    skip_icons: bool,
    #[arg(long)]
    skip_reload: bool,
}

impl IntegrationOptions {
    fn apply_flags(self, skip_wallpaper: bool) -> ApplyFlags {
        ApplyFlags {
            skip_apply: self.skip_apply,
            skip_gnome: self.skip_gnome,
            skip_icons: self.skip_icons,
            skip_reload: self.skip_reload,
            skip_wallpaper,
        }
    }
}

#[derive(Parser)]
#[command(name = "gnist", about = "Atomic Wayland theme switcher")]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, clap::ValueEnum)]
enum ModeArg {
    Light,
    Dark,
    Toggle,
}

#[derive(Subcommand)]
enum Cmd {
    /// Create the Gnist theme directories
    Init,

    /// Render and apply an installed theme
    Set {
        theme: String,
        #[command(flatten)]
        integrations: IntegrationOptions,
        #[arg(long)]
        skip_wallpaper: bool,
    },

    /// Switch the current theme to its light or dark counterpart
    ///
    /// The counterpart is the theme's `light_theme` / `dark_theme` key in
    /// colors.toml, else `<name>-light` / the name without `-light`.
    Mode {
        mode: ModeArg,
        #[command(flatten)]
        integrations: IntegrationOptions,
        #[arg(long)]
        skip_wallpaper: bool,
    },

    /// Re-render and apply the current theme
    Reapply {
        #[command(flatten)]
        integrations: IntegrationOptions,
        /// Advance and apply the current theme's wallpaper
        #[arg(long)]
        wallpaper: bool,
    },

    /// Show installed themes
    List,

    /// Inspect installed palettes without publishing or applying anything (JSON).
    Inspect { theme: Option<String> },

    /// Print the active theme name
    Current,

    /// Clone a Git theme repository and apply it
    Install { url: String },

    /// Delete an installed theme
    Remove {
        theme: String,
        #[arg(long, short = 'y')]
        yes: bool,
        #[arg(long)]
        force: bool,
    },

    /// Pull updates for one or more Git-installed themes
    Update {
        /// Update every theme with a `.git` directory when no name is given
        theme: Option<String>,
    },

    /// Reload supported applications without changing themes
    Reload,

    /// Apply the active theme's GNOME appearance settings
    Gnome {
        #[arg(long)]
        skip_icons: bool,
    },

    /// Switch to the next wallpaper, or apply IMAGE directly
    Wallpaper { image: Option<PathBuf> },
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    output::init(command_title(&cli.cmd));
    let ctx = Ctx::new().context("initialise context")?;

    match cli.cmd {
        Cmd::Init => cmd_init(&ctx),
        Cmd::Set {
            theme,
            integrations,
            skip_wallpaper,
        } => cmd_set(&ctx, &theme, integrations.apply_flags(skip_wallpaper)),
        Cmd::Mode {
            mode,
            integrations,
            skip_wallpaper,
        } => cmd_mode(&ctx, mode, integrations.apply_flags(skip_wallpaper)),
        Cmd::Reapply {
            integrations,
            wallpaper,
        } => cmd_reapply(&ctx, integrations.apply_flags(!wallpaper)),
        Cmd::List => cmd_list(&ctx),
        Cmd::Inspect { theme } => inspect::run(&ctx, theme.as_deref()),
        Cmd::Current => cmd_current(&ctx),
        Cmd::Install { url } => {
            let name = install::run(&ctx, &url).context("install theme")?;
            cmd_set(&ctx, &name, ApplyFlags::default())
        }
        Cmd::Remove { theme, yes, force } => {
            cmd_remove(&ctx, &theme, yes, force, ApplyFlags::default())
        }
        Cmd::Update { theme } => cmd_update(&ctx, theme.as_deref()),
        Cmd::Reload => {
            let values = Theme::load_current(&ctx)
                .map(|theme| theme.vars)
                .unwrap_or_default();
            apply::reload::run(&ctx, &values);
            Ok(())
        }
        Cmd::Gnome { skip_icons } => {
            let theme = Theme::load_current(&ctx)?;
            apply::gnome::run(&ctx, &theme, skip_icons)?;
            Ok(())
        }
        Cmd::Wallpaper { image } => match image {
            Some(image) => apply::wallpaper::set(&ctx, &image),
            None => {
                let theme = Theme::load_current(&ctx)?;
                apply::wallpaper::run(&ctx, &theme)
            }
        },
    }
}

/// What this command is doing, for the heading.
const fn command_title(command: &Cmd) -> &'static str {
    match command {
        Cmd::Init => "Prepare the theme directories",
        Cmd::Set { .. } => "Switch theme",
        Cmd::Mode { .. } => "Switch between light and dark",
        Cmd::Reapply { .. } => "Reapply the active theme",
        Cmd::List => "Installed themes",
        Cmd::Inspect { .. } => "Theme palettes",
        Cmd::Current => "The active theme",
        Cmd::Install { .. } => "Install a theme",
        Cmd::Remove { .. } => "Remove a theme",
        Cmd::Update { .. } => "Update installed themes",
        Cmd::Reload => "Reload applications",
        Cmd::Gnome { .. } => "Apply GNOME appearance",
        Cmd::Wallpaper { .. } => "Wallpaper",
    }
}

fn cmd_init(ctx: &Ctx) -> Result<()> {
    let dirs = [
        ("data", &ctx.data_dir),
        ("templates", &ctx.templates_dir),
        ("user-templates", &ctx.user_templates_dir),
        ("generated/live", &ctx.live_dir),
    ];

    let mut rows = Vec::with_capacity(dirs.len());
    for (label, path) in &dirs {
        let existed = path.is_dir();
        std::fs::create_dir_all(path).with_context(|| format!("create {}", path.display()))?;
        let (marker, ink) = output::marker(if existed {
            output::Kind::Changed
        } else {
            output::Kind::Added
        });
        rows.push(
            Row::new(vec![
                ((*label).to_owned(), Ink::Text),
                (path.display().to_string(), Ink::Muted),
                (
                    if existed { "exists" } else { "created" }.to_owned(),
                    if existed { Ink::Muted } else { Ink::Green },
                ),
            ])
            .marked(marker, ink),
        );
    }

    let mut doc = output::document("Prepare the theme directories", "INIT");
    doc.table(
        &[
            output::column("directory"),
            output::column("path"),
            output::column("state"),
        ],
        &rows,
    );
    print!("{}", doc.render());

    Ok(())
}

fn cmd_set(ctx: &Ctx, theme_name: &str, flags: apply::ApplyFlags) -> Result<()> {
    let _lock = lock_themes(ctx)?;
    set_theme(ctx, theme_name, flags)
}

#[cfg(test)]
thread_local! {
    /// Test fail point: stop a switch after `current.theme` is written and
    /// before the render is published, as a logout or crash would.
    static INTERRUPT_BEFORE_PUBLISH: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Switches to `theme_name`; the caller holds the theme lock.
///
/// The order is what makes an interrupted switch safe: the theme is rendered
/// into the stage first (a theme that does not render never becomes the
/// named theme), then `current.theme` names it, then the render is
/// published. A switch stopped between the last two leaves the old render on
/// screen with the new name, and the next `gnist reapply` (at sign-in and on
/// every update) completes it instead of silently reverting it.
fn set_theme(ctx: &Ctx, theme_name: &str, flags: apply::ApplyFlags) -> Result<()> {
    let theme = Theme::load(ctx, theme_name).context("load theme")?;

    let txn = stage_theme(ctx, &theme)?;

    // The selected theme is persistent state, not part of the rendered tree.
    // Written whole and renamed into place, so a reader never sees half.
    write_atomic(
        &ctx.current_theme_file,
        format!("{}\n", theme.name).as_bytes(),
    )
    .context("write current.theme")?;

    #[cfg(test)]
    if INTERRUPT_BEFORE_PUBLISH.with(std::cell::Cell::get) {
        bail!("interrupted before publishing (test fail point)");
    }

    txn.commit().context("commit transaction")?;
    apply_theme(ctx, &theme, flags)
}

fn cmd_reapply(ctx: &Ctx, flags: apply::ApplyFlags) -> Result<()> {
    let _lock = lock_themes(ctx)?;
    let theme = Theme::load_current(ctx)?;
    publish_theme(ctx, &theme)?;
    apply_theme(ctx, &theme, flags)
}

/// Serializes the commands that publish a render or write `current.theme`.
/// Two quick Day/Night presses then run one after the other, the second
/// reading the name the first wrote, instead of interleaving their publishes.
/// The lock is released when the returned file is closed (also on a crash).
fn lock_themes(ctx: &Ctx) -> Result<std::fs::File> {
    let dir = ctx
        .current_theme_file
        .parent()
        .context("current.theme has no directory")?;
    std::fs::create_dir_all(dir).with_context(|| format!("create {}", dir.display()))?;
    let path = dir.join(".gnist.lock");
    let file = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(&path)
        .with_context(|| format!("open {}", path.display()))?;
    loop {
        match rustix::fs::flock(&file, rustix::fs::FlockOperation::LockExclusive) {
            Ok(()) => return Ok(file),
            Err(rustix::io::Errno::INTR) => continue,
            Err(error) => {
                return Err(error).with_context(|| format!("lock {}", path.display()));
            }
        }
    }
}

/// Renders `theme` into a private stage; nothing is visible until commit.
fn stage_theme(ctx: &Ctx, theme: &Theme) -> Result<Transaction> {
    let txn = Transaction::begin(ctx).context("begin transaction")?;
    let template_dirs: Vec<PathBuf> = ctx
        .source_roots
        .iter()
        .map(|root| root.join("templates"))
        .collect();
    render::engine::render_all(
        &template_dirs,
        &ctx.user_templates_dir,
        &theme.root,
        txn.stage(),
        &theme.vars,
    )
    .context("render templates")?;
    theme.stage(txn.stage()).context("stage assets")?;
    Ok(txn)
}

fn publish_theme(ctx: &Ctx, theme: &Theme) -> Result<()> {
    stage_theme(ctx, theme)?
        .commit()
        .context("commit transaction")
}

fn apply_theme(ctx: &Ctx, theme: &Theme, flags: apply::ApplyFlags) -> Result<()> {
    if flags.skip_apply {
        return Ok(());
    }

    // Replacing the symlinks wakes GTK file monitors in apps that do not load
    // the Gnist GIO module.
    apply::gtk_css::refresh_user_css_links(ctx);

    // Settings must be published before the change signal reaches already
    // running GTK clients; otherwise a reload can race the new preferences.
    if !flags.skip_gnome
        && let Err(error) = apply::gnome::run(ctx, theme, flags.skip_icons)
    {
        // The theme is already published. Keep its CSS signal and remaining
        // integrations moving, but make the missing platform settings visible.
        eprintln!(
            "{}",
            output::warn(&format!("accessibility settings failed: {error:#}"))
        );
    }

    if let Err(e) = apply::gtk_css::emit_current(ctx, Some(theme)) {
        eprintln!("{}", output::warn(&format!("gtk-css reload failed: {e:#}")));
    }

    if !flags.skip_reload {
        apply::reload::run(ctx, &theme.vars);
    }

    if !flags.skip_wallpaper
        && let Err(e) = apply::wallpaper::show_for_theme(ctx, theme)
    {
        eprintln!(
            "{}",
            output::warn(&format!("wallpaper apply failed: {e:#}"))
        );
    }

    Ok(())
}

fn cmd_list(ctx: &Ctx) -> Result<()> {
    let current = read_current_name(ctx);
    let names = theme::list_names(ctx)?;

    if names.is_empty() {
        eprintln!("No themes installed in {}", ctx.data_dir.display());
        return Ok(());
    }

    for name in names {
        let marker = if Some(&name) == current.as_ref() {
            " (current)"
        } else {
            ""
        };
        println!("{name}{marker}");
    }

    Ok(())
}

fn cmd_current(ctx: &Ctx) -> Result<()> {
    match read_current_name(ctx) {
        Some(name) => {
            println!("{name}");
            Ok(())
        }
        None => {
            eprintln!("No current theme set");
            std::process::exit(1);
        }
    }
}

fn cmd_remove(
    ctx: &Ctx,
    name: &str,
    yes: bool,
    force: bool,
    apply_flags: ApplyFlags,
) -> Result<()> {
    let path = config_theme_path(ctx, name, "remove")?;
    let removing_current = read_current_name(ctx).as_deref() == Some(name);

    if !force && removing_current {
        anyhow::bail!("'{name}' is the current theme. Set another theme or use --force");
    }

    let fallback = if removing_current {
        lower_theme_root(ctx, name)
            .map(|root| Theme::load_root(root, name))
            .transpose()
            .context("load lower-precedence replacement theme")?
    } else {
        None
    };

    if !yes
        && !output::confirm(&format!("Remove '{name}'  ({})?", path.display()))
            .context("read confirmation")?
    {
        eprintln!("Cancelled");
        return Ok(());
    }

    let _lock = lock_themes(ctx)?;
    if removing_current {
        if let Some(theme) = fallback {
            publish_theme(ctx, &theme).context("publish lower-precedence replacement theme")?;
            std::fs::remove_dir_all(&path).with_context(|| format!("remove {}", path.display()))?;
            remove_file_if_exists(&ctx.background_link)?;
            apply_theme(
                ctx,
                &theme,
                ApplyFlags {
                    skip_wallpaper: false,
                    ..apply_flags
                },
            )?;
        } else {
            std::fs::remove_dir_all(&path).with_context(|| format!("remove {}", path.display()))?;
            clear_current_state(ctx)?;
        }
    } else {
        std::fs::remove_dir_all(&path).with_context(|| format!("remove {}", path.display()))?;
    }

    println!("Removed {name}");
    Ok(())
}

fn cmd_update(ctx: &Ctx, theme: Option<&str>) -> Result<()> {
    let targets: Vec<String> = match theme {
        Some(name) => {
            config_theme_path(ctx, name, "update")?;
            vec![name.to_owned()]
        }
        None => list_installed(&ctx.data_dir)?
            .into_iter()
            .filter(|n| ctx.data_dir.join(n).join(".git").is_dir())
            .collect(),
    };

    if targets.is_empty() {
        eprintln!("No git-installed themes to update");
        return Ok(());
    }

    let mut failed = Vec::new();
    for name in &targets {
        let path = ctx.data_dir.join(name);
        if !path.join(".git").is_dir() {
            eprintln!(
                "{}",
                output::pending(&format!("{name} — no .git, not installed via gnist"))
            );
            continue;
        }
        // On a terminal the line is replaced by its outcome, so the finished
        // list reads as results rather than as a transcript of what was tried.
        output::working(name);
        match install::git_pull(&path) {
            Ok(()) => output::settled(&output::ok(name)),
            Err(e) => {
                output::settled(&output::fail(name));
                eprintln!("  {e:#}");
                failed.push(name.clone());
            }
        }
    }

    if !failed.is_empty() {
        anyhow::bail!(
            "{} theme(s) failed to update: {}",
            failed.len(),
            failed.join(", ")
        );
    }
    Ok(())
}

fn config_theme_path(ctx: &Ctx, name: &str, operation: &str) -> Result<PathBuf> {
    let path = ctx.data_dir.join(name);
    if path.is_dir() {
        return Ok(path);
    }

    if let Some(lower) = lower_theme_root(ctx, name) {
        anyhow::bail!(
            "cannot {operation} theme '{name}': it is visible only from lower-precedence source {}; only config-installed themes in {} can be modified",
            lower.display(),
            ctx.data_dir.display()
        );
    }

    anyhow::bail!("theme not found: {}", path.display())
}

fn lower_theme_root(ctx: &Ctx, name: &str) -> Option<PathBuf> {
    ctx.source_roots
        .iter()
        .skip(1)
        .map(|source| source.join("data").join(name))
        .find(|root| root.is_dir())
}

fn clear_current_state(ctx: &Ctx) -> Result<()> {
    remove_file_if_exists(&ctx.current_theme_file)?;
    remove_file_if_exists(&ctx.current_link)?;

    match std::fs::symlink_metadata(&ctx.live_dir) {
        Ok(metadata) if metadata.file_type().is_dir() => std::fs::remove_dir_all(&ctx.live_dir)
            .with_context(|| format!("remove {}", ctx.live_dir.display()))?,
        Ok(_) => std::fs::remove_file(&ctx.live_dir)
            .with_context(|| format!("remove {}", ctx.live_dir.display()))?,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => {
            return Err(error).with_context(|| format!("inspect {}", ctx.live_dir.display()));
        }
    }

    remove_file_if_exists(&ctx.background_link)?;

    Ok(())
}

fn remove_file_if_exists(path: &Path) -> Result<()> {
    match std::fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error).with_context(|| format!("remove {}", path.display())),
    }
}

fn list_installed(data_dir: &Path) -> Result<Vec<String>> {
    let rd = match std::fs::read_dir(data_dir) {
        Ok(rd) => rd,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(e).with_context(|| format!("read {}", data_dir.display())),
    };
    Ok(rd
        .flatten()
        .filter(|e| e.path().is_dir())
        .filter_map(|e| e.file_name().into_string().ok())
        .collect())
}

/// Writes `path` whole: a uniquely named temporary file in the same
/// directory, flushed to disk, renamed over `path`, then the directory
/// flushed, so neither a concurrent writer nor a power loss leaves it empty
/// or half written.
fn write_atomic(path: &std::path::Path, contents: &[u8]) -> Result<()> {
    use std::io::Write;
    let dir = path
        .parent()
        .with_context(|| format!("{} has no directory", path.display()))?;
    let mut tmp = tempfile::Builder::new()
        .prefix(".gnist.")
        .suffix(".tmp")
        .tempfile_in(dir)
        .with_context(|| format!("create a temporary file in {}", dir.display()))?;
    tmp.write_all(contents)
        .and_then(|()| tmp.as_file().sync_all())
        .with_context(|| format!("write {}", tmp.path().display()))?;
    tmp.persist(path)
        .map_err(|error| error.error)
        .with_context(|| format!("rename into {}", path.display()))?;
    std::fs::File::open(dir)
        .and_then(|dir| dir.sync_all())
        .with_context(|| format!("sync {}", dir.display()))
}

/// The light or dark counterpart of `theme`: its `light_theme` /
/// `dark_theme` key, else `<name>-light` / the name without `-light`.
/// The counterpart must exist and be of the wanted mode.
fn counterpart(ctx: &Ctx, theme: &Theme, want_light: bool) -> Result<String> {
    let key = if want_light {
        "light_theme"
    } else {
        "dark_theme"
    };
    let name = match theme.vars.get(key) {
        Some(render::value::Value::Str(name)) if !name.trim().is_empty() => name.trim().to_owned(),
        _ if want_light => format!("{}-light", theme.name),
        _ => match theme.name.strip_suffix("-light") {
            Some(base) => base.to_owned(),
            None => bail!(
                "theme '{}' names no dark counterpart (set {key} in its colors.toml)",
                theme.name
            ),
        },
    };
    let other = Theme::load(ctx, &name).with_context(|| {
        format!(
            "theme '{}' has no {} counterpart '{name}' (set {key} in its colors.toml)",
            theme.name,
            if want_light { "light" } else { "dark" }
        )
    })?;
    if other.is_light != want_light {
        bail!(
            "'{name}', the counterpart of '{}', is not a {} theme",
            theme.name,
            if want_light { "light" } else { "dark" }
        );
    }
    Ok(name)
}

fn cmd_mode(ctx: &Ctx, mode: ModeArg, flags: apply::ApplyFlags) -> Result<()> {
    let _lock = lock_themes(ctx)?;
    // Read under the lock: a second toggle sees the first one's result.
    let current = read_current_name(ctx).context("no current theme: run `gnist set` first")?;
    let theme =
        Theme::load(ctx, &current).with_context(|| format!("load current theme '{current}'"))?;
    let want_light = match mode {
        ModeArg::Light => true,
        ModeArg::Dark => false,
        ModeArg::Toggle => !theme.is_light,
    };
    if want_light == theme.is_light {
        println!(
            "{current} is already {}",
            if want_light { "light" } else { "dark" }
        );
        return Ok(());
    }
    let target = counterpart(ctx, &theme, want_light)?;
    set_theme(ctx, &target, flags)
}

fn read_current_name(ctx: &Ctx) -> Option<String> {
    let raw = std::fs::read_to_string(&ctx.current_theme_file).ok()?;
    let trimmed = raw.trim();
    (!trimmed.is_empty()).then(|| trimmed.to_owned())
}

#[cfg(test)]
mod tests {
    use super::{
        Cli, Cmd, ModeArg, cmd_mode, cmd_reapply, cmd_remove, cmd_set, cmd_update,
        read_current_name,
    };
    use crate::{apply::ApplyFlags, ctx::Ctx, theme::Theme};
    use clap::Parser;
    use std::{ffi::OsStr, fs};

    fn test_ctx(dir: &tempfile::TempDir) -> Ctx {
        let config_home = dir.path().join("config");
        let data_home = dir.path().join("data");
        Ctx::from_xdg(
            dir.path(),
            Some(config_home.as_os_str()),
            Some(data_home.as_os_str()),
            Some(OsStr::new("")),
        )
    }

    fn write_theme(root: &std::path::Path, name: &str, accent: &str) {
        let theme = root.join("data").join(name);
        fs::create_dir_all(&theme).unwrap();
        fs::write(
            theme.join("colors.toml"),
            format!("accent = \"{accent}\"\n"),
        )
        .unwrap();
    }

    fn skip_apply() -> ApplyFlags {
        ApplyFlags {
            skip_apply: true,
            skip_wallpaper: true,
            ..ApplyFlags::default()
        }
    }

    fn mode_ctx(dir: &tempfile::TempDir) -> Ctx {
        let ctx = test_ctx(dir);
        let data_root = &ctx.source_roots[1];
        fs::create_dir_all(data_root.join("templates")).unwrap();
        fs::write(
            data_root.join("templates/app.conf.tpl"),
            "mode={{ mode }}\n",
        )
        .unwrap();
        fs::create_dir_all(ctx.current_theme_file.parent().unwrap()).unwrap();
        ctx
    }

    fn write_mode_theme(ctx: &Ctx, name: &str, light: bool, extra: &str) {
        let theme = ctx.source_roots[1].join("data").join(name);
        fs::create_dir_all(&theme).unwrap();
        fs::write(
            theme.join("colors.toml"),
            format!("accent = \"#123456\"\n{extra}"),
        )
        .unwrap();
        if light {
            fs::write(theme.join("light.mode"), "").unwrap();
        }
    }

    #[test]
    fn mode_parses_light_dark_and_toggle() {
        for (word, mode) in [
            ("light", ModeArg::Light),
            ("dark", ModeArg::Dark),
            ("toggle", ModeArg::Toggle),
        ] {
            let cli = Cli::try_parse_from(["gnist", "mode", word]).unwrap();
            let Cmd::Mode { mode: parsed, .. } = cli.cmd else {
                panic!("not mode")
            };
            assert_eq!(parsed, mode);
        }
        assert!(Cli::try_parse_from(["gnist", "mode", "sepia"]).is_err());
    }

    #[test]
    fn mode_switches_by_name_rule_and_back() {
        let dir = tempfile::tempdir().unwrap();
        let ctx = mode_ctx(&dir);
        write_mode_theme(&ctx, "herda", false, "");
        write_mode_theme(&ctx, "herda-light", true, "");
        cmd_set(&ctx, "herda", skip_apply()).unwrap();

        cmd_mode(&ctx, ModeArg::Toggle, skip_apply()).unwrap();
        assert_eq!(read_current_name(&ctx).as_deref(), Some("herda-light"));
        assert_eq!(
            fs::read_to_string(ctx.live_dir.join("app.conf")).unwrap(),
            "mode=light\n"
        );

        // Already light: nothing changes.
        cmd_mode(&ctx, ModeArg::Light, skip_apply()).unwrap();
        assert_eq!(read_current_name(&ctx).as_deref(), Some("herda-light"));

        cmd_mode(&ctx, ModeArg::Dark, skip_apply()).unwrap();
        assert_eq!(read_current_name(&ctx).as_deref(), Some("herda"));
        assert_eq!(
            fs::read_to_string(ctx.live_dir.join("app.conf")).unwrap(),
            "mode=dark\n"
        );
        assert_eq!(leftover_temporaries(&ctx), Vec::<String>::new());
    }

    fn leftover_temporaries(ctx: &Ctx) -> Vec<String> {
        fs::read_dir(ctx.current_theme_file.parent().unwrap())
            .unwrap()
            .flatten()
            .map(|entry| entry.file_name().to_string_lossy().into_owned())
            .filter(|name| name.ends_with(".tmp"))
            .collect()
    }

    #[test]
    fn an_interrupted_switch_completes_at_the_next_reapply() {
        let dir = tempfile::tempdir().unwrap();
        let ctx = mode_ctx(&dir);
        write_mode_theme(&ctx, "herda", false, "");
        write_mode_theme(&ctx, "herda-light", true, "");
        cmd_set(&ctx, "herda", skip_apply()).unwrap();

        // Stopped after the name is written, before the render is published.
        super::INTERRUPT_BEFORE_PUBLISH.with(|stop| stop.set(true));
        let error = cmd_mode(&ctx, ModeArg::Light, skip_apply()).unwrap_err();
        super::INTERRUPT_BEFORE_PUBLISH.with(|stop| stop.set(false));
        assert!(format!("{error:#}").contains("interrupted"), "{error:#}");
        assert_eq!(read_current_name(&ctx).as_deref(), Some("herda-light"));
        assert_eq!(
            fs::read_to_string(ctx.live_dir.join("app.conf")).unwrap(),
            "mode=dark\n"
        );

        // The next reapply finishes the person's choice instead of reverting it.
        cmd_reapply(&ctx, skip_apply()).unwrap();
        assert_eq!(read_current_name(&ctx).as_deref(), Some("herda-light"));
        assert_eq!(
            fs::read_to_string(ctx.live_dir.join("app.conf")).unwrap(),
            "mode=light\n"
        );
    }

    #[test]
    fn a_theme_that_does_not_render_is_never_named() {
        let dir = tempfile::tempdir().unwrap();
        let ctx = mode_ctx(&dir);
        write_mode_theme(&ctx, "herda", false, "");
        cmd_set(&ctx, "herda", skip_apply()).unwrap();
        // A template that is not UTF-8 cannot be read, so the render fails.
        fs::write(
            ctx.source_roots[1].join("templates/broken.conf.tpl"),
            b"\xff\xfe\n",
        )
        .unwrap();
        write_mode_theme(&ctx, "herda-light", true, "");
        let error = cmd_set(&ctx, "herda-light", skip_apply()).unwrap_err();
        assert!(
            format!("{error:#}").contains("render templates"),
            "{error:#}"
        );
        assert_eq!(read_current_name(&ctx).as_deref(), Some("herda"));
        assert_eq!(
            fs::read_to_string(ctx.live_dir.join("app.conf")).unwrap(),
            "mode=dark\n"
        );
    }

    #[test]
    fn the_theme_lock_excludes_a_second_holder() {
        use rustix::fs::{FlockOperation, flock};
        let dir = tempfile::tempdir().unwrap();
        let ctx = mode_ctx(&dir);
        let held = super::lock_themes(&ctx).unwrap();
        let other = fs::OpenOptions::new()
            .write(true)
            .open(ctx.current_theme_file.parent().unwrap().join(".gnist.lock"))
            .unwrap();
        assert_eq!(
            flock(&other, FlockOperation::NonBlockingLockExclusive),
            Err(rustix::io::Errno::WOULDBLOCK)
        );
        drop(held);
        flock(&other, FlockOperation::NonBlockingLockExclusive).unwrap();
    }

    #[test]
    fn quick_toggles_run_one_after_the_other() {
        let dir = tempfile::tempdir().unwrap();
        let ctx = mode_ctx(&dir);
        write_mode_theme(&ctx, "herda", false, "");
        write_mode_theme(&ctx, "herda-light", true, "");
        cmd_set(&ctx, "herda", skip_apply()).unwrap();
        std::thread::scope(|scope| {
            for _ in 0..6 {
                scope.spawn(|| cmd_mode(&ctx, ModeArg::Toggle, skip_apply()).unwrap());
            }
        });
        // Six toggles, each reading the previous one's result: back at Night,
        // and the published render agrees with the name.
        assert_eq!(read_current_name(&ctx).as_deref(), Some("herda"));
        assert_eq!(
            fs::read_to_string(ctx.live_dir.join("app.conf")).unwrap(),
            "mode=dark\n"
        );
        assert_eq!(leftover_temporaries(&ctx), Vec::<String>::new());
    }

    #[test]
    fn mode_follows_an_explicit_pair_key() {
        let dir = tempfile::tempdir().unwrap();
        let ctx = mode_ctx(&dir);
        write_mode_theme(&ctx, "night", false, "light_theme = \"day\"\n");
        write_mode_theme(&ctx, "day", true, "dark_theme = \"night\"\n");
        cmd_set(&ctx, "night", skip_apply()).unwrap();
        cmd_mode(&ctx, ModeArg::Toggle, skip_apply()).unwrap();
        assert_eq!(read_current_name(&ctx).as_deref(), Some("day"));
        cmd_mode(&ctx, ModeArg::Toggle, skip_apply()).unwrap();
        assert_eq!(read_current_name(&ctx).as_deref(), Some("night"));
    }

    #[test]
    fn mode_refuses_a_missing_or_wrong_counterpart() {
        let dir = tempfile::tempdir().unwrap();
        let ctx = mode_ctx(&dir);
        write_mode_theme(&ctx, "lonely", false, "");
        write_mode_theme(&ctx, "odd", false, "light_theme = \"lonely\"\n");
        cmd_set(&ctx, "lonely", skip_apply()).unwrap();
        let error = cmd_mode(&ctx, ModeArg::Light, skip_apply()).unwrap_err();
        assert!(
            format!("{error:#}").contains("no light counterpart"),
            "{error:#}"
        );
        cmd_set(&ctx, "odd", skip_apply()).unwrap();
        let error = cmd_mode(&ctx, ModeArg::Light, skip_apply()).unwrap_err();
        assert!(
            format!("{error:#}").contains("is not a light theme"),
            "{error:#}"
        );
        assert_eq!(read_current_name(&ctx).as_deref(), Some("odd"));
    }

    #[test]
    fn reapply_skips_wallpaper_by_default_and_can_opt_in() {
        let cli = Cli::try_parse_from(["gnist", "reapply", "--skip-icons"]).unwrap();
        let Cmd::Reapply {
            integrations,
            wallpaper,
        } = cli.cmd
        else {
            panic!("expected reapply command");
        };
        let flags = integrations.apply_flags(!wallpaper);
        assert!(flags.skip_icons);
        assert!(flags.skip_wallpaper);

        let cli = Cli::try_parse_from(["gnist", "reapply", "--wallpaper"]).unwrap();
        let Cmd::Reapply {
            integrations,
            wallpaper,
        } = cli.cmd
        else {
            panic!("expected reapply command");
        };
        assert!(!integrations.apply_flags(!wallpaper).skip_wallpaper);
    }

    #[test]
    fn reapply_fails_clearly_without_a_current_theme() {
        let dir = tempfile::tempdir().unwrap();
        let ctx = test_ctx(&dir);

        let error = cmd_reapply(&ctx, skip_apply()).unwrap_err();

        assert!(error.to_string().contains("current theme is not set"));
    }

    #[test]
    fn reapply_republishes_fallback_theme_without_changing_current_state() {
        let dir = tempfile::tempdir().unwrap();
        let ctx = test_ctx(&dir);
        let data_root = &ctx.source_roots[1];
        let theme = data_root.join("data/active");
        let templates = data_root.join("templates");
        fs::create_dir_all(&theme).unwrap();
        fs::create_dir_all(&templates).unwrap();
        fs::create_dir_all(ctx.current_theme_file.parent().unwrap()).unwrap();
        fs::write(theme.join("colors.toml"), "accent = \"#123456\"\n").unwrap();
        fs::write(templates.join("app.conf.tpl"), "accent={{ accent }}\n").unwrap();
        fs::write(&ctx.current_theme_file, "active\n").unwrap();

        cmd_reapply(&ctx, skip_apply()).unwrap();

        assert_eq!(
            fs::read_to_string(ctx.live_dir.join("app.conf")).unwrap(),
            "accent=#123456\n"
        );
        assert_eq!(
            fs::read_to_string(&ctx.current_theme_file).unwrap(),
            "active\n"
        );
    }

    #[test]
    fn named_lower_layer_themes_have_specific_mutation_diagnostics() {
        let dir = tempfile::tempdir().unwrap();
        let ctx = test_ctx(&dir);
        write_theme(&ctx.source_roots[1], "shared", "#222222");

        for (operation, error) in [
            (
                "remove",
                cmd_remove(&ctx, "shared", true, true, skip_apply()).unwrap_err(),
            ),
            ("update", cmd_update(&ctx, Some("shared")).unwrap_err()),
        ] {
            let diagnostic = error.to_string();
            assert!(diagnostic.contains(&format!("cannot {operation} theme 'shared'")));
            assert!(diagnostic.contains("lower-precedence source"));
            assert!(diagnostic.contains(&ctx.source_roots[1].display().to_string()));
            assert!(diagnostic.contains(&ctx.data_dir.display().to_string()));
            assert!(!diagnostic.contains("not found"));
        }
    }

    #[test]
    fn force_removing_current_override_republishes_lower_theme() {
        let dir = tempfile::tempdir().unwrap();
        let ctx = test_ctx(&dir);
        write_theme(&ctx.source_roots[0], "shared", "#111111");
        write_theme(&ctx.source_roots[1], "shared", "#222222");
        fs::create_dir_all(&ctx.templates_dir).unwrap();
        fs::write(
            ctx.templates_dir.join("app.conf.tpl"),
            "accent={{ accent }}\n",
        )
        .unwrap();
        cmd_set(&ctx, "shared", skip_apply()).unwrap();
        fs::write(&ctx.background_link, "stale background").unwrap();

        cmd_remove(&ctx, "shared", true, true, skip_apply()).unwrap();

        assert_eq!(read_current_name(&ctx).as_deref(), Some("shared"));
        assert_eq!(
            fs::read_to_string(ctx.live_dir.join("app.conf")).unwrap(),
            "accent=#222222\n"
        );
        assert_eq!(
            Theme::load_current(&ctx).unwrap().root,
            ctx.source_roots[1].join("data/shared")
        );
        assert!(fs::symlink_metadata(&ctx.background_link).is_err());
    }

    #[test]
    fn fallback_render_failure_preserves_current_publication_and_source() {
        let dir = tempfile::tempdir().unwrap();
        let ctx = test_ctx(&dir);
        write_theme(&ctx.source_roots[0], "shared", "#111111");
        write_theme(&ctx.source_roots[1], "shared", "#222222");
        fs::create_dir_all(&ctx.templates_dir).unwrap();
        fs::write(
            ctx.templates_dir.join("app.conf.tpl"),
            "accent={{ accent }}\n",
        )
        .unwrap();
        cmd_set(&ctx, "shared", skip_apply()).unwrap();
        fs::write(ctx.templates_dir.join("broken.tpl"), [0xff]).unwrap();
        fs::write(&ctx.background_link, "current background").unwrap();

        let error = cmd_remove(&ctx, "shared", true, true, skip_apply()).unwrap_err();

        assert!(
            error
                .to_string()
                .contains("publish lower-precedence replacement theme")
        );
        assert!(ctx.source_roots[0].join("data/shared").is_dir());
        assert_eq!(read_current_name(&ctx).as_deref(), Some("shared"));
        assert_eq!(
            fs::read_to_string(ctx.live_dir.join("app.conf")).unwrap(),
            "accent=#111111\n"
        );
        assert!(ctx.current_link.join("app.conf").is_file());
        assert_eq!(
            fs::read_to_string(&ctx.background_link).unwrap(),
            "current background"
        );
    }

    #[test]
    fn force_removing_current_theme_without_fallback_clears_active_state() {
        let dir = tempfile::tempdir().unwrap();
        let ctx = test_ctx(&dir);
        write_theme(&ctx.source_roots[0], "only", "#111111");
        fs::create_dir_all(&ctx.templates_dir).unwrap();
        fs::write(ctx.templates_dir.join("app.conf.tpl"), "configured\n").unwrap();
        cmd_set(&ctx, "only", skip_apply()).unwrap();
        fs::write(&ctx.background_link, "stale background").unwrap();

        cmd_remove(&ctx, "only", true, true, skip_apply()).unwrap();

        assert_eq!(read_current_name(&ctx), None);
        assert!(fs::symlink_metadata(&ctx.current_theme_file).is_err());
        assert!(fs::symlink_metadata(&ctx.current_link).is_err());
        assert!(fs::symlink_metadata(&ctx.live_dir).is_err());
        assert!(fs::symlink_metadata(&ctx.background_link).is_err());
        assert!(Theme::load_current(&ctx).is_err());
    }
}

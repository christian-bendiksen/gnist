use anyhow::{Context, Result};
use clap::{Args, Parser, Subcommand};
use std::path::{Path, PathBuf};

mod apply;
mod ctx;
mod install;
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
    let ctx = Ctx::new().context("initialise context")?;

    match cli.cmd {
        Cmd::Init => cmd_init(&ctx),
        Cmd::Set {
            theme,
            integrations,
            skip_wallpaper,
        } => cmd_set(&ctx, &theme, integrations.apply_flags(skip_wallpaper)),
        Cmd::Reapply {
            integrations,
            wallpaper,
        } => cmd_reapply(&ctx, integrations.apply_flags(!wallpaper)),
        Cmd::List => cmd_list(&ctx),
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
            apply::gnome::run(&theme, skip_icons);
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

fn cmd_init(ctx: &Ctx) -> Result<()> {
    let dirs = [
        ("data", &ctx.data_dir),
        ("templates", &ctx.templates_dir),
        ("user-templates", &ctx.user_templates_dir),
        ("generated/live", &ctx.live_dir),
    ];

    for (label, path) in &dirs {
        let existed = path.is_dir();
        std::fs::create_dir_all(path).with_context(|| format!("create {}", path.display()))?;
        let status = if existed { "exists " } else { "created" };
        println!("  {status}  {label:<16} {}", path.display());
    }

    Ok(())
}

fn cmd_set(ctx: &Ctx, theme_name: &str, flags: apply::ApplyFlags) -> Result<()> {
    let theme = Theme::load(ctx, theme_name).context("load theme")?;

    publish_theme(ctx, &theme)?;

    // The selected theme is persistent state, not part of the rendered tree.
    std::fs::write(&ctx.current_theme_file, format!("{}\n", theme.name))
        .context("write current.theme")?;

    apply_theme(ctx, &theme, flags)
}

fn cmd_reapply(ctx: &Ctx, flags: apply::ApplyFlags) -> Result<()> {
    let theme = Theme::load_current(ctx)?;
    publish_theme(ctx, &theme)?;
    apply_theme(ctx, &theme, flags)
}

fn publish_theme(ctx: &Ctx, theme: &Theme) -> Result<()> {
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
    txn.commit().context("commit transaction")?;
    Ok(())
}

fn apply_theme(ctx: &Ctx, theme: &Theme, flags: apply::ApplyFlags) -> Result<()> {
    if flags.skip_apply {
        return Ok(());
    }

    // Replacing the symlinks wakes GTK file monitors in apps that do not load
    // the Gnist GIO module.
    apply::gtk_css::refresh_user_css_links(ctx);

    if let Err(e) = apply::gtk_css::emit_current(ctx, Some(theme)) {
        eprintln!("warn: gtk-css reload failed: {e:#}");
    }

    if !flags.skip_gnome {
        apply::gnome::run(theme, flags.skip_icons);
    }

    if !flags.skip_reload {
        apply::reload::run(ctx, &theme.vars);
    }

    if !flags.skip_wallpaper
        && let Err(e) = apply::wallpaper::run(ctx, theme)
    {
        eprintln!("warn: wallpaper apply failed: {e:#}");
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

    if !yes {
        eprint!("Remove '{name}'  ({})? [y/N] ", path.display());
        std::io::Write::flush(&mut std::io::stderr()).ok();
        let mut line = String::new();
        std::io::stdin()
            .read_line(&mut line)
            .context("read confirmation")?;
        if !matches!(line.trim().to_lowercase().as_str(), "y" | "yes") {
            eprintln!("Cancelled");
            return Ok(());
        }
    }

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
            eprintln!("skip  {name} (no .git - not installed via gnist)");
            continue;
        }
        print!("pull  {name} ... ");
        std::io::Write::flush(&mut std::io::stdout()).ok();
        match install::git_pull(&path) {
            Ok(()) => println!("ok"),
            Err(e) => {
                println!("failed");
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

fn read_current_name(ctx: &Ctx) -> Option<String> {
    let raw = std::fs::read_to_string(&ctx.current_theme_file).ok()?;
    let trimmed = raw.trim();
    (!trimmed.is_empty()).then(|| trimmed.to_owned())
}

#[cfg(test)]
mod tests {
    use super::{Cli, Cmd, cmd_reapply, cmd_remove, cmd_set, cmd_update, read_current_name};
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

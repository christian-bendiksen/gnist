use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use std::path::Path;

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
        #[arg(long)]
        skip_apply: bool,
        #[arg(long)]
        skip_gnome: bool,
        #[arg(long)]
        skip_icons: bool,
        #[arg(long)]
        skip_reload: bool,
        #[arg(long)]
        skip_wallpaper: bool,
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

    /// Switch to the next wallpaper in the active theme
    Wallpaper,
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    let ctx = Ctx::new().context("initialise context")?;

    match cli.cmd {
        Cmd::Init => cmd_init(&ctx),
        Cmd::Set {
            theme,
            skip_apply,
            skip_gnome,
            skip_icons,
            skip_reload,
            skip_wallpaper,
        } => cmd_set(
            &ctx,
            &theme,
            apply::ApplyFlags {
                skip_apply,
                skip_gnome,
                skip_icons,
                skip_reload,
                skip_wallpaper,
            },
        ),
        Cmd::List => cmd_list(&ctx),
        Cmd::Current => cmd_current(&ctx),
        Cmd::Install { url } => {
            let name = install::run(&ctx, &url).context("install theme")?;
            cmd_set(&ctx, &name, ApplyFlags::default())
        }
        Cmd::Remove { theme, yes, force } => cmd_remove(&ctx, &theme, yes, force),
        Cmd::Update { theme } => cmd_update(&ctx, theme.as_deref()),
        Cmd::Reload => {
            apply::reload::run(&ctx);
            Ok(())
        }
        Cmd::Gnome { skip_icons } => {
            let theme = Theme::load_current(&ctx)?;
            apply::gnome::run(&theme, skip_icons);
            Ok(())
        }
        Cmd::Wallpaper => {
            let theme = Theme::load_current(&ctx)?;
            apply::wallpaper::run(&ctx, &theme)
        }
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
    let theme = Theme::load(&ctx.data_dir, theme_name).context("load theme")?;

    let txn = Transaction::begin(ctx).context("begin transaction")?;
    render::engine::render_all(
        &ctx.templates_dir,
        &ctx.user_templates_dir,
        &theme.root,
        txn.stage(),
        &theme.vars,
    )
    .context("render templates")?;
    theme.stage(txn.stage()).context("stage assets")?;
    txn.commit().context("commit transaction")?;

    // The selected theme is persistent state, not part of the rendered tree.
    std::fs::write(&ctx.current_theme_file, format!("{}\n", theme.name))
        .context("write current.theme")?;

    if flags.skip_apply {
        return Ok(());
    }

    // Replacing the symlinks wakes GTK file monitors in apps that do not load
    // the Gnist GIO module.
    apply::gtk_css::refresh_user_css_links(ctx);

    if let Err(e) = apply::gtk_css::emit_current(ctx, Some(&theme)) {
        eprintln!("warn: gtk-css reload failed: {e:#}");
    }

    if !flags.skip_gnome {
        apply::gnome::run(&theme, flags.skip_icons);
    }

    if !flags.skip_reload {
        apply::reload::run(ctx);
    }

    if !flags.skip_wallpaper && let Err(e) = apply::wallpaper::run(ctx, &theme) {
        eprintln!("warn: wallpaper apply failed: {e:#}");
    }

    Ok(())
}

fn cmd_list(ctx: &Ctx) -> Result<()> {
    let current = read_current_name(ctx);
    let mut names = list_installed(&ctx.data_dir)?;
    names.sort();

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

fn cmd_remove(ctx: &Ctx, name: &str, yes: bool, force: bool) -> Result<()> {
    let path = ctx.data_dir.join(name);
    if !path.is_dir() {
        anyhow::bail!("theme not found: {}", path.display());
    }

    if !force && read_current_name(ctx).as_deref() == Some(name) {
        anyhow::bail!("'{name}' is the current theme. Set another theme or use --force");
    }

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

    std::fs::remove_dir_all(&path).with_context(|| format!("remove {}", path.display()))?;
    println!("Removed {name}");
    Ok(())
}

fn cmd_update(ctx: &Ctx, theme: Option<&str>) -> Result<()> {
    let targets: Vec<String> = match theme {
        Some(name) => {
            let path = ctx.data_dir.join(name);
            if !path.is_dir() {
                anyhow::bail!("theme not found: {}", path.display());
            }
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

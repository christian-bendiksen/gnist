mod border;
pub mod colors;
pub mod palette;
mod vars;

use crate::ctx::Ctx;
use crate::render::value::Value;
use anyhow::{Context, Result, bail};
use std::{
    collections::{BTreeSet, HashMap},
    fs,
    path::{Component, Path, PathBuf},
};
use vars::build_vars_from_colors;

pub const LIGHT_MODE_FILE: &str = "light.mode";
pub const ICONS_THEME_FILE: &str = "icons.theme";
pub const BACKGROUNDS_DIR: &str = "backgrounds";
pub const PREVIEW_FILE: &str = "preview.png";

/// Theme files and directories that are metadata or repository clutter and are
/// never published verbatim into `generated/live/`.
pub const METADATA: &[&str] = &[
    ".git",
    "colors.toml",
    LIGHT_MODE_FILE,
    ICONS_THEME_FILE,
    BACKGROUNDS_DIR,
    "gnome.toml",
    "wallpaper.toml",
    "theme.toml",
    PREVIEW_FILE,
    "preview-unlock.png",
    "unlock.png",
    "keyboard.rgb",
    "shell.toml",
];

/// Supported wallpaper file extensions (matched case-insensitively).
pub const WALLPAPER_EXTENSIONS: &[&str] = &["png", "jpg", "jpeg", "webp", "avif", "jxl", "gif"];

/// Returns true if a path **relative to a theme root** is theme metadata or
/// repository clutter that must not be published into `generated/live/`.
pub fn is_metadata(rel: &Path) -> bool {
    rel.components().any(|component| {
        let Some(name) = component.as_os_str().to_str() else {
            return false;
        };
        if METADATA.contains(&name) {
            return true;
        }
        let upper = name.to_ascii_uppercase();
        if upper.starts_with("README") || upper.starts_with("LICENSE") {
            return true;
        }
        name.starts_with("shell.") && name.ends_with(".toml")
    })
}

/// Returns true if `path` is a regular file with a supported wallpaper image extension.
pub fn is_wallpaper_image(path: &Path) -> bool {
    path.is_file()
        && path
            .extension()
            .and_then(|ext| ext.to_str())
            .is_some_and(|ext| {
                WALLPAPER_EXTENSIONS
                    .iter()
                    .any(|allowed| ext.eq_ignore_ascii_case(allowed))
            })
}

#[derive(Clone, Debug)]
pub struct Theme {
    pub name: String,
    pub display_name: Option<String>,
    pub root: PathBuf,
    pub vars: HashMap<String, Value>,
    pub is_light: bool,
    pub generated: bool,
    pub icon_theme: Option<String>,
    pub preview: Option<PathBuf>,
    pub backgrounds_dir: Option<PathBuf>,
}

/// Locate the highest-precedence directory for `name`, rejecting path traversal.
pub fn find_root(ctx: &Ctx, name: &str) -> Option<PathBuf> {
    if name.is_empty() || name.starts_with('.') {
        return None;
    }
    let mut components = Path::new(name).components();
    match (components.next(), components.next()) {
        (Some(Component::Normal(_)), None) => {}
        _ => return None,
    }
    ctx.source_roots
        .iter()
        .map(|source| source.join("data").join(name))
        .find(|root| root.is_dir())
}

impl Theme {
    pub fn load(ctx: &Ctx, name: &str) -> Result<Self> {
        Self::load_from_roots(&ctx.source_roots, name)
    }

    fn load_from_roots(source_roots: &[PathBuf], name: &str) -> Result<Self> {
        let first = source_roots
            .first()
            .map(|root| root.join("data").join(name))
            .unwrap_or_else(|| PathBuf::from(name));
        let Some(root) = source_roots
            .iter()
            .map(|source| source.join("data").join(name))
            .find(|root| root.is_dir())
        else {
            bail!("theme not found: {}", first.display());
        };

        Self::load_root(root, name)
    }

    pub(crate) fn load_root(root: PathBuf, name: &str) -> Result<Self> {
        if !root.is_dir() {
            bail!("theme not found: {}", root.display());
        }

        let colors_file = root.join("colors.toml");
        let (mut vars, generated) = if colors_file.is_file() {
            let vars = build_vars_from_colors(&colors_file)
                .with_context(|| format!("build vars for theme '{name}'"))?;
            (vars, false)
        } else if let Some(generated_toml) = colors::generate(&root) {
            let vars = vars::build_vars_from_str(&generated_toml)
                .with_context(|| format!("build vars for generated theme '{name}'"))?;
            (vars, true)
        } else {
            bail!(
                "missing colors.toml in theme '{name}': {}",
                colors_file.display()
            );
        };

        // Read optional display name before inject_context overwrites `name` with the id.
        let display_name = read_display_name(&root, &vars);

        palette::resolve(&mut vars);
        vars::derive_all_color_keys(&mut vars);

        let borders = border::resolve(&root);
        let active = borders
            .active
            .or_else(|| vars.get("accent").and_then(value_string));
        let inactive = borders
            .inactive
            .or_else(|| vars.get("color8").and_then(value_string))
            .or_else(|| active.clone());
        if let Some(c) = &active {
            vars::insert_border(&mut vars, "active", c);
        }
        if let Some(c) = &inactive {
            vars::insert_border(&mut vars, "inactive", c);
        }

        let bg_dir = root.join(BACKGROUNDS_DIR);
        let is_light = resolve_is_light(&root, &vars);
        let wallpapers: Vec<String> = image_files(&bg_dir)
            .into_iter()
            .map(|path| path.to_string_lossy().into_owned())
            .collect();
        vars::inject_context(&mut vars, name, is_light, &wallpapers);

        let preview = root.join(PREVIEW_FILE);

        Ok(Self {
            name: name.to_owned(),
            display_name,
            is_light,
            generated,
            icon_theme: read_trimmed(&root.join(ICONS_THEME_FILE))?,
            preview: preview.is_file().then_some(preview),
            backgrounds_dir: bg_dir.is_dir().then_some(bg_dir),
            root,
            vars,
        })
    }

    pub fn load_current(ctx: &Ctx) -> Result<Self> {
        let raw = match std::fs::read_to_string(&ctx.current_theme_file) {
            Ok(s) => s,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
            Err(e) => {
                return Err(e)
                    .with_context(|| format!("read {}", ctx.current_theme_file.display()));
            }
        };
        let name = raw.trim();
        anyhow::ensure!(
            !name.is_empty(),
            "current theme is not set ({})",
            ctx.current_theme_file.display()
        );
        Self::load(ctx, name).context("load current theme")
    }

    pub fn stage(&self, stage: &Path) -> Result<()> {
        if self.is_light {
            fs::write(stage.join(LIGHT_MODE_FILE), b"").context("write light.mode")?;
        }
        let icons = self.root.join(ICONS_THEME_FILE);
        if icons.is_file() {
            crate::util::symlink_force(&icons, &stage.join(ICONS_THEME_FILE))
                .context("symlink icons.theme")?;
        }
        if let Some(bg) = &self.backgrounds_dir {
            crate::util::symlink_force(bg, &stage.join(BACKGROUNDS_DIR))
                .context("symlink backgrounds")?;
        }
        Ok(())
    }
}

/// Optional human-readable display name from `theme.toml` or `colors.toml`.
fn read_display_name(root: &Path, vars: &HashMap<String, Value>) -> Option<String> {
    if let Ok(text) = fs::read_to_string(root.join("theme.toml"))
        && let Ok(table) = toml::from_str::<toml::Table>(&text)
        && let Some(name) = table
            .get("name")
            .and_then(toml::Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
    {
        return Some(name.to_owned());
    }
    match vars.get("name") {
        Some(Value::Str(s)) => {
            let trimmed = s.trim();
            (!trimmed.is_empty()).then(|| trimmed.to_owned())
        }
        _ => None,
    }
}

/// Determine light/dark mode using Omarchy's precedence:
/// 1. `mode` key (`"light"` / `"dark"`)
/// 2. `theme_type` key (`"light"` / `"dark"`)
/// 3. `light.mode` marker file
/// 4. `background` luminance (`r + g + b > 382` is light)
/// 5. dark
fn resolve_is_light(root: &Path, vars: &HashMap<String, Value>) -> bool {
    for key in ["mode", "theme_type"] {
        if let Some(Value::Str(s)) = vars.get(key) {
            match s.trim().to_ascii_lowercase().as_str() {
                "light" => return true,
                "dark" => return false,
                _ => {}
            }
        }
    }
    if root.join("light.mode").is_file() {
        return true;
    }
    if let Some(Value::Str(bg)) = vars.get("background")
        && let Some([r, g, b]) = colors::parse_hex(bg)
    {
        return u16::from(r) + u16::from(g) + u16::from(b) > 382;
    }
    false
}

pub fn list_names(ctx: &Ctx) -> Result<Vec<String>> {
    list_names_from_roots(&ctx.source_roots)
}

fn list_names_from_roots(source_roots: &[PathBuf]) -> Result<Vec<String>> {
    let mut names = BTreeSet::new();

    for source_root in source_roots {
        let data_dir = source_root.join("data");
        let entries = match fs::read_dir(&data_dir) {
            Ok(entries) => entries,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
            Err(e) => return Err(e).with_context(|| format!("read {}", data_dir.display())),
        };

        names.extend(
            entries
                .flatten()
                .filter(|entry| entry.path().is_dir())
                .filter_map(|entry| entry.file_name().into_string().ok())
                .filter(|name| !name.starts_with('.')),
        );
    }

    Ok(names.into_iter().collect())
}

fn image_files(dir: &Path) -> Vec<PathBuf> {
    let mut files: Vec<PathBuf> = fs::read_dir(dir)
        .map(|entries| {
            entries
                .flatten()
                .map(|entry| entry.path())
                .filter(|path| is_wallpaper_image(path))
                .collect()
        })
        .unwrap_or_default();
    files.sort();
    files
}

/// Extract a colour-capable string from a value, used for border fallbacks.
fn value_string(v: &Value) -> Option<String> {
    match v {
        Value::Str(s) => Some(s.clone()),
        Value::Int(i) => Some(i.to_string()),
        Value::Float(f) => Some(f.to_string()),
        Value::Bool(b) => Some(b.to_string()),
        Value::Color(c) => Some(c.to_hex()),
        _ => None,
    }
}

fn read_trimmed(path: &Path) -> Result<Option<String>> {
    match fs::read_to_string(path) {
        Ok(s) => {
            let t = s.trim();
            Ok((!t.is_empty()).then(|| t.to_owned()))
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e).with_context(|| format!("read {}", path.display())),
    }
}

#[cfg(test)]
mod tests {
    use super::{Theme, list_names_from_roots};
    use std::{fs, path::Path};

    fn write_theme(source_root: &Path, name: &str, accent: &str) {
        let root = source_root.join("data").join(name);
        fs::create_dir_all(&root).unwrap();
        fs::write(root.join("colors.toml"), format!("accent = \"{accent}\"\n")).unwrap();
    }

    #[test]
    fn highest_matching_theme_source_wins() {
        let dir = tempfile::tempdir().unwrap();
        let config = dir.path().join("config");
        let user_data = dir.path().join("user-data");
        write_theme(&config, "shared", "#111111");
        write_theme(&user_data, "shared", "#222222");

        let theme = Theme::load_from_roots(&[config.clone(), user_data], "shared").unwrap();

        assert_eq!(theme.root, config.join("data/shared"));
        assert_eq!(theme.vars["accent"].display(), "#111111");
    }

    #[test]
    fn invalid_higher_precedence_theme_does_not_fall_back() {
        let dir = tempfile::tempdir().unwrap();
        let config = dir.path().join("config");
        let system = dir.path().join("system");
        fs::create_dir_all(config.join("data/shared")).unwrap();
        write_theme(&system, "shared", "#222222");

        let error = Theme::load_from_roots(&[config.clone(), system], "shared").unwrap_err();

        assert!(error.to_string().contains("missing colors.toml"));
        assert!(error.to_string().contains(&config.display().to_string()));
    }

    #[test]
    fn theme_names_are_a_sorted_union_of_all_sources() {
        let dir = tempfile::tempdir().unwrap();
        let config = dir.path().join("config");
        let user_data = dir.path().join("user-data");
        let system = dir.path().join("system");
        write_theme(&config, "zeta", "#111111");
        write_theme(&config, "shared", "#111111");
        write_theme(&user_data, "alpha", "#222222");
        write_theme(&user_data, "shared", "#222222");
        write_theme(&system, "middle", "#333333");

        let names = list_names_from_roots(&[config, user_data, system]).unwrap();

        assert_eq!(names, ["alpha", "middle", "shared", "zeta"]);
    }

    #[test]
    fn mode_precedence_follows_omarchy_order() {
        let dir = tempfile::tempdir().unwrap();

        // 1. mode key beats light.mode marker (dark key overrides marker).
        let dark_with_marker = dir.path().join("dark-with-marker");
        fs::create_dir_all(&dark_with_marker).unwrap();
        fs::write(
            dark_with_marker.join("colors.toml"),
            "mode = \"dark\"\nbackground = \"#ffffff\"\n",
        )
        .unwrap();
        fs::write(dark_with_marker.join("light.mode"), "").unwrap();
        let theme = Theme::load_root(dark_with_marker, "dark-with-marker").unwrap();
        assert!(!theme.is_light);
        assert_eq!(theme.vars["mode"].display(), "dark");
        assert_eq!(theme.vars["theme_type"].display(), "dark");

        // 2. mode = "light" works without light.mode file, and stage writes light.mode.
        let light_by_key = dir.path().join("light-by-key");
        fs::create_dir_all(&light_by_key).unwrap();
        fs::write(
            light_by_key.join("colors.toml"),
            "mode = \"light\"\nbackground = \"#101010\"\n",
        )
        .unwrap();
        let theme = Theme::load_root(light_by_key, "light-by-key").unwrap();
        assert!(theme.is_light);
        assert_eq!(theme.vars["mode"].display(), "light");
        assert_eq!(theme.vars["theme_type"].display(), "light");
        let stage = dir.path().join("stage-light");
        fs::create_dir_all(&stage).unwrap();
        theme.stage(&stage).unwrap();
        assert!(stage.join("light.mode").is_file());

        // 3. theme_type beats marker and luminance.
        let light_by_type = dir.path().join("light-by-type");
        fs::create_dir_all(&light_by_type).unwrap();
        fs::write(
            light_by_type.join("colors.toml"),
            "theme_type = \"light\"\nbackground = \"#000000\"\n",
        )
        .unwrap();
        assert!(
            Theme::load_root(light_by_type, "light-by-type")
                .unwrap()
                .is_light
        );

        // 4. light.mode marker beats dark background luminance.
        let marker_beats_lum = dir.path().join("marker-beats-lum");
        fs::create_dir_all(&marker_beats_lum).unwrap();
        fs::write(
            marker_beats_lum.join("colors.toml"),
            "background = \"#000000\"\n",
        )
        .unwrap();
        fs::write(marker_beats_lum.join("light.mode"), "").unwrap();
        assert!(
            Theme::load_root(marker_beats_lum, "marker-beats-lum")
                .unwrap()
                .is_light
        );

        // 5. Luminance threshold: r+g+b = 383 is light, 382 is dark.
        // 0x80 + 0x80 + 0x7f = 128 + 128 + 127 = 383
        let lum_383 = dir.path().join("lum-383");
        fs::create_dir_all(&lum_383).unwrap();
        fs::write(lum_383.join("colors.toml"), "background = \"#80807f\"\n").unwrap();
        assert!(Theme::load_root(lum_383, "lum-383").unwrap().is_light);

        // 0x80 + 0x7f + 0x7f = 128 + 127 + 127 = 382
        let lum_382 = dir.path().join("lum-382");
        fs::create_dir_all(&lum_382).unwrap();
        fs::write(lum_382.join("colors.toml"), "background = \"#807f7f\"\n").unwrap();
        assert!(!Theme::load_root(lum_382, "lum-382").unwrap().is_light);
    }

    #[test]
    fn wallpaper_filter_keeps_images_only_and_skips_text_and_subdirs() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("theme");
        let bg = root.join("backgrounds");
        fs::create_dir_all(bg.join("intros")).unwrap();
        fs::create_dir_all(bg.join("loops")).unwrap();
        fs::write(root.join("colors.toml"), "accent = \"#123456\"\n").unwrap();
        for name in [
            "a.png",
            "b.JPG",
            "c.jpeg",
            "d.webp",
            "e.avif",
            "f.jxl",
            "g.GIF",
            "notes.txt",
            "README.md",
            "video.mp4",
        ] {
            fs::write(bg.join(name), "data").unwrap();
        }
        fs::write(bg.join("intros/a.mp4"), "video").unwrap();

        let theme = Theme::load_root(root, "theme").unwrap();
        let names: Vec<String> = match &theme.vars["wallpapers"] {
            crate::render::value::Value::List(items) => items
                .iter()
                .map(|v| {
                    std::path::PathBuf::from(v.display())
                        .file_name()
                        .unwrap()
                        .to_string_lossy()
                        .into_owned()
                })
                .collect(),
            other => panic!("expected list, got {other:?}"),
        };
        assert_eq!(
            names,
            [
                "a.png", "b.JPG", "c.jpeg", "d.webp", "e.avif", "f.jxl", "g.GIF"
            ]
        );
    }

    #[test]
    fn legacy_theme_generates_colors_in_memory_without_writing_to_disk() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("legacy");
        fs::create_dir_all(&root).unwrap();
        let mut kitty = String::from(
            "foreground #cdd6f4\nbackground #1e1e2e\ncursor #f5e0dc\n\
             selection_foreground #cdd6f4\nselection_background #45475a\n",
        );
        for i in 0..16 {
            kitty.push_str(&format!("color{i} #{i:02x}{i:02x}{i:02x}\n"));
        }
        fs::write(root.join("kitty.conf"), kitty).unwrap();

        let theme = Theme::load_root(root.clone(), "legacy").unwrap();
        assert!(theme.generated);
        assert_eq!(theme.vars["foreground"].display(), "#cdd6f4");
        assert_eq!(theme.vars["accent"].display(), "#040404");
        assert!(!root.join("colors.toml").exists());
    }

    #[test]
    fn display_name_and_preview_are_loaded_from_theme_metadata() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("tokyo-night");
        fs::create_dir_all(&root).unwrap();
        fs::write(
            root.join("colors.toml"),
            "name = \"From Colors\"\naccent = \"#7aa2f7\"\n",
        )
        .unwrap();
        fs::write(root.join("preview.png"), "png").unwrap();

        let theme = Theme::load_root(root.clone(), "tokyo-night").unwrap();
        assert_eq!(theme.display_name.as_deref(), Some("From Colors"));
        assert_eq!(theme.preview, Some(root.join("preview.png")));
        // Template `name` remains the theme id.
        assert_eq!(theme.vars["name"].display(), "tokyo-night");

        // theme.toml takes precedence over colors.toml for display_name.
        fs::write(root.join("theme.toml"), "name = \"Tokyo Night\"\n").unwrap();
        let theme = Theme::load_root(root, "tokyo-night").unwrap();
        assert_eq!(theme.display_name.as_deref(), Some("Tokyo Night"));
    }
}

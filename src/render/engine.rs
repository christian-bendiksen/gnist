//! Merges user, theme, and base files into one rendered tree.

use super::parser::{Segment, parse};
use anyhow::{Context, Result, bail};
use std::{
    collections::{HashMap, HashSet},
    fs,
    path::{Path, PathBuf},
};
use walkdir::WalkDir;

/// Build the output tree with user templates taking precedence over theme
/// files, followed by base templates.
pub fn render_all(
    templates_dir: &Path,
    user_templates_dir: &Path,
    theme_files_dir: &Path,
    out_dir: &Path,
    vars: &HashMap<String, String>,
) -> Result<()> {
    if !templates_dir.is_dir() {
        bail!("templates directory not found: {}", templates_dir.display());
    }
    fs::create_dir_all(out_dir).context("create output directory")?;

    let mut claimed: HashSet<PathBuf> = HashSet::new();

    if user_templates_dir.is_dir() {
        for tpl in templates_in(user_templates_dir) {
            let rel = tpl.strip_prefix(user_templates_dir)?.to_path_buf();
            render_one(&tpl, &rel, vars, out_dir)?;
            claimed.insert(rel.with_extension(""));
        }
    }

    if theme_files_dir.is_dir() {
        for src in theme_files_in(theme_files_dir) {
            let rel = src.strip_prefix(theme_files_dir)?.to_path_buf();
            if !claimed.contains(&rel) {
                let out_path = out_dir.join(&rel);
                if let Some(parent) = out_path.parent() {
                    fs::create_dir_all(parent)?;
                }
                fs::copy(&src, &out_path)?;
                claimed.insert(rel);
            }
        }
    }

    for tpl in templates_in(templates_dir) {
        let rel = tpl.strip_prefix(templates_dir)?.to_path_buf();
        if !claimed.contains(&rel.with_extension("")) {
            render_one(&tpl, &rel, vars, out_dir)?;
        }
    }

    Ok(())
}

/// Render one template under `out_dir`, dropping its `.tpl` extension.
fn render_one(
    tpl_path: &Path,
    rel: &Path,
    vars: &HashMap<String, String>,
    out_dir: &Path,
) -> Result<()> {
    let src = fs::read_to_string(tpl_path)
        .with_context(|| format!("read template {}", tpl_path.display()))?;

    let rendered = expand(&src, vars);

    let out_path = out_dir.join(rel.with_extension(""));

    if let Some(parent) = out_path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("create output subdir {}", parent.display()))?;
    }
    fs::write(&out_path, rendered).with_context(|| format!("write {}", out_path.display()))
}

/// Replace known variables and leave unknown placeholders visible.
///
/// Keeping unknown keys intact makes partial renders easier to inspect.
fn expand(src: &str, vars: &HashMap<String, String>) -> String {
    let segments = parse(src);

    let capacity: usize = segments
        .iter()
        .map(|s| match s {
            Segment::Lit(t) => t.len(),
            Segment::Var(k) => vars.get(*k).map_or(k.len() + 6, String::len),
        })
        .sum();

    let mut out = String::with_capacity(capacity);

    for seg in &segments {
        match seg {
            Segment::Lit(t) => out.push_str(t),
            Segment::Var(k) => match vars.get(*k) {
                Some(v) => out.push_str(v),
                None => {
                    out.push_str("{{ ");
                    out.push_str(k);
                    out.push_str(" }}");
                }
            },
        }
    }

    out
}

fn templates_in(dir: &Path) -> impl Iterator<Item = PathBuf> {
    WalkDir::new(dir)
        .follow_links(true)
        .into_iter()
        .filter_map(|e| e.ok())
        .filter(|e| {
            e.file_type().is_file() && e.path().extension().and_then(|x| x.to_str()) == Some("tpl")
        })
        .map(|e| e.into_path())
}

fn theme_files_in(dir: &Path) -> impl Iterator<Item = PathBuf> {
    WalkDir::new(dir)
        .follow_links(true)
        .into_iter()
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().is_file() && !is_theme_metadata(e.path()))
        .map(|e| e.into_path())
}

fn is_theme_metadata(path: &Path) -> bool {
    path.components().any(|c| {
        matches!(
            c.as_os_str().to_str(),
            Some("colors.toml" | "light.mode" | "icons.theme" | "backgrounds")
        )
    })
}

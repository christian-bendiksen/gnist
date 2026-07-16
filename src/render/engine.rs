//! Merges user, theme, and base files into one rendered tree.

use super::parser::{Segment, parse};
use anyhow::{Context, Result, bail};
use std::{
    collections::{HashMap, HashSet},
    fs,
    path::{Path, PathBuf},
};
use walkdir::WalkDir;

/// Build the output tree with config user templates taking precedence over
/// theme files, followed by template roots in their supplied order.
pub fn render_all(
    template_dirs: &[PathBuf],
    user_templates_dir: &Path,
    theme_files_dir: &Path,
    out_dir: &Path,
    vars: &HashMap<String, String>,
) -> Result<()> {
    if !template_dirs.iter().any(|dir| dir.is_dir()) {
        let missing = template_dirs
            .first()
            .map(PathBuf::as_path)
            .unwrap_or_else(|| Path::new("templates"));
        bail!("templates directory not found: {}", missing.display());
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

    for templates_dir in template_dirs.iter().filter(|dir| dir.is_dir()) {
        for tpl in templates_in(templates_dir) {
            let rel = tpl.strip_prefix(templates_dir)?.to_path_buf();
            let output_rel = rel.with_extension("");
            if !claimed.contains(&output_rel) {
                render_one(&tpl, &rel, vars, out_dir)?;
                claimed.insert(output_rel);
            }
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

#[cfg(test)]
mod tests {
    use super::render_all;
    use std::{collections::HashMap, fs, path::Path};

    fn write(path: &Path, content: &str) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, content).unwrap();
    }

    #[test]
    fn merges_all_template_layers_in_precedence_order() {
        let dir = tempfile::tempdir().unwrap();
        let user_templates = dir.path().join("config/user-templates");
        let theme = dir.path().join("selected-theme");
        let config_templates = dir.path().join("config/templates");
        let user_data_templates = dir.path().join("user-data/templates");
        let system_one_templates = dir.path().join("system-one/templates");
        let system_two_templates = dir.path().join("system-two/templates");
        let out = dir.path().join("out");

        write(&user_templates.join("all.tpl"), "config user {{ accent }}");
        write(&theme.join("all"), "theme");
        write(&theme.join("theme-wins"), "theme");
        write(&config_templates.join("all.tpl"), "config");
        write(&config_templates.join("theme-wins.tpl"), "config");
        write(&config_templates.join("config-wins.tpl"), "config");
        write(&user_data_templates.join("config-wins.tpl"), "user data");
        write(&user_data_templates.join("user-data-wins.tpl"), "user data");
        write(
            &system_one_templates.join("user-data-wins.tpl"),
            "system one",
        );
        write(&system_one_templates.join("system-wins.tpl"), "system one");
        write(&system_two_templates.join("system-wins.tpl"), "system two");

        let template_dirs = vec![
            config_templates,
            user_data_templates,
            system_one_templates,
            system_two_templates,
        ];
        let vars = HashMap::from([("accent".to_owned(), "blue".to_owned())]);
        render_all(&template_dirs, &user_templates, &theme, &out, &vars).unwrap();

        assert_eq!(
            fs::read_to_string(out.join("all")).unwrap(),
            "config user blue"
        );
        assert_eq!(fs::read_to_string(out.join("theme-wins")).unwrap(), "theme");
        assert_eq!(
            fs::read_to_string(out.join("config-wins")).unwrap(),
            "config"
        );
        assert_eq!(
            fs::read_to_string(out.join("user-data-wins")).unwrap(),
            "user data"
        );
        assert_eq!(
            fs::read_to_string(out.join("system-wins")).unwrap(),
            "system one"
        );
    }

    #[test]
    fn fallback_templates_work_without_config_templates() {
        let dir = tempfile::tempdir().unwrap();
        let missing_config = dir.path().join("config/templates");
        let fallback = dir.path().join("data/templates");
        let out = dir.path().join("out");
        write(&fallback.join("app.conf.tpl"), "fallback");

        render_all(
            &[missing_config, fallback],
            &dir.path().join("config/user-templates"),
            &dir.path().join("theme"),
            &out,
            &HashMap::new(),
        )
        .unwrap();

        assert_eq!(
            fs::read_to_string(out.join("app.conf")).unwrap(),
            "fallback"
        );
    }
}

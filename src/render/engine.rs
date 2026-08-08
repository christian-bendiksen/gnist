//! Merges user, theme, and base files into one rendered tree.
//!
//! Each `.tpl` file is expanded with the theme's typed value context. Output
//! filenames are templated too, so `foo.{{ name }}.conf.tpl` can render to
//! `foo.dusk.conf`.

use super::expr::{eval_condition, eval_expr};
use super::parser::{Token, parse, raw_span};
use super::value::Value;
use anyhow::{Context, Result};
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
    values: &HashMap<String, Value>,
) -> Result<()> {
    // A global templates directory is optional: themes may carry their own
    // `.tpl` templates, or ship only static files.
    fs::create_dir_all(out_dir).context("create output directory")?;

    let mut claimed: HashSet<PathBuf> = HashSet::new();

    if user_templates_dir.is_dir() {
        for tpl in templates_in(user_templates_dir) {
            let rel = tpl.strip_prefix(user_templates_dir)?.to_path_buf();
            let out_rel = render_path(&rel, values);
            render_one(&tpl, &out_rel, values, out_dir)?;
            claimed.insert(out_rel);
        }
    }

    if theme_files_dir.is_dir() {
        // A theme can ship self-contained templates: `.tpl` files in its own
        // directory are rendered here, ahead of static files and global
        // templates, but after user templates.
        for tpl in theme_templates_in(theme_files_dir) {
            let rel = tpl.strip_prefix(theme_files_dir)?.to_path_buf();
            let out_rel = render_path(&rel, values);
            if !claimed.contains(&out_rel) {
                render_one(&tpl, &out_rel, values, out_dir)?;
                claimed.insert(out_rel);
            }
        }
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
            let out_rel = render_path(&rel, values);
            if !claimed.contains(&out_rel) {
                render_one(&tpl, &out_rel, values, out_dir)?;
                claimed.insert(out_rel);
            }
        }
    }

    Ok(())
}

/// Render one template under `out_dir`, templating its output filename too.
fn render_one(
    tpl_path: &Path,
    rel: &Path,
    values: &HashMap<String, Value>,
    out_dir: &Path,
) -> Result<()> {
    let src = fs::read_to_string(tpl_path)
        .with_context(|| format!("read template {}", tpl_path.display()))?;

    let rendered = render_str(&src, values);

    let out_path = out_dir.join(rel);

    if let Some(parent) = out_path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("create output subdir {}", parent.display()))?;
    }
    fs::write(&out_path, rendered).with_context(|| format!("write {}", out_path.display()))
}

/// Render `.tpl` relative path into its output name, dropping the extension and
/// expanding any `{{ ... }}` placeholders in the path itself.
fn render_path(rel: &Path, values: &HashMap<String, Value>) -> PathBuf {
    let rel = rel.with_extension("");
    let rendered = render_str(&rel.to_string_lossy(), values);
    PathBuf::from(rendered)
}

/// Expand a template source string against the value context.
pub fn render_str(src: &str, values: &HashMap<String, Value>) -> String {
    let tokens = parse(src);
    let empty = HashMap::new();
    let mut out = String::new();
    render_range(&tokens, 0, tokens.len(), values, &empty, &mut out);
    out
}

fn lookup<'a>(
    values: &'a HashMap<String, Value>,
    overlay: &'a HashMap<String, Value>,
    key: &str,
) -> Option<&'a Value> {
    overlay.get(key).or_else(|| values.get(key))
}

fn render_range(
    tokens: &[Token],
    start: usize,
    end: usize,
    values: &HashMap<String, Value>,
    overlay: &HashMap<String, Value>,
    out: &mut String,
) {
    let mut i = start;
    while i < end {
        match &tokens[i] {
            Token::Lit(t) => out.push_str(t),
            Token::Var { key, raw } => match lookup(values, overlay, key) {
                Some(v) => out.push_str(&v.display()),
                None => out.push_str(raw),
            },
            Token::Expr { expr, raw } => {
                let v = eval_expr(expr, values, overlay);
                if v.is_missing() {
                    out.push_str(raw);
                } else {
                    out.push_str(&v.display());
                }
            }
            Token::Comment => {}
            Token::Close { raw, .. } | Token::Else { raw } => {
                // An unmatched closer is passed through literally.
                out.push_str(raw);
            }
            Token::Block { spec, .. } => {
                let close = find_block_end(tokens, i, end);
                let unterminated = close >= end || !matches!(&tokens[close], Token::Close { .. });
                let inner_end = if unterminated { end } else { close };
                let rendered = match split_block(spec) {
                    Some((kind, rest)) => match kind {
                        "if" | "unless" => {
                            let mut cond = eval_condition(rest, values, overlay);
                            if kind == "unless" {
                                cond = !cond;
                            }
                            let else_idx = find_else(tokens, i + 1, inner_end);
                            let (sel_start, sel_end) = if cond {
                                (i + 1, else_idx.unwrap_or(inner_end))
                            } else {
                                match else_idx {
                                    Some(e) => (e + 1, inner_end),
                                    None => (inner_end, inner_end),
                                }
                            };
                            let mut inner = String::new();
                            render_range(tokens, sel_start, sel_end, values, overlay, &mut inner);
                            inner
                        }
                        "each" => {
                            let mut inner = String::new();
                            if let Some((list_expr, item)) = split_each(rest) {
                                if let Value::List(items) = eval_expr(list_expr, values, overlay) {
                                    for item_value in items {
                                        let mut child = overlay.clone();
                                        child.insert(item.to_owned(), item_value);
                                        render_range(
                                            tokens, i + 1, inner_end, values, &child, &mut inner,
                                        );
                                    }
                                }
                            }
                            inner
                        }
                        _ => {
                            // Unknown block: keep the source verbatim.
                            let span = if unterminated { end } else { close + 1 };
                            raw_span(tokens, i, span)
                        }
                    },
                    None => {
                        let span = if unterminated { end } else { close + 1 };
                        raw_span(tokens, i, span)
                    }
                };
                out.push_str(&rendered);
                i = if unterminated { end } else { close + 1 };
                continue;
            }
        }
        i += 1;
    }
}

/// Return the index of the `Close` token matching `open`, or `end` when the
/// block is unterminated within the current range.
fn find_block_end(tokens: &[Token], open: usize, end: usize) -> usize {
    let mut depth: usize = 1;
    let mut i = open + 1;
    while i < end {
        match &tokens[i] {
            Token::Block { .. } => depth += 1,
            Token::Close { .. } => {
                depth -= 1;
                if depth == 0 {
                    return i;
                }
            }
            _ => {}
        }
        i += 1;
    }
    end
}

/// Find the first top-level `{{else}}` inside `[start, end)`.
fn find_else(tokens: &[Token], start: usize, end: usize) -> Option<usize> {
    let mut depth: usize = 0;
    for i in start..end {
        match &tokens[i] {
            Token::Block { .. } => depth += 1,
            Token::Close { .. } => depth = depth.saturating_sub(1),
            Token::Else { .. } if depth == 0 => return Some(i),
            _ => {}
        }
    }
    None
}

fn split_block(spec: &str) -> Option<(&str, &str)> {
    let trimmed = spec.trim();
    let mut parts = trimmed.splitn(2, char::is_whitespace);
    let kind = parts.next()?;
    if kind.is_empty() {
        return None;
    }
    Some((kind, parts.next().unwrap_or("").trim()))
}

fn split_each(rest: &str) -> Option<(&str, &str)> {
    let rest = rest.trim();
    if rest.is_empty() {
        return None;
    }
    if let Some(pos) = rest.find(" as ")
        && pos > 0
    {
        let list = rest[..pos].trim();
        let item = rest[pos + 4..].trim();
        if !list.is_empty()
            && !item.is_empty()
            && item.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
        {
            return Some((list, item));
        }
    }
    Some((rest, "item"))
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

/// `.tpl` files inside a selected theme are rendered as its own templates.
fn theme_templates_in(dir: &Path) -> impl Iterator<Item = PathBuf> {
    templates_in(dir)
}

/// Static theme files that are copied verbatim (templates excluded).
fn theme_files_in(dir: &Path) -> impl Iterator<Item = PathBuf> {
    WalkDir::new(dir)
        .follow_links(true)
        .into_iter()
        .filter_map(|e| e.ok())
        .filter(|e| {
            e.file_type().is_file()
                && !is_theme_metadata(e.path())
                && e.path().extension().and_then(|x| x.to_str()) != Some("tpl")
        })
        .map(|e| e.into_path())
}

fn is_theme_metadata(path: &Path) -> bool {
    path.components().any(|c| {
        matches!(
            c.as_os_str().to_str(),
            Some(
                "colors.toml"
                    | "light.mode"
                    | "icons.theme"
                    | "backgrounds"
                    | "gnome.toml"
                    | "wallpaper.toml"
            )
        )
    })
}

#[cfg(test)]
mod tests {
    use super::{render_all, render_str};
    use crate::render::value::Value;
    use std::{collections::HashMap, fs, path::Path};

    fn str_map(pairs: &[(&str, &str)]) -> HashMap<String, Value> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), Value::Str(v.to_string())))
            .collect()
    }

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
        let vars = str_map(&[("accent", "blue")]);
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
    fn theme_own_templates_render_and_follow_precedence() {
        let dir = tempfile::tempdir().unwrap();
        let user_templates = dir.path().join("config/user-templates");
        let theme = dir.path().join("theme");
        let config_templates = dir.path().join("config/templates");
        let out = dir.path().join("out");

        write(&theme.join("self.tpl"), "theme {{ accent }}");
        write(&theme.join("static.conf"), "static");
        write(&config_templates.join("self.tpl"), "global");
        write(&config_templates.join("global-only.tpl"), "global-only");
        write(&user_templates.join("own.tpl"), "user");

        let vars = str_map(&[("accent", "#89b4fa")]);
        render_all(&[config_templates], &user_templates, &theme, &out, &vars).unwrap();

        assert_eq!(
            fs::read_to_string(out.join("self")).unwrap(),
            "theme #89b4fa"
        );
        assert_eq!(
            fs::read_to_string(out.join("static.conf")).unwrap(),
            "static"
        );
        assert_eq!(
            fs::read_to_string(out.join("global-only")).unwrap(),
            "global-only"
        );
        assert_eq!(fs::read_to_string(out.join("own")).unwrap(), "user");
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

    #[test]
    fn unknown_tokens_stay_visible() {
        let vars = str_map(&[("accent", "#89b4fa")]);
        assert_eq!(
            render_str("{{ accent }} {{ missing }}", &vars),
            "#89b4fa {{ missing }}"
        );
    }

    #[test]
    fn filters_expand_in_line() {
        let vars = str_map(&[("accent", "#89b4fa")]);
        assert_eq!(
            render_str("{{ accent | lighten 0.1 }}", &vars),
            "#95bcfb"
        );
    }

    #[test]
    fn blocks_branch_and_loop() {
        let vars = str_map(&[("accent", "#89b4fa"), ("mode", "dark")]);
        let src = "{{#if mode == \"dark\"}}night{{else}}day{{/if}}";
        assert_eq!(render_str(src, &vars), "night");
    }

    #[test]
    fn each_loops_over_lists() {
        let vars = HashMap::from([(
            "colors".to_string(),
            Value::List(vec![Value::Str("a".into()), Value::Str("b".into())]),
        )]);
        assert_eq!(
            render_str("{{#each colors as c}}{{ c }},{{/each}}", &vars),
            "a,b,"
        );
    }

    #[test]
    fn each_does_not_clobber_outer_values() {
        let vars = HashMap::from([
            ("c".to_string(), Value::Str("outer".into())),
            (
                "colors".to_string(),
                Value::List(vec![Value::Str("inner".into())]),
            ),
        ]);
        assert_eq!(
            render_str("{{#each colors as c}}{{ c }}{{/each}} {{ c }}", &vars),
            "inner outer"
        );
    }

    #[test]
    fn unless_and_else_branches() {
        let vars = HashMap::from([("mode".to_string(), Value::Str("dark".into()))]);
        assert_eq!(
            render_str("{{#if mode == \"light\"}}l{{else}}d{{/if}}", &vars),
            "d"
        );
        assert_eq!(
            render_str(
                "{{#unless mode == \"light\"}}dark{{/unless}}",
                &vars
            ),
            "dark"
        );
    }

    #[test]
    fn blocks_nest_recursively() {
        let vars = HashMap::from([
            ("mode".to_string(), Value::Str("dark".into())),
            (
                "colors".to_string(),
                Value::List(vec![Value::Str("a".into()), Value::Str("b".into())]),
            ),
        ]);
        assert_eq!(
            render_str(
                "{{#if mode == \"dark\"}}{{#each colors as c}}-{{ c }} {{/each}}{{/if}}",
                &vars
            ),
            "-a -b "
        );
    }

    #[test]
    fn unknown_blocks_pass_through() {
        let vars = HashMap::new();
        assert_eq!(
            render_str("{{#widget}}x{{/widget}}", &vars),
            "{{#widget}}x{{/widget}}"
        );
    }

    #[test]
    fn templated_output_filenames() {
        let dir = tempfile::tempdir().unwrap();
        let templates = dir.path().join("templates");
        let out = dir.path().join("out");
        write(&templates.join("night.{{ name }}.conf.tpl"), "dfm");
        let vars = str_map(&[("name", "dusk")]);

        render_all(
            &[templates],
            &dir.path().join("user"),
            &dir.path().join("theme"),
            &out,
            &vars,
        )
        .unwrap();

        assert!(out.join("night.dusk.conf").is_file());
        assert!(!out.join("night..conf").exists());
    }
}

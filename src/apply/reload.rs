use crate::ctx::Ctx;
use crate::render::engine::render_str;
use crate::render::value::Value;
use anyhow::{Context, Result};
use kdl::KdlDocument;
use std::{
    collections::HashMap,
    fs,
    path::{Path, PathBuf},
    process::{Command, Stdio},
};

#[derive(Debug, PartialEq, Eq)]
enum ReloadAction {
    Signal { process: String, signal: String },
    Command { cmd: String, args: Vec<String> },
    Touch { path: PathBuf },
}

pub fn run(ctx: &Ctx, values: &HashMap<String, Value>) {
    let paths = binding_paths(&ctx.config_dir);

    let Some((actions, warnings)) = parse_reload_files(&paths, values) else {
        eprintln!(
            "info: no reload bindings found in {}",
            ctx.config_dir.display()
        );
        return;
    };

    for warning in warnings {
        eprintln!("{warning}");
    }

    for action in actions {
        execute_action(action);
    }
}

fn binding_paths(config_dir: &Path) -> Vec<PathBuf> {
    let mut paths = Vec::new();
    let base = config_dir.join("bindings.kdl");
    if base.is_file() {
        paths.push(base);
    }

    let dropin_dir = config_dir.join("bindings.d");
    let mut dropins: Vec<PathBuf> = fs::read_dir(dropin_dir)
        .map(|entries| {
            entries
                .flatten()
                .map(|entry| entry.path())
                .filter(|path| {
                    path.is_file() && path.extension().and_then(|ext| ext.to_str()) == Some("kdl")
                })
                .collect()
        })
        .unwrap_or_default();
    dropins.sort();
    paths.extend(dropins);
    paths
}

fn parse_reload_files(
    paths: &[PathBuf],
    values: &HashMap<String, Value>,
) -> Option<(Vec<ReloadAction>, Vec<String>)> {
    if paths.is_empty() {
        return None;
    }

    let mut actions = Vec::new();
    let mut warnings = Vec::new();
    for path in paths {
        match parse_reload_actions(path, values) {
            Ok(parsed) => actions.extend(parsed),
            Err(error) => warnings.push(format!(
                "warn: failed to load reload bindings from {}: {error:#}",
                path.display()
            )),
        }
    }
    Some((actions, warnings))
}

fn parse_reload_actions(
    kdl_path: &Path,
    values: &HashMap<String, Value>,
) -> Result<Vec<ReloadAction>> {
    let content =
        fs::read_to_string(kdl_path).with_context(|| format!("read {}", kdl_path.display()))?;
    // Bindings may reference theme values, e.g. generated paths.
    let rendered = render_str(&content, values);
    let doc: KdlDocument = rendered
        .parse()
        .with_context(|| format!("parse {}", kdl_path.display()))?;

    let mut actions = Vec::new();

    for bind_node in doc.nodes().iter().filter(|n| n.name().value() == "bind") {
        let Some(reload_node) = bind_node.children().and_then(|c| c.get("reload")) else {
            continue;
        };
        let Some(strategies) = reload_node.children() else {
            continue;
        };

        for strategy in strategies.nodes() {
            match strategy.name().value() {
                "signal" => {
                    if let (Some(process), Some(signal)) = (
                        strategy.get("process").and_then(|v| v.as_string()),
                        strategy.get("signal").and_then(|v| v.as_string()),
                    ) {
                        actions.push(ReloadAction::Signal {
                            process: process.to_owned(),
                            signal: signal.to_owned(),
                        })
                    };
                }
                "command" => {
                    if let Some(argv) = strategy.children().and_then(|c| c.get("argv")) {
                        let mut args: Vec<String> = argv
                            .iter()
                            .filter_map(|a| a.value().as_string().map(expand_tilde))
                            .collect();
                        if !args.is_empty() {
                            let cmd = args.remove(0);
                            actions.push(ReloadAction::Command { cmd, args });
                        }
                    }
                }
                "touch" => {
                    if let Some(path_str) = strategy.get("path").and_then(|v| v.as_string()) {
                        actions.push(ReloadAction::Touch {
                            path: PathBuf::from(expand_tilde(path_str)),
                        });
                    }
                }
                _ => {}
            }
        }
    }

    Ok(actions)
}

fn execute_action(action: ReloadAction) {
    match action {
        ReloadAction::Signal { process, signal } => {
            let flag = if signal.starts_with('-') {
                signal
            } else {
                format!("-{signal}")
            };
            spawn_detached("pkill", &[&flag, &process]);
        }
        ReloadAction::Command { cmd, args } => {
            let arg_refs: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
            spawn_detached(&cmd, &arg_refs);
        }
        ReloadAction::Touch { path } => {
            spawn_detached("touch", &[&path.to_string_lossy()]);
        }
    }
}

fn spawn_detached(cmd: &str, args: &[&str]) {
    Command::new(cmd)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .ok();
}

fn expand_tilde(s: &str) -> String {
    if let Some(stripped) = s.strip_prefix("~/")
        && let Ok(home) = std::env::var("HOME")
    {
        return format!("{home}/{stripped}");
    }
    s.to_string()
}

#[cfg(test)]
mod tests {
    use super::{ReloadAction, binding_paths, parse_reload_files};
    use std::{collections::HashMap, fs, path::PathBuf};

    fn empty_values() -> HashMap<String, crate::render::value::Value> {
        HashMap::new()
    }

    fn binding(path: &std::path::Path, target: &str) {
        fs::write(
            path,
            format!("bind {{ reload {{ touch path=\"{target}\" }} }}\n"),
        )
        .unwrap();
    }

    #[test]
    fn loads_base_then_kdl_dropins_in_lexical_order() {
        let dir = tempfile::tempdir().unwrap();
        let dropins = dir.path().join("bindings.d");
        fs::create_dir(&dropins).unwrap();
        binding(&dir.path().join("bindings.kdl"), "/base");
        binding(&dropins.join("20-last.kdl"), "/last");
        binding(&dropins.join("10-first.kdl"), "/first");
        binding(&dropins.join("ignored.txt"), "/ignored");
        fs::create_dir(dropins.join("05-directory.kdl")).unwrap();

        let paths = binding_paths(dir.path());
        let (actions, warnings) = parse_reload_files(&paths, &empty_values()).unwrap();

        assert!(warnings.is_empty());
        assert_eq!(
            paths
                .iter()
                .map(|path| path.file_name().unwrap().to_string_lossy().into_owned())
                .collect::<Vec<_>>(),
            ["bindings.kdl", "10-first.kdl", "20-last.kdl"]
        );
        assert_eq!(
            actions,
            ["/base", "/first", "/last"]
                .map(PathBuf::from)
                .map(|path| ReloadAction::Touch { path })
        );
    }

    #[test]
    fn loads_dropins_without_a_base_file() {
        let dir = tempfile::tempdir().unwrap();
        let dropins = dir.path().join("bindings.d");
        fs::create_dir(&dropins).unwrap();
        binding(&dropins.join("theme.kdl"), "/dropin");

        let (actions, warnings) = parse_reload_files(&binding_paths(dir.path()), &empty_values()).unwrap();

        assert!(warnings.is_empty());
        assert_eq!(
            actions,
            [ReloadAction::Touch {
                path: PathBuf::from("/dropin")
            }]
        );
    }

    #[test]
    fn malformed_dropin_warns_without_discarding_other_bindings() {
        let dir = tempfile::tempdir().unwrap();
        let dropins = dir.path().join("bindings.d");
        fs::create_dir(&dropins).unwrap();
        binding(&dir.path().join("bindings.kdl"), "/base");
        fs::write(dropins.join("10-malformed.kdl"), "bind {").unwrap();
        binding(&dropins.join("20-valid.kdl"), "/dropin");

        let (actions, warnings) = parse_reload_files(&binding_paths(dir.path()), &empty_values()).unwrap();

        assert_eq!(
            actions,
            ["/base", "/dropin"]
                .map(PathBuf::from)
                .map(|path| ReloadAction::Touch { path })
        );
        assert_eq!(warnings.len(), 1);
        assert!(warnings[0].contains("10-malformed.kdl"));
    }

    #[test]
    fn unreadable_dropin_warns_without_discarding_valid_bindings() {
        let dir = tempfile::tempdir().unwrap();
        let dropins = dir.path().join("bindings.d");
        fs::create_dir(&dropins).unwrap();
        binding(&dir.path().join("bindings.kdl"), "/base");
        fs::write(dropins.join("10-unreadable.kdl"), [0xff]).unwrap();
        binding(&dropins.join("20-valid.kdl"), "/dropin");

        let (actions, warnings) = parse_reload_files(&binding_paths(dir.path()), &empty_values()).unwrap();

        assert_eq!(
            actions,
            ["/base", "/dropin"]
                .map(PathBuf::from)
                .map(|path| ReloadAction::Touch { path })
        );
        assert_eq!(warnings.len(), 1);
        assert!(warnings[0].contains("10-unreadable.kdl"));
    }

    #[test]
    fn malformed_base_file_is_diagnosed() {
        let dir = tempfile::tempdir().unwrap();
        let base = dir.path().join("bindings.kdl");
        fs::write(&base, "bind {").unwrap();

        let (actions, warnings) = parse_reload_files(&[base], &empty_values()).unwrap();

        assert!(actions.is_empty());
        assert_eq!(warnings.len(), 1);
        assert!(warnings[0].contains("bindings.kdl"));
    }
}

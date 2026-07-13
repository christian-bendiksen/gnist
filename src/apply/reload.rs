use crate::ctx::Ctx;
use kdl::KdlDocument;
use std::{
    fs,
    path::{Path, PathBuf},
    process::{Command, Stdio},
};

#[derive(Debug)]
enum ReloadAction {
    Signal { process: String, signal: String },
    Command { cmd: String, args: Vec<String> },
    Touch { path: PathBuf },
}

pub fn run(ctx: &Ctx) {
    let kdl_path = ctx.config_dir.join("bindings.kdl");

    let Some(actions) = parse_reload_actions(&kdl_path) else {
        eprintln!("info: no reload bindings found at {}", kdl_path.display());
        return;
    };

    for action in actions {
        execute_action(action);
    }
}

fn parse_reload_actions(kdl_path: &Path) -> Option<Vec<ReloadAction>> {
    let content = fs::read_to_string(kdl_path).ok()?;
    let doc: KdlDocument = content.parse().ok()?;

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

    Some(actions)
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
    if let Some(stripped) = s.strip_prefix("~/") {
        if let Ok(home) = std::env::var("HOME") {
            return format!("{home}/{stripped}");
        }
    }
    s.to_string()
}

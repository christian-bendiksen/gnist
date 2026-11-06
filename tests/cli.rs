use std::{
    fs,
    os::unix::fs::PermissionsExt,
    path::PathBuf,
    process::{Command, Output},
};

/// Every tool the CLI's apply path (or install path) can spawn. Each one is a
/// stand-in inside the test's temporary PATH: it records its name and arguments
/// and exits 0. None of the real tools is ever on PATH.
const STAND_INS: &[&str] = &[
    "gsettings",
    "dbus-send",
    "pkill",
    "touch",
    "awww",
    "notify-send",
    "git",
    "gtk-update-icon-cache",
];

/// A CLI test's whole world: a temporary HOME and XDG tree, a dead bus, no
/// display, and a PATH that holds only recording stand-ins.
struct Guarded {
    dir: tempfile::TempDir,
    bin: PathBuf,
    log: PathBuf,
}

impl Guarded {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        for directory in ["config", "data", "cache", "state", "run"] {
            fs::create_dir(dir.path().join(directory)).unwrap();
        }
        fs::set_permissions(dir.path().join("run"), fs::Permissions::from_mode(0o700)).unwrap();

        let bin = dir.path().join("stand-ins");
        fs::create_dir(&bin).unwrap();
        let log = dir.path().join("stand-in.log");
        for name in STAND_INS {
            let script = format!(
                "#!/bin/sh\nprintf '%s\\n' \"{name} $*\" >> '{}'\nexit 0\n",
                log.display()
            );
            write_executable(&bin.join(name), &script);
        }

        Self { dir, bin, log }
    }

    fn path(&self, name: &str) -> PathBuf {
        self.dir.path().join(name)
    }

    /// Run the binary with nothing inherited. This is the only place in this
    /// file that spawns it; `only_the_guarded_builder_spawns_the_binary`
    /// enforces that.
    fn run(&self, args: &[&str]) -> Output {
        let dead_bus = format!("unix:path={}", self.dir.path().join("no-bus").display());
        Command::new(env!("CARGO_BIN_EXE_gnist"))
            .args(args)
            .env_clear()
            .env("HOME", self.dir.path())
            .env("XDG_CONFIG_HOME", self.dir.path().join("config"))
            .env("XDG_DATA_HOME", self.dir.path().join("data"))
            .env("XDG_DATA_DIRS", "")
            .env("XDG_RUNTIME_DIR", self.dir.path().join("run"))
            .env("XDG_CACHE_HOME", self.dir.path().join("cache"))
            .env("XDG_STATE_HOME", self.dir.path().join("state"))
            .env("DBUS_SESSION_BUS_ADDRESS", &dead_bus)
            .env("DBUS_SYSTEM_BUS_ADDRESS", &dead_bus)
            .env("PATH", &self.bin)
            .env("TERM", "xterm-256color")
            .env("LANG", "C")
            .env("LC_ALL", "C")
            .output()
            .unwrap()
    }

    fn calls(&self) -> Option<String> {
        fs::read_to_string(&self.log).ok()
    }

    fn assert_nothing_reached_a_tool(&self) {
        assert!(
            !self.log.exists(),
            "a skipped apply called {:?}",
            self.calls().unwrap_or_default()
        );
    }
}

fn write_theme(root: &std::path::Path, name: &str, accent: &str) {
    let theme = root.join("gnist/themes/data").join(name);
    fs::create_dir_all(&theme).unwrap();
    fs::write(
        theme.join("colors.toml"),
        format!("accent = \"{accent}\"\n"),
    )
    .unwrap();
}

fn write_executable(path: &std::path::Path, contents: &str) {
    fs::write(path, contents).unwrap();
    let mut permissions = fs::metadata(path).unwrap().permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(path, permissions).unwrap();
}

#[test]
fn inspection_is_read_only_and_uses_effective_theme_precedence() {
    let g = Guarded::new();
    let config = g.path("config");
    let data = g.path("data");
    write_theme(&config, "dusk", "#123456");
    write_theme(&data, "dusk", "#ffffff");
    write_theme(&config, "light", "#abcdef");
    fs::write(config.join("gnist/themes/data/light/light.mode"), "").unwrap();
    fs::write(config.join("gnist/themes/current.theme"), "dusk\n").unwrap();
    let run = |args: &[&str]| g.run(args);
    let output = run(&["inspect"]);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let result: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(result["themes"].as_array().unwrap().len(), 2);
    assert_eq!(result["themes"][0]["colors"]["accent"], "#123456");
    assert_eq!(result["themes"][1]["light"], true);
    assert!(!config.join("gnist/themes/generated").exists());
    assert_eq!(
        fs::read_to_string(config.join("gnist/themes/current.theme")).unwrap(),
        "dusk\n"
    );
    assert!(!run(&["inspect", "../../other"]).status.success());
    fs::write(
        config.join("gnist/themes/data/light/colors.toml"),
        "invalid TOML",
    )
    .unwrap();
    let result: serde_json::Value = serde_json::from_slice(&run(&["inspect"]).stdout).unwrap();
    assert!(result["themes"][1]["error"].is_string());
    assert_eq!(result["themes"][0]["id"], "dusk");
}

#[test]
fn rich_templates_expand_filters_blocks_and_filenames() {
    let g = Guarded::new();
    let config = g.path("config");
    let themes = config.join("gnist/themes");
    let theme = themes.join("data/dusk");
    fs::create_dir_all(&theme).unwrap();
    fs::create_dir_all(themes.join("templates")).unwrap();
    fs::write(theme.join("colors.toml"), "accent = \"#89b4fa\"\n").unwrap();
    fs::write(
        themes.join("templates/kitty.{{ name }}.conf.tpl"),
        "foreground {{ accent | lighten 0.1 }}\n\
         {{#if mode == \"dark\"}}theme=dark{{else}}theme=light{{/if}}\n",
    )
    .unwrap();
    fs::write(themes.join("templates/quiet.txt.tpl"), "{{ accent }}\n").unwrap();

    let set = g.run(&["set", "dusk", "--skip-apply"]);
    assert!(
        set.status.success(),
        "{}",
        String::from_utf8_lossy(&set.stderr)
    );
    g.assert_nothing_reached_a_tool();

    let rendered = fs::read_to_string(themes.join("generated/live/kitty.dusk.conf")).unwrap();
    assert_eq!(rendered, "foreground #95bcfb\ntheme=dark\n");
    assert_eq!(
        fs::read_to_string(themes.join("generated/live/quiet.txt")).unwrap(),
        "#89b4fa\n"
    );
}

#[test]
fn current_fails_after_active_state_is_cleared() {
    let g = Guarded::new();
    let output = g.run(&["current"]);

    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    assert_eq!(output.stderr, b"No current theme set\n");
}

#[test]
fn force_removing_current_theme_clears_reported_and_published_state() {
    let g = Guarded::new();
    let config = g.path("config");
    let themes = config.join("gnist/themes");
    let theme = themes.join("data/only");
    fs::create_dir_all(&theme).unwrap();
    fs::create_dir_all(themes.join("templates")).unwrap();
    fs::write(theme.join("colors.toml"), "accent = \"#123456\"\n").unwrap();
    fs::write(themes.join("templates/app.conf.tpl"), "{{ accent }}\n").unwrap();

    let set = g.run(&["set", "only", "--skip-apply"]);
    assert!(
        set.status.success(),
        "{}",
        String::from_utf8_lossy(&set.stderr)
    );
    g.assert_nothing_reached_a_tool();
    fs::write(themes.join("background"), "stale background").unwrap();

    let remove = g.run(&["remove", "only", "--yes", "--force"]);
    assert!(
        remove.status.success(),
        "{}",
        String::from_utf8_lossy(&remove.stderr)
    );

    let current = g.run(&["current"]);

    assert!(!current.status.success());
    assert!(current.stdout.is_empty());
    assert_eq!(current.stderr, b"No current theme set\n");
    assert!(fs::symlink_metadata(themes.join("current.theme")).is_err());
    assert!(fs::symlink_metadata(themes.join("current")).is_err());
    assert!(fs::symlink_metadata(themes.join("generated/live")).is_err());
    assert!(fs::symlink_metadata(themes.join("background")).is_err());
}

#[test]
fn force_removing_current_override_publishes_and_applies_lower_fallback() {
    let g = Guarded::new();
    let config = g.path("config");
    let data = g.path("data");
    let themes = config.join("gnist/themes");
    let config_theme = themes.join("data/shared");
    let fallback_theme = data.join("gnist/themes/data/shared");
    write_theme(&config, "shared", "#111111");
    write_theme(&data, "shared", "#222222");
    fs::create_dir_all(themes.join("templates")).unwrap();
    fs::create_dir_all(fallback_theme.join("backgrounds")).unwrap();
    fs::write(themes.join("templates/app.conf.tpl"), "{{ accent }}\n").unwrap();
    let fallback_background = fallback_theme.join("backgrounds/fallback.png");
    fs::write(&fallback_background, "image").unwrap();

    let set = g.run(&["set", "shared", "--skip-apply"]);
    assert!(
        set.status.success(),
        "{}",
        String::from_utf8_lossy(&set.stderr)
    );
    g.assert_nothing_reached_a_tool();
    fs::write(themes.join("background"), "stale background").unwrap();

    let remove = g.run(&["remove", "shared", "--yes", "--force"]);
    assert!(
        remove.status.success(),
        "{}",
        String::from_utf8_lossy(&remove.stderr)
    );

    assert!(!config_theme.exists());
    assert_eq!(
        fs::read_to_string(themes.join("current.theme")).unwrap(),
        "shared\n"
    );
    assert_eq!(
        fs::read_to_string(themes.join("generated/live/app.conf")).unwrap(),
        "#222222\n"
    );
    assert_eq!(
        fs::canonicalize(themes.join("background")).unwrap(),
        fs::canonicalize(&fallback_background).unwrap()
    );
    let calls = g.calls().expect("the stand-ins record every call");
    let published_background = themes.join("current/backgrounds/fallback.png");
    assert!(calls.lines().any(|line| line == "awww query"));
    assert!(calls.lines().any(|line| {
        line.starts_with("awww img ") && line.contains(&published_background.display().to_string())
    }));
}

#[test]
fn fallback_render_failure_preserves_source_and_active_state() {
    let g = Guarded::new();
    let config = g.path("config");
    let data = g.path("data");
    let themes = config.join("gnist/themes");
    let config_theme = themes.join("data/shared");
    write_theme(&config, "shared", "#111111");
    write_theme(&data, "shared", "#222222");
    fs::create_dir_all(themes.join("templates")).unwrap();
    fs::write(themes.join("templates/app.conf.tpl"), "{{ accent }}\n").unwrap();

    let set = g.run(&["set", "shared", "--skip-apply"]);
    assert!(
        set.status.success(),
        "{}",
        String::from_utf8_lossy(&set.stderr)
    );
    g.assert_nothing_reached_a_tool();
    fs::write(themes.join("templates/broken.tpl"), [0xff]).unwrap();
    fs::write(themes.join("background"), "current background").unwrap();

    let remove = g.run(&["remove", "shared", "--yes", "--force"]);

    assert!(!remove.status.success());
    g.assert_nothing_reached_a_tool();
    assert!(config_theme.is_dir());
    assert_eq!(
        fs::read_to_string(themes.join("current.theme")).unwrap(),
        "shared\n"
    );
    assert_eq!(
        fs::read_to_string(themes.join("generated/live/app.conf")).unwrap(),
        "#111111\n"
    );
    assert_eq!(
        fs::read_to_string(themes.join("current/app.conf")).unwrap(),
        "#111111\n"
    );
    assert_eq!(
        fs::read_to_string(themes.join("background")).unwrap(),
        "current background"
    );
}

/// The outputs a script reads stay exactly what they were.
///
/// Gnist renders through the shared Herda terminal design now, and the
/// temptation with a design crate is to put a heading above everything. But
/// `gnist current` prints a theme name that a keybinding or a status bar
/// substitutes straight into a command, `gnist list` prints one name per line,
/// and `gnist inspect` prints a JSON document. Decorating those because the
/// output happens to be a terminal would break every caller.
#[test]
fn scriptable_output_is_never_decorated() {
    let g = Guarded::new();
    let config = g.path("config");
    write_theme(&config, "dusk", "#123456");
    write_theme(&config, "noon", "#abcdef");
    fs::write(config.join("gnist/themes/current.theme"), "dusk\n").unwrap();
    let run = |args: &[&str]| g.run(args);

    let current = run(&["current"]);
    assert_eq!(current.stdout, b"dusk\n");
    assert!(current.stderr.is_empty());

    let list = run(&["list"]);
    assert_eq!(list.stdout, b"dusk (current)\nnoon\n");
    assert!(list.stderr.is_empty());

    let inspect = run(&["inspect"]);
    assert!(!inspect.stdout.contains(&b'\x1b'));
    serde_json::from_slice::<serde_json::Value>(&inspect.stdout).unwrap();

    // The reports do carry the design — and still no escapes off a terminal.
    let init = run(&["init"]);
    let text = String::from_utf8(init.stdout).unwrap();
    assert!(
        text.contains("gnist  Prepare the theme directories"),
        "{text}"
    );
    assert!(text.contains("DIRECTORY"), "{text}");
    assert!(!text.contains('\x1b'), "{text}");
}

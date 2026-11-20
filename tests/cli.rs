use std::{
    fs,
    os::unix::{fs::PermissionsExt, net::UnixListener},
    path::{Path, PathBuf},
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
    _awww_socket: UnixListener,
}

impl Guarded {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        for directory in ["config", "data", "cache", "state", "run"] {
            fs::create_dir(dir.path().join(directory)).unwrap();
        }
        fs::set_permissions(dir.path().join("run"), fs::Permissions::from_mode(0o700)).unwrap();
        let awww_socket =
            UnixListener::bind(dir.path().join("run/wayland-0-awww-daemon.sock")).unwrap();

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

        // A private in-memory stand-in: required settings are read back by a
        // separate process, just like gsettings against a real session store.
        let settings = dir.path().join("fake-settings");
        fs::create_dir(&settings).unwrap();
        write_executable(
            &bin.join("gsettings"),
            &format!(
                r##"#!/bin/sh
printf '%s\n' "gsettings $*" >> '{}'
file='{}/'$2'.'$3
case "$1" in
    set) printf 'unrelated module diagnostic\n' >&2; printf '%s' "$4" > "$file" ;;
    get) case "$3" in gtk-theme) printf "'%s'\n" "$(cat "$file")" ;; *) cat "$file"; printf '\n' ;; esac ;;
esac
"##,
                log.display(),
                settings.display()
            ),
        );
        // The guarded PATH holds no host utilities. Provide a tiny cat solely
        // for this fake settings store.
        write_executable(
            &bin.join("cat"),
            "#!/bin/sh\nwhile IFS= read -r line || [ -n \"$line\" ]; do printf '%s' \"$line\"; done < \"$1\"\n",
        );
        Self {
            dir,
            bin,
            log,
            _awww_socket: awww_socket,
        }
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
            .current_dir(self.dir.path())
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

/// The regression the guard exists for: a `set` that applies must reach the
/// recording stand-ins, with nothing of the real desktop in its environment.
#[test]
fn an_applying_set_only_reaches_the_stand_ins() {
    let g = Guarded::new();
    let config = g.path("config");
    write_theme(&config, "dusk", "#123456");
    let themes = config.join("gnist/themes");
    fs::create_dir_all(themes.join("templates")).unwrap();
    fs::write(
        themes.join("templates/gtk.css.tpl"),
        "window { color: {{ accent }}; }\n",
    )
    .unwrap();

    let set = g.run(&["set", "dusk"]);
    assert!(
        set.status.success(),
        "{}",
        String::from_utf8_lossy(&set.stderr)
    );

    let calls = g.calls().expect("the stand-ins record every call");
    assert!(
        calls.contains("gsettings set org.gnome.desktop.interface color-scheme prefer-dark"),
        "{calls}"
    );
    assert!(
        calls.contains("gsettings set org.gnome.desktop.interface gtk-theme adw-gtk3-dark"),
        "{calls}"
    );
    assert!(
        calls.contains(
            "dbus-send --session --type=signal /org/gnist/Appearance1 \
             org.gnist.Appearance1.Changed"
        ),
        "{calls}"
    );
    assert!(
        calls.contains("notify-send No wallpaper found for theme 'dusk' -t 2000"),
        "{calls}"
    );
    for line in calls.lines() {
        assert!(
            STAND_INS
                .iter()
                .any(|tool| line.starts_with(&format!("{tool} "))),
            "a tool outside the stand-ins was called: {line}"
        );
    }
}

#[test]
fn accessibility_settings_apply_after_theme_extras_and_restore_the_selected_theme() {
    let g = Guarded::new();
    let config = g.path("config");
    write_theme(&config, "dusk", "#123456");
    let themes = config.join("gnist/themes");
    fs::create_dir_all(themes.join("templates")).unwrap();
    fs::write(
        themes.join("templates/gtk.css.tpl"),
        "window { color: #000; }\n",
    )
    .unwrap();
    let settings = themes.join("templates/herda-accessibility.toml.tpl");
    fs::write(
        &settings,
        "enable-animations = false\nhigh-contrast = true\ntext-scaling-factor = 1.25\n",
    )
    .unwrap();
    fs::write(
        themes.join("data/dusk/gnome.toml"),
        "[[extra]]\nkey = 'enable-animations'\nvalue = 'true'\n",
    )
    .unwrap();
    let flags = [
        "set",
        "dusk",
        "--skip-reload",
        "--skip-icons",
        "--skip-wallpaper",
    ];
    let set = g.run(&flags);
    assert!(
        set.status.success(),
        "{}",
        String::from_utf8_lossy(&set.stderr)
    );
    let calls = g.calls().unwrap();
    assert!(
        calls.contains("gsettings set org.gnome.desktop.interface gtk-theme HighContrast"),
        "{calls}"
    );
    assert!(
        calls
            .find("gsettings set org.gnome.desktop.interface enable-animations false")
            .unwrap()
            < calls
                .find("gsettings set org.gnome.desktop.interface gtk-theme HighContrast")
                .unwrap(),
        "motion must be disabled before changing the theme: {calls}"
    );
    assert!(
        calls.contains("gsettings set org.gnome.desktop.interface text-scaling-factor 1.25"),
        "{calls}"
    );
    assert!(
        calls.contains("gsettings set org.gnome.desktop.a11y.interface high-contrast true"),
        "{calls}"
    );
    assert!(
        calls
            .find("gsettings set org.gnome.desktop.a11y.interface high-contrast true")
            .unwrap()
            < calls.find("dbus-send --session --type=signal").unwrap(),
        "accessibility settings must precede the change signal: {calls}"
    );
    assert!(
        calls
            .rfind("gsettings set org.gnome.desktop.interface enable-animations false")
            .unwrap()
            > calls
                .rfind("gsettings set org.gnome.desktop.interface enable-animations true")
                .unwrap(),
        "{calls}"
    );

    fs::write(
        &settings,
        "enable-animations = true\nhigh-contrast = false\ntext-scaling-factor = 1.0\n",
    )
    .unwrap();
    let set = g.run(&flags);
    assert!(
        set.status.success(),
        "{}",
        String::from_utf8_lossy(&set.stderr)
    );
    let calls = g.calls().unwrap();
    assert!(
        calls.contains("gsettings set org.gnome.desktop.interface gtk-theme adw-gtk3-dark"),
        "{calls}"
    );
    assert!(
        calls.contains("gsettings set org.gnome.desktop.a11y.interface high-contrast false"),
        "{calls}"
    );
}

#[test]
fn accessibility_preserves_custom_theme_targets_and_reports_failed_settings() {
    let g = Guarded::new();
    let config = g.path("config");
    write_theme(&config, "dusk", "#123456");
    let themes = config.join("gnist/themes");
    fs::create_dir_all(themes.join("templates")).unwrap();
    fs::write(
        themes.join("templates/gtk.css.tpl"),
        "window { color: #000; }\n",
    )
    .unwrap();
    fs::write(
        themes.join("data/dusk/gnome.toml"),
        "schema = 'org.example.Theme'\ngtk_theme_key = 'theme'\n",
    )
    .unwrap();
    fs::write(
        themes.join("templates/herda-accessibility.toml.tpl"),
        "enable-animations = false\nhigh-contrast = true\ntext-scaling-factor = 1.25\n",
    )
    .unwrap();
    let flags = [
        "set",
        "dusk",
        "--skip-reload",
        "--skip-icons",
        "--skip-wallpaper",
    ];
    let set = g.run(&flags);
    assert!(
        set.status.success(),
        "{}",
        String::from_utf8_lossy(&set.stderr)
    );
    let calls = g.calls().unwrap();
    assert!(
        calls.contains("gsettings set org.example.Theme theme adw-gtk3-dark"),
        "{calls}"
    );
    assert!(
        calls.contains("gsettings set org.gnome.desktop.interface gtk-theme HighContrast"),
        "{calls}"
    );

    write_executable(&g.bin.join("gsettings"), "#!/bin/sh\nexit 2\n");
    fs::write(&g.log, "").unwrap();
    let failed = g.run(&flags);
    assert!(
        failed.status.success(),
        "settings failure blocked the published theme"
    );
    assert!(String::from_utf8_lossy(&failed.stderr).contains("gsettings failed"));
    let calls = g.calls().unwrap();
    assert!(
        calls.contains("dbus-send --session --type=signal"),
        "CSS change signal missing: {calls}"
    );
    let direct = g.run(&["gnome", "--skip-icons"]);
    assert!(
        !direct.status.success(),
        "direct settings apply lost its failure status"
    );
    assert!(String::from_utf8_lossy(&direct.stderr).contains("gsettings failed"));

    // The dconf CLI can emit a commit warning while returning success.
    write_executable(
        &g.bin.join("gsettings"),
        "#!/bin/sh\nprintf 'failed to commit changes to dconf\\n' >&2\nexit 0\n",
    );
    let warned = g.run(&flags);
    assert!(
        warned.status.success(),
        "dconf failure blocked the published theme"
    );
    assert!(String::from_utf8_lossy(&warned.stderr).contains("failed to commit changes to dconf"));
    let direct = g.run(&["gnome", "--skip-icons"]);
    assert!(!direct.status.success(), "direct dconf failure was ignored");

    // dconf may report a successful set and silently discard it. The other
    // required values read back correctly, but the changed contrast does not.
    write_executable(
        &g.bin.join("gsettings"),
        r##"#!/bin/sh
if [ "$1" = get ]; then
  case "$3" in
    enable-animations) printf 'false\n' ;;
    text-scaling-factor) printf '1.25\n' ;;
    high-contrast) printf 'false\n' ;;
  esac
fi
exit 0
"##,
    );
    let dropped = g.run(&flags);
    assert!(
        dropped.status.success(),
        "dropped setting blocked the published theme"
    );
    assert!(
        String::from_utf8_lossy(&dropped.stderr)
            .contains("read-back mismatch for org.gnome.desktop.a11y.interface high-contrast")
    );
    let direct = g.run(&["gnome", "--skip-icons"]);
    assert!(
        !direct.status.success(),
        "direct silently dropped write was accepted"
    );
    assert!(
        String::from_utf8_lossy(&direct.stderr)
            .contains("read-back mismatch for org.gnome.desktop.a11y.interface high-contrast")
    );
}

#[test]
fn skipped_gnome_never_writes_accessibility_settings() {
    let g = Guarded::new();
    let config = g.path("config");
    write_theme(&config, "dusk", "#123456");
    let templates = config.join("gnist/themes/templates");
    fs::create_dir_all(&templates).unwrap();
    fs::write(
        templates.join("herda-accessibility.toml.tpl"),
        "enable-animations = false\nhigh-contrast = true\ntext-scaling-factor = 1.25\n",
    )
    .unwrap();
    let set = g.run(&[
        "set",
        "dusk",
        "--skip-gnome",
        "--skip-reload",
        "--skip-icons",
        "--skip-wallpaper",
    ]);
    assert!(
        set.status.success(),
        "{}",
        String::from_utf8_lossy(&set.stderr)
    );
    assert!(!g.calls().unwrap_or_default().contains("gsettings "));
}

#[test]
fn missing_wallpaper_socket_skips_awww_without_stalling() {
    let g = Guarded::new();
    fs::remove_file(g.path("run/wayland-0-awww-daemon.sock")).unwrap();
    let config = g.path("config");
    write_theme(&config, "dusk", "#123456");
    let themes = config.join("gnist/themes");
    fs::create_dir_all(themes.join("templates")).unwrap();
    fs::create_dir_all(themes.join("data/dusk/backgrounds")).unwrap();
    fs::write(themes.join("templates/gtk.css.tpl"), "window {}\n").unwrap();
    fs::write(themes.join("data/dusk/backgrounds/0-night.png"), "image").unwrap();

    let set = g.run(&[
        "set",
        "dusk",
        "--skip-gnome",
        "--skip-reload",
        "--skip-icons",
    ]);
    assert!(
        set.status.success(),
        "{}",
        String::from_utf8_lossy(&set.stderr)
    );
    assert!(
        !g.calls().unwrap_or_default().contains("awww "),
        "awww should not be spawned when no daemon socket exists"
    );
}

#[test]
fn omarchy4_cloned_and_local_theme_rendering_and_manifest() {
    use std::os::unix::fs::symlink;

    let g = Guarded::new();
    let config = g.path("config");
    let themes = config.join("gnist/themes");
    let theme = themes.join("data/omatest");
    let templates = themes.join("templates");
    fs::create_dir_all(theme.join(".git/objects")).unwrap();
    fs::create_dir_all(theme.join("backgrounds/intros")).unwrap();
    fs::create_dir_all(&templates).unwrap();

    fs::write(
        theme.join("colors.toml"),
        r##"mode = "light"
name = "Oma Test"
look = "omarchy"
accent = "#7aa2f7"
selection = "#292e42"
muted = "#414868"
background = "#f0f0f0"
foreground = "#222222"
bright_foreground = "#111111"
red = "#f7768e"
blue = "#7aa2f7"
hyprland_active_border = "rgba(33ccffee) rgba(00ff99ee) 45deg"
"##,
    )
    .unwrap();
    fs::write(theme.join(".git/HEAD"), "ref: refs/heads/main\n").unwrap();
    fs::write(theme.join(".git/objects/blob"), "blob").unwrap();
    fs::write(theme.join("kitty.conf"), "shell /bin/evil\n").unwrap();
    fs::write(theme.join("neovim.lua"), "os.execute('evil')\n").unwrap();
    fs::write(theme.join("README.md"), "# readme\n").unwrap();
    fs::write(theme.join("LICENSE"), "MIT\n").unwrap();
    fs::write(theme.join("preview.png"), "png").unwrap();
    fs::write(theme.join("backgrounds/notes.txt"), "not a wallpaper\n").unwrap();
    fs::write(theme.join("backgrounds/wall.png"), "png").unwrap();
    fs::write(theme.join("backgrounds/intros/wall.mp4"), "video").unwrap();
    let outside = g.path("outside.txt");
    fs::write(&outside, "TOP-SECRET").unwrap();
    symlink(&outside, theme.join("linked.conf")).unwrap();

    fs::write(
        templates.join("probe.conf.tpl"),
        "color1={{ color1 }}\n\
         cursor={{ cursor }}\n\
         selbg={{ selection_background }}\n\
         red={{ red }}\n\
         mix={{ mix background foreground 15% }}\n\
         mix_strip={{ mix_strip background accent 0.35 }}\n\
         hypr={{ hypr_gradient hyprland_active_border accent }}\n\
         shell={{ shell_gradient hyprland_active_border accent }}\n\
         start={{ gradient_start hyprland_active_border accent }}\n\
         mode={{ mode }}\n\
         is_light={{ is_light }}\n",
    )
    .unwrap();
    fs::write(
        templates.join("kitty.conf.tpl"),
        "global_kitty {{ color1 }}\n",
    )
    .unwrap();

    // 1. Cloned (.git present) theme.
    let set = g.run(&["set", "omatest", "--skip-apply"]);
    assert!(
        set.status.success(),
        "{}",
        String::from_utf8_lossy(&set.stderr)
    );
    let stderr = String::from_utf8_lossy(&set.stderr);
    assert!(stderr.contains("warn:"), "expected warn line: {stderr}");
    assert!(stderr.contains("kitty.conf"), "{stderr}");
    assert!(stderr.contains("neovim.lua"), "{stderr}");
    assert!(stderr.contains("linked.conf"), "{stderr}");

    let probe = fs::read_to_string(themes.join("current/probe.conf")).unwrap();
    assert!(!probe.contains("{{"), "unrendered placeholder in:\n{probe}");
    assert!(probe.contains("color1=#f7768e\n"), "{probe}");
    assert!(probe.contains("cursor=#111111\n"), "{probe}");
    assert!(probe.contains("selbg=#292e42\n"), "{probe}");
    assert!(probe.contains("mode=light\n"), "{probe}");
    assert!(probe.contains("is_light=true\n"), "{probe}");
    assert!(probe.contains("start=#33ccff\n"), "{probe}");
    assert!(themes.join("current/light.mode").is_file());

    // Cloned theme drops .git, README, LICENSE, preview, symlinks, and denied configs.
    let live = themes.join("generated/live");
    assert_eq!(
        fs::read_to_string(live.join("kitty.conf")).unwrap(),
        "global_kitty #f7768e\n"
    );
    for absent in [
        ".git",
        "README.md",
        "LICENSE",
        "preview.png",
        "neovim.lua",
        "linked.conf",
    ] {
        assert!(!live.join(absent).exists(), "{absent} leaked into live");
    }

    // palette.json manifest is published atomically in current/.
    let manifest: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(themes.join("current/palette.json")).unwrap())
            .unwrap();
    assert_eq!(manifest["schema_version"], 1);
    assert_eq!(manifest["name"], "omatest");
    assert_eq!(manifest["display_name"], "Oma Test");
    assert_eq!(manifest["mode"], "light");
    assert_eq!(manifest["revision"], 1);
    assert_eq!(manifest["colors"]["color1"], "#f7768e");
    assert_eq!(manifest["colors"]["cursor"], "#111111");
    assert_eq!(manifest["values"]["look"], "omarchy");
    assert_eq!(
        manifest["values"]["hyprland_active_border"],
        "rgba(33ccffee) rgba(00ff99ee) 45deg"
    );
    let wallpapers = manifest["wallpapers"].as_array().unwrap();
    assert_eq!(wallpapers.len(), 1);
    assert!(wallpapers[0].as_str().unwrap().ends_with("wall.png"));

    // gnist inspect agrees with palette.json and filters wallpapers.
    let inspect = g.run(&["inspect", "omatest"]);
    assert!(inspect.status.success());
    let inspected: serde_json::Value = serde_json::from_slice(&inspect.stdout).unwrap();
    let entry = &inspected["themes"][0];
    assert_eq!(entry["id"], "omatest");
    assert_eq!(entry["name"], "Oma Test");
    assert_eq!(entry["light"], true);
    assert_eq!(entry["mode"], "light");
    assert_eq!(entry["generated"], false);
    assert!(entry["preview"].as_str().unwrap().ends_with("preview.png"));
    assert_eq!(entry["wallpapers"].as_array().unwrap().len(), 1);

    // 2. Local theme (no .git): theme kitty.conf and symlink are kept, clutter still skipped.
    fs::remove_dir_all(theme.join(".git")).unwrap();
    let set_local = g.run(&["set", "omatest", "--skip-apply"]);
    assert!(
        set_local.status.success(),
        "{}",
        String::from_utf8_lossy(&set_local.stderr)
    );
    assert_eq!(
        fs::read_to_string(live.join("kitty.conf")).unwrap(),
        "shell /bin/evil\n"
    );
    assert_eq!(
        fs::read_to_string(live.join("linked.conf")).unwrap(),
        "TOP-SECRET"
    );
    assert!(!live.join("README.md").exists());
    assert!(!live.join("preview.png").exists());

    let manifest2: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(themes.join("current/palette.json")).unwrap())
            .unwrap();
    assert_eq!(manifest2["revision"], 2);
}

#[test]
fn reload_propagates_theme_errors_and_set_is_quiet_without_bindings() {
    let g = Guarded::new();
    let config = g.path("config");

    // No current theme -> gnist reload fails.
    let missing = g.run(&["reload"]);
    assert!(!missing.status.success());

    write_theme(&config, "dusk", "#123456");
    let themes = config.join("gnist/themes");
    fs::create_dir_all(themes.join("templates")).unwrap();
    fs::write(themes.join("templates/gtk.css.tpl"), "window {}\n").unwrap();

    // gnist set without bindings does NOT print "info: no reload bindings found".
    let set = g.run(&[
        "set",
        "dusk",
        "--skip-gnome",
        "--skip-icons",
        "--skip-wallpaper",
    ]);
    assert!(
        set.status.success(),
        "{}",
        String::from_utf8_lossy(&set.stderr)
    );
    assert!(
        !String::from_utf8_lossy(&set.stderr).contains("no reload bindings found"),
        "{}",
        String::from_utf8_lossy(&set.stderr)
    );

    // Explicit gnist reload DOES print the notice when no bindings exist.
    let reload = g.run(&["reload"]);
    assert!(reload.status.success());
    assert!(
        String::from_utf8_lossy(&reload.stderr).contains("no reload bindings found"),
        "{}",
        String::from_utf8_lossy(&reload.stderr)
    );

    // Broken current theme -> gnist reload returns an error instead of empty vars.
    fs::write(themes.join("data/dusk/colors.toml"), "not = [valid\n").unwrap();
    let broken = g.run(&["reload"]);
    assert!(
        !broken.status.success(),
        "gnist reload hid broken current theme"
    );
}

#[test]
fn hung_gsettings_times_out_and_still_emits_css_signal() {
    let g = Guarded::new();
    let config = g.path("config");
    write_theme(&config, "dusk", "#123456");
    let themes = config.join("gnist/themes");
    fs::create_dir_all(themes.join("templates")).unwrap();
    fs::write(themes.join("templates/gtk.css.tpl"), "window {}\n").unwrap();

    // Stand-in gsettings that hangs indefinitely.
    write_executable(&g.bin.join("gsettings"), "#!/bin/sh\nexec /bin/sleep 30\n");

    let started = std::time::Instant::now();
    let set = g.run(&[
        "set",
        "dusk",
        "--skip-reload",
        "--skip-icons",
        "--skip-wallpaper",
    ]);
    let elapsed = started.elapsed();

    assert!(
        set.status.success(),
        "{}",
        String::from_utf8_lossy(&set.stderr)
    );
    assert!(
        elapsed < std::time::Duration::from_secs(8),
        "hung gsettings stalled too long: {elapsed:?}"
    );
    assert!(
        String::from_utf8_lossy(&set.stderr).contains("timed out"),
        "{}",
        String::from_utf8_lossy(&set.stderr)
    );
    let calls = g.calls().unwrap_or_default();
    assert!(
        calls.contains("dbus-send --session --type=signal"),
        "CSS change signal should still fire after gsettings timeout: {calls}"
    );
}

fn rust_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            rust_files(&path, out);
        } else if path.extension().and_then(|extension| extension.to_str()) == Some("rs") {
            out.push(path);
        }
    }
}

#[test]
fn only_the_guarded_builder_spawns_the_binary() {
    // The token is built from pieces so this scan does not match itself.
    let token = ["CARGO_BIN_EXE_", "gnist"].concat();
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let builder = root.join("tests/cli.rs");
    let mut files = Vec::new();
    rust_files(&root.join("tests"), &mut files);
    rust_files(&root.join("src"), &mut files);
    for path in files {
        let source = fs::read_to_string(&path).unwrap();
        let count = source.matches(&token).count();
        if path == builder {
            assert_eq!(count, 1, "spawn gnist only through Guarded::run: {path:?}");
        } else {
            assert_eq!(
                count,
                0,
                "{} names the gnist binary path directly; use Guarded::run",
                path.display()
            );
        }
    }
}

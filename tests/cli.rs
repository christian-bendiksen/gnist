use std::{fs, os::unix::fs::PermissionsExt, process::Command};

fn gnist() -> Command {
    Command::new(env!("CARGO_BIN_EXE_gnist"))
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
    let dir = tempfile::tempdir().unwrap();
    let config = dir.path().join("config");
    let data = dir.path().join("data");
    write_theme(&config, "dusk", "#123456");
    write_theme(&data, "dusk", "#ffffff");
    write_theme(&config, "light", "#abcdef");
    fs::write(config.join("gnist/themes/data/light/light.mode"), "").unwrap();
    fs::write(config.join("gnist/themes/current.theme"), "dusk\n").unwrap();
    let run = |args: &[&str]| gnist().args(args).env("HOME", dir.path())
        .env("XDG_CONFIG_HOME", &config).env("XDG_DATA_HOME", &data).env("XDG_DATA_DIRS", "").output().unwrap();
    let output = run(&["inspect"]);
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    let result: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(result["themes"].as_array().unwrap().len(), 2);
    assert_eq!(result["themes"][0]["colors"]["accent"], "#123456");
    assert_eq!(result["themes"][1]["light"], true);
    assert!(!config.join("gnist/themes/generated").exists());
    assert_eq!(fs::read_to_string(config.join("gnist/themes/current.theme")).unwrap(), "dusk\n");
    assert!(!run(&["inspect", "../../other"]).status.success());
    fs::write(config.join("gnist/themes/data/light/colors.toml"), "invalid TOML").unwrap();
    let result: serde_json::Value = serde_json::from_slice(&run(&["inspect"]).stdout).unwrap();
    assert!(result["themes"][1]["error"].is_string());
    assert_eq!(result["themes"][0]["id"], "dusk");
}

#[test]
fn rich_templates_expand_filters_blocks_and_filenames() {
    let dir = tempfile::tempdir().unwrap();
    let config = dir.path().join("config");
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

    let set = gnist()
        .args(["set", "dusk", "--skip-apply"])
        .env("HOME", dir.path())
        .env("XDG_CONFIG_HOME", &config)
        .env("XDG_DATA_HOME", dir.path().join("data"))
        .env("XDG_DATA_DIRS", "")
        .output()
        .unwrap();
    assert!(
        set.status.success(),
        "{}",
        String::from_utf8_lossy(&set.stderr)
    );

    let rendered =
        fs::read_to_string(themes.join("generated/live/kitty.dusk.conf")).unwrap();
    assert_eq!(rendered, "foreground #95bcfb\ntheme=dark\n");
    assert_eq!(
        fs::read_to_string(themes.join("generated/live/quiet.txt")).unwrap(),
        "#89b4fa\n"
    );
}

#[test]
fn current_fails_after_active_state_is_cleared() {
    let dir = tempfile::tempdir().unwrap();
    let output = gnist()
        .arg("current")
        .env("HOME", dir.path())
        .env("XDG_CONFIG_HOME", dir.path().join("config"))
        .env("XDG_DATA_HOME", dir.path().join("data"))
        .env("XDG_DATA_DIRS", "")
        .output()
        .unwrap();

    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    assert_eq!(output.stderr, b"No current theme set\n");
}

#[test]
fn force_removing_current_theme_clears_reported_and_published_state() {
    let dir = tempfile::tempdir().unwrap();
    let config = dir.path().join("config");
    let themes = config.join("gnist/themes");
    let theme = themes.join("data/only");
    fs::create_dir_all(&theme).unwrap();
    fs::create_dir_all(themes.join("templates")).unwrap();
    fs::write(theme.join("colors.toml"), "accent = \"#123456\"\n").unwrap();
    fs::write(themes.join("templates/app.conf.tpl"), "{{ accent }}\n").unwrap();

    let set = gnist()
        .args(["set", "only", "--skip-apply"])
        .env("HOME", dir.path())
        .env("XDG_CONFIG_HOME", &config)
        .env("XDG_DATA_HOME", dir.path().join("data"))
        .env("XDG_DATA_DIRS", "")
        .output()
        .unwrap();
    assert!(
        set.status.success(),
        "{}",
        String::from_utf8_lossy(&set.stderr)
    );
    fs::write(themes.join("background"), "stale background").unwrap();

    let remove = gnist()
        .args(["remove", "only", "--yes", "--force"])
        .env("HOME", dir.path())
        .env("XDG_CONFIG_HOME", &config)
        .env("XDG_DATA_HOME", dir.path().join("data"))
        .env("XDG_DATA_DIRS", "")
        .output()
        .unwrap();
    assert!(
        remove.status.success(),
        "{}",
        String::from_utf8_lossy(&remove.stderr)
    );

    let current = gnist()
        .arg("current")
        .env("HOME", dir.path())
        .env("XDG_CONFIG_HOME", &config)
        .env("XDG_DATA_HOME", dir.path().join("data"))
        .env("XDG_DATA_DIRS", "")
        .output()
        .unwrap();

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
    let dir = tempfile::tempdir().unwrap();
    let config = dir.path().join("config");
    let data = dir.path().join("data");
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

    let set = gnist()
        .args(["set", "shared", "--skip-apply"])
        .env("HOME", dir.path())
        .env("XDG_CONFIG_HOME", &config)
        .env("XDG_DATA_HOME", &data)
        .env("XDG_DATA_DIRS", "")
        .output()
        .unwrap();
    assert!(
        set.status.success(),
        "{}",
        String::from_utf8_lossy(&set.stderr)
    );
    fs::write(themes.join("background"), "stale background").unwrap();

    let bin = dir.path().join("bin");
    fs::create_dir(&bin).unwrap();
    let awww_log = dir.path().join("awww.log");
    write_executable(
        &bin.join("awww"),
        "#!/bin/sh\nprintf '%s\\n' \"$*\" >> \"$AWWW_LOG\"\n",
    );

    let remove = gnist()
        .args(["remove", "shared", "--yes", "--force"])
        .env("HOME", dir.path())
        .env("XDG_CONFIG_HOME", &config)
        .env("XDG_DATA_HOME", &data)
        .env("XDG_DATA_DIRS", "")
        .env("PATH", &bin)
        .env("AWWW_LOG", &awww_log)
        .output()
        .unwrap();
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
    let awww_calls = fs::read_to_string(awww_log).unwrap();
    let published_background = themes.join("current/backgrounds/fallback.png");
    assert!(awww_calls.lines().any(|line| line == "query"));
    assert!(awww_calls.lines().any(|line| {
        line.starts_with("img ") && line.contains(&published_background.display().to_string())
    }));
}

#[test]
fn fallback_render_failure_preserves_source_and_active_state() {
    let dir = tempfile::tempdir().unwrap();
    let config = dir.path().join("config");
    let data = dir.path().join("data");
    let themes = config.join("gnist/themes");
    let config_theme = themes.join("data/shared");
    write_theme(&config, "shared", "#111111");
    write_theme(&data, "shared", "#222222");
    fs::create_dir_all(themes.join("templates")).unwrap();
    fs::write(themes.join("templates/app.conf.tpl"), "{{ accent }}\n").unwrap();

    let set = gnist()
        .args(["set", "shared", "--skip-apply"])
        .env("HOME", dir.path())
        .env("XDG_CONFIG_HOME", &config)
        .env("XDG_DATA_HOME", &data)
        .env("XDG_DATA_DIRS", "")
        .output()
        .unwrap();
    assert!(
        set.status.success(),
        "{}",
        String::from_utf8_lossy(&set.stderr)
    );
    fs::write(themes.join("templates/broken.tpl"), [0xff]).unwrap();
    fs::write(themes.join("background"), "current background").unwrap();

    let remove = gnist()
        .args(["remove", "shared", "--yes", "--force"])
        .env("HOME", dir.path())
        .env("XDG_CONFIG_HOME", &config)
        .env("XDG_DATA_HOME", &data)
        .env("XDG_DATA_DIRS", "")
        .output()
        .unwrap();

    assert!(!remove.status.success());
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
    let dir = tempfile::tempdir().unwrap();
    let config = dir.path().join("config");
    let data = dir.path().join("data");
    write_theme(&config, "dusk", "#123456");
    write_theme(&config, "noon", "#abcdef");
    fs::write(config.join("gnist/themes/current.theme"), "dusk\n").unwrap();
    let run = |args: &[&str]| {
        gnist()
            .args(args)
            .env("HOME", dir.path())
            .env("XDG_CONFIG_HOME", &config)
            .env("XDG_DATA_HOME", &data)
            .env("XDG_DATA_DIRS", "")
            .output()
            .unwrap()
    };

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
    assert!(text.contains("gnist  Prepare the theme directories"), "{text}");
    assert!(text.contains("DIRECTORY"), "{text}");
    assert!(!text.contains('\x1b'), "{text}");
}

fn accent_fixture() -> serde_json::Value {
    use serde_json::json;
    let mode = |color: &str, ink: &str| {
        let mut roles = serde_json::Map::new();
        for key in ["accent", "accent_dim", "accent_bg", "accent_text", "on_accent", "focus_ring",
                    "selection_background", "selection_foreground", "cursor", "border_active", "brand_herda"] {
            roles.insert(key.to_owned(), json!(if key == "on_accent" { ink } else { color }));
        }
        json!({"swatch": color, "roles": roles})
    };
    json!({"schema": 1, "default": "clay", "choices": [
        {"id": "clay", "label": "Clay", "modes": {"dark": mode("#EEEEEE", "#111111"), "light": mode("#222222", "#FFFFFF")}},
        {"id": "sea", "label": "Sea", "modes": {"dark": mode("#99BDF1", "#111111"), "light": mode("#325890", "#FFFFFF")}}
    ]})
}

#[test]
fn accent_selection_reapplies_all_derived_roles_and_survives_modes_and_fixed_themes() {
    let dir = tempfile::tempdir().unwrap();
    let config = dir.path().join("config");
    let themes = config.join("gnist/themes");
    for name in ["atelier", "atelier-light", "fixed"] {
        write_theme(&config, name, "#EEEEEE");
        fs::write(themes.join(format!("data/{name}/colors.toml")), "accent = \"#EEEEEE\"\nbackground = \"#121212\"\ncolor4 = \"#123456\"\n").unwrap();
        if name != "fixed" {
            fs::write(themes.join(format!("data/{name}/accent-choices.json")), accent_fixture().to_string()).unwrap();
        }
    }
    fs::write(themes.join("data/atelier-light/light.mode"), "").unwrap();
    fs::create_dir_all(themes.join("templates")).unwrap();
    fs::write(themes.join("templates/palette.txt.tpl"),
        "{{ accent }} {{ accent_rgb }} {{ accent_strip }} {{ border_active }} {{ border_active_rgb }} {{ on_accent }} {{ background }} {{ color4 }}\n").unwrap();
    let run = |args: &[&str]| gnist().args(args).env("HOME", dir.path())
        .env("XDG_CONFIG_HOME", &config).env("XDG_DATA_HOME", dir.path().join("data"))
        .env("XDG_DATA_DIRS", dir.path().join("system")).output().unwrap();
    let palette = || fs::read_to_string(themes.join("generated/live/palette.txt")).unwrap();
    let set = |theme: &str| {
        let output = run(&["set", theme, "--skip-apply"]);
        assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    };
    set("atelier");
    assert!(palette().starts_with("#EEEEEE 238,238,238 eeeeee"));
    fs::write(config.join("gnist/accent.json"), r#"{"accent":"sea"}"#).unwrap();
    let output = run(&["reapply", "--skip-apply"]);
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    assert_eq!(palette(), "#99BDF1 153,189,241 99bdf1 #99BDF1 153,189,241 #111111 #121212 #123456\n");
    let inspect: serde_json::Value = serde_json::from_slice(&run(&["inspect", "atelier"]).stdout).unwrap();
    assert_eq!(inspect["themes"][0]["accent"], "sea");
    assert_eq!(inspect["themes"][0]["accentChoices"][1], serde_json::json!({"id":"sea", "label":"Sea", "color":"#99BDF1"}));
    set("atelier-light");
    assert_eq!(palette(), "#325890 50,88,144 325890 #325890 50,88,144 #FFFFFF #121212 #123456\n");
    set("fixed");
    let fixed = palette();
    let inspect: serde_json::Value = serde_json::from_slice(&run(&["inspect", "fixed"]).stdout).unwrap();
    assert_eq!(inspect["themes"][0]["accentChoices"], serde_json::json!([]));
    // A fixed theme ignores even a broken selection, preserving the legacy path byte for byte.
    fs::write(config.join("gnist/accent.json"), "broken").unwrap();
    set("fixed");
    assert_eq!(palette(), fixed);
    fs::write(config.join("gnist/accent.json"), r#"{"accent":"sea"}"#).unwrap();
    set("atelier");
    assert!(palette().starts_with("#99BDF1"));
    // A saved choice from another declaring theme uses this theme's own default.
    fs::write(config.join("gnist/accent.json"), r#"{"accent":"elsewhere"}"#).unwrap();
    set("atelier");
    assert!(palette().starts_with("#EEEEEE"));
}

#[test]
fn invalid_accent_metadata_never_publishes_a_partial_palette() {
    let dir = tempfile::tempdir().unwrap();
    let config = dir.path().join("config");
    let themes = config.join("gnist/themes");
    write_theme(&config, "atelier", "#EEEEEE");
    fs::create_dir_all(themes.join("templates")).unwrap();
    fs::write(themes.join("templates/palette.txt.tpl"), "{{ accent }}").unwrap();
    let metadata = themes.join("data/atelier/accent-choices.json");
    fs::write(&metadata, accent_fixture().to_string()).unwrap();
    let run = || gnist().args(["set", "atelier", "--skip-apply"]).env("HOME", dir.path())
        .env("XDG_CONFIG_HOME", &config).env("XDG_DATA_HOME", dir.path().join("data"))
        .env("XDG_DATA_DIRS", dir.path().join("system")).output().unwrap();
    assert!(run().status.success());
    let before = fs::read(themes.join("generated/live/palette.txt")).unwrap();
    for case in 0..6 {
        let mut data = accent_fixture();
        match case {
            0 => data["choices"][1]["modes"]["light"]["roles"]["on_accent"] = serde_json::json!("#325890"),
            1 => data["choices"][1]["modes"]["dark"]["roles"]["background"] = serde_json::json!("#FFFFFF"),
            2 => data["choices"][1]["modes"]["dark"]["roles"]["accent"] = serde_json::json!("not a color"),
            3 => data["choices"][1]["id"] = serde_json::json!("clay"),
            4 => data["default"] = serde_json::json!("missing"),
            _ => { data["choices"][1]["modes"]["dark"]["roles"].as_object_mut().unwrap().remove("selection_foreground"); }
        }
        fs::write(&metadata, data.to_string()).unwrap();
        assert!(!run().status.success(), "invalid case {case} was accepted");
        assert_eq!(fs::read(themes.join("generated/live/palette.txt")).unwrap(), before);
    }
}

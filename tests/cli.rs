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

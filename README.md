# Gnist

Gnist is a runtime theme switcher for Wayland desktops. It reads one palette,
renders application-specific theme files, and publishes the result under a
stable `current` path.

## Install

Gnist builds on Linux with a current stable Rust toolchain.

```sh
git clone https://github.com/christian-bendiksen/gnist.git
cd gnist
cargo install --locked --path .
gnist init
```

`gnist init` creates the directory structure but does not install or apply a
theme.

Theme data follows `XDG_CONFIG_HOME`, with `~/.config` as the default. The
examples below use this variable:

```sh
GNIST_CONFIG="${XDG_CONFIG_HOME:-$HOME/.config}/gnist"
```

## Create a theme

Create a theme directory:

```sh
mkdir -p "$GNIST_CONFIG/themes/data/dusk"
```

Add `$GNIST_CONFIG/themes/data/dusk/colors.toml`:

```toml
accent               = "#89b4fa"
foreground           = "#cdd6f4"
background           = "#1e1e2e"
cursor               = "#f5e0dc"
selection_foreground = "#cdd6f4"
selection_background = "#45475a"
color0               = "#45475a"
color8               = "#585b70"
```

Only values used by your templates are required. String, integer, float, and
boolean values are supported. Nested TOML keys are joined with underscores.

For a light theme, add an empty `light.mode` file:

```sh
touch "$GNIST_CONFIG/themes/data/dusk/light.mode"
```

Optional theme content includes:

- `backgrounds/` for wallpapers
- `icons.theme` containing a GTK icon theme name
- application files that should be copied without template expansion

## Add templates

Templates live in `$GNIST_CONFIG/themes/templates/` and use `{{ name }}`
placeholders.

Create `$GNIST_CONFIG/themes/templates/kitty.conf.tpl`:

```text
foreground {{ foreground }}
background {{ background }}
cursor     {{ cursor }}
color0     {{ color0 }}
```

Create `$GNIST_CONFIG/themes/templates/gtk.css.tpl` if Gnist should manage GTK
colors:

```css
@define-color accent_color {{ accent }};
@define-color window_bg_color {{ background }};
@define-color window_fg_color {{ foreground }};
```

A six-digit color also provides stripped and decimal RGB forms:

| Template value | Result for `#cdd6f4` |
|---|---|
| `{{ foreground }}` | `#cdd6f4` |
| `{{ foreground_strip }}` | `cdd6f4` |
| `{{ foreground_rgb }}` | `205,214,244` |

Unknown placeholders remain in the output so missing values are visible.

Source precedence is:

1. `themes/user-templates/`
2. Files from the selected theme
3. `themes/templates/`

## Connect applications

Render the theme without running desktop integrations:

```sh
gnist set dusk --skip-apply
```

Rendered files are published under:

```text
$GNIST_CONFIG/themes/current/
```

Point each application at the relevant file. For example, with the default
config location, Kitty can include:

```text
include ~/.config/gnist/themes/current/kitty.conf
```

Applications without an include directive can use a symlink instead.

For GTK, Gnist manages these standard paths:

```text
~/.config/gtk-3.0/gtk.css
~/.config/gtk-4.0/gtk.css
```

Create the parent directories first:

```sh
mkdir -p ~/.config/gtk-3.0 ~/.config/gtk-4.0
```

A normal `gnist set` replaces an existing `gtk.css` file or symlink without a
backup, so preserve anything you still need. Use `--skip-apply` if Gnist should
render files without managing GTK or other desktop integrations.

## Configure reload bindings

Gnist reads reload actions from `$GNIST_CONFIG/bindings.kdl`. These actions run
after `gnist set` unless `--skip-reload` or `--skip-apply` is used. You can also
run them directly with `gnist reload`.

Create `$GNIST_CONFIG/bindings.kdl` manually:

```kdl
bind "waybar" {
    reload {
        signal process="waybar" signal="SIGUSR2"
    }
}

bind "mako" {
    reload {
        command {
            argv "makoctl" "reload"
        }
    }
}

bind "hyprland" {
    reload {
        command {
            argv "hyprctl" "reload"
        }
    }
}
```

Each `bind` needs a `reload` block. Supported actions are:

```kdl
signal process="waybar" signal="SIGUSR2"

command {
    argv "program" "argument-one" "argument-two"
}

touch path="~/.config/example/reload"
```

`command` executes the argument vector directly, without a shell. Leading `~/`
paths are expanded for command arguments and `touch` paths. Remove bindings for
programs you do not use.

## Apply and switch themes

Once application includes and reload bindings are ready:

```sh
gnist set dusk
```

A normal apply publishes the theme, refreshes GTK CSS links, applies GNOME
settings, runs reload bindings, and advances the wallpaper.

Use flags to skip integrations you do not want:

```sh
gnist set dusk --skip-gnome --skip-wallpaper
gnist set dusk --skip-apply
```

Useful commands:

```sh
gnist list
gnist current
gnist reload
gnist wallpaper
gnist wallpaper ~/Pictures/background.png
gnist update
```

`gnist wallpaper` advances through the active theme's backgrounds. Pass an
image path to select that file directly. Gnist waits briefly for the
session-managed Awww daemon, applies the image, then updates its background
symlink.

Run `gnist COMMAND --help` for all command-specific options.

## Install themes from Git

`gnist install URL` clones and immediately applies a theme repository. Only use
repositories you trust. The repository must contain `colors.toml`, or a
complete Kitty, Ghostty, or Alacritty palette that Gnist can convert.

```sh
gnist install https://github.com/you/gnist-tokyo-night-theme.git
gnist update tokyo-night
gnist set tokyo-night
```

The example repository name becomes `tokyo-night`. Installing an existing name
replaces that theme before cloning; use `gnist update` for an existing Git
theme. `gnist update` only pulls changes, so run `gnist set` afterward to render
and apply them.

## Directory layout

```text
~/.config/gnist/
|-- bindings.kdl
|-- backgrounds/<theme>/
`-- themes/
    |-- data/<theme>/
    |-- templates/
    |-- user-templates/
    |-- generated/live/
    |-- current
    |-- current.theme
    `-- background
```

## Malm and Smia

[Malm](https://github.com/christian-bendiksen/malm) can deploy Gnist themes,
templates, reload bindings, and application includes. Gnist still performs the
runtime theme switch.

[Smia](https://github.com/christian-bendiksen/smia) is a complete desktop
configuration built this way. Its Malm profiles select a default Gnist theme,
and `smia-session` applies that theme when the desktop starts. You can still run
`gnist set THEME` directly for a temporary runtime change.

## License

Gnist is available under the MIT license.

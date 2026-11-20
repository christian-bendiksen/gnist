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

## Inspect a theme

Desktop clients can inspect installed palettes without applying them:

```sh
gnist inspect
gnist inspect dusk
```

Both commands emit one JSON document with `schema_version: 1`, `current` and
`themes`. Each theme includes its `id`, optional display `name` (from
`theme.toml` or `colors.toml`), `light` / `mode`, `generated` (true when colors
were synthesized in memory from a legacy terminal palette), optional `preview`
path (`preview.png`), effective semantic `colors`, non-color scalar `values`,
`icon_theme` and `wallpapers`. The same source precedence and derived values used
for rendering are used for previews. A malformed installed theme carries an
`error` without hiding other themes. Inspection does not render templates,
create publication directories, switch wallpaper or change the active theme.
Use `gnist set NAME` to publish and apply a chosen theme.

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

For a light theme, set `mode = "light"` in `colors.toml` or add an empty
`light.mode` file:

```sh
touch "$GNIST_CONFIG/themes/data/dusk/light.mode"
```

Light/dark mode is resolved in Omarchy's precedence order: `mode` key in
`colors.toml`, then `theme_type`, then the `light.mode` file, then background
luminance (`r + g + b > 382`), else dark. When a theme resolves to light, Gnist
writes an empty `light.mode` into `themes/current/` so consumers checking that
marker work even for marker-less Omarchy themes.

Omarchy 4 semantic `colors.toml` palettes (defining `red`, `blue`, `muted`,
`selection`, `bright_foreground`, etc. without `color0`..`color15`) are resolved
via a fill-only alias cascade so both semantic and ANSI keys work in templates
without overwriting any key a theme explicitly defined.

Optional theme content includes:

- `backgrounds/` for wallpapers (`png`, `jpg`, `jpeg`, `webp`, `avif`, `jxl`, `gif`)
- `icons.theme` containing a GTK icon theme name
- `preview.png` exposed through `gnist inspect` (not copied into `current/`)
- application files that should be copied without template expansion

Omarchy extras with no Herda consumer (`keyboard.rgb`, `shell.toml`,
`shell.<section>.toml`, `backgrounds/intros/`, `backgrounds/loops/`, icon sets)
are left unpublished.

## Add templates

Templates use `{{ ... }}` placeholders. They can live in
`$GNIST_CONFIG/themes/templates/`, or a selected theme can be self-contained by
shipping its own `.tpl` files directly in `themes/data/<theme>/` — those are
rendered too, ahead of the global templates.

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
2. Templates and files from the selected theme (its `.tpl` files are rendered)
3. `themes/templates/`

A global `templates/` directory is optional: a theme may ship only static
files or its own `.tpl` templates.

### Template language

Beyond plain values, templates accept expressions, filter pipelines, and
conditional blocks.

Every `colors.toml` scalar is available with its own type, plus a few generated
values: `name`, `mode` (`light`/`dark`), `is_light`, `colors` (the 16 ANSI
colors as a list), `wallpapers` (background image paths), and
`border_active` / `border_inactive`.

A nested key can be read by its dotted name: `{{ workpiece.accent }}` reads
`[workpiece] accent`, which is the same value as `{{ workpiece_accent }}`.
Combined with `default`, a dotted name falls back step by step:

```text
{{ workpiece.accent | default seam_color | default accent }}
```

Expressions and math:

```text
{{ radius * 2 }}px
{{ mode == "dark" }}
{{ 'gnist' | upper }}      -> GNIST
```

Color helpers derive new colors from a single palette:

| Template value | Result for `accent = "#89b4fa"` |
|---|---|
| `{{ accent }}` | `#89b4fa` |
| `{{ accent \| lighten 0.1 }}` | `#95bcfb` |
| `{{ accent \| darken 0.1 }}` | `#7ba2e1` |
| `{{ accent \| mix background 0.5 }}` | `#546994` |
| `{{ mix background foreground 15% }}` | Omarchy bare-word mix (`#rrggbb`) |
| `{{ mix_strip background accent 0.35 }}` | Omarchy bare-word mix (`rrggbb`) |
| `{{ mix_rgb background accent 50 }}` | Omarchy bare-word mix (`r,g,b`) |
| `{{ hypr_gradient KEY FALLBACK }}` | Omarchy Hyprland/Lua gradient string or table |
| `{{ shell_gradient KEY FALLBACK }}` | Omarchy space-separated gradient + angle |
| `{{ gradient_start KEY FALLBACK }}` | First gradient colour as `#rrggbb` |
| `{{ accent \| alpha 0.5 \| rgba }}` | `rgba(137,180,250,0.5)` |
| `{{ accent \| rgb }}` | `137,180,250` |
| `{{ accent \| invert }}` | `#764b05` |
| `{{ accent \| contrast }}` | readable black or white |
| `{{ accent \| complement }}` | hue-rotated 180° |
| `{{ accent \| grayscale }}` | `#727272` |

Color helpers: `hex`, `rgb`, `rgba`, `hsl`, `oklab`, `strip`, `lighten`,
`darken`, `mix OTHER AMOUNT`, `alpha`, `saturate`, `adjust_hue DEGREES`,
`ring DEGREES MODE` (rotate, then light .58 dark / .42 light and saturation at
least .45, as a theme's room ring falls back),
`invert`, `complement`, `grayscale`, `contrast`, `luminance`, `red`, `green`,
`blue` (0-255 channels) and `red_n`, `green_n`, `blue_n` (0..1, for COSMIC).

String and number helpers: `upper`, `lower`, `titlecase`, `trim`, `length`,
`replace OLD NEW`, `default X`, `join SEP`, `int`, `round`, `floor`, `ceil`,
`to_string`.

Blocks branch and iterate:

```text
{{#if mode == "dark"}}theme=dark{{else}}theme=light{{/if}}
{{#each colors as c}}, {{ c }}{{/each}}
{{! a comment }}
```

Unknown expressions, names, and blocks are left visible in the output, so
partial renders stay inspectable. `{{ missing | default "#fff" }}` fills in a
fallback instead.

Output filenames are templated too: `kitty.{{ name }}.conf.tpl` renders to
`kitty.dusk.conf`.

Templates are rendered when a theme is published (`gnist set`, `gnist
reapply`), not while it is current. A value that changes at runtime, such as
the accent of the work in front of you, belongs to the program that draws
it, which receives it over its own interface. The template supplies only the
fallback that program uses when nothing has been set.

### Templated integrations

Reload bindings in `bindings.kdl` and `bindings.d/` are expanded with the same
values before use, so actions can reference theme values and generated paths.

A theme may override integrations with optional files of its own:

`gnome.toml`:

```toml
schema             = "org.gnome.desktop.interface"
light_theme        = "adw-gtk3"
dark_theme         = "adw-gtk3-dark"

[[extra]]
key                = "cursor-theme"
value              = "{{ cursor_theme }}"
```

`wallpaper.toml`:

```toml
transition_type     = "fade"
transition_duration = "0.6"
```

These files are rendered with the theme context and are not published under
`current/`. Without them Gnist uses the built-in defaults.

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
gnist mode light
gnist mode dark
gnist mode toggle
gnist reapply
gnist reapply --wallpaper
gnist remove NAME --yes
gnist reload
gnist wallpaper
gnist wallpaper ~/Pictures/background.png
gnist update
```

`gnist mode light|dark|toggle` switches the active theme to its light or dark
counterpart. `gnist reapply` re-renders and reapplies the current theme (skipping
wallpaper by default unless `--wallpaper` is passed). `gnist remove NAME` deletes
a config-installed theme (pass `--force` to remove the active theme).

`gnist wallpaper` advances through the active theme's backgrounds. Pass an
image path to select that file directly. Gnist waits briefly for the
session-managed Awww daemon, applies the image, then updates its background
symlink.

Run `gnist COMMAND --help` for all command-specific options.

## Integrating

Desktop shells and session scripts that drive Gnist should rely on these four
contracts:

- **Resolved manifest (`themes/current/palette.json`).** Every publish writes a
  resolved JSON manifest (`schema_version: 1`, `name`, `display_name`, `mode`,
  monotonic `revision`, resolved `colors`, non-color scalar `values`,
  `wallpapers`, `icon_theme`, `preview`, `generated`) into the staging tree so
  it swaps atomically with all rendered configs when `themes/current` updates.
  Consumers should read `palette.json` (and `gnist inspect` for non-current
  themes) instead of parsing `colors.toml` directly.
- **Light/dark mode switching (`gnist mode light|dark|toggle`).** Counterparts
  are resolved from the theme's `light_theme` / `dark_theme` keys in
  `colors.toml`, falling back to `<name>-light` / stripping `-light`.
- **Interrupted-switch recovery (`gnist reapply [--wallpaper]`).** `gnist set`
  and `gnist mode` render into a private stage first, write `current.theme`
  atomically, and then commit the stage. If a switch is interrupted between
  writing `current.theme` and committing, running `gnist reapply` at sign-in
  completes the intended switch instead of reverting it.
- **Busy lock exit status (`75`).** Publishing commands take an exclusive lock
  (`themes/.gnist.lock`) for up to 20 seconds. If another Gnist process still
  holds the lock past the limit, Gnist exits with status `75` (`EX_TEMPFAIL`) so
  callers can retry or keep the current state. Each `gsettings` call is bounded
  to 2 seconds, and wallpaper application runs after releasing the lock.

## Install themes from Git

`gnist install URL` clones and immediately applies a theme repository. Only use
repositories you trust. The repository must contain `colors.toml`, or a
complete Kitty, Ghostty, or Alacritty palette that Gnist converts in memory
without dirtying the clone.

```sh
gnist install https://github.com/you/gnist-tokyo-night-theme.git
gnist update tokyo-night
gnist set tokyo-night
```

The example repository name becomes `tokyo-night`. Re-installing an existing
name clones into a sibling temporary directory first and only replaces the
installed theme after the clone succeeds, so a network failure preserves the
previous theme. Use `gnist update` for an existing Git theme; it only pulls
changes, so run `gnist set` or `gnist reapply` afterward to render and apply
them.

Git-cloned themes (directories containing `.git/`) are sandboxed at render time:
Gnist skips `.git/`, repository clutter (`README*`, `LICENSE*`, `preview*.png`),
all symlinks, and executable/application configs (`*.lua`, `alacritty.toml`,
`foot.ini`, `ghostty.conf`, `kitty.conf`, `vscode.json`, plus theme `.tpl` files
targeting them), emitting one `warn:` line and letting global templates render
safe replacements.

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

Smia's Herda themes are generated from one perceptual specification
(`tools/herda-palette.py` in Smia). Besides the semantic keys above they carry
`role1` … `role8`, Ambolt's eight workpiece accents (one ring at OKLCH L .62
C .15, hues 45° apart from the accent's), `machine` (a desaturated violet
for what the machine wrote) and `on_role` (ink on an accent chip). Smia's
templates publish them to GTK (`@herda_role1_color` …), to the shell
(`quickshell.json`) and to the compositor. A theme without them falls back
to its nearest ANSI colours.

## Testing

`cargo test` runs the unit tests and the CLI tests in `tests/cli.rs`. Every CLI
test spawns the binary through one guarded builder: it clears the environment,
gives the child a temporary HOME and XDG tree, points the session and system bus
addresses at a dead socket inside that tree, leaves `WAYLAND_DISPLAY` and
`DISPLAY` unset, and puts on PATH only recording stand-ins for `gsettings`,
`dbus-send`, `pkill`, `touch`, `awww`, `notify-send`, `git` and
`gtk-update-icon-cache`. The stand-ins write their name and arguments to
`<tempdir>/stand-in.log` and exit 0, so a test that applies a theme can prove
exactly what was called without ever reaching a real desktop tool. A source
check keeps the binary spawn inside that builder.

## License

Gnist is available under the MIT license.

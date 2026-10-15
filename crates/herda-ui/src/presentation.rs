//! The output boundary: one place where a Herda tool decides how it speaks.
//!
//! Every tool settles three things once, at startup, and then stops asking:
//! which tool this is, whether the person asked for plain text, and whether
//! stdout carries a machine response rather than a document. Everything that
//! prints goes through here, so those three answers are applied in one place
//! instead of being re-derived — differently — at each call site.
//!
//! This was `pkg`'s alone. It is here because `pkg`, Malm and Gnist each had
//! their own answer to the same question and disagreed: Malm carried a
//! `--color` tri-state, Gnist printed with `println!` and had no answer at all,
//! and `pkg` had one that worked but that the other two could not reach.

use std::io::{IsTerminal, Write};
use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, Ordering};

use crate::document::Document;
use crate::ink::Ink;
use crate::terminal::{Theme, fold, plain};
use crate::width::Layout;

struct Presentation {
    app: &'static str,
    title: &'static str,
    plain: bool,
    machine: bool,
    header: AtomicBool,
}

static PRESENTATION: OnceLock<Presentation> = OnceLock::new();

/// Settle how this process speaks. The first call wins; later ones are no-ops,
/// so a command cannot change its mind halfway through a document.
///
/// `app` is the tool's own name as it appears in a heading — `pkg`, `malm`,
/// `gnist`. `title` is what this command did.
pub fn init(app: &'static str, title: &'static str, plain: bool, machine: bool) {
    let _ = PRESENTATION.set(Presentation {
        app,
        title,
        plain,
        machine,
        header: AtomicBool::new(false),
    });
}

/// The tool's name. `herda` until [`init`] runs, which is only reachable if
/// something prints before flag handling.
#[must_use]
pub fn app() -> &'static str {
    PRESENTATION.get().map_or("herda", |p| p.app)
}

/// Whether stdout carries a machine response rather than a human document.
///
/// Set once from `--json`. [`interactive`] conflates this with `--plain` and
/// with being piped, so it cannot answer the question — which is why every
/// `pkg` handler used to thread a `bool` of its own, and five forgot to.
#[must_use]
pub fn machine() -> bool {
    PRESENTATION.get().is_some_and(|p| p.machine)
}

/// Whether stdout is a human terminal that has not asked for plain text.
#[must_use]
pub fn interactive() -> bool {
    PRESENTATION.get().is_none_or(|p| !p.plain && !p.machine)
        && crate::terminal::interactive_output()
}

/// The theme for this process's stdout.
#[must_use]
pub fn theme() -> Theme {
    if !interactive() {
        return Theme::PLAIN;
    }
    Theme::detect_for(machine())
}

/// The shape of this process's stdout.
#[must_use]
pub fn layout() -> Layout {
    Layout::detect()
}

/// Open a document titled for this command, at this terminal's width.
#[must_use]
pub fn open(title: &str, badge: &str) -> Document {
    Document::new(app(), title, badge, layout().columns(), theme())
}

/// One document per command: the last thing a handler does.
///
/// Takes a fully built value, so there is nothing left to print afterwards and
/// a late `if json { … }` branch — which is how prose came to be interleaved
/// with JSON — cannot be expressed.
pub fn emit(rendered: &str) {
    if let Some(p) = PRESENTATION.get() {
        p.header.store(true, Ordering::Relaxed);
    }
    let _ = std::io::stdout().lock().write_all(rendered.as_bytes());
}

/// Human progress belongs on stderr when stdout carries a machine response.
pub fn activity(args: std::fmt::Arguments<'_>) {
    if machine() {
        let _ = writeln!(std::io::stderr().lock(), "{args}");
    } else {
        line(args);
    }
}

/// A section break in streamed output: a blank line, the name in bold, a blank
/// line.
///
/// This replaces the hairline `── label ──────` that the retired Ledger half
/// drew. The output boundary used to detect those by sniffing its own text for
/// a leading `──` and re-theme them on the way past, because two renderers in
/// one binary could not otherwise be told apart downstream.
pub fn section(name: &str) {
    if !interactive() {
        let _ = writeln!(std::io::stdout().lock(), "\n{name}\n");
        return;
    }
    let theme = theme();
    let width = layout().content();
    let mut stdout = std::io::stdout().lock();
    header(&mut stdout, theme, width);
    let _ = writeln!(stdout);
    for line in fold(name, width) {
        let _ = writeln!(stdout, "  {}", theme.paint(&line, Ink::Text, true));
    }
    let _ = writeln!(stdout);
}

/// A phase line at this terminal's width: `── label ─────────────── tail`.
///
/// Nearly every caller of [`Theme::hairline`] wants the real width, and the
/// renderer cannot ask for it without stopping being a pure function — so the
/// boundary, which already knows, asks on their behalf.
#[must_use]
pub fn phase(label: &str, tail: &str) -> String {
    theme().hairline(label, tail, layout().content())
}

/// All ordinary stdout goes through this boundary. Data and redirected output
/// retain their bytes; only a human terminal receives the shared document style.
pub fn line(args: std::fmt::Arguments<'_>) {
    let text = args.to_string();
    // A real terminal must never receive control characters from external text,
    // even under `--plain` — plain is a styling choice, not a licence for a
    // package description or path to rewrite the screen. A pipe or file keeps
    // its exact bytes.
    let terminal_output = std::io::stdout().is_terminal();
    let mut stdout = std::io::stdout().lock();
    if !interactive() {
        if terminal_output {
            let _ = writeln!(stdout, "{}", crate::terminal::clean(&text));
        } else {
            let _ = writeln!(stdout, "{text}");
        }
        return;
    }
    let theme = theme();
    let width = layout().content();
    header(&mut stdout, theme, width);
    let text = plain(&text);
    for raw in text.lines() {
        for line in fold(raw, width) {
            let _ = writeln!(stdout, "  {}", theme.paint(&line, ink_for(raw), false));
        }
    }
    if text.is_empty() {
        let _ = writeln!(stdout);
    }
}

/// What a streamed line is saying, from the glyph it leads with.
///
/// A concession to streamed output, which has no structure to read: a handler
/// that builds a [`Document`] states its inks and never comes through here.
fn ink_for(raw: &str) -> Ink {
    let lead = raw.trim_start();
    if lead.starts_with(crate::ink::glyph::FAILED) || raw.starts_with("error") {
        Ink::Danger
    } else if lead.starts_with(crate::ink::glyph::DONE) {
        Ink::Green
    } else {
        Ink::Text
    }
}

/// The command's heading, printed before the first thing it says.
fn header(stdout: &mut impl Write, theme: Theme, width: usize) {
    let Some(p) = PRESENTATION.get() else { return };
    if p.header.swap(true, Ordering::Relaxed) {
        return;
    }
    let _ = writeln!(
        stdout,
        "\n  {}\n\n  {}",
        theme.heading_for(p.app, p.title, "", width),
        theme.seam(width)
    );
}

/// What the command could not do, on stderr, as its own document.
pub fn error(message: &str) {
    let mut stderr = std::io::stderr().lock();
    if std::io::stderr().is_terminal()
        && PRESENTATION.get().is_some_and(|p| !p.plain && !p.machine)
        && !crate::terminal::dumb()
    {
        // stderr is a terminal even when stdout is a pipe, which is exactly the
        // case this branch exists for — so the theme is decided from the
        // environment rather than from stdout.
        let theme = Theme::detect_for(false);
        let theme = Theme {
            color: std::env::var_os("NO_COLOR").is_none()
                && std::env::var("FORCE_COLOR").as_deref() != Ok("0"),
            ..theme
        };
        let mut doc = Document::new(
            app(),
            "Could not complete the command",
            "ERROR",
            Layout::detect().columns(),
            theme,
        );
        doc.text(message, Ink::Danger);
        let _ = stderr.write_all(doc.render().as_bytes());
    } else {
        let _ = writeln!(stderr, "error: {}", plain(message));
    }
}

/// Consent stays line-based and opt-in; presentation never reads input.
pub fn prompt(text: &str) -> std::io::Result<()> {
    let mut out = std::io::stderr().lock();
    if std::io::stderr().is_terminal() && interactive() {
        let width = Layout::detect().content();
        let text = fold(text, width).join("\n  ");
        write!(out, "  {} ", theme().paint(&text, Ink::Gold, true))?;
    } else {
        write!(out, "{text}")?;
    }
    out.flush()
}

/// Write a rendered surface out for comparison against the design.
///
/// Set `HERDA_TERMINAL_ARTIFACTS` to a directory and the pty suites drop one
/// `.ansi` per surface into it, to be looked at beside the artboards. `pkg` and
/// Malm each had their own variable for this and so could not be captured in
/// one run.
pub fn design_fixture(name: &str, lines: &[String]) {
    let Some(root) = std::env::var_os("HERDA_TERMINAL_ARTIFACTS") else {
        return;
    };
    let root = std::path::PathBuf::from(root);
    if std::fs::create_dir_all(&root).is_ok() {
        let _ = std::fs::write(root.join(format!("{name}.ansi")), lines.join("\r\n"));
    }
}

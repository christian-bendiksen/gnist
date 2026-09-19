//! How Gnist talks to a person.
//!
//! One boundary, so the theme switcher looks like the rest of the OS's tools:
//! the palette, glyphs, widths and document grammar are all
//! [`herda_ui`], vendored into `crates/herda-ui` — see its `UPSTREAM.md`.
//!
//! What is *not* routed through here is deliberate. `gnist current` prints a
//! bare theme name and `gnist list` prints one name per line; both are read by
//! shell scripts, and a heading above them would break every one. The same goes
//! for `No current theme set` on stderr, which is pinned byte-for-byte by
//! `tests/cli.rs`. Decorating data because it happens to be on a terminal is
//! how a CLI stops being usable in a pipeline.

use herda_ui::document::{Column, Document};
use herda_ui::ink::{Ink, glyph};
use herda_ui::presentation;
use herda_ui::terminal::Theme;
use herda_ui::width::Layout;

/// Settle how this process speaks, once, from the command it is running.
pub fn init(command: &'static str) {
    presentation::init("gnist", command, false, false);
}

/// The theme for stdout.
#[must_use]
pub fn theme() -> Theme {
    presentation::theme()
}

/// Open a document for a report.
#[must_use]
pub fn document(title: &str, badge: &str) -> Document {
    Document::new(
        "gnist",
        title,
        badge,
        Layout::detect().columns(),
        presentation::theme(),
    )
}

/// A column of a report's table.
#[must_use]
pub fn column(label: &str) -> Column {
    Column::new(label)
}

/// A finished thing: `✓ text`.
#[must_use]
pub fn ok(text: &str) -> String {
    theme().ok(text)
}

/// Something that failed: `✗ text`.
#[must_use]
pub fn fail(text: &str) -> String {
    theme().fail(text)
}

/// Something skipped or not applicable: `· text`.
#[must_use]
pub fn pending(text: &str) -> String {
    theme().pending(text)
}

/// A caution: `│ text`.
///
/// A shape as well as a colour, so it survives `NO_COLOR` and a pipe — which
/// is where Gnist's warnings usually end up, since a theme switch is normally
/// run by a keybinding rather than watched.
#[must_use]
pub fn warn(text: &str) -> String {
    theme().warn(text)
}

/// The marker for a change: `+` added, `-` removed, `~` changed.
///
/// `Removed` has no caller yet; it is here because the three belong together
/// and a table that can only say two of them will grow a fourth spelling.
#[must_use]
#[allow(dead_code)]
pub const fn marker(kind: Kind) -> (&'static str, Ink) {
    match kind {
        Kind::Added => (glyph::ADD, Ink::Green),
        Kind::Removed => (glyph::REMOVE, Ink::Danger),
        Kind::Changed => (glyph::CHANGE, Ink::Gold),
    }
}

/// What happened to one thing in a list.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[allow(dead_code)]
pub enum Kind {
    /// Newly there.
    Added,
    /// Gone.
    Removed,
    /// Already there, and left alone.
    Changed,
}

/// Say that work on `name` has started.
///
/// On a terminal the line is held open to be replaced by its outcome; anywhere
/// else nothing is printed, because a pipe or a log must never receive a
/// carriage return or a rewrite — it would arrive as one unreadable line.
pub fn working(name: &str) {
    if !presentation::interactive() {
        return;
    }
    print!(
        "{} {name} …",
        theme().paint(glyph::WORKING, Ink::Gold, true)
    );
    let _ = std::io::Write::flush(&mut std::io::stdout());
}

/// Replace the open line with what actually happened.
pub fn settled(outcome: &str) {
    if presentation::interactive() {
        print!("\r\x1b[2K");
    }
    println!("{outcome}");
}

/// Ask a yes/no question on stderr, through the shared prompt.
pub fn confirm(question: &str) -> std::io::Result<bool> {
    presentation::prompt(&format!("{question} [y/N]"))?;
    let mut line = String::new();
    std::io::stdin().read_line(&mut line)?;
    Ok(matches!(line.trim().to_lowercase().as_str(), "y" | "yes"))
}

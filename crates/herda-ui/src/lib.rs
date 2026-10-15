//! How Herda's tools look on a terminal.
//!
//! One design, in one crate, for every user-facing binary the OS ships — `pkg`,
//! `malm`, `gnist`, `herda-setup`, `pkg-distro` — so they read as one system
//! with each other and with the desktop.
//!
//! - [`ink`] is the palette, the glyph table and the seam. Every colour and
//!   every glyph in the OS's terminal output is written down there once.
//! - [`width`] is the ladder: what a renderer gives up, and in what order, as a
//!   terminal falls short of the width the design is drawn at.
//! - [`terminal`] is the typography — headings, rules, cells, wrapping that
//!   never loses a character of a path or a digest.
//! - [`document`] is the scrollback document a command leaves behind.
//! - [`presentation`] is the output boundary: one place that knows whether this
//!   process is talking to a person or to a machine.
//! - [`progress`] is the staged region held open while something long runs.
//! - [`browser`] is the full-screen list-and-detail surface.
//! - [`cells`] is the cell buffer a full screen is drawn into, and what turns
//!   one into terminal bytes, plain text or an HTML page.
//! - [`frame`] is the full-screen frame a whole command happens inside,
//!   [`revision_line`] the line across its top, and [`pieces`] what its focal
//!   area is built from.
//! - [`scale`] is how a large statement is drawn: scaled where the terminal
//!   reports that it can, bold everywhere else.
//! - [`surface`] is the terminal a frame is shown on.
//! - `session` is a command happening inside a frame: the thread that holds
//!   the terminal for it, the keys, and where the process's output goes
//!   meanwhile. It is behind the `session` feature.
//!
//! Colour is an affordance, never data. Every renderer is a pure function of
//! its inputs plus a [`terminal::Theme`], and returns byte-for-byte plain text
//! when that theme is [`terminal::Theme::PLAIN`] — so a harness, a pipe and a
//! serial console receive exactly what they always did. What colour adds on a
//! real terminal is structure the eye can scan: the step that is running, the
//! package that failed, the one number that moved.
//!
//! The crate is vendored byte-identically into Malm and Gnist rather than
//! published, so building either needs no checkout of Herda, no credentials and
//! no live Git service. `UPSTREAM.md` in each copy records the commit it came
//! from; changes are made here first.

pub mod browser;
pub mod cells;
pub mod document;
pub mod frame;
pub mod ink;
pub mod pieces;
pub mod presentation;
pub mod progress;
pub mod revision_line;
pub mod scale;
#[cfg(feature = "session")]
pub mod session;
pub mod surface;
pub mod terminal;
pub mod width;
pub mod work;

/// Whether this is a copy of the crate carried in another repository.
///
/// `scripts/sync-herda-ui.sh` copies the manifest and `src/`, and the copy has
/// an `UPSTREAM.md` beside them that the crate itself never has. The held
/// screens and the terminal scripts under `tests/` are not copied, so the
/// tests that read them are run where the crate lives and say nothing in a
/// copy, whose bytes its repository checks against the manifest in that file.
#[cfg(test)]
pub(crate) fn vendored() -> bool {
    std::path::Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/UPSTREAM.md")).exists()
}

// The design fixtures are written as a consumer of the crate would write
// them, and are compiled into the gallery example as well as into the tests.
#[cfg(test)]
extern crate self as herda_ui;
#[cfg(test)]
mod frame_fixtures;

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
pub mod document;
pub mod ink;
pub mod presentation;
pub mod progress;
pub mod terminal;
pub mod width;

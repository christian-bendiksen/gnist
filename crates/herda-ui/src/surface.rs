//! The terminal a frame is shown on.
//!
//! [`Surface`] takes the alternate screen the way the browser does — through
//! [`crate::browser::Screen`], so raw mode, the hidden cursor and the
//! restoring of all of it on a panic or a signal are the same code — and then
//! does the three things a frame needs: it settles once what this terminal
//! can draw, it shows a [`Buffer`] by writing only what changed since the last
//! one, and it waits for a key or a change of size without holding up whoever
//! is doing the work.
//!
//! It draws nothing of its own and decides nothing. What is shown, and what a
//! key means, belong to the caller.

use std::io::Write as _;
use std::time::Duration;

use crossterm::event::{self, Event, KeyEventKind};

use crate::browser::{KeyEvent, Screen};
use crate::cells::Buffer;
use crate::scale::Sizing;
use crate::terminal::Theme;
use crate::width::Layout;

/// What happened while a surface waited.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Input {
    /// A key was pressed.
    Key(KeyEvent),
    /// The terminal changed size: draw again for the new [`Surface::layout`].
    Resized,
    /// Nothing happened in the time given.
    Quiet,
    /// The process was told to end. The caller stops and drops the surface,
    /// which restores the terminal and lets the signal take its course.
    Interrupted,
}

/// Exclusive use of the terminal for a full-screen frame.
pub struct Surface {
    screen: Screen,
    theme: Theme,
    sizing: Sizing,
    shown: Option<Buffer>,
}

impl Surface {
    /// Take the terminal: the alternate screen, raw mode, a hidden cursor.
    ///
    /// Asks the terminal once whether it scales text, where the crate is
    /// built to ask — see [`crate::scale`] — and leaves the screen clear. Fails where the browser would: when
    /// something else in the process already has the screen, or the terminal
    /// refuses.
    pub fn enter() -> std::io::Result<Self> {
        let screen = Screen::enter()?;
        // The event source starts listening for a change of size when it is
        // first asked for an event, so it is asked before anything is drawn.
        event::poll(Duration::ZERO)?;
        let sizing = crate::scale::detect();
        let mut out = std::io::stdout().lock();
        out.write_all(b"\x1b[0m\x1b[2J\x1b[H")?;
        out.flush()?;
        Ok(Self {
            screen,
            theme: Theme::detect(),
            sizing,
            shown: None,
        })
    }

    /// The colour this terminal is drawn in.
    #[must_use]
    pub const fn theme(&self) -> Theme {
        self.theme
    }

    /// Whether large statements are scaled or bold here.
    #[must_use]
    pub const fn sizing(&self) -> Sizing {
        self.sizing
    }

    /// The terminal's size now.
    #[must_use]
    pub fn layout(&self) -> Layout {
        Layout::detect()
    }

    /// Show a buffer, writing only the cells that differ from the last one
    /// shown, in one synchronized frame.
    pub fn show(&mut self, buffer: &Buffer) -> std::io::Result<()> {
        // One write for the frame, so nothing else lands inside it.
        let mut frame = Vec::new();
        buffer.present(&mut frame, self.shown.as_ref(), self.theme)?;
        let mut terminal = crate::browser::Terminal;
        terminal.write_all(&frame)?;
        terminal.flush()?;
        self.shown = Some(buffer.clone());
        Ok(())
    }

    /// Write frames, and the restoring of the terminal when the surface is
    /// dropped, to `terminal` in place of standard output.
    ///
    /// For a caller that turns the process's standard output elsewhere while
    /// the surface is up: see [`Screen::write_to`]. The surface asks nothing
    /// of the terminal after it was entered, so nothing else has to follow.
    pub fn write_to(&mut self, terminal: std::fs::File) {
        self.screen.write_to(Some(terminal));
    }

    /// Write `bytes` to the terminal as they are, where frames are written:
    /// a sequence that is not drawing, such as one that sets the clipboard.
    pub fn send(&mut self, bytes: &[u8]) -> std::io::Result<()> {
        let mut terminal = crate::browser::Terminal;
        terminal.write_all(bytes)?;
        terminal.flush()
    }

    /// Wait up to `patience` for a key or a change of size.
    ///
    /// Returns as soon as one arrives. Everything else a terminal can send —
    /// a paste, a focus change, a key being released — is read and dropped
    /// without ending the wait early as a key.
    pub fn wait(&mut self, patience: Duration) -> std::io::Result<Input> {
        if self.screen.interrupted() {
            return Ok(Input::Interrupted);
        }
        if !event::poll(patience)? {
            return Ok(if self.screen.interrupted() {
                Input::Interrupted
            } else {
                Input::Quiet
            });
        }
        Ok(match event::read()? {
            Event::Key(key) if key.kind != KeyEventKind::Release => Input::Key(key),
            Event::Resize(_, _) => {
                // The terminal reflows what it holds; nothing shown can be
                // patched after that.
                self.shown = None;
                Input::Resized
            }
            _ => Input::Quiet,
        })
    }
}

#[cfg(all(test, unix))]
#[path = "surface_tests.rs"]
mod surface_tests;

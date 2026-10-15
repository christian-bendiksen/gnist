//! The full-screen browser: a list on the left, what it means on the right.
//!
//! The shell here owns everything that is the same whatever is being browsed —
//! raw mode, the alternate screen, restoring the terminal when a panic or a
//! signal ends the process, the two-pane layout and its collapse on narrow
//! terminals, scrolling, the key legend — and knows nothing about what the
//! rows are. A tool supplies the rows and answers the keys the shell does not
//! claim, through [`Source`].
//!
//! This was `pkg`'s software history and could not be reached from anywhere
//! else, so Malm had no interactive history at all: the same data, a `--json`
//! dump and a paragraph of prose.

use std::io::{IsTerminal, Write};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

use crossterm::{
    cursor,
    event::{self, Event, KeyEventKind},
    execute,
    terminal::{self, Clear, ClearType, EnterAlternateScreen, LeaveAlternateScreen},
};

/// The key types a [`Source`] answers.
///
/// Re-exported so implementing a browser does not mean taking a direct
/// dependency on `crossterm` — which would put a terminal library in the
/// dependency graph of a CLI that only wanted to describe its own rows, and
/// would let the two drift apart on a version bump.
pub use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::ink::{Ink, glyph};
use crate::terminal::{Theme, clip, size, wrap};
use crate::width::{BROWSER, BROWSER_ROWS, Layout};

static ACTIVE: AtomicBool = AtomicBool::new(false);
static OWNED: AtomicBool = AtomicBool::new(false);
static HOOK: std::sync::Once = std::sync::Once::new();
static SIGNALS: OnceLock<std::io::Result<ScreenSignals>> = OnceLock::new();

struct ScreenSignals {
    default_action: Arc<AtomicBool>,
    interrupted: Arc<AtomicUsize>,
}

impl ScreenSignals {
    fn install() -> std::io::Result<Self> {
        let signals = Self {
            default_action: Arc::new(AtomicBool::new(true)),
            interrupted: Arc::new(AtomicUsize::new(0)),
        };
        for signal in [
            signal_hook::consts::SIGTERM,
            signal_hook::consts::SIGHUP,
            signal_hook::consts::SIGINT,
        ] {
            // Registrations live for the process lifetime. Unregistering a
            // signal-hook action does not restore the previous disposition.
            // Install the default action first; even partial setup failure
            // leaves every registered signal with normal termination behavior.
            signal_hook::flag::register_conditional_default(
                signal,
                signals.default_action.clone(),
            )?;
            signal_hook::flag::register_usize(
                signal,
                signals.interrupted.clone(),
                signal as usize,
            )?;
        }
        Ok(signals)
    }
}

struct ScreenOwnership;

impl ScreenOwnership {
    fn acquire() -> std::io::Result<Self> {
        OWNED
            .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
            .map_err(|_| {
                std::io::Error::new(
                    std::io::ErrorKind::AlreadyExists,
                    "a browser already owns the terminal",
                )
            })?;
        Ok(Self)
    }
}

impl Drop for ScreenOwnership {
    fn drop(&mut self) {
        OWNED.store(false, Ordering::SeqCst);
    }
}

/// A descriptor of the terminal, while a screen is written there in place of
/// standard output. See [`Screen::write_to`].
static TERMINAL: Mutex<Option<std::fs::File>> = Mutex::new(None);

/// Where a screen is written: standard output, unless the caller that turned
/// standard output elsewhere left a descriptor of the terminal.
pub(crate) struct Terminal;

impl Terminal {
    fn with<R>(write: impl FnOnce(&mut dyn Write) -> R) -> R {
        // Reached from the panic hook, so a lock another thread died holding,
        // or holds now, sends the bytes to standard output.
        let held = match TERMINAL.try_lock() {
            Ok(held) => Some(held),
            Err(std::sync::TryLockError::Poisoned(poisoned)) => Some(poisoned.into_inner()),
            Err(std::sync::TryLockError::WouldBlock) => None,
        };
        match held {
            Some(mut held) => match held.as_mut() {
                Some(terminal) => write(terminal),
                None => write(&mut std::io::stdout()),
            },
            None => write(&mut std::io::stdout()),
        }
    }
}

impl Write for Terminal {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        Self::with(|out| out.write(bytes))
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Self::with(|out| out.flush())
    }
}

/// Restore terminal state from normal code or the panic hook.
/// This performs I/O and must never run in an asynchronous signal handler.
fn restore_terminal() {
    if ACTIVE.swap(false, Ordering::SeqCst) {
        let _ = terminal::disable_raw_mode();
        let _ = write!(Terminal, "\x1b[?2026l\x1b[0m");
        let _ = execute!(
            Terminal,
            event::DisableBracketedPaste,
            cursor::Show,
            LeaveAlternateScreen
        );
    }
}

/// Exclusive alternate-screen ownership. TERM/HUP/INT request cleanup while
/// held; their default actions apply again after cleanup, including suspension.
/// Signal registrations persist for the process lifetime with conditional default
/// actions; dropping a screen never relies on unregistering a handler. A pending
/// termination signal is re-applied after restoring the terminal.
pub struct Screen {
    signals: &'static ScreenSignals,
    _ownership: ScreenOwnership,
}

impl Screen {
    pub fn enter() -> std::io::Result<Self> {
        Self::enter_with(|| {
            // The guard and handlers already exist if any setup step fails.
            ACTIVE.store(true, Ordering::SeqCst);
            terminal::enable_raw_mode()?;
            execute!(
                std::io::stdout(),
                EnterAlternateScreen,
                cursor::Hide,
                event::EnableBracketedPaste
            )
        })
    }

    fn enter_with(setup: impl FnOnce() -> std::io::Result<()>) -> std::io::Result<Self> {
        let ownership = ScreenOwnership::acquire()?;
        let signals = SIGNALS
            .get_or_init(ScreenSignals::install)
            .as_ref()
            .map_err(|error| std::io::Error::new(error.kind(), error.to_string()))?;
        HOOK.call_once(|| {
            let previous = std::panic::take_hook();
            std::panic::set_hook(Box::new(move |info| {
                restore_terminal();
                if let Some(Ok(signals)) = SIGNALS.get() {
                    signals.default_action.store(true, Ordering::SeqCst);
                }
                previous(info);
            }));
        });
        signals.interrupted.store(0, Ordering::SeqCst);
        signals.default_action.store(false, Ordering::SeqCst);
        let screen = Self {
            signals,
            _ownership: ownership,
        };
        setup()?;
        if screen.interrupted() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::Interrupted,
                "interrupted",
            ));
        }
        Ok(screen)
    }

    #[must_use]
    pub fn interrupted(&self) -> bool {
        self.signals.interrupted.load(Ordering::SeqCst) != 0
    }

    /// Write the screen, and the restoring of the terminal, to `terminal` in
    /// place of standard output, or to standard output again with `None`.
    ///
    /// For a caller that turns the process's standard output elsewhere while
    /// the screen is up, so that nothing else in the process, and no child of
    /// it, writes over the screen: it keeps a descriptor of the terminal here
    /// first. The descriptor is closed when the screen is dropped.
    pub fn write_to(&self, terminal: Option<std::fs::File>) {
        *TERMINAL
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = terminal;
    }
}

impl Drop for Screen {
    fn drop(&mut self) {
        restore_terminal();
        self.write_to(None);
        self.signals.default_action.store(true, Ordering::SeqCst);
        // Do not lose a signal that arrived during terminal cleanup or just
        // before suspension. Reapply its default action only after restoration.
        let signal = self.signals.interrupted.swap(0, Ordering::SeqCst);
        if signal != 0 {
            let _ = signal_hook::low_level::emulate_default_handler(signal as i32);
        }
    }
}

/// One row of the list pane.
#[derive(Clone, Debug, Default)]
pub struct Entry {
    /// The identity a person reads and can quote back — a revision, a deploy.
    pub id: String,
    /// What this entry did, in a few words.
    pub title: String,
    /// A short state word to the right of the identity: `Selected`, `Kept`.
    pub badge: String,
}

/// One line of the detail pane: the text, its ink, and whether it is bold.
pub type Line = (String, Ink, bool);

/// What the shell should do after a key it did not claim.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Answer {
    /// Nothing happened.
    Ignored,
    /// The state changed; draw again.
    Handled,
    /// Give the terminal back, call [`Source::suspended`], then take it again.
    ///
    /// Anything that runs a command belongs here: the command's own output and
    /// any authentication prompt must land in ordinary scrollback, not be
    /// painted over by the browser a tenth of a second later.
    Suspend,
    /// Close the browser.
    Quit,
}

/// What a browser is browsing.
///
/// The shell claims `q`, `Ctrl-C`, `Ctrl-D`, and the cursor keys; everything
/// else reaches [`Source::key`].
pub trait Source {
    /// The heading.
    fn title(&self) -> String;
    /// The line under the heading.
    fn subtitle(&self) -> String;
    /// The faint heading over the list pane: what these rows are.
    fn list_title(&self) -> String {
        "ITEMS".into()
    }
    /// What is shown at the top right of the heading.
    fn badge(&self) -> String;
    /// The rows, newest first.
    fn entries(&self) -> &[Entry];
    /// Which row the cursor is on.
    fn cursor(&self) -> usize;
    /// Move the cursor. Called only with a valid index.
    fn select(&mut self, index: usize);
    /// The detail pane for the row under the cursor, at this size.
    fn detail(&self, width: usize, height: usize) -> Vec<Line>;
    /// The key legend along the bottom.
    fn keys(&self) -> String;
    /// A line of feedback above the legend — the result of the last action.
    fn message(&self) -> String {
        String::new()
    }
    /// Answer a key the shell did not claim.
    fn key(&mut self, key: KeyEvent) -> Answer;
    /// Run whatever [`Answer::Suspend`] was for, with the terminal handed back.
    ///
    /// Takes no result: a source records what happened in its own
    /// [`Source::message`], which is where the person will look for it.
    fn suspended(&mut self) {}
}

/// Whether a browser can be opened at all: a terminal in both directions.
#[must_use]
pub fn available() -> bool {
    crate::presentation::interactive() && std::io::stdin().is_terminal()
}

/// Lay a source out for a terminal of this size.
///
/// Pure, so the pty suite and the design fixtures render the real screen
/// without one.
#[must_use]
pub fn render(source: &dyn Source, layout: Layout, theme: Theme) -> Vec<String> {
    let (columns, rows) = (layout.columns(), layout.rows());
    if !layout.browsable() {
        return [
            format!("{}  {}", crate::presentation::app(), source.title()),
            format!("Enlarge the terminal to at least {BROWSER} × {BROWSER_ROWS}."),
            "q Quit".to_owned(),
        ]
        .iter()
        .map(|s| clip(s, columns.saturating_sub(3)))
        .take(rows.saturating_sub(1))
        .collect();
    }
    let width = layout.content();
    let mut lines = vec![
        theme.heading_for(
            crate::presentation::app(),
            &source.title(),
            &source.badge(),
            width,
        ),
        theme.paint(&clip(&source.subtitle(), width), Ink::Muted, false),
        theme.seam(width),
        String::new(),
    ];
    let height = rows.saturating_sub(9);
    let cursor = source.cursor();

    match layout.panes() {
        Some((left_width, right_width)) => {
            let mut left: Vec<Line> = vec![
                (source.list_title(), Ink::Faint, false),
                (String::new(), Ink::Muted, false),
            ];
            // Four rows per entry: identity, title, gap, gap.
            let count = height.saturating_sub(2).div_euclid(4).max(1);
            let start = cursor.saturating_sub(count - 1);
            for (index, entry) in source.entries().iter().enumerate().skip(start).take(count) {
                let ink = if index == cursor {
                    Ink::Selected
                } else {
                    Ink::Text
                };
                let marker = if index == cursor { "›" } else { " " };
                left.push((format!("{marker} {}  {}", entry.id, entry.badge), ink, true));
                left.push((format!("  {}", entry.title), ink, false));
                left.push((String::new(), ink, false));
                left.push((String::new(), Ink::Text, false));
            }
            let mut right = source.detail(right_width, height);
            // Content stays inside its pane, including on a shrinking terminal.
            right.truncate(height);
            for row in 0..height {
                let (l, ink, bold) = left.get(row).cloned().unwrap_or_default();
                let (r, ri, rb) = right.get(row).cloned().unwrap_or_default();
                lines.push(format!(
                    "{} {} {}",
                    if ink == Ink::Selected {
                        format!(
                            "{}{}",
                            theme.paint(glyph::RAIL, Ink::Gold, false),
                            theme.cell(&l, left_width - 1, ink, bold)
                        )
                    } else {
                        theme.cell(&l, left_width, ink, bold)
                    },
                    theme.paint(glyph::RAIL, Ink::Rule, false),
                    theme.cell(&r, right_width, ri, rb)
                ));
            }
        }
        None => {
            if let Some(entry) = source.entries().get(cursor) {
                lines.push(theme.cell(
                    &format!("› {} · {} {}", entry.id, entry.title, entry.badge),
                    width,
                    Ink::Selected,
                    true,
                ));
            }
            for (text, ink, bold) in source
                .detail(width, height.saturating_sub(1))
                .into_iter()
                .take(height.saturating_sub(1))
            {
                lines.push(theme.cell(&text, width, ink, bold));
            }
        }
    }

    while lines.len() < rows - 5 {
        lines.push(String::new());
    }
    lines.push(theme.rule(width));
    lines.push(theme.paint(&clip(&source.message(), width), Ink::Gold, false));
    lines.push(theme.paint(&clip(&source.keys(), width), Ink::Muted, false));
    lines
}

/// A detail pane's own footer note, wrapped and muted. Sources build their
/// panes by hand; this is the one piece they all repeat.
#[must_use]
pub fn note(text: &str, width: usize) -> Vec<Line> {
    wrap(text, width)
        .into_iter()
        .map(|line| (line, Ink::Muted, false))
        .collect()
}

fn paint(source: &dyn Source, theme: Theme) -> std::io::Result<()> {
    let (columns, rows) = size();
    if columns == 0 || rows == 0 {
        return Ok(());
    }
    let lines = render(source, Layout::of(columns, rows), theme);
    let mut out = std::io::stdout().lock();
    write!(out, "\x1b[?2026h")?;
    execute!(out, cursor::MoveTo(0, 0))?;
    for line in lines {
        execute!(out, Clear(ClearType::CurrentLine))?;
        write!(out, "  {line}\r\n")?;
    }
    execute!(out, Clear(ClearType::FromCursorDown))?;
    write!(out, "\x1b[?2026l")?;
    out.flush()
}

/// Run a browser until the person closes it.
///
/// Terminal failures return `Err`. TERM/HUP/INT restore the terminal before
/// applying their default termination action. A source's own failures are its
/// [`Source::message`], not the loop's business.
pub fn run(source: &mut dyn Source) -> std::io::Result<()> {
    let theme = Theme::detect();
    let mut screen = Some(Screen::enter()?);
    // The event source starts listening for a change of window size when it
    // is first asked for an event. Asking once before the first paint means a
    // resize during that paint is seen, not dropped for want of a listener.
    event::poll(std::time::Duration::ZERO)?;
    let mut dirty = true;
    loop {
        if dirty {
            paint(source, theme)?;
            dirty = false;
        }
        loop {
            if screen.as_ref().is_some_and(Screen::interrupted) {
                return Err(std::io::Error::other("interrupted"));
            }
            if event::poll(std::time::Duration::from_millis(100))? {
                break;
            }
        }
        // Process one event before checking interruption again. Ignored events
        // never trigger a render, including paste and focus-event floods.
        let key = match event::read()? {
            Event::Key(key) => key,
            Event::Resize(_, _) => {
                dirty = true;
                continue;
            }
            _ => continue,
        };
        if key.kind == KeyEventKind::Release {
            continue;
        }
        if key.code == KeyCode::Char('q')
            || (key.modifiers.contains(KeyModifiers::CONTROL)
                && matches!(key.code, KeyCode::Char('c' | 'd')))
        {
            break;
        }
        // A terminal that has shrunk below the floor shows the refusal and
        // takes nothing but the quit keys above, so a resize cannot act on a
        // row the person can no longer see.
        if !Layout::detect().browsable() {
            continue;
        }
        let answer = match key.code {
            KeyCode::Up | KeyCode::Char('k') => {
                let cursor = source.cursor();
                if cursor > 0 {
                    source.select(cursor - 1);
                    Answer::Handled
                } else {
                    Answer::Ignored
                }
            }
            KeyCode::Down | KeyCode::Char('j') => {
                let last = source.entries().len().saturating_sub(1);
                let cursor = source.cursor();
                if cursor < last {
                    source.select(cursor + 1);
                    Answer::Handled
                } else {
                    Answer::Ignored
                }
            }
            _ => source.key(key),
        };
        match answer {
            Answer::Quit => break,
            Answer::Suspend => {
                drop(screen.take());
                source.suspended();
                screen = Some(Screen::enter()?);
                dirty = true;
            }
            Answer::Handled => dirty = true,
            Answer::Ignored => {}
        }
    }
    drop(screen);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::terminal::plain;
    use unicode_width::UnicodeWidthStr;

    #[test]
    fn exclusive_ownership_and_setup_failure_release_the_guard() {
        let first = Screen::enter_with(|| Ok(())).unwrap();
        assert!(matches!(Screen::enter_with(|| panic!("nested setup ran")),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists));
        drop(first);
        assert!(
            Screen::enter_with(|| Err(std::io::Error::other("injected setup failure"))).is_err()
        );
        assert!(
            SIGNALS
                .get()
                .unwrap()
                .as_ref()
                .unwrap()
                .default_action
                .load(Ordering::SeqCst)
        );
        drop(Screen::enter_with(|| Ok(())).unwrap());
    }

    struct Fake {
        entries: Vec<Entry>,
        cursor: usize,
    }

    impl Source for Fake {
        fn title(&self) -> String {
            "Software history".into()
        }
        fn subtitle(&self) -> String {
            "Review package changes and return to an earlier revision.".into()
        }
        fn list_title(&self) -> String {
            "REVISIONS".into()
        }
        fn badge(&self) -> String {
            format!("{} retained revisions", self.entries.len())
        }
        fn entries(&self) -> &[Entry] {
            &self.entries
        }
        fn cursor(&self) -> usize {
            self.cursor
        }
        fn select(&mut self, index: usize) {
            self.cursor = index;
        }
        fn detail(&self, width: usize, _height: usize) -> Vec<Line> {
            let mut lines = vec![("Compared with selected software".into(), Ink::Muted, false)];
            lines.extend(note("界面-editor was removed in this revision.", width));
            lines
        }
        fn keys(&self) -> String {
            "↑↓ Navigate  Enter Review  q Quit".into()
        }
        fn key(&mut self, _key: KeyEvent) -> Answer {
            Answer::Ignored
        }
    }

    fn fake() -> Fake {
        Fake {
            entries: [
                ("a83f19c2", "Installed Inkscape", "Selected"),
                ("7be204d1", "Updated software", ""),
                ("4cd890a6", "Initial system", "Kept"),
            ]
            .into_iter()
            .map(|(id, title, badge)| Entry {
                id: format!("Revision {id}"),
                title: title.into(),
                badge: badge.into(),
            })
            .collect(),
            cursor: 1,
        }
    }

    /// No pane ever writes past its edge, at any size the browser accepts.
    #[test]
    fn every_layout_stays_inside_the_terminal() {
        let source = fake();
        for columns in [BROWSER, 60, 80, 96, 140, 200] {
            for rows in [BROWSER_ROWS, 30, 40] {
                let lines = render(&source, Layout::of(columns, rows), Theme::PLAIN);
                assert!(lines.len() < rows, "{columns}x{rows}: {}", lines.len());
                for line in lines {
                    assert!(
                        plain(&line).width() + 2 < columns,
                        "{columns}x{rows}: {line:?}"
                    );
                }
            }
        }
    }

    /// Too small to be honest: say so rather than draw a broken screen.
    #[test]
    fn a_terminal_below_the_floor_is_refused_not_squeezed() {
        for (columns, rows) in [(BROWSER - 1, 40), (100, BROWSER_ROWS - 1), (20, 10)] {
            let lines = render(&fake(), Layout::of(columns, rows), Theme::PLAIN);
            let text = lines.join("\n");
            // On a very narrow terminal even the refusal is clipped; what
            // must survive is the word that tells the person what to do.
            assert!(text.contains("Enlarge"), "{columns}x{rows}: {text}");
            assert!(text.contains("q Quit"), "{columns}x{rows}: {text}");
            assert!(!text.contains("Installed Inkscape"), "{columns}x{rows}");
            for line in &lines {
                assert!(
                    plain(line).width() + 2 <= columns,
                    "{columns}x{rows}: {line:?}"
                );
            }
        }
    }

    /// The cursor row is marked by a shape as well as a background, so it is
    /// still findable with colour off.
    #[test]
    fn the_cursor_is_visible_without_colour() {
        let lines = render(&fake(), Layout::of(110, 36), Theme::PLAIN);
        let marked: Vec<_> = lines.iter().filter(|line| line.contains('›')).collect();
        assert_eq!(marked.len(), 1, "exactly one cursor: {marked:?}");
        assert!(marked[0].contains("7be204d1"), "{:?}", marked[0]);
    }

    /// Both panes are drawn above the two-pane threshold, and only one below.
    #[test]
    fn the_second_pane_appears_at_the_stated_width() {
        let wide = render(&fake(), Layout::of(110, 36), Theme::PLAIN).join("\n");
        assert!(
            wide.contains("REVISIONS") && wide.contains("Compared with"),
            "{wide}"
        );
        let narrow = render(&fake(), Layout::of(70, 36), Theme::PLAIN).join("\n");
        assert!(!narrow.contains("REVISIONS"), "{narrow}");
        assert!(narrow.contains("Compared with"), "{narrow}");
    }
}

#[cfg(all(test, unix))]
#[path = "screen_tests.rs"]
mod screen_tests;

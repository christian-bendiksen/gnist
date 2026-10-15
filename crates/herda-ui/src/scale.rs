//! Large statements: scaled text where the terminal reports it, bold elsewhere.
//!
//! kitty's text sizing protocol draws a grapheme in a block of cells larger
//! than one: `OSC 66 ; s=2:w=1 ; A ST` is an `A` two cells wide and two rows
//! high. The terminal Herda ships implements it; almost nothing else does, and
//! a terminal that does not would print nothing for the sequence and leave the
//! layout two rows short. So nothing here is assumed. A statement is scaled
//! only on a terminal that has just shown that it scales, and is ordinary bold
//! text everywhere else — and the layout is computed for whichever it is.
//!
//! The report is the one the protocol specifies: ask where the cursor is, draw
//! a space two cells wide, ask again, draw a space in a two by two block, ask
//! again. A terminal that moved the cursor two cells each time implements both
//! parts.
//!
//! Asking has two ways to go wrong, and both are closed here rather than left
//! to the caller. A terminal that never answers must not hold the command up:
//! the question is put from a helper thread and given a fixed time, after
//! which the answer is "bold", the helper is told to send nothing further, and
//! the cursor query it was waiting in has given up too, so keys are read.
//! And nothing asked or answered may be left on the screen: the question is
//! only put on the alternate screen in raw mode, by [`crate::surface::Surface`],
//! which clears the screen before it draws. Only a terminal that names itself
//! as one known to implement the protocol is asked at all.
//!
//! The question needs a cursor query that gives up when it is told to, which
//! crossterm as published does not have: its query waits two seconds and holds
//! the event reader for as long, so a silent terminal would cost two seconds
//! in which no key is read. Asking is therefore behind the crate's
//! `text-sizing` feature, for a tool built against a crossterm that carries
//! `cursor::position_within`. Without the feature no terminal is asked and
//! every statement is bold. With the feature and a crossterm that has no such
//! query, the crate still builds and still asks nothing (`cursor_within`):
//! a workspace that vendors the crate and builds every feature of every
//! member must not fail on one it cannot use.

#[cfg(feature = "text-sizing")]
use std::io::Write as _;
#[cfg(feature = "text-sizing")]
use std::sync::OnceLock;
#[cfg(any(test, feature = "text-sizing"))]
use std::sync::mpsc;
#[cfg(any(test, feature = "text-sizing"))]
use std::sync::{Arc, Mutex};
#[cfg(any(test, feature = "text-sizing"))]
use std::time::{Duration, Instant};

/// How a large statement is drawn on this terminal.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Sizing {
    /// Ordinary bold text, one row high.
    #[default]
    Bold,
    /// Text sizing sequences: a statement at scale two is two rows high.
    Scaled,
}

/// Draws a space in two cells: the width part of the protocol.
#[cfg(any(test, feature = "text-sizing"))]
const PROBE_WIDTH: &str = "\x1b]66;w=2; \x1b\\";
/// Draws a space in a two by two block: the scale part.
#[cfg(any(test, feature = "text-sizing"))]
const PROBE_SCALE: &str = "\x1b]66;s=2; \x1b\\";

/// How long a terminal has to answer before the answer is taken to be no.
#[cfg(feature = "text-sizing")]
const PATIENCE: Duration = Duration::from_millis(200);

/// The sequence that draws one grapheme `width` cells wide at ordinary size in
/// a block `scale` times that across and `scale` rows high.
///
/// The width is always stated, so the terminal lays the block out in the cells
/// it was measured at rather than by its own tables.
pub(crate) fn sequence(grapheme: &str, scale: u8, width: u8) -> String {
    format!("\x1b]66;s={scale}:w={width};{grapheme}\x1b\\")
}

/// What three cursor columns say: before anything, after the two-cell space,
/// after the two by two space.
#[cfg(any(test, feature = "text-sizing"))]
fn verdict(columns: [u16; 3]) -> Sizing {
    if columns[1] == columns[0] + 2 && columns[2] == columns[1] + 2 {
        Sizing::Scaled
    } else {
        Sizing::Bold
    }
}

/// Put the question through `write` and `column`, and read the answer.
///
/// Stops at the first failure, and does not send the scale probe to a terminal
/// that did nothing with the width probe.
#[cfg(any(test, feature = "text-sizing"))]
fn probe(
    mut write: impl FnMut(&str) -> std::io::Result<()>,
    mut column: impl FnMut() -> std::io::Result<u16>,
) -> Sizing {
    let mut ask = || -> std::io::Result<Sizing> {
        write("\r")?;
        let start = column()?;
        write(PROBE_WIDTH)?;
        let after_width = column()?;
        if after_width != start + 2 {
            return Ok(Sizing::Bold);
        }
        write(PROBE_SCALE)?;
        Ok(verdict([start, after_width, column()?]))
    };
    ask().unwrap_or(Sizing::Bold)
}

/// Whether the environment names a terminal known to implement the protocol.
///
/// kitty sets `KITTY_WINDOW_ID` in every window and its terminfo name contains
/// `kitty`. Either is a reason to ask, not an answer: a multiplexer inside
/// kitty inherits both and swallows the sequence, and reports that it did.
#[cfg(any(test, feature = "text-sizing"))]
fn named(term: Option<&str>, kitty_window: bool) -> bool {
    term != Some("dumb") && (kitty_window || term.is_some_and(|term| term.contains("kitty")))
}

/// Ask the terminal, once per process, whether it scales text.
///
/// Must be called in raw mode on the alternate screen, with the screen cleared
/// afterwards: a terminal that scales draws two blank blocks at the cursor.
/// Never takes longer than [`PATIENCE`], and returns [`Sizing::Bold`] without
/// asking when the terminal is not one that is known to answer.
#[cfg(feature = "text-sizing")]
pub(crate) fn detect() -> Sizing {
    static ANSWER: OnceLock<Sizing> = OnceLock::new();
    *ANSWER.get_or_init(|| {
        let term = std::env::var("TERM").ok();
        if !named(
            term.as_deref(),
            std::env::var_os("KITTY_WINDOW_ID").is_some(),
        ) {
            return Sizing::Bold;
        }
        // Each query waits only for what is left of the time the whole
        // question has, so the event reader is free again when the wait ends.
        let deadline = Instant::now() + PATIENCE;
        ask(
            PATIENCE,
            |bytes| {
                let mut out = std::io::stdout().lock();
                out.write_all(bytes.as_bytes())?;
                out.flush()
            },
            move || {
                cursor_within(deadline.saturating_duration_since(Instant::now()))
                    .map(|(column, _)| column)
            },
        )
    })
}

/// The cursor's position, from a query that gives up after `patience`.
///
/// This is crossterm's `cursor::position_within` where the crossterm the
/// crate is built against has one. Names brought in by a glob import give way
/// to nothing but are looked up before the enclosing module, so the import
/// below selects crossterm's function when it exists and leaves the stand-in
/// of the same name, which answers that there is no such query, when it does
/// not. The question is then never put: [`probe`] stops at the first failure.
#[cfg(feature = "text-sizing")]
fn cursor_within(patience: Duration) -> std::io::Result<(u16, u16)> {
    #[allow(unused_imports, clippy::wildcard_imports)]
    use crossterm::cursor::*;
    position_within(patience)
}

/// What [`cursor_within`] falls back to against a crossterm without a bounded
/// cursor query. Unused wherever crossterm has one.
#[cfg(feature = "text-sizing")]
#[allow(dead_code)]
fn position_within(_patience: Duration) -> std::io::Result<(u16, u16)> {
    Err(std::io::Error::new(
        std::io::ErrorKind::Unsupported,
        "this crossterm has no cursor query that takes a deadline",
    ))
}

/// Without the `text-sizing` feature no terminal is asked: there is no cursor
/// query here that is sure to give up in time, and a statement is bold.
#[cfg(not(feature = "text-sizing"))]
pub(crate) fn detect() -> Sizing {
    Sizing::Bold
}

/// Put the question from a helper thread and wait this long for the answer.
///
/// The helper may outlive the wait by a moment: the cursor query it is blocked
/// in is given the same time and gives up when it has passed, which is what
/// frees the event reader for keys. What it must not do after the wait is
/// draw, so every write goes through a gate that the waiting side closes
/// before it returns. A late answer is read by the event reader and dropped
/// there.
#[cfg(any(test, feature = "text-sizing"))]
fn ask(
    patience: Duration,
    write: impl Fn(&str) -> std::io::Result<()> + Send + 'static,
    column: impl Fn() -> std::io::Result<u16> + Send + 'static,
) -> Sizing {
    let closed = Arc::new(Mutex::new(false));
    let gate = Arc::clone(&closed);
    let (answer, waiting) = mpsc::channel();
    let helper = std::thread::Builder::new()
        .name("text-sizing".into())
        .spawn(move || {
            let gated = |bytes: &str| {
                let closed = gate.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
                if *closed {
                    return Err(std::io::Error::other("the answer is no longer wanted"));
                }
                write(bytes)
            };
            let _ = answer.send(probe(gated, column));
        });
    if helper.is_err() {
        return Sizing::Bold;
    }
    let sizing = waiting.recv_timeout(patience).unwrap_or(Sizing::Bold);
    *closed
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner()) = true;
    sizing
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    /// The sequences, written out. They are data: this is the one place the
    /// bytes are pinned, and they were checked against kitty's own parser.
    #[test]
    fn the_sequences_are_the_ones_the_protocol_specifies() {
        assert_eq!(sequence("A", 2, 1), "\x1b]66;s=2:w=1;A\x1b\\");
        assert_eq!(sequence("界", 3, 2), "\x1b]66;s=3:w=2;界\x1b\\");
        assert_eq!(PROBE_WIDTH, "\x1b]66;w=2; \x1b\\");
        assert_eq!(PROBE_SCALE, "\x1b]66;s=2; \x1b\\");
    }

    /// A terminal that answers as kitty does, one that ignores the sequences,
    /// one that implements only the width part, and one that never answers.
    #[test]
    fn only_a_terminal_that_moved_twice_is_scaled() {
        fn run(moves: [u16; 2], answers: usize) -> (Sizing, String) {
            let sent = RefCell::new(String::new());
            let column = RefCell::new(0u16);
            let asked = RefCell::new(0usize);
            let sizing = probe(
                |bytes| {
                    sent.borrow_mut().push_str(bytes);
                    match bytes {
                        "\r" => *column.borrow_mut() = 0,
                        PROBE_WIDTH => *column.borrow_mut() += moves[0],
                        PROBE_SCALE => *column.borrow_mut() += moves[1],
                        other => panic!("unexpected write {other:?}"),
                    }
                    Ok(())
                },
                || {
                    *asked.borrow_mut() += 1;
                    if *asked.borrow() > answers {
                        return Err(std::io::Error::other("no answer"));
                    }
                    Ok(*column.borrow())
                },
            );
            (sizing, sent.into_inner())
        }

        let (sizing, sent) = run([2, 2], 3);
        assert_eq!(sizing, Sizing::Scaled);
        assert_eq!(sent, format!("\r{PROBE_WIDTH}{PROBE_SCALE}"));

        let (sizing, sent) = run([0, 0], 3);
        assert_eq!(sizing, Sizing::Bold);
        assert_eq!(sent, format!("\r{PROBE_WIDTH}"), "no second probe");

        assert_eq!(run([2, 0], 3).0, Sizing::Bold, "width without scale");
        assert_eq!(run([1, 2], 3).0, Sizing::Bold);

        let (sizing, sent) = run([2, 2], 0);
        assert_eq!(sizing, Sizing::Bold);
        assert_eq!(sent, "\r", "nothing is drawn for a terminal that is silent");
        assert_eq!(run([2, 2], 2).0, Sizing::Bold, "silent part way through");

        assert_eq!(verdict([5, 7, 9]), Sizing::Scaled);
        assert_eq!(verdict([5, 5, 5]), Sizing::Bold);
    }

    #[test]
    fn only_a_terminal_that_names_itself_is_asked() {
        assert!(named(Some("xterm-kitty"), false));
        assert!(named(Some("tmux-256color"), true), "asked, and answers no");
        assert!(named(None, true));
        assert!(!named(Some("xterm-256color"), false));
        assert!(!named(Some("linux"), false));
        assert!(!named(None, false));
        assert!(!named(Some("dumb"), true));
    }

    /// A terminal that answers the first question too late, or never: the
    /// wait ends on time with "bold", and once it has ended the helper draws
    /// nothing, however the terminal answers afterwards.
    #[test]
    fn a_terminal_that_is_slow_to_answer_costs_a_bounded_wait_and_no_bytes() {
        for late in [Ok(0), Err(())] {
            let sent = Arc::new(Mutex::new(String::new()));
            let log = Arc::clone(&sent);
            let started = Instant::now();
            let sizing = ask(
                Duration::from_millis(40),
                move |bytes| {
                    log.lock().unwrap().push_str(bytes);
                    Ok(())
                },
                move || {
                    std::thread::sleep(Duration::from_millis(250));
                    late.map_err(|()| std::io::Error::other("no answer"))
                },
            );
            assert_eq!(sizing, Sizing::Bold);
            assert!(
                started.elapsed() < Duration::from_millis(200),
                "{:?}",
                started.elapsed()
            );
            std::thread::sleep(Duration::from_millis(400));
            assert_eq!(*sent.lock().unwrap(), "\r");
        }
    }

    /// A terminal that answers in time is believed.
    #[test]
    fn a_terminal_that_answers_in_time_is_believed() {
        let column = Arc::new(Mutex::new(0u16));
        let moved = Arc::clone(&column);
        let sizing = ask(
            Duration::from_secs(5),
            move |bytes| {
                let mut column = moved.lock().unwrap();
                *column = if bytes == "\r" { 0 } else { *column + 2 };
                Ok(())
            },
            move || Ok(*column.lock().unwrap()),
        );
        assert_eq!(sizing, Sizing::Scaled);
    }
}

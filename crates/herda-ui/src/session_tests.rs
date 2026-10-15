#![cfg(unix)]

use super::{Action, Attached, CLIPBOARD_LIMIT, Pressed, Session, Shown, base64, clipboard};
use crate::browser::{KeyCode, KeyEvent, KeyModifiers};
use crate::cells::Buffer;
use crate::ink::Ink;
use crate::scale::Sizing;
use crate::width::{FRAME, FRAME_ROWS, Layout};
use std::time::{Duration, Instant};

fn attached() -> Attached {
    Attached {
        dumb: false,
        stdin_terminal: true,
        stdout_terminal: true,
        stderr_terminal: true,
        size: Some((100, 34)),
    }
}

/// The whole table of where a screen can be shown.
#[test]
fn a_frame_is_taken_only_by_a_terminal_that_can_hold_it() {
    let whole = attached();
    assert!(whole.redraws() && whole.takes_a_frame());
    for (attached, redraws, why) in [
        (
            Attached {
                stdout_terminal: false,
                size: None,
                ..whole
            },
            false,
            "a pipe",
        ),
        (
            Attached {
                dumb: true,
                ..whole
            },
            false,
            "TERM=dumb",
        ),
        (
            Attached {
                stdin_terminal: false,
                ..whole
            },
            true,
            "no keys to read",
        ),
        (
            Attached {
                stderr_terminal: false,
                ..whole
            },
            true,
            "errors go elsewhere",
        ),
        (
            Attached {
                size: Some((FRAME - 1, 34)),
                ..whole
            },
            true,
            "too narrow",
        ),
        (
            Attached {
                size: Some((100, FRAME_ROWS - 1)),
                ..whole
            },
            true,
            "too short",
        ),
        (
            Attached {
                size: None,
                ..whole
            },
            true,
            "no size",
        ),
    ] {
        assert_eq!(attached.redraws(), redraws, "{why}");
        assert!(!attached.takes_a_frame(), "{why}");
    }
    let floor = Attached {
        size: Some((FRAME, FRAME_ROWS)),
        ..whole
    };
    assert!(floor.takes_a_frame(), "the floor itself");
}

#[test]
fn a_key_with_control_or_alt_is_not_the_models() {
    let plain = |code| KeyEvent::new(code, KeyModifiers::NONE);
    for (code, pressed) in [
        (KeyCode::Enter, Pressed::Enter),
        (KeyCode::Esc, Pressed::Escape),
        (KeyCode::Up, Pressed::Up),
        (KeyCode::Down, Pressed::Down),
        (KeyCode::Left, Pressed::Left),
        (KeyCode::Right, Pressed::Right),
        (KeyCode::PageUp, Pressed::PageUp),
        (KeyCode::PageDown, Pressed::PageDown),
        (KeyCode::Home, Pressed::Home),
        (KeyCode::End, Pressed::End),
        (KeyCode::Char('l'), Pressed::Char('l')),
    ] {
        assert_eq!(super::pressed(plain(code)), Some(pressed));
    }
    assert_eq!(
        super::pressed(KeyEvent::new(KeyCode::Char('Y'), KeyModifiers::SHIFT)),
        Some(Pressed::Char('Y'))
    );
    assert_eq!(super::pressed(plain(KeyCode::Tab)), None);
    for modifiers in [KeyModifiers::CONTROL, KeyModifiers::ALT] {
        assert_eq!(
            super::pressed(KeyEvent::new(KeyCode::Char('c'), modifiers)),
            None
        );
    }
    let interrupt = KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL);
    assert!(super::interrupts(interrupt));
    assert!(!super::interrupts(plain(KeyCode::Char('c'))));
}

/// The alphabet and the padding, against the vectors of RFC 4648.
#[test]
fn the_clipboard_sequence_carries_base64() {
    for (plain, encoded) in [
        ("", ""),
        ("f", "Zg=="),
        ("fo", "Zm8="),
        ("foo", "Zm9v"),
        ("foob", "Zm9vYg=="),
        ("fooba", "Zm9vYmE="),
        ("foobar", "Zm9vYmFy"),
    ] {
        assert_eq!(base64(plain.as_bytes()), encoded);
    }
    assert_eq!(base64(&[0xfb, 0xff, 0xbe]), "+/++");
    assert_eq!(clipboard("hi"), "\x1b]52;c;aGk=\x1b\\");
    // A long text keeps its end, and is cut between characters: here the
    // limit falls inside one.
    let long = format!("{}a", "界".repeat(CLIPBOARD_LIMIT));
    let sequence = clipboard(&long);
    assert!(sequence.len() <= CLIPBOARD_LIMIT.div_ceil(3) * 4 + 9);
    assert!(sequence.ends_with(&format!("{}\x1b\\", base64("界a".as_bytes()))));
}

/// A command that says what it is told, for the terminal the harness plays.
#[derive(Default)]
struct Told {
    heard: Vec<String>,
    said: Vec<&'static str>,
    question: bool,
    stoppable: bool,
    ended: bool,
}

impl Shown for Told {
    fn draw(&mut self, layout: Layout, _sizing: Sizing, _now: Instant) -> Buffer {
        if layout.columns() < FRAME || layout.rows() < FRAME_ROWS {
            return crate::frame::notice(layout, "told", "A session", "Nothing changes.");
        }
        let mut buffer = Buffer::new(layout.columns(), layout.rows());
        let mut canvas = buffer.canvas();
        let width = canvas.width();
        // Each thing has rows of its own, so that it is written whole when
        // it first appears and the harness can look for it as text.
        canvas.fit(0, 0, width, "told  A session", Ink::Text);
        for (y, line) in self.heard.iter().enumerate() {
            canvas.fit(0, 2 + y, width, &format!("heard: {line}"), Ink::Text);
        }
        for (y, line) in self.said.iter().enumerate() {
            canvas.fit(0, 8 + y, width, line, Ink::Text);
        }
        if self.question {
            canvas.fit(0, 14, width, "Go on?", Ink::Text);
        }
        if self.stoppable {
            canvas.fit(0, 15, width, "Esc stops", Ink::Muted);
        }
        buffer
    }

    fn settle(&mut self, _now: Instant) -> bool {
        false
    }

    fn ticking(&self) -> bool {
        false
    }

    fn key(&mut self, key: Pressed, _now: Instant) -> Action {
        match key {
            Pressed::Enter if self.question => {
                self.question = false;
                Action::Answer(true)
            }
            Pressed::Char('c') => Action::Copy("what was copied".to_owned()),
            Pressed::Escape if self.stoppable => {
                self.stoppable = false;
                self.said.push("stopping");
                Action::Cancel
            }
            Pressed::Enter if self.ended => Action::Leave,
            _ => Action::None,
        }
    }

    fn waits_at_the_end(&self) -> bool {
        self.ended
    }

    fn output(&mut self, line: &str) {
        self.heard.push(line.to_owned());
    }
}

// Run only in the disposable PTY child: it takes the terminal it is given.
#[test]
fn session_child() {
    let Some(mode) = std::env::var_os("HERDA_SESSION_TEST_MODE") else {
        return;
    };
    let session = Session::new(Told::default());
    let tell = |change: &dyn Fn(&mut Told)| {
        change(&mut session.model());
        session.changed();
    };
    session.take();
    assert!(session.up());
    // What anything else in the process writes is the model's, not the
    // terminal's.
    println!("a line \x1b[31mfrom elsewhere\x1b[0m");
    eprintln!("and one on standard error");
    session.ask();
    tell(&|told| told.question = true);
    let accepted = session.answer();
    if mode == "interrupt" {
        // The harness ends the process at the question.
        unreachable!("answered {accepted} after Ctrl-C");
    }
    let answer = if accepted {
        "answered yes"
    } else {
        "answered no"
    };
    tell(&|told| told.said.push(answer));
    assert!(session.lend());
    assert!(!session.up());
    println!("the terminal is lent");
    session.take();
    assert!(session.up());
    tell(&|told| told.stoppable = true);
    while !session.stop_requested() {
        std::thread::sleep(Duration::from_millis(10));
    }
    tell(&|told| {
        told.said.push("stopped");
        told.ended = true;
    });
    assert!(session.leave());
    assert!(!session.up());
    println!("left the screen");
}

#[test]
fn the_session_in_a_real_terminal() {
    if crate::vendored() {
        return;
    }
    let output = std::process::Command::new("python3")
        .arg(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/session_terminal.py"
        ))
        .arg(std::env::current_exe().unwrap())
        .arg("session::session_tests::session_child")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

#![cfg(unix)]

use super::{Answer, Entry, KeyCode, KeyEvent, Line, Screen, Source};
use std::io::Write as _;

fn ready() {
    println!("SCREEN-READY");
    std::io::stdout().flush().unwrap();
}

fn wait_forever() -> ! {
    loop {
        std::thread::park_timeout(std::time::Duration::from_millis(10));
    }
}

struct SuspendSource;
impl Source for SuspendSource {
    fn title(&self) -> String {
        "SCREEN-BROWSER".into()
    }
    fn subtitle(&self) -> String {
        String::new()
    }
    fn badge(&self) -> String {
        String::new()
    }
    fn entries(&self) -> &[Entry] {
        &[]
    }
    fn cursor(&self) -> usize {
        0
    }
    fn select(&mut self, _: usize) {}
    fn detail(&self, _: usize, _: usize) -> Vec<Line> {
        Vec::new()
    }
    fn keys(&self) -> String {
        "s Suspend".into()
    }
    fn key(&mut self, key: KeyEvent) -> Answer {
        if key.code == KeyCode::Char('s') {
            Answer::Suspend
        } else {
            Answer::Ignored
        }
    }
    fn suspended(&mut self) {
        ready();
        wait_forever();
    }
}

/// A browser that is slow to paint at one width, so that a resize and a key
/// can both be waiting when it next asks for an event.
struct ResizeSource {
    was_busy: std::cell::Cell<bool>,
}
impl Source for ResizeSource {
    fn title(&self) -> String {
        "RESIZE-BROWSER".into()
    }
    fn subtitle(&self) -> String {
        let columns = crate::terminal::size().0;
        if columns == 90 && !self.was_busy.replace(true) {
            println!("SCREEN-BUSY");
            std::io::stdout().flush().unwrap();
            std::thread::sleep(std::time::Duration::from_millis(600));
        }
        // What a repaint for a new size shows, and nothing else does.
        format!("COLUMNS-{columns}")
    }
    fn badge(&self) -> String {
        String::new()
    }
    fn entries(&self) -> &[Entry] {
        &[]
    }
    fn cursor(&self) -> usize {
        0
    }
    fn select(&mut self, _: usize) {}
    fn detail(&self, _: usize, _: usize) -> Vec<Line> {
        Vec::new()
    }
    fn keys(&self) -> String {
        "x Mark".into()
    }
    fn key(&mut self, key: KeyEvent) -> Answer {
        if key.code == KeyCode::Char('x') {
            println!("SCREEN-KEY-X");
            std::io::stdout().flush().unwrap();
        }
        // Nothing a key does here repaints: only a resize does.
        Answer::Ignored
    }
}

// Run only in the disposable PTY child; signals must never target the test runner.
#[test]
fn screen_child() {
    let Ok(mode) = std::env::var("HERDA_SCREEN_TEST_MODE") else {
        return;
    };
    if mode == "suspend" || mode == "unterminated-paste" {
        super::run(&mut SuspendSource).unwrap();
        return;
    }
    if mode == "resize-race" {
        super::run(&mut ResizeSource {
            was_busy: std::cell::Cell::new(false),
        })
        .unwrap();
        return;
    }
    if mode == "setup-signal" {
        let signal = std::env::var("HERDA_SCREEN_TEST_SIGNAL")
            .unwrap()
            .parse()
            .unwrap();
        let _screen = Screen::enter_with(|| {
            super::ACTIVE.store(true, std::sync::atomic::Ordering::SeqCst);
            crossterm::terminal::enable_raw_mode()?;
            signal_hook::low_level::raise(signal)?;
            Err(std::io::Error::other("setup interrupted"))
        });
        panic!("setup termination signal was lost");
    }
    if mode == "setup-failure" {
        assert!(
            Screen::enter_with(|| {
                super::ACTIVE.store(true, std::sync::atomic::Ordering::SeqCst);
                crossterm::terminal::enable_raw_mode()?;
                crossterm::execute!(
                    std::io::stdout(),
                    crossterm::terminal::EnterAlternateScreen,
                    crossterm::cursor::Hide
                )?;
                Err(std::io::Error::other(
                    "injected failure after raw mode and cursor hide",
                ))
            })
            .is_err()
        );
        ready();
        wait_forever();
    }
    if mode == "caught-panic" {
        assert!(
            std::panic::catch_unwind(|| {
                let _screen = Screen::enter().unwrap();
                panic!("injected panic");
            })
            .is_err()
        );
        ready();
        wait_forever();
    }
    let mut screen = Screen::enter().unwrap();
    if mode == "nested" {
        assert!(matches!(Screen::enter(), Err(error)
            if error.kind() == std::io::ErrorKind::AlreadyExists));
    } else if mode == "reenter" {
        drop(screen);
        screen = Screen::enter().unwrap();
    } else if mode == "dropped" {
        drop(screen);
        ready();
        wait_forever();
    } else if mode == "panic" {
        panic!("injected panic with raw terminal");
    }
    ready();
    while !screen.interrupted() {
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    drop(screen);
    panic!("pending termination signal was swallowed");
}

#[test]
fn screen_lifecycle_in_a_real_terminal() {
    if crate::vendored() {
        return;
    }
    let output = std::process::Command::new("python3")
        .arg(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/screen_lifecycle.py"
        ))
        .arg(std::env::current_exe().unwrap())
        .arg("browser::screen_tests::screen_child")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

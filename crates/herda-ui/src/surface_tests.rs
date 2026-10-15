#![cfg(unix)]

use super::{Input, Surface};
use crate::browser::KeyCode;
use std::time::Duration;

// Run only in the disposable PTY child: it takes the terminal it is given.
#[test]
fn surface_child() {
    if std::env::var_os("HERDA_SURFACE_TEST_MODE").is_none() {
        return;
    }
    let mut surface = Surface::enter().unwrap();
    let frame = crate::frame_fixtures::frame("review");
    let mut dirty = true;
    loop {
        if dirty {
            let buffer = frame
                .draw(surface.layout(), surface.sizing())
                .expect("the harness gives the surface room");
            surface.show(&buffer).unwrap();
            dirty = false;
        }
        match surface.wait(Duration::from_millis(20)).unwrap() {
            Input::Key(key) if key.code == KeyCode::Char('q') => break,
            // Anything else that arrives as a key is something the terminal
            // said that was mistaken for one.
            Input::Key(key) => {
                drop(surface);
                panic!("an unexpected key: {key:?}");
            }
            Input::Resized => dirty = true,
            Input::Quiet => {}
            Input::Interrupted => break,
        }
    }
}

#[test]
fn the_surface_in_a_real_terminal() {
    if crate::vendored() {
        return;
    }
    let output = std::process::Command::new("python3")
        .arg(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/surface_terminal.py"
        ))
        .arg(std::env::current_exe().unwrap())
        .arg("surface::surface_tests::surface_child")
        // Whether this build asks a terminal if it scales text at all.
        .arg(if cfg!(feature = "text-sizing") {
            "asks"
        } else {
            "never-asks"
        })
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

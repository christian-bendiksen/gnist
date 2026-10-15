//! Every screen of the design, held as text.
//!
//! The fixtures in `frame_fixtures.rs` are drawn at the three sizes the design
//! is judged at, scaled and bold, and compared with the text files under
//! `tests/screens`. A change to a screen is a change to one of those files,
//! which is what a reviewer reads. To write the files again after a deliberate
//! change, run the tests with `HERDA_UI_BLESS=1` and read the difference.

use std::path::PathBuf;

use unicode_width::UnicodeWidthStr;

use crate::cells::Buffer;
use crate::cells::replay::Terminal;
use crate::frame::Rung;
use crate::frame_fixtures::{STATES, frame, frame_naming};
use crate::ink::{Ink, glyph};
use crate::scale::Sizing;
use crate::terminal::Theme;
use crate::width::Layout;

/// The design's own size, a common terminal, and the smallest a frame is
/// drawn at.
const SIZES: [(usize, usize); 3] = [(100, 34), (80, 24), (60, 20)];

/// The screens that list packages by name.
const AWKWARD_STATES: [&str; 3] = ["review", "working-download", "done"];

/// Names with wide, combining and joined graphemes in them.
const AWKWARD: [&str; 3] = [
    "界面-editor",
    "cre\u{300}me-bru\u{302}le\u{301}e",
    "👩‍💻-tools",
];

fn name(state: &str, (columns, rows): (usize, usize), sizing: Sizing) -> String {
    let bold = match sizing {
        Sizing::Scaled => "",
        Sizing::Bold => "-bold",
    };
    format!("{state}-{columns}x{rows}{bold}")
}

/// Compare with the held file, or write it when blessing.
fn hold(name: &str, text: &str) {
    if crate::vendored() {
        return;
    }
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/screens")
        .join(format!("{name}.txt"));
    if std::env::var_os("HERDA_UI_BLESS").is_some() {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, text).unwrap();
        return;
    }
    let held = std::fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("{}: {error}", path.display()));
    assert!(
        held == text,
        "{name} is not the screen that is held in {}.\n--- held\n{held}\n--- drawn\n{text}",
        path.display()
    );
}

fn draw(state: &str, size: (usize, usize), sizing: Sizing) -> Buffer {
    frame(state)
        .draw(Layout::of(size.0, size.1), sizing)
        .unwrap_or_else(|| panic!("{state} does not fit {size:?}"))
}

/// What a terminal holds after these bytes, starting from nothing.
fn terminal(buffer: &Buffer, theme: Theme) -> Terminal {
    let mut terminal = Terminal::new(buffer.columns(), buffer.rows());
    terminal.feed(&buffer.paint(None, theme));
    terminal
}

fn each(mut check: impl FnMut(&str, (usize, usize), Sizing)) {
    for state in STATES {
        for size in SIZES {
            for sizing in [Sizing::Scaled, Sizing::Bold] {
                check(state, size, sizing);
            }
        }
    }
}

#[test]
fn every_screen_is_the_one_that_is_held() {
    each(|state, size, sizing| {
        let buffer = draw(state, size, sizing);
        let name = name(state, size, sizing);
        hold(&name, &buffer.text());
        crate::presentation::design_fixture(&format!("frame-{name}"), &buffer.lines(Theme::FULL));
    });
}

/// With colour off, at 256 colours and at full depth a terminal ends up
/// holding the same characters in the same cells, and with colour off it is
/// sent no style at all.
#[test]
fn every_screen_is_the_same_screen_with_colour_off() {
    each(|state, size, sizing| {
        let buffer = draw(state, size, sizing);
        let context = name(state, size, sizing);
        for theme in [Theme::PLAIN, Theme::INDEXED, Theme::FULL] {
            let shown = terminal(&buffer, theme);
            assert!(shown.unknown.is_empty(), "{context}: {:?}", shown.unknown);
            assert_eq!(shown.text(), buffer.text(), "{context} with {theme:?}");
        }
        let plain = buffer.paint(None, Theme::PLAIN);
        assert_eq!(
            plain.matches("\x1b[0").count(),
            1,
            "{context}: only the reset"
        );
        let indexed = buffer.paint(None, Theme::INDEXED);
        assert!(
            !indexed.contains(";2;"),
            "{context}: no truecolor at 256 colours"
        );
        if sizing == Sizing::Bold {
            assert!(
                !plain.contains("\x1b]66;"),
                "{context}: scaled text was not asked for"
            );
        }
    });
}

#[test]
fn every_screen_keeps_inside_the_terminal() {
    each(|state, size, sizing| {
        let buffer = draw(state, size, sizing);
        for (y, row) in buffer.text().lines().enumerate() {
            assert!(
                row.width() <= size.0 - 2,
                "{}: row {y} reaches the right gutter: {row:?}",
                name(state, size, sizing)
            );
            assert!(
                row.is_empty() || row.starts_with("  "),
                "{}: row {y} is in the left gutter: {row:?}",
                name(state, size, sizing)
            );
        }
    });
}

/// Every status on every screen has its mark and its word in the text, where
/// no ink can reach.
#[test]
fn every_status_keeps_its_mark_and_its_word() {
    each(|state, size, sizing| {
        let text = draw(state, size, sizing).text();
        let context = format!("{}\n{text}", name(state, size, sizing));
        let has = |words: &[&str]| {
            for word in words {
                assert!(text.contains(word), "no {word:?} in {context}");
            }
        };
        match state {
            "home" => has(&[
                "a83f19c2",
                "Selected",
                "✓ Healthy",
                "No change is in progress.",
            ]),
            "review" => has(&[
                "REVIEW",
                "✓ Resolve",
                "○ Download",
                "waits for you",
                "›+   inkscape",
                "Nothing changes until you confirm.",
                " Enter  Install",
            ]),
            "working-download" => has(&[
                "00:08",
                "● Download",
                "○ Verify",
                "Working",
                "Complete",
                "Your current software revision is still active.",
            ]),
            "working-log" => has(&["● Create", "○ Activate", "LOG", "21:04:27", " l  Close log"]),
            "done" => has(&[
                "DONE",
                "✓ Activate",
                "7be204d1",
                "The new software revision is active.",
            ]),
            "stopped" => has(&[
                "STOPPED",
                "✗ Create",
                "Stopped",
                "○ Activate",
                "Not started",
                "not created",
                "Invalid cross-device link",
                "Your current software revision is still active.",
            ]),
            "walked" => has(&["› 5d02b7e1", "No change is in progress."]),
            "moved-away" => has(&[
                "a83f19c2",
                "moved away",
                "◇",
                "›~",
                "Edited",
                "14 - background_opacity 0.86",
                "14 + background_opacity 1.0",
            ]),
            other => panic!("{other} has no words to keep"),
        }
    });
}

/// On every screen the one gold point of the line stands under the identity
/// of the revision the machine runs. Where a cursor has walked the line away
/// from that revision and only one fits, there is no gold point at all.
#[test]
fn every_screen_has_its_gold_point_on_the_selected_revision() {
    each(|state, size, sizing| {
        let frame = frame(state);
        let buffer = draw(state, size, sizing);
        let context = name(state, size, sizing);
        let rows: Vec<String> = buffer.text().lines().map(str::to_owned).collect();
        let selected = &frame.line.points[frame.line.selected.unwrap()].id;
        let Some(line) = rows
            .iter()
            .position(|row| row.contains(glyph::POINT_SELECTED) && row.contains(['━', '─', '╌']))
        else {
            assert!(
                frame.line.cursor.is_some() && !buffer.text().contains(selected.as_str()),
                "{context}: no line"
            );
            let gold = (0..buffer.columns()).any(|x| {
                (0..buffer.rows()).any(|y| {
                    buffer.cell(x, y).is_some_and(|(text, style)| {
                        text == glyph::POINT_SELECTED && style.ink == Ink::Gold
                    })
                })
            });
            assert!(!gold, "{context}: a gold point without its revision");
            return;
        };
        let gold: Vec<usize> = (0..buffer.columns())
            .filter(|x| {
                buffer.cell(*x, line).is_some_and(|(text, style)| {
                    text == glyph::POINT_SELECTED && style.ink == Ink::Gold
                })
            })
            .collect();
        assert_eq!(gold.len(), 1, "{context}");
        let ids = &rows[line - 1];
        let at = ids
            .find(selected.as_str())
            .unwrap_or_else(|| panic!("{context}: {ids}"));
        let column = ids[..at].width();
        assert!(
            column == gold[0] || column + selected.width() - 1 == gold[0],
            "{context}: gold at {}, {selected} at {column}",
            gold[0]
        );
        let (_, style) = buffer.cell(column, line - 1).unwrap();
        assert!(style.bold, "{context}: the selected identity is bold");
    });
}

/// Moving from any screen to any other by writing only what differs leaves a
/// terminal holding exactly the second screen. Where scaled text changes the
/// screen is painted whole, and only there.
#[test]
fn any_screen_can_be_repainted_into_any_other() {
    for size in SIZES {
        for sizing in [Sizing::Scaled, Sizing::Bold] {
            let screens: Vec<Buffer> = STATES
                .iter()
                .map(|state| draw(state, size, sizing))
                .collect();
            for (from, first) in screens.iter().enumerate() {
                for (to, second) in screens.iter().enumerate() {
                    let mut shown = terminal(first, Theme::FULL);
                    let bytes = second.paint(Some(first), Theme::FULL);
                    shown.feed(&bytes);
                    assert_eq!(
                        shown.text(),
                        second.text(),
                        "{} to {} at {size:?}",
                        STATES[from],
                        STATES[to]
                    );
                    assert!(shown.unknown.is_empty());
                    assert_eq!(
                        bytes.contains("\x1b[2J"),
                        second.scaled_text_moved(first),
                        "{} to {} at {size:?}",
                        STATES[from],
                        STATES[to]
                    );
                    if sizing == Sizing::Bold {
                        assert!(!bytes.contains("\x1b[2J"));
                    }
                    assert_eq!(bytes.is_empty(), from == to, "{}", STATES[from]);
                }
            }
        }
    }
}

/// A statement is scaled where the terminal scales text and it fits, and is
/// bold where either is not so.
#[test]
fn statements_are_scaled_only_where_they_fit() {
    let scaled = |state, size| {
        draw(state, size, Sizing::Scaled)
            .paint(None, Theme::PLAIN)
            .matches("\x1b]66;")
            .count()
    };
    // Every grapheme of the statement is its own block, spaces included.
    assert_eq!(
        scaled("review", SIZES[0]),
        "Add inkscape and the 2 packages it needs".len()
    );
    assert_eq!(
        scaled("review", SIZES[1]),
        0,
        "eighty cells do not fit seventy-six"
    );
    assert_eq!(scaled("working-download", SIZES[2]), 3);
    assert_eq!(
        scaled("done", SIZES[2]),
        "✓inkscape is installed".chars().count()
    );
    assert_eq!(scaled("home", SIZES[0]), 0);
    // Scaled, a statement is two rows; bold, one. The rest of the screen
    // moves up to meet it.
    let row = |sizing| {
        let text = draw("review", SIZES[0], sizing).text();
        text.lines()
            .position(|row| row.contains("33.78 MiB to download"))
            .unwrap()
    };
    assert_eq!(row(Sizing::Scaled), row(Sizing::Bold) + 1);
}

/// One tick of a download rewrites the cells that moved and nothing else.
#[test]
fn a_tick_of_progress_is_a_small_write() {
    use crate::revision_line::StepState;
    let at = |fraction: f32| {
        let mut frame = frame("working-download");
        if let Some(stretch) = &mut frame.line.stretch {
            stretch.steps[1].state = StepState::Working {
                fraction: Some(fraction),
                figure: "4.0s".into(),
            };
        }
        frame.draw(Layout::of(100, 34), Sizing::Scaled).unwrap()
    };
    let (before, after) = (at(0.67), at(0.74));
    let bytes = after.paint(Some(&before), Theme::FULL);
    assert!(
        !bytes.is_empty() && bytes.len() < 80,
        "{} bytes: {bytes:?}",
        bytes.len()
    );
    assert!(
        bytes.contains('━') && !bytes.contains("Download"),
        "{bytes:?}"
    );
    let mut shown = terminal(&before, Theme::FULL);
    shown.feed(&bytes);
    assert_eq!(shown.text(), after.text());
}

/// The three sizes show the ladder: everything at the design's size, the short
/// line with neither graph nor tiles below it.
#[test]
fn the_three_sizes_stand_on_the_rungs_the_design_names() {
    for state in STATES {
        for sizing in [Sizing::Scaled, Sizing::Bold] {
            let rung = |(columns, rows)| frame(state).rung(Layout::of(columns, rows), sizing);
            assert_eq!(rung(SIZES[0]), Some(Rung::Full), "{state}");
            assert!(rung(SIZES[1]) >= Some(Rung::ShortLine), "{state}");
            assert!(rung(SIZES[2]) >= Some(Rung::ShortLine), "{state}");
        }
    }
    let text = |state, size| draw(state, size, Sizing::Bold).text();
    assert!(text("working-download", SIZES[0]).contains("SPEED"));
    assert!(!text("working-download", SIZES[1]).contains("SPEED"));
    assert!(text("done", SIZES[0]).contains("NOW SELECTED"));
    assert!(!text("done", SIZES[1]).contains("NOW SELECTED"));
    assert!(text("review", SIZES[0]).contains("4cd890a6"));
    assert!(!text("review", SIZES[1]).contains("4cd890a6"));
    assert!(text("review", SIZES[1]).contains("a83f19c2"));
}

/// As a terminal shrinks a row or a column at a time, a screen only ever
/// gives things up, and below the frame's floor it is not drawn at all.
#[test]
fn a_shrinking_terminal_never_gets_anything_back() {
    for state in STATES {
        for sizing in [Sizing::Scaled, Sizing::Bold] {
            let frame = frame(state);
            for columns in [100, 80, 60] {
                let mut previous = Some(Rung::Full);
                for rows in (18..=40).rev() {
                    let rung = frame.rung(Layout::of(columns, rows), sizing);
                    assert!(
                        rung.is_none() || rung >= previous,
                        "{state} at {columns}x{rows}: {rung:?} after {previous:?}"
                    );
                    if rung.is_none() {
                        previous = Some(Rung::Bare);
                    } else {
                        previous = rung;
                    }
                }
            }
            for rows in [34, 24] {
                let mut previous = Some(Rung::Full);
                for columns in (58..=110).rev() {
                    let rung = frame.rung(Layout::of(columns, rows), sizing);
                    assert!(
                        rung.is_none() || rung >= previous,
                        "{state} at {columns}x{rows}: {rung:?} after {previous:?}"
                    );
                    previous = rung.or(Some(Rung::Bare));
                }
            }
            assert_eq!(frame.rung(Layout::of(59, 40), sizing), None);
            assert_eq!(frame.rung(Layout::of(100, 19), sizing), None);
        }
    }
}

/// Package names with wide, combining and joined graphemes: the screens are
/// held like the others, every column after the name starts where it does for
/// plain names, and a terminal ends up holding the same cells.
#[test]
fn awkward_package_names_keep_every_column() {
    for state in AWKWARD_STATES {
        for size in SIZES {
            for sizing in [Sizing::Scaled, Sizing::Bold] {
                let layout = Layout::of(size.0, size.1);
                let plain = draw(state, size, sizing);
                let awkward = frame_naming(state, AWKWARD)
                    .draw(layout, sizing)
                    .unwrap_or_else(|| panic!("{state} does not fit {size:?}"));
                let context = format!("{}-names", name(state, size, sizing));
                hold(&context, &awkward.text());
                assert_eq!(
                    terminal(&awkward, Theme::PLAIN).text(),
                    awkward.text(),
                    "{context}"
                );

                // Find a table row by its plain name, and compare the cells
                // to the right of the name column with the awkward screen.
                let rows: Vec<String> = plain.text().lines().map(str::to_owned).collect();
                let mut compared = 0;
                for (y, row) in rows.iter().enumerate() {
                    let Some(at) = row.find("lib2geom").or_else(|| row.find("potrace")) else {
                        continue;
                    };
                    // The name column, then the two columns after it.
                    let from = row[..at].width() + 22;
                    for x in from..(from + 25).min(size.0) {
                        assert_eq!(
                            plain.cell(x, y).map(|cell| cell.0.to_owned()),
                            awkward.cell(x, y).map(|cell| cell.0.to_owned()),
                            "{context}: row {y}, cell {x}"
                        );
                    }
                    compared += 1;
                }
                assert!(compared >= 2, "{context}: {compared} rows compared");
                for row in awkward.text().lines() {
                    assert!(row.width() <= size.0 - 2, "{context}: {row:?}");
                }
            }
        }
    }
}

/// The log pane is scrolled through the frame the way it is on its own.
#[test]
fn the_log_pane_scrolls_inside_the_frame() {
    use crate::frame::{Focal, Frame};
    use crate::pieces::{LogLine, LogPane};
    let pane = |top| LogPane {
        file: "/var/log/herda/pkg/20261012-210418.log".into(),
        lines: (0..60)
            .map(|index| LogLine::new(&format!("21:04:{index:02}"), "create", "placed a file"))
            .collect(),
        top,
        note: "Written to the file as it happens".into(),
        live: true,
    };
    let screen = |top| {
        let fixture = frame("working-log");
        Frame {
            focal: Focal::new().piece(pane(top)),
            ..fixture
        }
        .draw(Layout::of(100, 34), Sizing::Bold)
        .unwrap()
        .text()
    };
    let following = screen(None);
    assert!(
        following.contains("21:04:59") && !following.contains("21:04:30"),
        "{following}"
    );
    assert!(following.contains("42–60 of 60"), "{following}");
    let scrolled = screen(Some(10));
    assert!(
        scrolled.contains("21:04:10") && !scrolled.contains("21:04:59"),
        "{scrolled}"
    );
    assert!(scrolled.contains("11–29 of 60"), "{scrolled}");
}

/// kitty's own parser and screen, given the bytes of every screen, end up
/// holding the cells the buffer says: the same width for every grapheme, the
/// same block for every scaled one, and the same result after a repaint.
///
/// Needs `kitty` on the path, which the image and this crate's developers
/// have. Where there is none the test says so and checks nothing; set
/// `HERDA_UI_REQUIRE_KITTY` to make that a failure.
#[test]
fn kitty_holds_every_screen_as_it_was_drawn() {
    if crate::vendored() {
        return;
    }
    let found = std::process::Command::new("kitty")
        .arg("--version")
        .output()
        .is_ok_and(|output| output.status.success());
    if !found {
        assert!(
            std::env::var_os("HERDA_UI_REQUIRE_KITTY").is_none(),
            "kitty is required and was not found"
        );
        eprintln!("kitty was not found: no screen was replayed through it");
        return;
    }
    let directory = std::env::temp_dir().join(format!("herda-ui-kitty-{}", std::process::id()));
    std::fs::create_dir_all(&directory).unwrap();
    let mut count = 0;
    let mut case = |name: &str, steps: &[String], last: &Buffer| {
        let path = |suffix: &str| directory.join(format!("{count:03}.{suffix}"));
        std::fs::write(path("name"), name).unwrap();
        std::fs::write(path("size"), format!("{} {}", last.columns(), last.rows())).unwrap();
        std::fs::write(path("expect"), last.text()).unwrap();
        for (index, bytes) in steps.iter().enumerate() {
            std::fs::write(path(&index.to_string()), bytes).unwrap();
        }
        count += 1;
    };
    each(|state, size, sizing| {
        let buffer = draw(state, size, sizing);
        case(
            &name(state, size, sizing),
            &[buffer.paint(None, Theme::FULL)],
            &buffer,
        );
        if AWKWARD_STATES.contains(&state) {
            let awkward = frame_naming(state, AWKWARD)
                .draw(Layout::of(size.0, size.1), sizing)
                .unwrap();
            case(
                &format!("{}-names", name(state, size, sizing)),
                &[awkward.paint(None, Theme::PLAIN)],
                &awkward,
            );
        }
    });
    // A command walks the screens in order, and a repaint is all it sends.
    for size in SIZES {
        for sizing in [Sizing::Scaled, Sizing::Bold] {
            let screens: Vec<Buffer> = STATES
                .iter()
                .map(|state| draw(state, size, sizing))
                .collect();
            let mut steps = vec![screens[0].paint(None, Theme::FULL)];
            for pair in screens.windows(2) {
                steps.push(pair[1].paint(Some(&pair[0]), Theme::FULL));
            }
            steps.push(screens[1].paint(screens.last(), Theme::FULL));
            case(
                &format!("walk-{}", name("all", size, sizing)),
                &steps,
                &screens[1],
            );
        }
    }
    let output = std::process::Command::new("kitty")
        .arg("+launch")
        .arg(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/kitty_screen.py"
        ))
        .arg(&directory)
        .output()
        .unwrap();
    let said = String::from_utf8_lossy(&output.stdout);
    assert!(
        output.status.success(),
        "{said}\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    // The check is only worth its name if scaled text reached kitty as such.
    assert!(said.contains(&format!("{count} screens")), "{said}");
    assert!(!said.contains(" 0 scaled graphemes"), "{said}");
    let _ = std::fs::remove_dir_all(&directory);
}

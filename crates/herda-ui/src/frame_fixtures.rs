//! The six screens of the approved design, as fixture data.
//!
//! Each is one moment of `pkg add inkscape`, composed the way a tool composes
//! a screen: a [`Frame`] whose focal area is pieces from [`herda_ui::pieces`].
//! The strings are the design's. Nothing here is read from a machine, and
//! nothing here is what `pkg` will say — that belongs to `pkg`.
//!
//! Two more screens follow them. They are not of that design: they hold what
//! the line and the pieces draw for a tool whose line a person walks with a
//! cursor, and whose files can have moved away from the selected revision.
//!
//! This file is compiled twice: into the crate's tests, which hold every
//! screen as text, and into the gallery example, which writes them out to be
//! looked at beside the design. It uses only what a consumer of the crate can
//! reach.

use std::time::Duration;

use herda_ui::cells::Style;
use herda_ui::document::Row;
use herda_ui::frame::{Focal, Frame, Key, Rank};
use herda_ui::ink::{Ink, Wash, glyph};
use herda_ui::pieces::{
    Columns, Difference, DifferenceRow, Field, Fields, Graph, ListRow, Listing, LogLine, LogPane,
    Narrow, Plate, Prompt, Split, Statement, Table, TableColumn, Text, Tile, Tiles, Transfers,
    Version, titled, titled_ranked,
};
use herda_ui::progress::{Step, Transfer};
use herda_ui::revision_line::{Point, RevisionLine, StepState, Stretch, StretchEnd, StretchStep};

/// The screens, in the order a command passes through them.
pub const STATES: [&str; 8] = [
    "home",
    "review",
    "working-download",
    "working-log",
    "done",
    "stopped",
    "walked",
    "moved-away",
];

const LOG_FILE: &str = "/var/log/herda/pkg/20261012-210418.log";

/// One of [`STATES`], with the design's package names.
pub fn frame(state: &str) -> Frame {
    frame_naming(state, ["inkscape", "lib2geom", "potrace"])
}

/// One of [`STATES`], with these three package names in place of the design's.
pub fn frame_naming(state: &str, names: [&str; 3]) -> Frame {
    match state {
        "home" => home(),
        "review" => review(names),
        "working-download" => download(names),
        "working-log" => log(),
        "done" => done(names),
        "stopped" => stopped(),
        "walked" => walked(),
        "moved-away" => moved_away(),
        other => panic!("no screen is called {other}"),
    }
}

fn points() -> Vec<Point> {
    vec![
        Point::new("4cd890a6")
            .titled("Initial system")
            .badged("Kept"),
        Point::new("5d02b7e1").titled("Installed software"),
        Point::new("a83f19c2")
            .titled("Updated software")
            .badged("Selected"),
    ]
}

fn took(tenths: u64) -> StepState {
    StepState::Done {
        took: Duration::from_millis(tenths * 100),
    }
}

/// The line during the change: the five steps from revision `a83f19c2`.
fn change(states: [StepState; 5], end: StretchEnd) -> RevisionLine {
    RevisionLine {
        points: points(),
        selected: Some(2),
        stretch: Some(Stretch {
            origin: 2,
            steps: ["Resolve", "Download", "Verify", "Create", "Activate"]
                .iter()
                .zip(states)
                .map(|(label, state)| StretchStep::new(label, state))
                .collect(),
            end,
        }),
        ..RevisionLine::default()
    }
}

fn install(badge: &str, line: RevisionLine, focal: Focal) -> Frame {
    Frame {
        app: "pkg".into(),
        title: "Install inkscape".into(),
        badge: badge.into(),
        badge_ink: Ink::Muted,
        line,
        focal,
        advisory: String::new(),
        state: "Your current software revision is still active.".into(),
        keys: Vec::new(),
    }
}

fn home() -> Frame {
    let system = Fields {
        label_width: 18,
        rows: vec![
            Field::new("Revision", "a83f19c2").style(Style::new(Ink::Gold).bold()),
            Field::new("Packages", "412"),
            Field::new("Recorded health", "Healthy").marked(glyph::DONE, Ink::Green),
            Field::new("Updates", "None waiting"),
            Field::new("Restart", "Not needed"),
        ],
    };
    let start = Listing::new(vec![
        ListRow::key("a", "Add software", true),
        ListRow::key("r", "Remove software", false),
        ListRow::key("u", "Update everything", false),
        ListRow::key("←→", "Walk the line to compare any revision", false),
        ListRow::key("Enter", "Return to the revision under the cursor", false),
    ])
    .gutter(7);
    Frame {
        app: "pkg".into(),
        title: "Software".into(),
        badge: "SYSTEM".into(),
        badge_ink: Ink::Muted,
        line: RevisionLine {
            points: points(),
            selected: Some(2),
            stretch: None,
            open_end: "your next change".into(),
            ..RevisionLine::default()
        },
        focal: Focal::new()
            .piece(Prompt {
                hint: "Search software, or type a command: add, remove, update, restore".into(),
            })
            .gap(2)
            .piece(Columns {
                left: titled("This system", system),
                // The key row says the same in fewer words.
                right: titled_ranked(Rank::Detail, "Start", start),
                split: Split::Left(44),
                gap: 4,
                least: 28,
                narrow: Narrow::Stack,
            }),
        advisory: "Every change adds a point to the line. Nothing on it is ever overwritten."
            .into(),
        state: "No change is in progress.".into(),
        keys: vec![
            Key::new("/", "Search"),
            Key::new("a", "Add").primary(),
            Key::new("r", "Remove"),
            Key::new("u", "Update"),
            Key::new("l", "Log"),
            Key::new("?", "Help"),
            Key::new("q", "Quit"),
        ],
    }
}

/// The three packages as rows: name, version, where from, size, and why.
/// The size is a column of the review only, and so is the brighter ink on the
/// reason of the row under its cursor.
fn packages(names: [&str; 3], review: bool) -> Vec<Row> {
    let asked = if review { Ink::Text } else { Ink::Muted };
    [
        ("1.4.2-1", "extra", "32.4 MiB", "You asked for it", asked),
        ("1.4-1", "extra", "1.2 MiB", "", Ink::Muted),
        ("1.16-3", "core", "180 KiB", "", Ink::Muted),
    ]
    .iter()
    .zip(names)
    .map(|((version, from, size, why, ink), name)| {
        let why = if why.is_empty() {
            format!("Needed by {}", names[0])
        } else {
            (*why).to_owned()
        };
        let mut cells = vec![
            (name.to_owned(), Ink::Text),
            ((*version).to_owned(), Ink::Text),
            ((*from).to_owned(), Ink::repository(from)),
        ];
        if review {
            cells.push(((*size).to_owned(), Ink::Text));
        }
        cells.push((why, *ink));
        Row::new(cells).marked(glyph::ADD, Ink::Green)
    })
    .collect()
}

fn review(names: [&str; 3]) -> Frame {
    let table = Table {
        columns: vec![
            TableColumn::new("Package", 22).bold(),
            TableColumn::new("Version", 12),
            TableColumn::new("From", 9),
            TableColumn::new("Download", 12).right().keep_above(88),
            TableColumn::rest("Why").keep_above(70),
        ],
        rows: packages(names, true),
        header: true,
        cursor: Some(0),
    };
    // What is known before the packages' file lists are: where the change
    // starts, and that going back is one step.
    let tiles = Tiles(vec![
        Tile::new("Revision", "a83f19c2 → new"),
        Tile::new("Going back", "One step, any time"),
    ]);
    let mut frame = install(
        "REVIEW",
        change(
            [
                took(4),
                StepState::Held,
                StepState::Waiting,
                StepState::Waiting,
                StepState::Waiting,
            ],
            StretchEnd::Open("new revision".into()),
        ),
        Focal::new()
            .piece(Statement::new(&format!(
                "Add {} and the 2 packages it needs",
                names[0]
            )))
            .piece(Text::new(
                "33.78 MiB to download · 128 MiB installed",
                Ink::Muted,
            ))
            .gap(1)
            .piece(table)
            .gap(2)
            .ranked(Rank::Tiles, Text::label("What this touches"))
            .gap(1)
            .ranked(Rank::Tiles, tiles),
    );
    frame.state = "Nothing changes until you confirm.".into();
    frame.keys = vec![
        Key::new("Enter", "Install").primary(),
        Key::new("Esc", "Cancel"),
        Key::new("↑↓", "Inspect"),
        Key::new("l", "Log"),
        Key::new("?", "Help"),
    ];
    frame
}

/// Sixty-one samples along the design's curve, the last of them 5.72 MiB/s.
fn speeds() -> Vec<u64> {
    // The curve's height at each of its seventeen points.
    const CURVE: [u64; 17] = [
        4, 11, 17, 15, 23, 21, 28, 26, 32, 30, 27, 34, 32, 35, 31, 34, 36,
    ];
    let peak = 5_997_854u64;
    (0..=60u64)
        .map(|sample| {
            let (at, part) = ((sample * 16) / 60, (sample * 16) % 60);
            let from = CURVE[at as usize];
            let to = CURVE[(at as usize + 1).min(16)];
            (from * (60 - part) + to * part) * peak / (60 * 36)
        })
        .collect()
}

fn download(names: [&str; 3]) -> Frame {
    let transfer = |name: &str, bytes, total| Transfer {
        name: name.to_owned(),
        bytes,
        total: Some(total),
        complete: bytes == total,
        cached: false,
    };
    let mut frame = install(
        "00:08",
        change(
            [
                took(4),
                StepState::Working {
                    fraction: Some(0.67),
                    figure: "4.0s".into(),
                },
                StepState::Waiting,
                StepState::Waiting,
                StepState::Waiting,
            ],
            StretchEnd::Open("new revision".into()),
        ),
        Focal::new()
            .piece(
                Statement::new("67%")
                    .scale(3)
                    .beside("22.88 of 33.78 MiB", Ink::Text)
                    .beside("About 2 seconds left", Ink::Muted),
            )
            .gap(1)
            .ranked(
                Rank::Graph,
                Graph {
                    label: "Speed".into(),
                    figure: "5.72 MiB/s".into(),
                    samples: speeds(),
                },
            )
            .gap(1)
            .piece(Transfers(vec![
                transfer(names[0], 22_544_384, 33_973_862),
                transfer(names[1], 1_258_291, 1_258_291),
                transfer(names[2], 184_320, 184_320),
            ])),
    );
    frame.advisory =
        "Next, each archive's signature is checked in a sandbox before anything is unpacked."
            .into();
    frame.keys = vec![
        Key::new("Esc", "Cancel"),
        Key::new("l", "Log").primary(),
        Key::new("?", "Help"),
    ];
    frame
}

fn log() -> Frame {
    let inside = Listing::new(vec![
        ListRow::step("Merge into system", Step::Done, "0.2s"),
        ListRow::step("Build generation", Step::Working, "1.2s"),
        ListRow::step("Check boot tools", Step::Waiting, ""),
        ListRow::step("Refresh system caches", Step::Waiting, ""),
        ListRow::step("Record revision", Step::Waiting, ""),
    ]);
    let pane = LogPane {
        file: LOG_FILE.into(),
        lines: [
            (
                "21:04:18",
                "resolve",
                "inkscape 1.4.2-1 with 2 dependencies",
            ),
            ("21:04:19", "review", "accepted"),
            ("21:04:25", "download", "33.78 MiB in 3 archives"),
            ("21:04:26", "verify", "3 signatures good"),
            ("21:04:26", "import", "3 packages unpacked in the sandbox"),
            ("21:04:27", "create", "merged into the system's file list"),
            ("21:04:27", "create", "building generation from a83f19c2"),
        ]
        .iter()
        .map(|(time, step, text)| LogLine::new(time, step, text))
        .collect(),
        top: None,
        note: "Written to the file as it happens".into(),
        live: true,
    };
    let mut frame = install(
        "00:12",
        change(
            [
                took(4),
                took(59),
                took(10),
                StepState::Working {
                    fraction: Some(0.4),
                    figure: "1.4s".into(),
                },
                StepState::Waiting,
            ],
            StretchEnd::Open("new revision".into()),
        ),
        Focal::new()
            .piece(Statement::new("Building the new generation"))
            .piece(Text::new(
                "A snapshot of revision a83f19c2 with the new files placed in it.",
                Ink::Muted,
            ))
            .gap(1)
            .piece(Columns {
                left: titled("Inside this step", inside),
                right: Focal::new().piece(pane),
                split: Split::Left(30),
                gap: 3,
                least: 40,
                // The person asked for the log: it is the side that stays.
                narrow: Narrow::Right,
            }),
    );
    frame.keys = vec![
        Key::new("l", "Close log").primary(),
        Key::new("c", "Copy"),
        Key::new("↑↓", "Scroll"),
        Key::new("Esc", "Cancel"),
    ];
    frame
}

fn done(names: [&str; 3]) -> Frame {
    let mut line = change(
        [took(4), took(59), took(10), took(26), took(8)],
        StretchEnd::Reached(3),
    );
    line.points.push(Point::new("7be204d1"));
    line.selected = Some(3);
    let added = Table {
        columns: vec![
            TableColumn::new("Package", 22).bold(),
            TableColumn::new("Version", 12),
            TableColumn::new("From", 9),
            TableColumn::rest("Why").keep_above(70),
        ],
        rows: packages(names, false),
        header: false,
        cursor: None,
    };
    let mut frame = install(
        "DONE",
        line,
        Focal::new()
            .piece(
                Statement::new(&format!("{} is installed", names[0]))
                    .marked(glyph::DONE, Ink::Green),
            )
            .piece(Text::new("It is ready to use now.", Ink::Muted))
            .gap(1)
            .ranked(
                Rank::Tiles,
                Tiles(vec![
                    Tile::new("Now selected", "7be204d1").ink(Ink::Gold),
                    Tile::new("Previous", "a83f19c2, kept"),
                    Tile::new("Verified", "3 signatures"),
                    Tile::new("Took", "14 seconds"),
                ]),
            )
            .gap(2)
            .piece(Text::label("Added"))
            .gap(1)
            .piece(added),
    );
    frame.advisory = "To go back to a83f19c2: pkg rollback --to a83f19c2".into();
    frame.state = "The new software revision is active.".into();
    frame.keys = vec![
        Key::new("Enter", "Done").primary(),
        Key::new("l", "Log"),
        Key::new("c", "Copy summary"),
    ];
    frame
}

fn stopped() -> Frame {
    let wrong = Plate {
        wash: Wash::Alarm,
        lines: vec![
            ("Refreshing the system caches failed.".into(), Ink::Text),
            (
                "the finalizer-ca-trust-v1 generator failed:".into(),
                Ink::Danger,
            ),
            (
                "finalizer I/O: Invalid cross-device link (os error 18)".into(),
                Ink::Danger,
            ),
        ],
    };
    let undone = Listing::new(vec![
        ListRow::done("The unfinished generation").detail("was discarded"),
        ListRow::done("inkscape was taken out of").detail("your declaration again"),
    ]);
    let next = Listing::new(vec![
        ListRow::key("r", "Try the change again", true),
        ListRow::key(
            "c",
            "Copy a report: this screen and the log, ready to paste into an issue",
            false,
        ),
        ListRow::key("l", "Read the full log", false),
    ])
    .gutter(5);
    let mut frame = install(
        "STOPPED",
        change(
            [
                took(4),
                took(59),
                took(10),
                StepState::Failed { reached: Some(0.7) },
                StepState::NotStarted,
            ],
            StretchEnd::Open("not created".into()),
        ),
        Focal::new()
            .piece(Statement::new("The change was stopped").marked(glyph::FAILED, Ink::Danger))
            .piece(Text::new(
                "Your system is unchanged. Revision a83f19c2 is still the one in use.",
                Ink::Muted,
            ))
            .gap(1)
            .piece(Columns {
                left: titled("What went wrong", wrong),
                right: titled_ranked(Rank::Detail, "Already undone", undone),
                split: Split::Right(34),
                gap: 3,
                least: 40,
                narrow: Narrow::Stack,
            })
            .gap(2)
            .ranked(Rank::Detail, Text::label("What you can do"))
            .gap(1)
            .ranked(Rank::Detail, next),
    );
    frame.badge_ink = Ink::Danger;
    frame.advisory = format!("Log kept at {LOG_FILE}");
    frame.keys = vec![
        Key::new("r", "Retry").primary(),
        Key::new("c", "Copy report"),
        Key::new("l", "Log"),
        Key::new("Enter", "Close"),
    ];
    frame
}

/// A line a person walks: the cursor stands on an older revision, and the
/// focal area says what that one was.
fn walked() -> Frame {
    let revision = Fields {
        label_width: 18,
        rows: vec![
            Field::new("Revision", "5d02b7e1").style(Style::new(Ink::Text).bold()),
            Field::new("Made by", "Installed software"),
            Field::new("Packages", "409"),
            Field::new("Kept", "Until three newer revisions exist"),
        ],
    };
    let here = Listing::new(vec![
        ListRow::key("Enter", "Return to this revision", true),
        ListRow::key("←→", "Walk the line", false),
        ListRow::key("Esc", "Back to the revision in use", false),
    ])
    .gutter(7);
    Frame {
        app: "pkg".into(),
        title: "Software".into(),
        badge: "SYSTEM".into(),
        badge_ink: Ink::Muted,
        line: RevisionLine {
            points: points(),
            selected: Some(2),
            open_end: "your next change".into(),
            cursor: Some(1),
            ..RevisionLine::default()
        },
        focal: Focal::new().piece(Columns {
            left: titled("The revision under the cursor", revision),
            right: titled_ranked(Rank::Detail, "From here", here),
            split: Split::Left(52),
            gap: 4,
            least: 28,
            narrow: Narrow::Stack,
        }),
        advisory: "Returning to a revision adds a point to the line. Nothing on it is overwritten."
            .into(),
        state: "No change is in progress.".into(),
        keys: vec![
            Key::new("Enter", "Return").primary(),
            Key::new("←→", "Walk"),
            Key::new("Esc", "Back"),
            Key::new("?", "Help"),
        ],
    }
}

/// Files that are no longer what the selected revision wrote: the line ends
/// where they are now, and one of them is shown beside what was written.
fn moved_away() -> Frame {
    let changed = Table {
        columns: vec![TableColumn::rest("File").bold(), TableColumn::new("", 8)],
        rows: vec![
            Row::new(vec![
                (".config/kitty/kitty.conf".into(), Ink::Text),
                ("Edited".into(), Ink::Muted),
            ])
            .marked(glyph::CHANGE, Ink::Gold),
            Row::new(vec![
                (".config/mimeapps.list".into(), Ink::Text),
                ("Missing".into(), Ink::Muted),
            ])
            .marked(glyph::REMOVE, Ink::Danger),
        ],
        header: false,
        cursor: Some(0),
    };
    let difference = Difference {
        rows: vec![
            DifferenceRow::line(12, Version::Both, "font_size 12.0"),
            DifferenceRow::line(13, Version::Both, "cursor_trail 3"),
            DifferenceRow::line(14, Version::Before, "background_opacity 0.86"),
            DifferenceRow::line(14, Version::Now, "background_opacity 1.0"),
            DifferenceRow::line(15, Version::Both, "window_padding_width 12"),
            DifferenceRow::Gap,
            DifferenceRow::line(41, Version::Both, "map ctrl+shift+c copy_to_clipboard"),
            DifferenceRow::line(42, Version::Now, "map ctrl+shift+t new_tab_with_cwd"),
        ],
        top: 0,
    };
    Frame {
        app: "pkg".into(),
        title: "Status".into(),
        badge: "CHANGED".into(),
        badge_ink: Ink::Gold,
        line: RevisionLine {
            points: points(),
            selected: Some(2),
            open_end: "2 files moved away".into(),
            open_end_note: "your files now".into(),
            moved_away: true,
            ..RevisionLine::default()
        },
        focal: Focal::new()
            .piece(Statement::new("2 files differ from what was written").marked("!", Ink::Gold))
            .ranked(
                Rank::Detail,
                Text::new(
                    "The other 1,282 are exactly as revision a83f19c2 wrote them.",
                    Ink::Muted,
                ),
            )
            .gap(1)
            .piece(Columns {
                left: titled("Changed", changed),
                right: titled("The difference", difference),
                split: Split::Left(40),
                gap: 4,
                least: 36,
                narrow: Narrow::Stack,
            }),
        advisory: String::new(),
        state: "Nothing was changed. This is what was observed.".into(),
        keys: vec![
            Key::new("r", "Put back").primary(),
            Key::new("↑↓", "Choose"),
            Key::new("Esc", "Close"),
            Key::new("?", "Help"),
        ],
    }
}

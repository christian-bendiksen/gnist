//! The frame: one full screen that a whole command happens inside.
//!
//! Top to bottom, in one column at most [`crate::width::REFERENCE`] cells wide
//! and centred: the heading and its badge, the revision line, a hairline, the
//! focal area, an advisory on the gold rail, a hairline, one sentence saying
//! what state the machine is in, and the keys that work now. The frame is the
//! same for the whole command; what changes is the line, the focal area and
//! the three lines at the bottom.
//!
//! What is here is the shape. The focal area is the caller's: a [`Focal`] is a
//! column of pieces from [`crate::pieces`], and the words on every one of them
//! belong to the tool.
//!
//! A terminal smaller than the design gives things up in one order, and only
//! ever gives up: the speed graph, the fact tiles, the older revisions on the
//! line, then whatever the caller marked as detail. That order is the
//! [`Rung`]. Blank rows go before anything a person reads does. A frame that
//! still does not fit is not drawn at all — [`Frame::draw`] returns `None` and
//! the caller falls back to the region and the documents it had before.

use crate::cells::{Buffer, Canvas, Style};
use crate::ink::{Ink, Wash, glyph};
use crate::revision_line::RevisionLine;
use crate::scale::Sizing;
use crate::terminal::{clean, plain};
use crate::width::Layout;
use unicode_width::UnicodeWidthStr;

/// What a frame has given up to fit, in the order it gives things up.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Rung {
    /// Everything the design draws.
    Full,
    /// Without the speed graph.
    NoGraph,
    /// Without the fact tiles either.
    NoTiles,
    /// With the line shortened to the selected point and the stretch.
    ShortLine,
    /// Without anything the caller ranked as detail.
    Bare,
}

impl Rung {
    const ALL: [Self; 5] = [
        Self::Full,
        Self::NoGraph,
        Self::NoTiles,
        Self::ShortLine,
        Self::Bare,
    ];

    /// Whether a piece of this rank is still drawn.
    #[must_use]
    pub fn shows(self, rank: Rank) -> bool {
        match rank {
            Rank::Essential => true,
            Rank::Detail => self < Self::Bare,
            Rank::Tiles => self < Self::NoTiles,
            Rank::Graph => self < Self::NoGraph,
        }
    }

    /// How far blank rows may be taken out before this rung is given up.
    const fn squeezes(self) -> u8 {
        match self {
            Self::Bare => 2,
            _ => 1,
        }
    }
}

/// When a piece of the focal area is given up.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Rank {
    /// Never: without it the screen does not say what it is for.
    #[default]
    Essential,
    /// Last, when the frame would otherwise not fit: something the keys or
    /// the log also say.
    Detail,
    /// With the fact tiles.
    Tiles,
    /// First, with the speed graph.
    Graph,
}

/// How a piece is being asked to draw itself.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Fit {
    /// Whether large statements are scaled or bold here.
    pub sizing: Sizing,
    /// What the frame has given up.
    pub rung: Rung,
    /// How far blank rows are taken out: none, doubles to one, all.
    pub squeeze: u8,
}

/// The rows a piece needs.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Rows {
    /// Fewer than this and the piece cannot say what it has to.
    pub least: usize,
    /// More than this is no use to it, unless it [`Rows::grows`].
    pub most: usize,
    /// Whether it takes whatever rows are left over.
    pub grows: bool,
}

impl Rows {
    /// A piece that is always this high.
    #[must_use]
    pub const fn exactly(rows: usize) -> Self {
        Self {
            least: rows,
            most: rows,
            grows: false,
        }
    }
}

/// One piece of the focal area.
pub trait Piece {
    /// The rows this piece needs when it is `width` cells wide, or `None` when
    /// it cannot be drawn that narrow.
    fn rows(&self, width: usize, fit: Fit) -> Option<Rows>;

    /// Draw into a canvas as wide as was asked about and at least
    /// [`Rows::least`] high.
    fn draw(&self, canvas: &mut Canvas<'_>, fit: Fit);
}

enum Part {
    Gap(usize),
    Piece(Rank, Box<dyn Piece>),
}

/// A column of pieces and the gaps between them.
///
/// A piece that is given up takes the gap after it with it, so what is left
/// keeps its spacing.
#[derive(Default)]
pub struct Focal {
    parts: Vec<Part>,
}

enum Shown<'a> {
    Gap(usize),
    Piece(&'a dyn Piece),
}

impl Focal {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Add a piece that is never given up.
    #[must_use]
    pub fn piece(self, piece: impl Piece + 'static) -> Self {
        self.ranked(Rank::Essential, piece)
    }

    /// Add a piece that is given up at its rank.
    #[must_use]
    pub fn ranked(mut self, rank: Rank, piece: impl Piece + 'static) -> Self {
        self.parts.push(Part::Piece(rank, Box::new(piece)));
        self
    }

    /// Add blank rows.
    #[must_use]
    pub fn gap(mut self, rows: usize) -> Self {
        self.parts.push(Part::Gap(rows));
        self
    }

    /// Whether anything is left to draw at this rung.
    #[must_use]
    pub fn shows_anything(&self, fit: Fit) -> bool {
        self.parts
            .iter()
            .any(|part| matches!(part, Part::Piece(rank, _) if fit.rung.shows(*rank)))
    }

    fn shown(&self, fit: Fit) -> Vec<Shown<'_>> {
        let mut shown = Vec::new();
        let mut dropped = false;
        for part in &self.parts {
            match part {
                Part::Piece(rank, piece) if fit.rung.shows(*rank) => {
                    shown.push(Shown::Piece(piece.as_ref()));
                    dropped = false;
                }
                Part::Piece(..) => dropped = true,
                Part::Gap(_) if dropped => dropped = false,
                Part::Gap(rows) => {
                    if matches!(shown.last(), Some(Shown::Piece(_))) {
                        shown.push(Shown::Gap(match fit.squeeze {
                            0 => *rows,
                            1 => (*rows).min(1),
                            _ => 0,
                        }));
                    }
                }
            }
        }
        while matches!(shown.last(), Some(Shown::Gap(_))) {
            shown.pop();
        }
        shown
    }
}

impl Piece for Focal {
    fn rows(&self, width: usize, fit: Fit) -> Option<Rows> {
        let mut total = Rows::default();
        for part in self.shown(fit) {
            let rows = match part {
                Shown::Gap(rows) => Rows::exactly(rows),
                Shown::Piece(piece) => piece.rows(width, fit)?,
            };
            total.least += rows.least;
            total.most += rows.most.max(rows.least);
            total.grows |= rows.grows;
        }
        Some(total)
    }

    fn draw(&self, canvas: &mut Canvas<'_>, fit: Fit) {
        let width = canvas.width();
        let shown = self.shown(fit);
        let wanted: Vec<Rows> = shown
            .iter()
            .map(|part| match part {
                Shown::Gap(rows) => Rows::exactly(*rows),
                Shown::Piece(piece) => piece.rows(width, fit).unwrap_or_default(),
            })
            .collect();
        let mut given: Vec<usize> = wanted.iter().map(|rows| rows.least).collect();
        let mut spare = canvas.height().saturating_sub(given.iter().sum());
        // First what each piece can use, in order; then the rest to the one
        // that takes whatever is left.
        for (given, wanted) in given.iter_mut().zip(&wanted) {
            let more = wanted.most.saturating_sub(*given).min(spare);
            *given += more;
            spare -= more;
        }
        if let Some(index) = wanted.iter().position(|rows| rows.grows) {
            given[index] += spare;
        }
        let mut y = 0;
        for (part, rows) in shown.iter().zip(given) {
            if let Shown::Piece(piece) = part {
                piece.draw(&mut canvas.part(0, y, width, rows), fit);
            }
            y += rows;
        }
    }
}

/// A key that works now, and what it does.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Key {
    /// The key as it is labelled: `Enter`, `Esc`, `l`.
    pub name: String,
    /// What pressing it does, in a word or two.
    pub action: String,
    /// Whether this is the key that moves the command forward.
    pub primary: bool,
}

impl Key {
    #[must_use]
    pub fn new(name: &str, action: &str) -> Self {
        Self {
            name: name.to_owned(),
            action: action.to_owned(),
            primary: false,
        }
    }

    /// Mark this as the key that moves the command forward.
    #[must_use]
    pub fn primary(mut self) -> Self {
        self.primary = true;
        self
    }
}

/// Everything one screen of a command shows, as values.
pub struct Frame {
    /// The tool's name, in the brand ink.
    pub app: String,
    /// What the command is, constant for the whole command.
    pub title: String,
    /// The top right of the heading: `REVIEW`, a clock, `DONE`, `STOPPED`.
    pub badge: String,
    /// The badge's ink: muted, or the danger ink when the command stopped.
    pub badge_ink: Ink,
    pub line: RevisionLine,
    pub focal: Focal,
    /// One caution on the gold rail above the footer. Empty for none.
    pub advisory: String,
    /// One sentence saying what state the machine is in.
    pub state: String,
    /// The keys that work now, the most needed first: when they do not all
    /// fit, the last ones are left off.
    pub keys: Vec<Key>,
}

/// Where the rows of a frame go, for one terminal.
struct Plan {
    fit: Fit,
    /// Blank rows above the heading and below the keys.
    margin: usize,
    /// Whether there is a blank row under the heading, under the line, and
    /// under the first hairline.
    air: [bool; 3],
    advisory: Vec<String>,
}

impl Frame {
    /// The rows of the frame that are not the focal area or the advisory.
    fn chrome(margin: usize, air: usize) -> usize {
        // Heading, the line, a hairline; a hairline, the state and the keys.
        2 * margin + 1 + RevisionLine::ROWS + 1 + air + 3
    }

    fn plan(&self, layout: Layout, sizing: Sizing) -> Option<Plan> {
        if layout.columns() < crate::width::FRAME || layout.rows() < crate::width::FRAME_ROWS {
            return None;
        }
        let width = layout.content();
        // Two rows at most, and what does not fit leaves its mark.
        let advisory = crate::pieces::folded(&self.advisory, width.saturating_sub(3).max(1), 2);
        for rung in Rung::ALL {
            if !self.line.fits(width, rung >= Rung::ShortLine) {
                continue;
            }
            // Blank rows go before anything that is read: the frame is as
            // loose as it can be with every piece at the height it wants, and
            // as tight as this rung allows when no looseness gives them that.
            let mut tightest = None;
            for squeeze in 0..=rung.squeezes() {
                let fit = Fit {
                    sizing,
                    rung,
                    squeeze,
                };
                let Some(rows) = self.focal.rows(width, fit) else {
                    break;
                };
                let margin = usize::from(squeeze == 0);
                let air = if squeeze < 2 { 3 } else { 0 };
                // An unsqueezed frame keeps a blank row over its footer.
                let fixed = Self::chrome(margin, air) + margin + advisory.len();
                if fixed + rows.least > layout.rows() {
                    continue;
                }
                // Rows the pieces have no use for go back between the parts
                // of the frame, nearest the focal area first.
                let spare =
                    layout.rows() - fixed - rows.least.max(rows.most.min(layout.rows() - fixed));
                let plan = Plan {
                    fit,
                    margin,
                    air: [air + spare >= 3, air + spare >= 2, air + spare >= 1],
                    advisory: advisory.clone(),
                };
                if fixed + rows.most <= layout.rows() {
                    return Some(plan);
                }
                tightest = Some(plan);
            }
            if tightest.is_some() {
                return tightest;
            }
        }
        None
    }

    /// What the frame gives up on a terminal of this size, or `None` when it
    /// cannot be drawn there.
    #[must_use]
    pub fn rung(&self, layout: Layout, sizing: Sizing) -> Option<Rung> {
        self.plan(layout, sizing).map(|plan| plan.fit.rung)
    }

    /// Draw the frame for a terminal of this size.
    ///
    /// Pure, so a fixture and a test render the real screen without a
    /// terminal. `None` when the terminal is too small for the frame; the
    /// caller then shows what it showed before there was one.
    #[must_use]
    pub fn draw(&self, layout: Layout, sizing: Sizing) -> Option<Buffer> {
        let plan = self.plan(layout, sizing)?;
        let (columns, rows) = (layout.columns(), layout.rows());
        let width = layout.content();
        let mut buffer = Buffer::new(columns, rows);
        let mut screen = buffer.canvas();
        let mut canvas = screen.part((columns - width) / 2, 0, width, rows);

        let mut y = plan.margin;
        let app = canvas.put(0, y, &self.app, Style::new(Ink::Gold).bold());
        let badge = cells(&self.badge).min(width / 2);
        canvas.fit(
            app + 2,
            y,
            width.saturating_sub(app + 2 + badge + 2),
            &self.title,
            Style::new(Ink::Text).bold(),
        );
        canvas.end(width - badge, width, y, &self.badge, self.badge_ink);
        y += 1 + usize::from(plan.air[0]);

        self.line.draw(
            &mut canvas.part(0, y, width, RevisionLine::ROWS),
            plan.fit.rung >= Rung::ShortLine,
        );
        y += RevisionLine::ROWS + usize::from(plan.air[1]);
        canvas.repeat(0, y, width, glyph::RULE, Ink::Rule);
        y += 1 + usize::from(plan.air[2]);

        // The footer is laid from the bottom, and the focal area has the rest.
        let keys = rows - plan.margin - 1;
        let state = keys - 1;
        let hairline = state - 1;
        let advisory = hairline - plan.advisory.len();
        for (index, line) in plan.advisory.iter().enumerate() {
            canvas.put(0, advisory + index, glyph::RAIL, Ink::Gold);
            canvas.put(3, advisory + index, line, Ink::Muted);
        }
        canvas.repeat(0, hairline, width, glyph::RULE, Ink::Rule);
        canvas.put(0, state, &self.state, Ink::Muted);
        self.draw_keys(&mut canvas.part(0, keys, width, 1));

        let focal = advisory.saturating_sub(y + plan.margin);
        self.focal
            .draw(&mut canvas.part(0, y, width, focal), plan.fit);
        Some(buffer)
    }

    /// The key row: each key's name on a wash, then what it does.
    fn draw_keys(&self, canvas: &mut Canvas<'_>) {
        let width = canvas.width();
        let size = |key: &Key| cells(&key.name) + 2 + 1 + cells(&key.action);
        // Three cells between keys where they all fit, two where that is what
        // it takes, and otherwise as many keys as there is room for.
        let fits = |gap: usize, count: usize| {
            let keys = &self.keys[..count];
            keys.iter().map(size).sum::<usize>() + gap * count.saturating_sub(1) <= width
        };
        let (gap, count) = [3, 2]
            .into_iter()
            .find(|gap| fits(*gap, self.keys.len()))
            .map_or_else(
                || {
                    let count = (0..=self.keys.len())
                        .rev()
                        .find(|count| fits(2, *count))
                        .unwrap_or(0);
                    (2, count)
                },
                |gap| (gap, self.keys.len()),
            );
        let mut x = 0;
        for key in &self.keys[..count] {
            let name = cells(&key.name) + 2;
            let (wash, name_style, action) = if key.primary {
                (Wash::Accent, Style::new(Ink::Gold).bold(), Ink::Text)
            } else {
                (Wash::Key, Style::new(Ink::Text).bold(), Ink::Muted)
            };
            canvas.wash(x, 0, name, 1, wash);
            canvas.put(x + 1, 0, &key.name, name_style);
            x += name + 1;
            x += canvas.put(x, 0, &key.action, action) + gap;
        }
    }
}

/// What is shown where a frame cannot be drawn: the tool and its command,
/// the size the terminal has to have, and the sentence saying what state the
/// machine is in. A terminal made too small while a screen is up shows this
/// until it has room again.
#[must_use]
pub fn notice(layout: Layout, app: &str, title: &str, state: &str) -> Buffer {
    let mut buffer = Buffer::new(layout.columns().max(1), layout.rows().max(1));
    let mut canvas = buffer.canvas();
    let width = canvas.width();
    canvas.fit(0, 0, width, &format!("{app}  {title}"), Ink::Text);
    canvas.fit(
        0,
        1,
        width,
        &format!(
            "Enlarge the terminal to at least {} × {}.",
            crate::width::FRAME,
            crate::width::FRAME_ROWS
        ),
        Ink::Muted,
    );
    canvas.fit(0, 2, width, state, Ink::Muted);
    buffer
}

/// The cells a piece of untrusted text takes once it is sanitized.
fn cells(text: &str) -> usize {
    clean(&plain(text)).width()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::revision_line::{Point, StepState, Stretch, StretchEnd, StretchStep};

    /// A piece of a fixed height that says what it was given.
    struct Block(&'static str, usize);

    impl Piece for Block {
        fn rows(&self, _: usize, _: Fit) -> Option<Rows> {
            Some(Rows::exactly(self.1))
        }
        fn draw(&self, canvas: &mut Canvas<'_>, _: Fit) {
            for y in 0..canvas.height() {
                canvas.put(0, y, self.0, Ink::Text);
            }
        }
    }

    fn line() -> RevisionLine {
        RevisionLine {
            points: vec![
                Point::new("4cd890a6"),
                Point::new("5d02b7e1"),
                Point::new("a83f19c2"),
            ],
            selected: Some(2),
            stretch: Some(Stretch {
                origin: 2,
                steps: ["Resolve", "Download", "Verify", "Create", "Activate"]
                    .iter()
                    .map(|name| StretchStep::new(name, StepState::Waiting))
                    .collect(),
                end: StretchEnd::Open("new revision".into()),
            }),
            ..RevisionLine::default()
        }
    }

    fn frame(focal: Focal) -> Frame {
        Frame {
            app: "pkg".into(),
            title: "Install inkscape".into(),
            badge: "REVIEW".into(),
            badge_ink: Ink::Muted,
            line: line(),
            focal,
            advisory: "Nothing on the line is ever overwritten.".into(),
            state: "Nothing changes until you confirm.".into(),
            keys: vec![
                Key::new("Enter", "Install").primary(),
                Key::new("Esc", "Cancel"),
                Key::new("↑↓", "Inspect"),
                Key::new("l", "Log"),
                Key::new("?", "Help"),
            ],
        }
    }

    fn ranked() -> Focal {
        Focal::new()
            .piece(Block("statement", 2))
            .gap(1)
            .ranked(Rank::Graph, Block("graph", 3))
            .gap(1)
            .ranked(Rank::Tiles, Block("tiles", 2))
            .gap(2)
            .piece(Block("table", 4))
            .gap(2)
            .ranked(Rank::Detail, Block("detail", 3))
    }

    #[test]
    fn the_frame_is_laid_out_top_to_bottom_as_the_design_draws_it() {
        let buffer = frame(Focal::new().piece(Block("focal", 2)))
            .draw(Layout::of(100, 34), Sizing::Bold)
            .unwrap();
        let text = buffer.text();
        let rows: Vec<&str> = text.lines().collect();
        assert_eq!(rows.len(), 34);
        assert_eq!(rows[0], "");
        assert!(
            rows[1].starts_with("  pkg  Install inkscape"),
            "{}",
            rows[1]
        );
        assert!(rows[1].ends_with("REVIEW"), "{}", rows[1]);
        assert_eq!(rows[1].width(), 98, "the badge ends at the content edge");
        assert_eq!(rows[2], "");
        assert!(
            rows[3].starts_with("  4cd890a6  5d02b7e1  a83f19c2"),
            "{}",
            rows[3]
        );
        assert!(
            rows[4].starts_with("  ○─────────○─────────●╌"),
            "{}",
            rows[4]
        );
        assert_eq!(rows[7], "");
        assert_eq!(rows[8], format!("  {}", "─".repeat(96)));
        assert_eq!(rows[9], "");
        assert_eq!(&rows[10..12], ["  focal", "  focal"]);
        assert_eq!(rows[28], "");
        assert_eq!(rows[29], "  │  Nothing on the line is ever overwritten.");
        assert_eq!(rows[30], format!("  {}", "─".repeat(96)));
        assert_eq!(rows[31], "  Nothing changes until you confirm.");
        assert_eq!(
            rows[32],
            "   Enter  Install    Esc  Cancel    ↑↓  Inspect    l  Log    ?  Help"
        );
        assert_eq!(rows[33], "");
        // The forward key is on the accent wash in gold, the others on the
        // key wash, and each action is in its own ink.
        let (name, style) = buffer.cell(3, 32).unwrap();
        assert_eq!(
            (name, style.ink, style.bold, style.wash),
            ("E", Ink::Gold, true, Wash::Accent)
        );
        assert_eq!(buffer.cell(21, 32).unwrap().1.wash, Wash::Key);
        assert_eq!(buffer.cell(10, 32).unwrap().1.wash, Wash::None);
    }

    #[test]
    fn a_wider_terminal_centres_the_same_column() {
        let wide = frame(Focal::new().piece(Block("focal", 1)))
            .draw(Layout::of(140, 34), Sizing::Bold)
            .unwrap()
            .text();
        let row = wide.lines().nth(8).unwrap();
        assert_eq!(row, format!("{}{}", " ".repeat(22), "─".repeat(96)));
    }

    /// The ladder: each thing is given up in its turn and never comes back as
    /// the terminal shrinks.
    #[test]
    fn things_are_given_up_in_one_order_and_never_come_back() {
        let frame = frame(ranked());
        let has = |rows: usize, columns: usize| {
            let rung = frame.rung(Layout::of(columns, rows), Sizing::Bold);
            let text = frame
                .draw(Layout::of(columns, rows), Sizing::Bold)
                .map(|buffer| buffer.text())
                .unwrap_or_default();
            (
                rung,
                ["graph", "tiles", "5d02b7e1", "detail", "table"].map(|word| text.contains(word)),
            )
        };
        assert_eq!(has(40, 100), (Some(Rung::Full), [true; 5]));
        assert_eq!(has(34, 100).0, Some(Rung::Full));
        let mut previous = [true; 5];
        let mut order = Vec::new();
        for rows in (10..=40).rev() {
            let (rung, now) = has(rows, 100);
            for (index, (was, is)) in previous.iter().zip(now).enumerate() {
                assert!(*was || !is, "{rows} rows: {index} came back");
                if *was && !is {
                    order.push(index);
                }
            }
            assert_eq!(rung.is_some(), now[4], "{rows} rows");
            previous = now;
        }
        assert_eq!(order, [0, 1, 2, 3, 4], "graph, tiles, line, detail, frame");

        // Width alone shortens the line, and takes the graph and tiles first.
        assert_eq!(
            has(40, 80),
            (Some(Rung::ShortLine), [false, false, false, true, true])
        );
        assert_eq!(has(40, 59).0, None);
        assert_eq!(has(19, 100).0, None);
    }

    #[test]
    fn blank_rows_go_before_anything_that_is_read() {
        let frame = frame(
            Focal::new()
                .piece(Block("statement", 2))
                .gap(2)
                .piece(Block("table", 4)),
        );
        let text = |rows| {
            frame
                .draw(Layout::of(100, rows), Sizing::Bold)
                .unwrap()
                .text()
        };
        let gap = |text: &str| {
            let rows: Vec<&str> = text.lines().collect();
            let last = rows
                .iter()
                .rposition(|row| row.contains("statement"))
                .unwrap();
            let first = rows.iter().position(|row| row.contains("table")).unwrap();
            first - last - 1
        };
        // Sixteen rows of frame and advisory, eight of focal area.
        assert_eq!(gap(&text(24)), 2);
        assert!(text(24).starts_with('\n'), "a margin");
        assert_eq!(gap(&text(23)), 1);
        assert!(text(23).starts_with("  pkg"), "no margin");
        assert_eq!(
            frame.rung(Layout::of(100, 23), Sizing::Bold),
            Some(Rung::Full)
        );
        assert_eq!(gap(&text(20)), 1);
        assert_eq!(
            frame.rung(Layout::of(100, 20), Sizing::Bold),
            Some(Rung::Full)
        );
    }

    #[test]
    fn a_piece_that_grows_takes_the_rows_that_are_left() {
        struct Pane;
        impl Piece for Pane {
            fn rows(&self, _: usize, _: Fit) -> Option<Rows> {
                Some(Rows {
                    least: 3,
                    most: 3,
                    grows: true,
                })
            }
            fn draw(&self, canvas: &mut Canvas<'_>, _: Fit) {
                for y in 0..canvas.height() {
                    canvas.put(0, y, "pane", Ink::Text);
                }
            }
        }
        let frame = frame(Focal::new().piece(Block("statement", 1)).gap(1).piece(Pane));
        let text = frame
            .draw(Layout::of(100, 34), Sizing::Bold)
            .unwrap()
            .text();
        // Sixteen rows of frame and advisory, and two above the pane.
        assert_eq!(text.matches("pane").count(), 34 - 16 - 2);
    }

    #[test]
    fn keys_that_do_not_fit_are_left_off_from_the_end() {
        let frame = frame(Focal::new().piece(Block("x", 1)));
        let keys = |columns| {
            let text = frame
                .draw(Layout::of(columns, 30), Sizing::Bold)
                .unwrap()
                .text();
            text.lines()
                .rev()
                .find(|row| row.contains("Enter"))
                .unwrap()
                .to_owned()
        };
        assert_eq!(
            keys(72),
            "   Enter  Install    Esc  Cancel    ↑↓  Inspect    l  Log    ?  Help"
        );
        assert_eq!(
            keys(68),
            "   Enter  Install   Esc  Cancel   ↑↓  Inspect   l  Log   ?  Help"
        );
        assert_eq!(
            keys(60),
            "   Enter  Install   Esc  Cancel   ↑↓  Inspect   l  Log"
        );
    }

    #[test]
    fn untrusted_text_stays_inside_the_frame() {
        let mut frame = frame(Focal::new().piece(Block("x", 1)));
        frame.title =
            "Install 界面\x1b[2J and a very long list of other packages besides".repeat(3);
        frame.badge = "STOPPED\r\n".into();
        frame.badge_ink = Ink::Danger;
        frame.state = "界".repeat(80);
        frame.advisory = "/var/log/herda/pkg/".to_owned() + &"a".repeat(300);
        for columns in [60, 61, 80, 100, 101] {
            let buffer = frame.draw(Layout::of(columns, 30), Sizing::Bold).unwrap();
            for row in buffer.text().lines() {
                assert!(row.width() <= columns - 2, "{columns}: {row:?}");
            }
            assert!(
                !buffer
                    .paint(None, crate::terminal::Theme::FULL)
                    .contains("\x1b[2J\x1b[2J")
            );
            assert!(buffer.text().contains("STOPPED"));
            // An advisory longer than its two rows says that it was cut.
            let text = buffer.text();
            let rail: Vec<&str> = text
                .lines()
                .filter(|row| row.trim_start().starts_with(glyph::RAIL))
                .collect();
            assert_eq!(rail.len(), 2, "{columns}: {rail:?}");
            assert!(rail[1].trim_end().ends_with('…'), "{columns}: {rail:?}");
        }
    }
}

#[cfg(test)]
#[path = "frame_tests.rs"]
mod held;

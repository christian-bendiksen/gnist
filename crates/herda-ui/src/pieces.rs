//! The pieces a focal area is built from.
//!
//! Each is a value that knows how many rows it needs at a width and draws
//! itself into the canvas it is given: a large statement, a line of text, fact
//! tiles, the speed graph, the transfer table, a table with a cursor row, a
//! marked list, a plate of text, the log pane, the difference between two
//! versions of a text, and two columns side by side.
//! None of them knows what command it is part of. The words are the caller's.
//!
//! Every state a piece shows has a character and a word as well as an ink, so
//! a piece reads the same with colour off.

use unicode_width::UnicodeWidthStr;

use crate::cells::{Canvas, Style};
use crate::document::{Align, Row};
use crate::frame::{Fit, Focal, Piece, Rank, Rows};
use crate::ink::{Ink, Wash, glyph};
use crate::progress::{Step, Transfer, moved};
use crate::scale::Sizing;
use crate::terminal::{clean, clip, fold, plain};

/// The cells a piece of untrusted text takes once it is sanitized.
fn cells(text: &str) -> usize {
    clean(&plain(text)).width()
}

/// Text folded to a width and held to a number of lines; the last line is
/// clipped with an ellipsis when there was more.
pub(crate) fn folded(text: &str, width: usize, limit: usize) -> Vec<String> {
    if text.trim().is_empty() || width == 0 || limit == 0 {
        return Vec::new();
    }
    let mut lines = fold(text, width);
    if lines.len() > limit {
        let rest = lines.split_off(limit - 1).join(" ");
        // One cell too wide, so the clip always leaves its mark.
        lines.push(clip(&format!("{rest} …"), width));
    }
    lines
}

/// The one thing a screen says, larger than everything around it.
///
/// Scaled where the terminal scales text and the statement fits at that size;
/// ordinary bold text everywhere else, folded if it is long.
#[derive(Clone, Debug, Default)]
pub struct Statement {
    pub text: String,
    /// A mark before the text: the check of a change that finished, the cross
    /// of one that stopped.
    pub mark: Option<(String, Ink)>,
    /// How many times the ordinary size, where the terminal scales text.
    pub scale: u8,
    /// Short lines that stand beside the statement, at its foot.
    pub beside: Vec<(String, Ink)>,
}

enum StatementForm {
    Scaled { scale: u8, right: bool },
    Bold { lines: Vec<String>, right: bool },
}

impl Statement {
    /// A statement at twice the ordinary size.
    #[must_use]
    pub fn new(text: &str) -> Self {
        Self {
            text: text.to_owned(),
            mark: None,
            scale: 2,
            beside: Vec::new(),
        }
    }

    #[must_use]
    pub fn scale(mut self, scale: u8) -> Self {
        self.scale = scale.clamp(1, 7);
        self
    }

    #[must_use]
    pub fn marked(mut self, mark: &str, ink: Ink) -> Self {
        self.mark = Some((mark.to_owned(), ink));
        self
    }

    #[must_use]
    pub fn beside(mut self, text: &str, ink: Ink) -> Self {
        self.beside.push((text.to_owned(), ink));
        self
    }

    fn beside_width(&self) -> usize {
        self.beside
            .iter()
            .map(|(text, _)| cells(text))
            .max()
            .unwrap_or(0)
    }

    /// The cells the mark and the statement take at a scale.
    fn head(&self, scale: usize) -> (usize, usize) {
        let mark = self.mark.as_ref().map_or(0, |(mark, _)| {
            cells(mark) * scale + if scale > 1 { 2 } else { 1 }
        });
        (mark, mark + cells(&self.text) * scale)
    }

    fn form(&self, width: usize, fit: Fit) -> StatementForm {
        let beside = self.beside_width();
        let beside_fits = |head: usize| self.beside.is_empty() || head + 4 + beside <= width;
        if fit.sizing == Sizing::Scaled {
            for scale in (2..=self.scale.max(1)).rev() {
                let (_, head) = self.head(usize::from(scale));
                if head <= width {
                    return StatementForm::Scaled {
                        scale,
                        right: beside_fits(head),
                    };
                }
            }
        }
        let (mark, head) = self.head(1);
        let lines = folded(&self.text, width.saturating_sub(mark).max(1), 3);
        let right = lines.len() <= 1 && beside_fits(head);
        StatementForm::Bold { lines, right }
    }
}

impl Piece for Statement {
    fn rows(&self, width: usize, fit: Fit) -> Option<Rows> {
        let beside = self.beside.len();
        Some(Rows::exactly(match self.form(width, fit) {
            StatementForm::Scaled { scale, right: true } => usize::from(scale).max(beside),
            StatementForm::Scaled {
                scale,
                right: false,
            } => usize::from(scale) + beside,
            StatementForm::Bold { lines, right: true } => lines.len().max(beside).max(1),
            StatementForm::Bold {
                lines,
                right: false,
            } => lines.len().max(1) + beside,
        }))
    }

    fn draw(&self, canvas: &mut Canvas<'_>, fit: Fit) {
        let width = canvas.width();
        let bold = Style::new(Ink::Text).bold();
        let (below, right) = match self.form(width, fit) {
            StatementForm::Scaled { scale, right } => {
                let (mark, head) = self.head(usize::from(scale));
                if let Some((glyph, ink)) = &self.mark {
                    canvas.scaled(0, 0, glyph, Style::new(*ink).bold(), scale);
                }
                canvas.scaled(mark, 0, &self.text, bold, scale);
                (usize::from(scale), right.then_some(head + 4))
            }
            StatementForm::Bold { lines, right } => {
                let (mark, head) = self.head(1);
                if let Some((glyph, ink)) = &self.mark {
                    canvas.put(0, 0, glyph, Style::new(*ink).bold());
                }
                for (y, line) in lines.iter().enumerate() {
                    canvas.put(mark, y, line, bold);
                }
                (lines.len().max(1), right.then_some(head + 4))
            }
        };
        match right {
            // Beside the statement, standing on its last row.
            Some(x) => {
                let top = below.max(self.beside.len()) - self.beside.len();
                for (index, (text, ink)) in self.beside.iter().enumerate() {
                    canvas.put(x, top + index, text, *ink);
                }
            }
            None => {
                for (index, (text, ink)) in self.beside.iter().enumerate() {
                    canvas.put(0, below + index, text, *ink);
                }
            }
        }
    }
}

/// A line or two of ordinary text.
#[derive(Clone, Debug, Default)]
pub struct Text {
    pub text: String,
    pub ink: Ink,
    pub bold: bool,
    /// The most rows it may fold onto.
    pub limit: usize,
}

impl Text {
    /// Text that folds onto at most two rows.
    #[must_use]
    pub fn new(text: &str, ink: Ink) -> Self {
        Self {
            text: text.to_owned(),
            ink,
            bold: false,
            limit: 2,
        }
    }

    /// A faint heading over a section, as the design writes them: capitals,
    /// one row.
    #[must_use]
    pub fn label(text: &str) -> Self {
        Self {
            text: text.to_uppercase(),
            ink: Ink::Faint,
            bold: false,
            limit: 1,
        }
    }

    #[must_use]
    pub fn limit(mut self, rows: usize) -> Self {
        self.limit = rows;
        self
    }
}

impl Piece for Text {
    fn rows(&self, width: usize, _: Fit) -> Option<Rows> {
        Some(Rows::exactly(folded(&self.text, width, self.limit).len()))
    }

    fn draw(&self, canvas: &mut Canvas<'_>, _: Fit) {
        let style = Style {
            ink: self.ink,
            bold: self.bold,
            wash: Wash::None,
        };
        for (y, line) in folded(&self.text, canvas.width(), self.limit)
            .iter()
            .enumerate()
        {
            canvas.put(0, y, line, style);
        }
    }
}

/// One fact on a plate: a faint label over a value.
#[derive(Clone, Debug, Default)]
pub struct Tile {
    pub label: String,
    pub value: String,
    /// The value's ink: text, or gold for the revision the machine now runs.
    pub ink: Ink,
}

impl Tile {
    #[must_use]
    pub fn new(label: &str, value: &str) -> Self {
        Self {
            label: label.to_uppercase(),
            value: value.to_owned(),
            ink: Ink::Text,
        }
    }

    #[must_use]
    pub fn ink(mut self, ink: Ink) -> Self {
        self.ink = ink;
        self
    }
}

/// A row of fact tiles.
///
/// Four to a row at most, each a quarter of the width, as the design draws
/// them. A width at which a value would be clipped is one this piece cannot be
/// drawn at, and the frame gives the tiles up rather than squeezing them.
#[derive(Clone, Debug, Default)]
pub struct Tiles(pub Vec<Tile>);

impl Tiles {
    /// Two cells between tiles and two of padding inside each.
    fn tile_width(&self, width: usize) -> usize {
        let across = self.0.len().max(4);
        (width + 2) / across - 2
    }
}

impl Piece for Tiles {
    fn rows(&self, width: usize, _: Fit) -> Option<Rows> {
        let widest = self
            .0
            .iter()
            .map(|tile| cells(&tile.label).max(cells(&tile.value)))
            .max()?;
        (width >= 8 && self.tile_width(width) >= widest + 4).then_some(Rows::exactly(2))
    }

    fn draw(&self, canvas: &mut Canvas<'_>, _: Fit) {
        let width = self.tile_width(canvas.width());
        for (index, tile) in self.0.iter().enumerate() {
            let x = index * (width + 2);
            canvas.wash(x, 0, width, 2, Wash::Plate);
            canvas.fit(x + 2, 0, width - 4, &tile.label, Ink::Faint);
            canvas.fit(
                x + 2,
                1,
                width - 4,
                &tile.value,
                Style::new(tile.ink).bold(),
            );
        }
    }
}

/// The speed graph: a label, the current figure, and two rows of columns.
///
/// One column a sample, oldest at the left, the newest that fit. A column is
/// sixteen steps high over its two rows; the tallest sample shown reaches the
/// top. The top of each column is gold and what is under it is the rule ink,
/// so the shape is the same graph with colour off.
#[derive(Clone, Debug, Default)]
pub struct Graph {
    pub label: String,
    /// The figure at the right of the label row: the speed now.
    pub figure: String,
    pub samples: Vec<u64>,
}

impl Piece for Graph {
    fn rows(&self, _: usize, _: Fit) -> Option<Rows> {
        Some(Rows::exactly(3))
    }

    fn draw(&self, canvas: &mut Canvas<'_>, _: Fit) {
        let width = canvas.width();
        canvas.put(0, 0, &self.label.to_uppercase(), Ink::Faint);
        canvas.end(cells(&self.label) + 2, width, 0, &self.figure, Ink::Text);
        let shown = &self.samples[self.samples.len().saturating_sub(width)..];
        let peak = shown.iter().copied().max().unwrap_or(0).max(1);
        for x in 0..width {
            let Some(sample) = shown.get(x) else {
                // No sample yet: the floor the graph stands on.
                canvas.repeat(x, 2, 1, glyph::LEVELS[0], Ink::Rule);
                continue;
            };
            // Widened before multiplying: a rate near the top of its type
            // would otherwise wrap into a short column.
            let steps =
                ((u128::from(*sample) * 16).div_ceil(u128::from(peak)) as usize).clamp(1, 16);
            if steps > 8 {
                canvas.repeat(x, 1, 1, glyph::LEVELS[steps - 9], Ink::Gold);
                canvas.repeat(x, 2, 1, glyph::LEVELS[7], Ink::Rule);
            } else {
                canvas.repeat(x, 2, 1, glyph::LEVELS[steps - 1], Ink::Gold);
            }
        }
    }
}

/// A thin bar: the done part heavy, the rest a hairline.
///
/// Half a cell at a time. `None` is an amount nobody knows, which is drawn as
/// nothing done rather than as a guess.
fn thin_bar(
    canvas: &mut Canvas<'_>,
    x: usize,
    y: usize,
    width: usize,
    done: Option<f64>,
    ink: Ink,
) {
    let halves = done.map_or(0, |done| {
        (done.clamp(0.0, 1.0) * (width * 2) as f64) as usize
    });
    let full = halves / 2;
    canvas.repeat(x, y, full, glyph::LINE_DONE, ink);
    let mut drawn = full;
    if halves % 2 == 1 {
        canvas.repeat(x + full, y, 1, glyph::LINE_HEAD, ink);
        drawn += 1;
    }
    canvas.repeat(x + drawn, y, width - drawn, glyph::RULE, Ink::Rule);
}

/// The transfer table: a thin bar, the amounts and a status word per package.
///
/// The rules are the staged region's: what is still moving sorts first, and
/// when there are more packages than rows the last row says how many more.
#[derive(Clone, Debug, Default)]
pub struct Transfers(pub Vec<Transfer>);

impl Transfers {
    /// What is still moving first, then by name.
    fn sorted(&self) -> Vec<&Transfer> {
        let mut transfers: Vec<&Transfer> = self.0.iter().collect();
        transfers.sort_by(|a, b| (a.complete, &a.name).cmp(&(b.complete, &b.name)));
        transfers
    }
}

impl Piece for Transfers {
    fn rows(&self, _: usize, _: Fit) -> Option<Rows> {
        let count = self.0.len();
        Some(Rows {
            least: 1 + count.min(3) + usize::from(count > 3),
            most: 1 + count,
            grows: false,
        })
    }

    fn draw(&self, canvas: &mut Canvas<'_>, _: Fit) {
        let width = canvas.width();
        // The amounts are the column a narrow terminal does without: the bar
        // and the status word say the same thing less exactly.
        let amounts = if width >= crate::width::SIZE_COLUMN {
            18
        } else {
            0
        };
        let status = 10;
        let name = 22.min(width.saturating_sub(status + 2 + 8 + 2));
        let bar_x = name + 2;
        let status_x = width.saturating_sub(status);
        let amounts_x = status_x.saturating_sub(amounts + 2);
        let bar = if amounts > 0 { amounts_x } else { status_x }.saturating_sub(bar_x + 2);
        canvas.fit(0, 0, name, "PACKAGE", Ink::Faint);
        canvas.fit(bar_x, 0, bar, "PROGRESS", Ink::Faint);
        if amounts > 0 {
            canvas.end(amounts_x, amounts_x + amounts, 0, "TRANSFER", Ink::Faint);
        }
        canvas.put(status_x, 0, "STATUS", Ink::Faint);

        let transfers = self.sorted();
        let room = canvas.height().saturating_sub(1);
        let shown = if transfers.len() > room {
            room.saturating_sub(1)
        } else {
            transfers.len()
        };
        for (index, transfer) in transfers.iter().take(shown).enumerate() {
            let y = index + 1;
            canvas.fit(0, y, name, &transfer.name, Style::new(Ink::Text).bold());
            let done = match transfer.total {
                _ if transfer.complete => Some(1.0),
                Some(total) if total > 0 => Some(transfer.bytes as f64 / total as f64),
                _ => None,
            };
            let ink = if transfer.complete {
                Ink::Green
            } else {
                Ink::Gold
            };
            thin_bar(canvas, bar_x, y, bar, done, ink);
            if amounts > 0 {
                canvas.end(
                    amounts_x,
                    amounts_x + amounts,
                    y,
                    &moved(transfer),
                    Ink::Text,
                );
            }
            canvas.put(status_x, y, transfer.status(), transfer.ink());
        }
        if transfers.len() > shown && room > 0 {
            canvas.put(
                0,
                shown + 1,
                &format!("… {} more packages", transfers.len() - shown),
                Ink::Muted,
            );
        }
    }
}

/// One column of a [`Table`].
#[derive(Clone, Debug, Default)]
pub struct TableColumn {
    /// The heading, in faint capitals.
    pub label: String,
    /// The cells it takes. Zero for the one column that takes what is left.
    pub width: usize,
    pub align: Align,
    /// The narrowest table that still shows this column, in cells.
    pub keep_above: usize,
    /// Whether its cells are bold: the name a reader scans down.
    pub bold: bool,
}

impl TableColumn {
    /// A column this many cells wide.
    #[must_use]
    pub fn new(label: &str, width: usize) -> Self {
        Self {
            label: label.to_uppercase(),
            width,
            ..Self::default()
        }
    }

    /// The column that takes what the others leave.
    #[must_use]
    pub fn rest(label: &str) -> Self {
        Self::new(label, 0)
    }

    #[must_use]
    pub fn right(mut self) -> Self {
        self.align = Align::Right;
        self
    }

    #[must_use]
    pub fn keep_above(mut self, cells: usize) -> Self {
        self.keep_above = cells;
        self
    }

    #[must_use]
    pub fn bold(mut self) -> Self {
        self.bold = true;
        self
    }
}

/// A table of marked rows, one of which may be under the cursor.
///
/// The cursor row is a wash and a `›` in the gutter, so it is findable with
/// colour off. When there are more rows than room the table keeps the cursor
/// in view and its last row says how many are not shown.
#[derive(Clone, Debug, Default)]
pub struct Table {
    pub columns: Vec<TableColumn>,
    pub rows: Vec<Row>,
    /// Whether the column headings are drawn.
    pub header: bool,
    pub cursor: Option<usize>,
}

impl Table {
    /// Each kept column's index, first cell and width.
    fn layout(&self, width: usize) -> Vec<(usize, usize, usize)> {
        let kept: Vec<usize> = (0..self.columns.len())
            .filter(|index| width >= self.columns[*index].keep_above)
            .collect();
        // Three cells of gutter for the cursor and the row's mark, and two
        // between columns.
        let fixed: usize = kept
            .iter()
            .map(|index| self.columns[*index].width + 2)
            .sum();
        let rest = width.saturating_sub(3 + fixed);
        let mut x = 3;
        kept.into_iter()
            .map(|index| {
                let size = match self.columns[index].width {
                    0 => rest,
                    size => size,
                };
                x += 2 + size;
                (index, x - size, size)
            })
            .collect()
    }
}

impl Piece for Table {
    fn rows(&self, _: usize, _: Fit) -> Option<Rows> {
        let head = usize::from(self.header);
        let count = self.rows.len();
        Some(Rows {
            least: head + count.min(3) + usize::from(count > 3),
            most: head + count,
            grows: false,
        })
    }

    fn draw(&self, canvas: &mut Canvas<'_>, _: Fit) {
        let width = canvas.width();
        let layout = self.layout(width);
        let head = usize::from(self.header);
        if self.header {
            for &(index, x, size) in &layout {
                let column = &self.columns[index];
                match column.align {
                    Align::Left => canvas.fit(x, 0, size, &column.label, Ink::Faint),
                    Align::Right => canvas.end(x, x + size, 0, &column.label, Ink::Faint),
                };
            }
        }
        let room = canvas.height().saturating_sub(head);
        let count = self.rows.len();
        let shown = if count > room {
            room.saturating_sub(1)
        } else {
            count
        };
        let first = self
            .cursor
            .map_or(0, |cursor| (cursor + 1).saturating_sub(shown))
            .min(count - shown);
        for (slot, (index, row)) in self
            .rows
            .iter()
            .enumerate()
            .skip(first)
            .take(shown)
            .enumerate()
        {
            let y = head + slot;
            if self.cursor == Some(index) {
                canvas.wash(0, y, width, 1, Wash::Cursor);
                canvas.put(0, y, glyph::ROW, Style::new(Ink::Gold).bold());
            }
            if let Some((mark, ink)) = &row.marker {
                canvas.fit(1, y, 2, mark, *ink);
            }
            for &(column, x, size) in &layout {
                let Some((text, ink)) = row.cells.get(column) else {
                    continue;
                };
                let style = Style {
                    ink: *ink,
                    bold: self.columns[column].bold,
                    wash: Wash::None,
                };
                match self.columns[column].align {
                    Align::Left => canvas.fit(x, y, size, text, style),
                    Align::Right => canvas.end(x, x + size, y, text, style),
                };
            }
        }
        if count > shown && room > 0 {
            canvas.put(
                5.min(width),
                head + shown,
                &format!("… {} more", count - shown),
                Ink::Muted,
            );
        }
    }
}

/// One row of a [`Listing`].
#[derive(Clone, Debug, Default)]
pub struct ListRow {
    /// What stands in the gutter: a state's glyph, or a key.
    pub mark: String,
    pub mark_style: Style,
    pub text: String,
    pub style: Style,
    /// A figure at the right edge: how long a step took.
    pub tail: String,
    pub tail_ink: Ink,
    /// A muted continuation under the text: what happened to the thing named.
    pub detail: String,
}

impl ListRow {
    /// One measured step inside the step the command is on.
    #[must_use]
    pub fn step(label: &str, state: Step, took: &str) -> Self {
        let (style, tail_ink) = match state {
            Step::Done => (Style::new(Ink::Muted), Ink::Faint),
            Step::Working => (Style::new(Ink::Text).bold(), Ink::Muted),
            Step::Waiting => (Style::new(Ink::Muted), Ink::Faint),
            Step::Failed => (Style::new(Ink::Danger).bold(), Ink::Danger),
        };
        Self {
            mark: state.glyph().to_owned(),
            mark_style: Style {
                ink: state.ink(),
                bold: matches!(state, Step::Failed),
                wash: Wash::None,
            },
            text: label.to_owned(),
            style,
            tail: took.to_owned(),
            tail_ink,
            detail: String::new(),
        }
    }

    /// Something that was done, under the check.
    #[must_use]
    pub fn done(text: &str) -> Self {
        Self {
            mark: glyph::DONE.to_owned(),
            mark_style: Style::new(Ink::Green),
            text: text.to_owned(),
            style: Style::new(Ink::Text),
            ..Self::default()
        }
    }

    /// Something a key does. The key that moves things forward is gold.
    #[must_use]
    pub fn key(name: &str, text: &str, primary: bool) -> Self {
        Self {
            mark: name.to_owned(),
            mark_style: Style::new(if primary { Ink::Gold } else { Ink::Text }).bold(),
            text: text.to_owned(),
            style: Style::new(Ink::Text),
            ..Self::default()
        }
    }

    #[must_use]
    pub fn detail(mut self, text: &str) -> Self {
        self.detail = text.to_owned();
        self
    }
}

/// A list of marked rows: the steps inside a step, what was undone, what a
/// person can do next.
#[derive(Clone, Debug)]
pub struct Listing {
    pub rows: Vec<ListRow>,
    /// The cells the mark column takes.
    pub gutter: usize,
}

impl Listing {
    /// A list whose marks are single glyphs.
    #[must_use]
    pub fn new(rows: Vec<ListRow>) -> Self {
        Self { rows, gutter: 3 }
    }

    #[must_use]
    pub fn gutter(mut self, cells: usize) -> Self {
        self.gutter = cells;
        self
    }

    /// Each row's folded text and folded detail at a width.
    fn folded(&self, width: usize) -> Vec<(Vec<String>, Vec<String>)> {
        self.rows
            .iter()
            .map(|row| {
                let tail = match cells(&row.tail) {
                    0 => 0,
                    tail => tail + 1,
                };
                let room = width.saturating_sub(self.gutter);
                let mut text = folded(&row.text, room.saturating_sub(tail).max(1), 3);
                if text.is_empty() {
                    text.push(String::new());
                }
                (text, folded(&row.detail, room.max(1), 3))
            })
            .collect()
    }
}

impl Piece for Listing {
    fn rows(&self, width: usize, _: Fit) -> Option<Rows> {
        Some(Rows::exactly(
            self.folded(width)
                .iter()
                .map(|(text, detail)| text.len() + detail.len())
                .sum(),
        ))
    }

    fn draw(&self, canvas: &mut Canvas<'_>, _: Fit) {
        let width = canvas.width();
        let mut y = 0;
        for (row, (text, detail)) in self.rows.iter().zip(self.folded(width)) {
            canvas.fit(
                0,
                y,
                self.gutter.saturating_sub(1),
                &row.mark,
                row.mark_style,
            );
            canvas.end(self.gutter, width, y, &row.tail, row.tail_ink);
            for line in &text {
                canvas.put(self.gutter, y, line, row.style);
                y += 1;
            }
            for line in &detail {
                canvas.put(self.gutter, y, line, Ink::Muted);
                y += 1;
            }
        }
    }
}

/// Labelled values, one to a row.
#[derive(Clone, Debug, Default)]
pub struct Fields {
    /// The cells the label column takes.
    pub label_width: usize,
    pub rows: Vec<Field>,
}

/// One labelled value.
#[derive(Clone, Debug, Default)]
pub struct Field {
    pub label: String,
    /// A state's glyph before the value, so the state is not told by ink.
    pub mark: Option<(String, Ink)>,
    pub value: String,
    pub style: Style,
}

impl Field {
    #[must_use]
    pub fn new(label: &str, value: &str) -> Self {
        Self {
            label: label.to_owned(),
            mark: None,
            value: value.to_owned(),
            style: Style::new(Ink::Text),
        }
    }

    #[must_use]
    pub fn marked(mut self, mark: &str, ink: Ink) -> Self {
        self.mark = Some((mark.to_owned(), ink));
        self
    }

    #[must_use]
    pub fn style(mut self, style: Style) -> Self {
        self.style = style;
        self
    }
}

impl Piece for Fields {
    fn rows(&self, _: usize, _: Fit) -> Option<Rows> {
        Some(Rows::exactly(self.rows.len()))
    }

    fn draw(&self, canvas: &mut Canvas<'_>, _: Fit) {
        let label = self.label_width.min(canvas.width() / 2);
        for (y, field) in self.rows.iter().enumerate() {
            canvas.fit(0, y, label.saturating_sub(1), &field.label, Ink::Muted);
            let mut x = label;
            if let Some((mark, ink)) = &field.mark {
                x += canvas.put(x, y, mark, *ink) + 1;
            }
            canvas.put(x, y, &field.value, field.style);
        }
    }
}

/// Lines of text on a plate: what went wrong, on the wash that says so.
///
/// Every line is folded rather than clipped, because an error names paths and
/// identities a person will want whole. Only when the frame cannot spare the
/// rows is the end left off, with a mark that says so.
#[derive(Clone, Debug, Default)]
pub struct Plate {
    pub wash: Wash,
    pub lines: Vec<(String, Ink)>,
}

impl Plate {
    fn folded(&self, width: usize) -> Vec<(String, Ink)> {
        let inner = width.saturating_sub(4).max(1);
        self.lines
            .iter()
            .flat_map(|(text, ink)| fold(text, inner).into_iter().map(|line| (line, *ink)))
            .collect()
    }
}

impl Piece for Plate {
    fn rows(&self, width: usize, _: Fit) -> Option<Rows> {
        let rows = self.folded(width).len();
        Some(Rows {
            least: rows.min(2),
            most: rows,
            grows: false,
        })
    }

    fn draw(&self, canvas: &mut Canvas<'_>, _: Fit) {
        let (width, height) = (canvas.width(), canvas.height());
        let inner = width.saturating_sub(4);
        let lines = self.folded(width);
        canvas.wash(0, 0, width, height.min(lines.len()), self.wash);
        for (y, (line, ink)) in lines.iter().enumerate().take(height) {
            if y + 1 == height && lines.len() > height {
                canvas.put(2, y, &clip(&format!("{line} …"), inner), *ink);
            } else {
                canvas.fit(2, y, inner, line, *ink);
            }
        }
    }
}

/// The line a command can be typed on, on a plate.
#[derive(Clone, Debug, Default)]
pub struct Prompt {
    /// What the line says while it is empty.
    pub hint: String,
}

impl Piece for Prompt {
    fn rows(&self, _: usize, fit: Fit) -> Option<Rows> {
        Some(Rows::exactly(if fit.squeeze == 0 { 3 } else { 1 }))
    }

    fn draw(&self, canvas: &mut Canvas<'_>, _: Fit) {
        let (width, height) = (canvas.width(), canvas.height());
        canvas.wash(0, 0, width, height, Wash::Plate);
        let y = height / 2;
        canvas.put(2, y, glyph::ROW, Style::new(Ink::Gold).bold());
        canvas.put(4, y, glyph::CURSOR, Ink::Gold);
        canvas.fit(6, y, width.saturating_sub(8), &self.hint, Ink::Faint);
    }
}

/// One event of a command's log.
#[derive(Clone, Debug, Default)]
pub struct LogLine {
    /// When, as the log file writes it: `21:04:18`.
    pub time: String,
    /// Which step it belongs to.
    pub step: String,
    pub text: String,
}

impl LogLine {
    #[must_use]
    pub fn new(time: &str, step: &str, text: &str) -> Self {
        Self {
            time: time.to_owned(),
            step: step.to_owned(),
            text: text.to_owned(),
        }
    }
}

/// The log pane: a plate with a header naming the file, one line per event
/// with its time, and a footer.
///
/// It takes whatever rows the screen has left. It follows the end of the log
/// until it is scrolled, and its footer then says which lines it shows.
#[derive(Clone, Debug, Default)]
pub struct LogPane {
    /// The file the log is being written to.
    pub file: String,
    pub lines: Vec<LogLine>,
    /// The first line shown, once the pane has been scrolled. `None` follows
    /// the end.
    pub top: Option<usize>,
    /// What the footer says about the file.
    pub note: String,
    /// Whether the command is still running, so the last line is what is
    /// happening now.
    pub live: bool,
}

impl LogPane {
    /// The rows of a pane that are not log lines: the header, a hairline when
    /// the pane is high enough to afford one, and the footer.
    const fn furniture(rows: usize) -> usize {
        if rows >= 6 { 3 } else { 2 }
    }

    /// The lines a pane `rows` high shows: the first, and how many.
    #[must_use]
    pub fn window(&self, rows: usize) -> (usize, usize) {
        let room = rows.saturating_sub(Self::furniture(rows));
        let count = self.lines.len().min(room);
        let last = self.lines.len() - count;
        (self.top.map_or(last, |top| top.min(last)), count)
    }

    /// The pane scrolled by `lines`, up for a negative number, when it is
    /// `rows` high. Scrolling to the end follows the end again.
    #[must_use]
    pub fn scrolled(&self, lines: isize, rows: usize) -> Option<usize> {
        let (first, count) = self.window(rows);
        let last = self.lines.len() - count;
        let top = first.saturating_add_signed(lines).min(last);
        (top < last).then_some(top)
    }
}

impl Piece for LogPane {
    fn rows(&self, width: usize, _: Fit) -> Option<Rows> {
        (width >= 24).then_some(Rows {
            least: 4,
            most: self.lines.len() + 3,
            grows: true,
        })
    }

    fn draw(&self, canvas: &mut Canvas<'_>, _: Fit) {
        let (width, height) = (canvas.width(), canvas.height());
        if height < 2 {
            return;
        }
        canvas.wash(0, 0, width, height, Wash::Plate);
        let mut pane = canvas.part(2, 0, width.saturating_sub(4), height);
        let inner = pane.width();
        pane.put(0, 0, "LOG", Ink::Faint);
        // A path is clipped from the front: its end is the part that differs.
        let room = inner.saturating_sub(5);
        let file = clean(&plain(&self.file));
        let file = if file.width() > room {
            let mut kept: Vec<char> = Vec::new();
            for character in file.chars().rev() {
                let next: String = std::iter::once(character)
                    .chain(kept.iter().rev().copied())
                    .collect();
                if next.width() + 1 > room {
                    break;
                }
                kept.push(character);
            }
            format!("…{}", kept.iter().rev().collect::<String>())
        } else {
            file
        };
        pane.end(5.min(inner), inner, 0, &file, Ink::Faint);
        let ruled = Self::furniture(height) == 3;
        if ruled {
            pane.repeat(0, 1, inner, glyph::RULE, Ink::Rule);
        }

        let (first, count) = self.window(height);
        let (time, step) = if inner >= 44 { (10, 10) } else { (9, 9) };
        for (slot, line) in self.lines.iter().skip(first).take(count).enumerate() {
            let y = 1 + usize::from(ruled) + slot;
            let now = self.live && first + slot + 1 == self.lines.len();
            pane.fit(0, y, time - 1, &line.time, Ink::Faint);
            pane.fit(
                time,
                y,
                step - 1,
                &line.step,
                if now { Ink::Gold } else { Ink::Muted },
            );
            pane.put(
                time + step,
                y,
                &line.text,
                if now { Ink::Text } else { Ink::Muted },
            );
        }

        let total = self.lines.len();
        let shown = if count == total {
            format!("{total} lines")
        } else {
            format!("{}–{} of {total}", first + 1, first + count)
        };
        let at = pane.end(0, inner, height - 1, &shown, Ink::Faint);
        pane.fit(0, height - 1, at.saturating_sub(2), &self.note, Ink::Faint);
    }
}

/// Which version of a text a line of a [`Difference`] is in.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Version {
    /// In both.
    #[default]
    Both,
    /// Only in the version that was there before.
    Before,
    /// Only in the version that is there now.
    Now,
}

/// One row of a [`Difference`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DifferenceRow {
    /// A line of text and its number in the version it is shown from.
    Line {
        number: usize,
        version: Version,
        text: String,
    },
    /// Lines that are the same in both versions and are left out.
    Gap,
}

impl DifferenceRow {
    #[must_use]
    pub fn line(number: usize, version: Version, text: &str) -> Self {
        Self::Line {
            number,
            version,
            text: text.to_owned(),
        }
    }
}

/// The difference between two versions of a text, line by line.
///
/// A line that is in only one version has a mark as well as an ink, `-` for
/// the version before and `+` for the one now, so the difference reads the
/// same with colour off. Lines are numbered as their own version numbers
/// them. A line is never folded: its end is cut with an ellipsis, since the
/// rows of a difference are lines of a file. When there are more rows than
/// room, the last row says which are shown.
#[derive(Clone, Debug, Default)]
pub struct Difference {
    pub rows: Vec<DifferenceRow>,
    /// The first row shown, once the difference has been scrolled.
    pub top: usize,
}

impl Difference {
    /// The rows a canvas `rows` high shows: the first, and how many.
    #[must_use]
    pub fn window(&self, rows: usize) -> (usize, usize) {
        let total = self.rows.len();
        if total <= rows {
            return (0, total);
        }
        // The last row says where in the difference this is.
        let count = rows.saturating_sub(1);
        (self.top.min(total - count), count)
    }

    /// The first row shown after scrolling by `lines`, up for a negative
    /// number, in a canvas `rows` high.
    #[must_use]
    pub fn scrolled(&self, lines: isize, rows: usize) -> usize {
        let (first, count) = self.window(rows);
        first
            .saturating_add_signed(lines)
            .min(self.rows.len() - count)
    }

    /// The cells the line numbers take.
    fn gutter(&self) -> usize {
        self.rows
            .iter()
            .filter_map(|row| match row {
                DifferenceRow::Line { number, .. } => Some(number.to_string().len()),
                DifferenceRow::Gap => None,
            })
            .max()
            .unwrap_or(1)
    }
}

/// A line as a terminal shows it: each tab reaches the next multiple of eight.
fn tabs_expanded(text: &str) -> String {
    let mut expanded = String::new();
    let mut column = 0;
    for character in text.chars() {
        if character == '\t' {
            let stop = 8 - column % 8;
            expanded.extend(std::iter::repeat_n(' ', stop));
            column += stop;
        } else {
            expanded.push(character);
            column += unicode_width::UnicodeWidthChar::width(character).unwrap_or(0);
        }
    }
    expanded
}

impl Piece for Difference {
    fn rows(&self, width: usize, _: Fit) -> Option<Rows> {
        let count = self.rows.len();
        (width >= self.gutter() + 8).then_some(Rows {
            least: count.min(3) + usize::from(count > 3),
            most: count,
            grows: false,
        })
    }

    fn draw(&self, canvas: &mut Canvas<'_>, _: Fit) {
        let (width, height) = (canvas.width(), canvas.height());
        let gutter = self.gutter();
        let (first, count) = self.window(height);
        for (y, row) in self.rows.iter().skip(first).take(count).enumerate() {
            match row {
                DifferenceRow::Gap => {
                    canvas.end(0, gutter, y, "…", Ink::Faint);
                }
                DifferenceRow::Line {
                    number,
                    version,
                    text,
                } => {
                    let (mark, mark_ink, ink) = match version {
                        Version::Both => ("", Ink::Faint, Ink::Muted),
                        Version::Before => (glyph::REMOVE, Ink::Danger, Ink::Danger),
                        Version::Now => (glyph::ADD, Ink::Green, Ink::Green),
                    };
                    canvas.end(0, gutter, y, &number.to_string(), Ink::Faint);
                    canvas.put(gutter + 1, y, mark, Style::new(mark_ink).bold());
                    canvas.put(gutter + 3, y, &tabs_expanded(text), ink);
                }
            }
        }
        if count < self.rows.len() && height > 0 {
            canvas.put(
                (gutter + 3).min(width),
                count,
                &format!(
                    "… {}–{} of {} rows",
                    first + 1,
                    first + count,
                    self.rows.len()
                ),
                Ink::Muted,
            );
        }
    }
}

/// Which side of two columns has a stated width.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Split {
    /// The left column is this many cells and the right has the rest.
    Left(usize),
    /// The right column is this many cells and the left has the rest.
    Right(usize),
}

/// What two columns become where they do not fit side by side.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Narrow {
    /// Only the left column.
    Left,
    /// Only the right column.
    Right,
    /// The left column over the right.
    Stack,
}

/// Two columns of pieces side by side.
///
/// A column with nothing left to show at the frame's rung gives its width to
/// the other.
pub struct Columns {
    pub left: Focal,
    pub right: Focal,
    pub split: Split,
    /// The cells between the columns.
    pub gap: usize,
    /// The narrowest the column without a stated width may be.
    pub least: usize,
    pub narrow: Narrow,
}

enum Arrangement {
    Left,
    Right,
    Stack,
    Beside(usize, usize),
}

impl Columns {
    fn arrangement(&self, width: usize, fit: Fit) -> Arrangement {
        match (
            self.left.shows_anything(fit),
            self.right.shows_anything(fit),
        ) {
            (_, false) => return Arrangement::Left,
            (false, true) => return Arrangement::Right,
            (true, true) => {}
        }
        let stated = match self.split {
            Split::Left(cells) | Split::Right(cells) => cells,
        };
        if width >= stated + self.gap + self.least {
            let rest = width - stated - self.gap;
            return match self.split {
                Split::Left(_) => Arrangement::Beside(stated, rest),
                Split::Right(_) => Arrangement::Beside(rest, stated),
            };
        }
        match self.narrow {
            Narrow::Left => Arrangement::Left,
            Narrow::Right => Arrangement::Right,
            Narrow::Stack => Arrangement::Stack,
        }
    }
}

impl Piece for Columns {
    fn rows(&self, width: usize, fit: Fit) -> Option<Rows> {
        Some(match self.arrangement(width, fit) {
            Arrangement::Left => self.left.rows(width, fit)?,
            Arrangement::Right => self.right.rows(width, fit)?,
            Arrangement::Stack => {
                let (left, right) = (self.left.rows(width, fit)?, self.right.rows(width, fit)?);
                let gap = usize::from(fit.squeeze < 2);
                Rows {
                    least: left.least + gap + right.least,
                    most: left.most + gap + right.most,
                    grows: left.grows || right.grows,
                }
            }
            Arrangement::Beside(left, right) => {
                let (left, right) = (self.left.rows(left, fit)?, self.right.rows(right, fit)?);
                Rows {
                    least: left.least.max(right.least),
                    most: left.most.max(right.most),
                    grows: left.grows || right.grows,
                }
            }
        })
    }

    fn draw(&self, canvas: &mut Canvas<'_>, fit: Fit) {
        let (width, height) = (canvas.width(), canvas.height());
        match self.arrangement(width, fit) {
            Arrangement::Left => self.left.draw(canvas, fit),
            Arrangement::Right => self.right.draw(canvas, fit),
            Arrangement::Stack => {
                let gap = usize::from(fit.squeeze < 2);
                let left = self.left.rows(width, fit).unwrap_or_default();
                let right = self.right.rows(width, fit).unwrap_or_default();
                // The upper column has what it can use; the lower one what is
                // left, and never less than it needs.
                let upper = left
                    .most
                    .min(height.saturating_sub(gap + right.least))
                    .max(left.least);
                self.left.draw(&mut canvas.part(0, 0, width, upper), fit);
                self.right
                    .draw(&mut canvas.part(0, upper + gap, width, height), fit);
            }
            Arrangement::Beside(left, right) => {
                self.left.draw(&mut canvas.part(0, 0, left, height), fit);
                self.right
                    .draw(&mut canvas.part(left + self.gap, 0, right, height), fit);
            }
        }
    }
}

/// A piece with a faint heading over it, as the design sets off a section.
#[must_use]
pub fn titled(title: &str, piece: impl Piece + 'static) -> Focal {
    Focal::new().piece(Text::label(title)).gap(1).piece(piece)
}

/// The same, given up as a whole at a rank.
#[must_use]
pub fn titled_ranked(rank: Rank, title: &str, piece: impl Piece + 'static) -> Focal {
    Focal::new()
        .ranked(rank, Text::label(title))
        .gap(1)
        .ranked(rank, piece)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cells::Buffer;
    use crate::frame::Rung;

    const BOLD: Fit = Fit {
        sizing: Sizing::Bold,
        rung: Rung::Full,
        squeeze: 0,
    };
    const SCALED: Fit = Fit {
        sizing: Sizing::Scaled,
        ..BOLD
    };

    /// Draw a piece at a width, as high as it asks to be.
    fn drawn(piece: &dyn Piece, width: usize, fit: Fit) -> Buffer {
        let rows = piece.rows(width, fit).expect("drawable at this width");
        let mut buffer = Buffer::new(width, rows.most.max(rows.least));
        piece.draw(&mut buffer.canvas(), fit);
        buffer
    }

    fn at(piece: &dyn Piece, width: usize, rows: usize) -> Buffer {
        let mut buffer = Buffer::new(width, rows);
        piece.draw(&mut buffer.canvas(), BOLD);
        buffer
    }

    #[test]
    fn a_statement_is_scaled_only_where_it_can_be() {
        let statement = Statement::new("Add inkscape and the 2 packages it needs");
        assert_eq!(statement.rows(96, SCALED), Some(Rows::exactly(2)));
        assert_eq!(statement.rows(96, BOLD), Some(Rows::exactly(1)));
        // Eighty cells at twice the size: it does not fit in seventy-six.
        assert_eq!(statement.rows(76, SCALED), Some(Rows::exactly(1)));
        let scaled = drawn(&statement, 96, SCALED);
        assert_eq!(
            scaled.text(),
            "A d d   i n k s c a p e   a n d   t h e   2   p a c k a g e s   i t   n e e d s\n\n"
        );
        assert!(
            scaled
                .paint(None, crate::terminal::Theme::PLAIN)
                .contains("\x1b]66;s=2:w=1;A\x1b\\")
        );
        let bold = drawn(&statement, 96, BOLD);
        assert_eq!(bold.text(), "Add inkscape and the 2 packages it needs\n");
        assert!(bold.cell(0, 0).unwrap().1.bold);
        assert!(
            !bold
                .paint(None, crate::terminal::Theme::FULL)
                .contains("\x1b]66")
        );
        // Too long for one row even at ordinary size: folded, never cut.
        assert_eq!(
            drawn(&statement, 30, BOLD).text(),
            "Add inkscape and the 2\npackages it needs\n"
        );
    }

    #[test]
    fn what_stands_beside_a_statement_stands_at_its_foot() {
        let statement = Statement::new("67%")
            .scale(3)
            .beside("22.88 of 33.78 MiB", Ink::Text)
            .beside("About 2 seconds left", Ink::Muted);
        assert_eq!(
            drawn(&statement, 96, SCALED).text(),
            "6  7  %\n             22.88 of 33.78 MiB\n             About 2 seconds left\n"
        );
        assert_eq!(
            drawn(&statement, 96, BOLD).text(),
            "67%    22.88 of 33.78 MiB\n       About 2 seconds left\n"
        );
        assert_eq!(
            drawn(&statement, 20, BOLD).text(),
            "67%\n22.88 of 33.78 MiB\nAbout 2 seconds left\n"
        );
        let marked = Statement::new("inkscape is installed").marked(glyph::DONE, Ink::Green);
        assert_eq!(drawn(&marked, 96, BOLD).text(), "✓ inkscape is installed\n");
        assert!(
            drawn(&marked, 96, SCALED)
                .text()
                .starts_with("✓   i n k s c a p e")
        );
    }

    #[test]
    fn text_folds_to_its_limit_and_says_when_there_was_more() {
        let text = Text::new(
            "Your system is unchanged. Revision a83f19c2 is still the one in use.",
            Ink::Muted,
        );
        assert_eq!(drawn(&text, 96, BOLD).text().lines().count(), 1);
        assert_eq!(
            drawn(&text, 40, BOLD).text(),
            "Your system is unchanged. Revision\na83f19c2 is still the one in use.\n"
        );
        let cut = drawn(&text, 20, BOLD).text();
        assert_eq!(cut.lines().count(), 2);
        assert!(cut.trim_end().ends_with('…'), "{cut}");
        assert_eq!(
            Text::new("  ", Ink::Text).rows(40, BOLD),
            Some(Rows::exactly(0))
        );
        assert_eq!(drawn(&Text::label("Added"), 20, BOLD).text(), "ADDED\n");
    }

    #[test]
    fn tiles_are_a_quarter_of_the_width_or_not_drawn() {
        let tiles = Tiles(vec![
            Tile::new("Now selected", "7be204d1").ink(Ink::Gold),
            Tile::new("Previous", "a83f19c2, kept"),
            Tile::new("Verified", "3 signatures"),
            Tile::new("Took", "14 seconds"),
        ]);
        let buffer = drawn(&tiles, 96, BOLD);
        assert_eq!(
            buffer.text(),
            "  NOW SELECTED            PREVIOUS                VERIFIED                TOOK\n  7be204d1                a83f19c2, kept          3 signatures            14 seconds\n"
        );
        assert_eq!(
            buffer.cell(2, 1).unwrap().1,
            Style::new(Ink::Gold).bold().on(Wash::Plate)
        );
        assert_eq!(buffer.cell(2, 0).unwrap().1.wash, Wash::Plate);
        assert!(
            buffer
                .paint(None, crate::terminal::Theme::FULL)
                .contains(Wash::Plate.code(true))
        );
        // Seventy-six cells is seventeen a tile, and "a83f19c2, kept" needs
        // eighteen: given up whole.
        assert_eq!(tiles.rows(76, BOLD), None);
        assert!(tiles.rows(78, BOLD).is_some());
        // Two tiles stay the size they have among four.
        let two = Tiles(tiles.0[..2].to_vec());
        assert_eq!(
            drawn(&two, 96, BOLD).text().lines().next().unwrap(),
            "  NOW SELECTED            PREVIOUS"
        );
    }

    #[test]
    fn the_graph_is_two_rows_and_says_its_figure() {
        let graph = Graph {
            label: "Speed".into(),
            figure: "5.72 MiB/s".into(),
            samples: vec![1, 4, 8, 9, 12, 16, 0],
        };
        assert_eq!(
            drawn(&graph, 12, BOLD).text(),
            "SPEED  5.72…\n   ▁▄█\n▁▄████▁▁▁▁▁▁\n"
        );
        // The newest samples are the ones kept, and the tallest fills the cell.
        let narrow = drawn(&graph, 3, BOLD).text();
        assert_eq!(narrow.lines().nth(1).unwrap(), "▄█");
        // A rate near the top of its type does not wrap into a short column.
        let huge = Graph {
            samples: vec![u64::MAX, u64::MAX / 2],
            ..Graph::default()
        };
        assert_eq!(drawn(&huge, 2, BOLD).text(), "\n█\n██\n");
        let none = Graph::default();
        assert_eq!(drawn(&none, 4, BOLD).text(), "\n\n▁▁▁▁\n");
    }

    fn transfers() -> Transfers {
        Transfers(vec![
            Transfer {
                name: "potrace".into(),
                bytes: 184_320,
                total: Some(184_320),
                complete: true,
                cached: false,
            },
            Transfer {
                name: "inkscape".into(),
                bytes: 22_544_384,
                total: Some(33_973_862),
                complete: false,
                cached: false,
            },
            Transfer {
                name: "lib2geom".into(),
                bytes: 1_258_291,
                total: Some(1_258_291),
                complete: true,
                cached: false,
            },
        ])
    }

    #[test]
    fn the_transfer_table_says_each_state_in_a_bar_and_a_word() {
        let buffer = drawn(&transfers(), 96, BOLD);
        assert_eq!(
            buffer.text(),
            "\
PACKAGE                 PROGRESS                                            TRANSFER  STATUS
inkscape                ━━━━━━━━━━━━━━━━━━━━━━━━━━╸─────────────     21.5 / 32.4 MiB  Working
lib2geom                ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━       1.2 / 1.2 MiB  Complete
potrace                 ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━       180 / 180 KiB  Complete
"
        );
        assert_eq!(buffer.cell(24, 1).unwrap().1.ink, Ink::Gold);
        assert_eq!(buffer.cell(24, 2).unwrap().1.ink, Ink::Green);
        assert_eq!(buffer.cell(63, 1).unwrap().1.ink, Ink::Rule);
        // Narrow: the amounts go, the bar and the word stay.
        assert_eq!(
            drawn(&transfers(), 56, BOLD).text().lines().nth(1).unwrap(),
            "inkscape                ━━━━━━━━━━━━━───────  Working"
        );
    }

    #[test]
    fn an_unknown_total_is_never_drawn_as_progress() {
        let mut transfers = transfers();
        transfers.0[1].total = None;
        let text = drawn(&transfers, 96, BOLD).text();
        let row = text.lines().nth(1).unwrap();
        assert!(!row.contains('━') && !row.contains('╸'), "{row}");
        assert!(row.contains("21.5 MiB / ?"), "{row}");
        assert!(row.ends_with("Working"), "{row}");
    }

    #[test]
    fn more_packages_than_rows_are_counted_not_dropped() {
        let many = Transfers(
            (0..40)
                .map(|index| Transfer {
                    name: format!("package-{index:02}"),
                    complete: index < 30,
                    ..Transfer::default()
                })
                .collect(),
        );
        assert_eq!(
            many.rows(96, BOLD),
            Some(Rows {
                least: 5,
                most: 41,
                grows: false
            })
        );
        let text = at(&many, 96, 5).text();
        assert!(
            text.contains("package-30") && text.contains("package-32"),
            "{text}"
        );
        assert!(text.ends_with("… 37 more packages\n"), "{text}");
    }

    fn review() -> Table {
        let row = |name: &str, version: &str, from: &str, size: &str, why: &str, ink| {
            Row::new(vec![
                (name.to_owned(), Ink::Text),
                (version.to_owned(), Ink::Text),
                (from.to_owned(), Ink::repository(from)),
                (size.to_owned(), Ink::Text),
                (why.to_owned(), ink),
            ])
            .marked(glyph::ADD, Ink::Green)
        };
        Table {
            columns: vec![
                TableColumn::new("Package", 22).bold(),
                TableColumn::new("Version", 12),
                TableColumn::new("From", 9),
                TableColumn::new("Download", 12).right().keep_above(88),
                TableColumn::rest("Why").keep_above(70),
            ],
            rows: vec![
                row(
                    "inkscape",
                    "1.4.2-1",
                    "extra",
                    "32.4 MiB",
                    "You asked for it",
                    Ink::Text,
                ),
                row(
                    "lib2geom",
                    "1.4-1",
                    "extra",
                    "1.2 MiB",
                    "Needed by inkscape",
                    Ink::Muted,
                ),
                row(
                    "potrace",
                    "1.16-3",
                    "core",
                    "180 KiB",
                    "Needed by inkscape",
                    Ink::Muted,
                ),
            ],
            header: true,
            cursor: Some(0),
        }
    }

    #[test]
    fn the_review_table_marks_its_cursor_row_with_a_shape_and_a_wash() {
        let buffer = drawn(&review(), 96, BOLD);
        assert_eq!(
            buffer.text(),
            "     PACKAGE                 VERSION       FROM           DOWNLOAD  WHY
›+   inkscape                1.4.2-1       extra          32.4 MiB  You asked for it
 +   lib2geom                1.4-1         extra           1.2 MiB  Needed by inkscape
 +   potrace                 1.16-3        core            180 KiB  Needed by inkscape
"
        );
        assert_eq!(
            buffer.cell(5, 1).unwrap().1,
            Style::new(Ink::Text).bold().on(Wash::Cursor)
        );
        assert_eq!(buffer.cell(5, 2).unwrap().1.wash, Wash::None);
        assert_eq!(buffer.cell(43, 1).unwrap().1.ink, Ink::Cyan);
        assert_eq!(buffer.cell(43, 3).unwrap().1.ink, Ink::Blue);
        // Columns go in their stated order as the table narrows.
        let narrower = drawn(&review(), 76, BOLD).text();
        assert!(
            !narrower.contains("DOWNLOAD") && narrower.contains("WHY"),
            "{narrower}"
        );
        let narrowest = drawn(&review(), 56, BOLD).text();
        assert!(
            !narrowest.contains("WHY") && narrowest.contains("FROM"),
            "{narrowest}"
        );
        assert!(narrowest.contains("›+   inkscape"), "{narrowest}");
    }

    #[test]
    fn a_long_table_keeps_its_cursor_in_view() {
        let mut table = review();
        table.rows = (0..20)
            .map(|index| Row::new(vec![(format!("package-{index:02}"), Ink::Text)]))
            .collect();
        table.cursor = Some(11);
        let text = at(&table, 60, 6).text();
        assert!(text.contains("›    package-11"), "{text}");
        assert!(
            text.contains("package-08") && !text.contains("package-07"),
            "{text}"
        );
        assert!(text.ends_with("     … 16 more\n"), "{text}");
        table.cursor = Some(19);
        assert!(at(&table, 60, 6).text().contains("›    package-19"));
    }

    #[test]
    fn wide_and_combining_names_keep_every_column_where_it_is() {
        let mut table = review();
        table.rows[0].cells[0].0 = "界面-editor".into();
        table.rows[1].cells[0].0 = "cre\u{300}me-bru\u{302}le\u{301}e".into();
        table.rows[2].cells[0].0 = "👩‍💻-tools-with-a-name-longer-than-its-column".into();
        let buffer = drawn(&table, 96, BOLD);
        for y in 1..=3 {
            assert!(
                buffer.cell(29, y).is_some(),
                "the version starts in the same cell"
            );
            assert_eq!(buffer.cell(28, y), None);
        }
        let text = buffer.text();
        assert!(text.contains("👩‍💻-tools-with-a-name-…  1.16-3"), "{text}");
        for row in text.lines() {
            assert!(row.width() <= 96, "{row:?}");
        }

        let mut moving = transfers();
        moving.0[1].name = "界面界面界面界面界面界面界面".into();
        moving.0[2].name = "e\u{301}".into();
        let buffer = drawn(&moving, 96, BOLD);
        for y in 1..=3 {
            assert_eq!(
                buffer.cell(24, y).unwrap().0,
                "━",
                "the bar starts in the same cell"
            );
        }
    }

    #[test]
    fn a_listing_keeps_marks_figures_and_details_in_their_columns() {
        let steps = Listing::new(vec![
            ListRow::step("Merge into system", Step::Done, "0.2s"),
            ListRow::step("Build generation", Step::Working, "1.2s"),
            ListRow::step("Check boot tools", Step::Waiting, ""),
        ]);
        let buffer = drawn(&steps, 30, BOLD);
        assert_eq!(
            buffer.text(),
            "✓  Merge into system      0.2s\n●  Build generation       1.2s\n○  Check boot tools\n"
        );
        assert_eq!(buffer.cell(0, 1).unwrap().1.ink, Ink::Gold);
        assert!(buffer.cell(3, 1).unwrap().1.bold);

        let undone = Listing::new(vec![
            ListRow::done("The unfinished generation").detail("was discarded"),
            ListRow::done("inkscape was taken out of your declaration again"),
        ]);
        assert_eq!(
            drawn(&undone, 34, BOLD).text(),
            "✓  The unfinished generation\n   was discarded\n✓  inkscape was taken out of your\n   declaration again\n"
        );
        let keys = Listing::new(vec![ListRow::key("r", "Try the change again", true)]).gutter(5);
        let buffer = drawn(&keys, 40, BOLD);
        assert_eq!(buffer.text(), "r    Try the change again\n");
        assert_eq!(buffer.cell(0, 0).unwrap().1, Style::new(Ink::Gold).bold());
    }

    #[test]
    fn fields_put_a_mark_before_a_state() {
        let fields = Fields {
            label_width: 18,
            rows: vec![
                Field::new("Revision", "a83f19c2").style(Style::new(Ink::Gold).bold()),
                Field::new("Recorded health", "Healthy").marked(glyph::DONE, Ink::Green),
            ],
        };
        assert_eq!(
            drawn(&fields, 44, BOLD).text(),
            "Revision          a83f19c2\nRecorded health   ✓ Healthy\n"
        );
    }

    fn error() -> Plate {
        Plate {
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
        }
    }

    #[test]
    fn a_plate_folds_an_error_and_marks_what_it_could_not_show() {
        let buffer = drawn(&error(), 58, BOLD);
        assert_eq!(
            buffer.text(),
            "  Refreshing the system caches failed.\n  the finalizer-ca-trust-v1 generator failed:\n  finalizer I/O: Invalid cross-device link (os error 18)\n"
        );
        assert_eq!(
            buffer.cell(2, 1).unwrap().1,
            Style::new(Ink::Danger).on(Wash::Alarm)
        );
        let folded = drawn(&error(), 40, BOLD).text();
        assert_eq!(folded.lines().count(), 5, "{folded}");
        assert_eq!(
            folded.split_whitespace().collect::<String>(),
            buffer.text().split_whitespace().collect::<String>()
        );
        let cut = at(&error(), 58, 2).text();
        assert!(cut.ends_with("generator failed: …\n"), "{cut}");
    }

    fn log() -> LogPane {
        LogPane {
            file: "/var/log/herda/pkg/20261012-210418.log".into(),
            lines: [
                (
                    "21:04:18",
                    "resolve",
                    "inkscape 1.4.2-1 with 2 dependencies",
                ),
                ("21:04:19", "review", "accepted"),
                ("21:04:25", "download", "33.78 MiB in 3 archives"),
                ("21:04:26", "verify", "3 signatures good"),
                ("21:04:27", "create", "building generation from a83f19c2"),
            ]
            .iter()
            .map(|(time, step, text)| LogLine::new(time, step, text))
            .collect(),
            top: None,
            note: "Written to the file as it happens".into(),
            live: true,
        }
    }

    #[test]
    fn the_log_pane_names_its_file_and_counts_its_lines() {
        let buffer = at(&log(), 62, 9);
        assert_eq!(
            buffer.text(),
            "  LOG                 /var/log/herda/pkg/20261012-210418.log
  ──────────────────────────────────────────────────────────
  21:04:18  resolve   inkscape 1.4.2-1 with 2 dependencies
  21:04:19  review    accepted
  21:04:25  download  33.78 MiB in 3 archives
  21:04:26  verify    3 signatures good
  21:04:27  create    building generation from a83f19c2

  Written to the file as it happens                  5 lines
"
        );
        // The line that is happening now is told from the ones before it.
        assert_eq!(buffer.cell(12, 6).unwrap().1.ink, Ink::Gold);
        assert_eq!(buffer.cell(12, 5).unwrap().1.ink, Ink::Muted);
        assert_eq!(buffer.cell(0, 8), None);
        assert_eq!(
            buffer
                .paint(None, crate::terminal::Theme::FULL)
                .matches("\x1b[2J")
                .count(),
            1
        );
    }

    #[test]
    fn the_log_pane_follows_the_end_until_it_is_scrolled() {
        let mut pane = log();
        assert_eq!(pane.window(6), (2, 3));
        let text = at(&pane, 62, 6).text();
        assert!(
            text.contains("21:04:25") && !text.contains("21:04:19"),
            "{text}"
        );
        assert!(text.ends_with("3–5 of 5\n"), "{text}");
        // Too short for a hairline: the header, the lines and the footer.
        assert_eq!(pane.window(4), (3, 2));
        assert!(!at(&pane, 62, 4).text().contains('─'));

        pane.top = pane.scrolled(-2, 6);
        assert_eq!(pane.top, Some(0));
        assert!(at(&pane, 62, 6).text().ends_with("1–3 of 5\n"));
        pane.top = pane.scrolled(-9, 6);
        assert_eq!(pane.top, Some(0), "cannot scroll above the first line");
        pane.top = pane.scrolled(1, 6);
        assert_eq!(pane.top, Some(1));
        pane.top = pane.scrolled(9, 6);
        assert_eq!(pane.top, None, "back at the end, it follows again");
        // A path too long for the header loses its start, not its end.
        let narrow = at(&pane, 30, 6).text();
        assert!(
            narrow
                .lines()
                .next()
                .unwrap()
                .ends_with("  LOG  …/20261012-210418.log"),
            "{narrow}"
        );
        assert_eq!(pane.rows(23, BOLD), None);
    }

    #[test]
    fn columns_stand_side_by_side_where_there_is_room() {
        let columns = |narrow| Columns {
            left: titled(
                "Inside this step",
                Listing::new(vec![ListRow::step("Merge", Step::Done, "0.2s")]),
            ),
            right: Focal::new().piece(log()),
            split: Split::Left(30),
            gap: 3,
            least: 40,
            narrow,
        };
        let wide = at(&columns(Narrow::Right), 96, 9).text();
        assert!(
            wide.starts_with("INSIDE THIS STEP                   LOG"),
            "{wide}"
        );
        assert!(
            wide.lines()
                .nth(2)
                .unwrap()
                .starts_with("✓  Merge                  0.2s     21:04:18"),
            "{wide}"
        );
        // Too narrow for both: the side that was asked for.
        let narrow = at(&columns(Narrow::Right), 56, 9).text();
        assert!(
            narrow.starts_with("  LOG") && !narrow.contains("INSIDE"),
            "{narrow}"
        );
        let left = at(&columns(Narrow::Left), 56, 9).text();
        assert!(
            left.starts_with("INSIDE THIS STEP") && !left.contains("LOG"),
            "{left}"
        );
        let stacked = columns(Narrow::Stack);
        let rows = stacked.rows(56, BOLD).unwrap();
        assert_eq!((rows.least, rows.grows), (3 + 1 + 4, true));
        let text = at(&stacked, 56, 12).text();
        assert!(text.starts_with("INSIDE THIS STEP\n\n✓  Merge"), "{text}");
        assert!(text.lines().nth(4).unwrap().starts_with("  LOG"), "{text}");
    }

    #[test]
    fn a_column_with_nothing_left_gives_its_width_away() {
        let columns = Columns {
            left: titled("What went wrong", error()),
            right: titled_ranked(
                Rank::Detail,
                "Already undone",
                Listing::new(vec![ListRow::done("x")]),
            ),
            split: Split::Right(34),
            gap: 3,
            least: 40,
            narrow: Narrow::Left,
        };
        let bare = Fit {
            rung: Rung::Bare,
            ..BOLD
        };
        let both = drawn(&columns, 96, BOLD).text();
        assert!(both.contains("ALREADY UNDONE"), "{both}");
        let mut buffer = Buffer::new(96, 5);
        columns.draw(&mut buffer.canvas(), bare);
        assert!(!buffer.text().contains("ALREADY UNDONE"));
        assert_eq!(buffer.cell(90, 2).map(|cell| cell.1.wash), None);
        assert_eq!(buffer.cell(2, 2).unwrap().1.wash, Wash::Alarm);
    }

    #[test]
    fn the_prompt_is_a_plate_with_a_cursor() {
        let prompt = Prompt {
            hint: "Search software, or type a command".into(),
        };
        assert_eq!(
            drawn(&prompt, 60, BOLD).text(),
            "\n  › ▌ Search software, or type a command\n\n"
        );
        let squeezed = Fit { squeeze: 1, ..BOLD };
        assert_eq!(prompt.rows(60, squeezed), Some(Rows::exactly(1)));
    }

    /// No piece writes outside the canvas it was given, at any width, with
    /// text that is hostile and text that is wide.
    fn difference() -> Difference {
        Difference {
            rows: vec![
                DifferenceRow::line(12, Version::Both, "font_size 12.0"),
                DifferenceRow::line(13, Version::Both, "cursor_trail 3"),
                DifferenceRow::line(14, Version::Before, "background_opacity 0.86"),
                DifferenceRow::line(14, Version::Now, "background_opacity 1.0"),
                DifferenceRow::line(15, Version::Both, "window_padding_width 12"),
                DifferenceRow::Gap,
                DifferenceRow::line(141, Version::Both, "map ctrl+shift+c\tcopy_to_clipboard"),
                DifferenceRow::line(142, Version::Now, "map ctrl+shift+t\tnew_tab_with_cwd"),
            ],
            top: 0,
        }
    }

    #[test]
    fn a_difference_marks_each_version_with_a_sign_as_well_as_an_ink() {
        let buffer = drawn(&difference(), 48, BOLD);
        assert_eq!(
            buffer.text(),
            [
                " 12   font_size 12.0",
                " 13   cursor_trail 3",
                " 14 - background_opacity 0.86",
                " 14 + background_opacity 1.0",
                " 15   window_padding_width 12",
                "  …",
                "141   map ctrl+shift+c        copy_to_clipboard",
                "142 + map ctrl+shift+t        new_tab_with_cwd",
                "",
            ]
            .join("\n")
        );
        // Numbers end in one column; a sign, then the line as it is written.
        assert_eq!(
            (
                buffer.cell(4, 2).unwrap().0,
                buffer.cell(4, 2).unwrap().1.ink
            ),
            ("-", Ink::Danger)
        );
        assert_eq!(buffer.cell(6, 2).unwrap().1.ink, Ink::Danger);
        assert_eq!(
            (
                buffer.cell(4, 3).unwrap().0,
                buffer.cell(4, 3).unwrap().1.ink
            ),
            ("+", Ink::Green)
        );
        assert_eq!(buffer.cell(6, 0).unwrap().1.ink, Ink::Muted);
        // A line too long for the canvas is cut, never folded onto a row
        // that would read as a line of the file.
        assert_eq!(
            drawn(&difference(), 24, BOLD)
                .text()
                .lines()
                .nth(2)
                .unwrap(),
            " 14 - background_opacit…"
        );
        assert_eq!(difference().rows(10, BOLD), None);
    }

    #[test]
    fn a_long_difference_says_which_rows_it_shows() {
        let mut difference = difference();
        assert_eq!(
            difference.rows(48, BOLD),
            Some(Rows {
                least: 4,
                most: 8,
                grows: false
            })
        );
        assert_eq!(difference.window(8), (0, 8));
        assert_eq!(difference.window(5), (0, 4));
        assert_eq!(
            at(&difference, 48, 5).text(),
            [
                " 12   font_size 12.0",
                " 13   cursor_trail 3",
                " 14 - background_opacity 0.86",
                " 14 + background_opacity 1.0",
                "      … 1–4 of 8 rows",
                "",
            ]
            .join("\n")
        );
        // Scrolling stops where the last row is shown.
        assert_eq!(difference.scrolled(1, 5), 1);
        assert_eq!(difference.scrolled(40, 5), 4);
        assert_eq!(difference.scrolled(-3, 5), 0);
        difference.top = difference.scrolled(40, 5);
        assert_eq!(
            at(&difference, 48, 5).text(),
            [
                " 15   window_padding_width 12",
                "  …",
                "141   map ctrl+shift+c        copy_to_clipboard",
                "142 + map ctrl+shift+t        new_tab_with_cwd",
                "      … 5–8 of 8 rows",
                "",
            ]
            .join("\n")
        );
        // A difference that fits is not scrolled, whatever it is told.
        difference.top = 3;
        assert_eq!(difference.window(8), (0, 8));
    }

    #[test]
    fn no_piece_leaves_its_canvas() {
        let hostile =
            "界面\x1b[2J\x1b]52;c;x\x07\r\n\u{202e}e\u{301}👩‍💻 a long and winding name".repeat(3);
        let mut statement = Statement::new(&hostile).marked(&hostile, Ink::Green);
        statement.beside.push((hostile.clone(), Ink::Text));
        let mut table = review();
        table.rows[0]
            .cells
            .iter_mut()
            .for_each(|cell| cell.0 = hostile.clone());
        table.rows[0].marker = Some((hostile.clone(), Ink::Green));
        let mut moving = transfers();
        moving.0[0].name = hostile.clone();
        let mut pane = log();
        pane.file = hostile.clone();
        pane.note = hostile.clone();
        pane.lines[4] = LogLine::new(&hostile, &hostile, &hostile);
        let pieces: Vec<Box<dyn Piece>> = vec![
            Box::new(statement),
            Box::new(Text::new(&hostile, Ink::Text)),
            Box::new(Tiles(vec![Tile::new("a", "b"), Tile::new("界面", "c")])),
            Box::new(Graph {
                label: hostile.clone(),
                figure: hostile.clone(),
                samples: vec![3, 9],
            }),
            Box::new(moving),
            Box::new(table),
            Box::new(Listing::new(vec![
                ListRow::step(&hostile, Step::Failed, &hostile).detail(&hostile),
            ])),
            Box::new(Fields {
                label_width: 18,
                rows: vec![Field::new(&hostile, &hostile).marked(&hostile, Ink::Green)],
            }),
            Box::new(Plate {
                wash: Wash::Alarm,
                lines: vec![(hostile.clone(), Ink::Danger)],
            }),
            Box::new(Prompt {
                hint: hostile.clone(),
            }),
            Box::new(pane),
            Box::new(Difference {
                rows: vec![
                    DifferenceRow::line(usize::MAX, Version::Before, &hostile),
                    DifferenceRow::Gap,
                    DifferenceRow::line(1, Version::Now, &hostile),
                    DifferenceRow::line(2, Version::Both, &hostile),
                    DifferenceRow::line(3, Version::Both, &hostile),
                ],
                top: 9,
            }),
        ];
        for piece in &pieces {
            for width in [1, 2, 7, 24, 40, 56, 76, 96] {
                for fit in [
                    BOLD,
                    SCALED,
                    Fit {
                        squeeze: 2,
                        rung: Rung::Bare,
                        ..SCALED
                    },
                ] {
                    let Some(rows) = piece.rows(width, fit) else {
                        continue;
                    };
                    assert!(rows.least <= rows.most.max(rows.least));
                    for height in [rows.least, rows.most.min(40), rows.least + 3] {
                        // A margin of marked cells around the canvas must
                        // come through untouched.
                        let mut buffer = Buffer::new(width + 4, height + 4);
                        buffer.wash(0, 0, width + 4, height + 4, Wash::Key);
                        piece.draw(&mut buffer.canvas().part(2, 2, width, height), fit);
                        for y in 0..height + 4 {
                            for x in 0..width + 4 {
                                let inside =
                                    (2..2 + width).contains(&x) && (2..2 + height).contains(&y);
                                assert!(
                                    inside || buffer.cell(x, y).is_none(),
                                    "width {width}, height {height}: wrote at {x},{y}"
                                );
                            }
                        }
                        let bytes = buffer.paint(None, crate::terminal::Theme::FULL);
                        assert!(
                            !bytes.contains("\x1b]52") && !bytes.contains(['\r', '\n', '\u{202e}'])
                        );
                    }
                }
            }
        }
    }
}

//! The cell buffer: the one grid a full screen is drawn into.
//!
//! A screen that is redrawn in place cannot be a list of styled strings. Two
//! pieces side by side, a plate behind a table and a statement twice the size
//! of the text beside it all need to address cells, and the thing that writes
//! to the terminal needs to know what is already there. So a screen is drawn
//! into a [`Buffer`] — a grapheme, an ink, a weight and a wash per cell — and
//! the buffer is what leaves the crate, three ways:
//!
//! - [`Buffer::paint`] is the bytes that turn what a terminal shows into this
//!   buffer, touching only the cells that differ;
//! - [`Buffer::text`] is the screen as plain text, which is what a test holds;
//! - [`page`] is the screen as a self-contained HTML page, which is what a
//!   person looks at beside the design.
//!
//! Drawing never consults a theme. Colour depth and `NO_COLOR` are applied
//! when a buffer is painted, so the same buffer is the same layout on every
//! terminal, and a state that is told by an ink is always also told by the
//! character in the cell.

use std::fmt::Write as _;

use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

use crate::ink::{Ink, Wash};
use crate::terminal::{Theme, clean, clip, plain};

/// How one cell is drawn: its ink, its weight and the wash behind it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Style {
    pub ink: Ink,
    pub bold: bool,
    /// [`Wash::None`] keeps whatever wash the cell already has, so text put on
    /// a plate stays on the plate without every caller naming it.
    pub wash: Wash,
}

impl Style {
    /// Regular weight in this ink, on whatever is behind it.
    #[must_use]
    pub const fn new(ink: Ink) -> Self {
        Self {
            ink,
            bold: false,
            wash: Wash::None,
        }
    }

    /// The same style in bold.
    #[must_use]
    pub const fn bold(mut self) -> Self {
        self.bold = true;
        self
    }

    /// The same style on this wash.
    #[must_use]
    pub const fn on(mut self, wash: Wash) -> Self {
        self.wash = wash;
        self
    }
}

impl From<Ink> for Style {
    fn from(ink: Ink) -> Self {
        Self::new(ink)
    }
}

/// What a cell holds.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
enum Content {
    /// Nothing but its wash.
    #[default]
    Blank,
    /// The top-left cell of a grapheme: its text, how many cells wide it is at
    /// ordinary size, and how many times that size it is drawn.
    Lead {
        text: Box<str>,
        width: u8,
        scale: u8,
    },
    /// A cell under a grapheme that starts to the left or above: how far away
    /// its lead is. Never written to a terminal on its own.
    Covered { left: u8, up: u8 },
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct Cell {
    content: Content,
    style: Style,
}

impl Cell {
    /// An empty cell that keeps this wash.
    fn blank(wash: Wash) -> Self {
        Self {
            content: Content::Blank,
            style: Style::new(Ink::default()).on(wash),
        }
    }
}

/// A screen of cells.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Buffer {
    columns: usize,
    rows: usize,
    cells: Vec<Cell>,
}

impl Buffer {
    /// An empty screen of this size.
    #[must_use]
    pub fn new(columns: usize, rows: usize) -> Self {
        Self {
            columns,
            rows,
            cells: vec![Cell::default(); columns * rows],
        }
    }

    #[must_use]
    pub const fn columns(&self) -> usize {
        self.columns
    }

    #[must_use]
    pub const fn rows(&self) -> usize {
        self.rows
    }

    /// The grapheme that starts in a cell and how it is drawn, if one does.
    ///
    /// An empty cell and a cell under a wide or scaled grapheme hold none.
    #[must_use]
    pub fn cell(&self, x: usize, y: usize) -> Option<(&str, Style)> {
        if x >= self.columns || y >= self.rows {
            return None;
        }
        let cell = &self.cells[y * self.columns + x];
        match &cell.content {
            Content::Lead { text, .. } => Some((text, cell.style)),
            Content::Blank | Content::Covered { .. } => None,
        }
    }

    /// The whole screen as a surface to draw on.
    pub fn canvas(&mut self) -> Canvas<'_> {
        Canvas {
            x: 0,
            y: 0,
            width: self.columns,
            height: self.rows,
            buffer: self,
        }
    }

    /// Write text at ordinary size, starting at a cell.
    ///
    /// The text is sanitized like every other human renderer's. It stops at
    /// the right edge without a mark; [`Canvas::put`] is the form that clips
    /// with an ellipsis. Returns the cells it advanced.
    pub fn put(&mut self, x: usize, y: usize, text: &str, style: Style) -> usize {
        self.put_scaled(x, y, text, style, 1)
    }

    /// Write text `scale` times the ordinary size: each grapheme takes a block
    /// `scale` rows high and `scale` times its width across.
    ///
    /// A grapheme that would cross the right or the bottom edge ends the text.
    /// Returns the cells it advanced.
    pub fn put_scaled(&mut self, x: usize, y: usize, text: &str, style: Style, scale: u8) -> usize {
        let scale = usize::from(scale.clamp(1, 7));
        let mut used = 0;
        for grapheme in clean(&plain(text)).graphemes(true) {
            let width = grapheme.width();
            // A mark with nothing to sit on has no cell of its own.
            if width == 0 {
                continue;
            }
            if !self.place(x + used, y, grapheme, width, scale, style) {
                break;
            }
            used += width * scale;
        }
        used
    }

    fn place(
        &mut self,
        x: usize,
        y: usize,
        grapheme: &str,
        width: usize,
        scale: usize,
        style: Style,
    ) -> bool {
        let (across, down) = (width * scale, scale);
        if x + across > self.columns || y + down > self.rows {
            return false;
        }
        for dy in 0..down {
            for dx in 0..across {
                self.erase(x + dx, y + dy);
            }
        }
        let wash = match style.wash {
            Wash::None => self.cells[y * self.columns + x].style.wash,
            wash => wash,
        };
        let style = Style { wash, ..style };
        for dy in 0..down {
            for dx in 0..across {
                self.cells[(y + dy) * self.columns + x + dx] = Cell {
                    content: if dx == 0 && dy == 0 {
                        Content::Lead {
                            text: grapheme.into(),
                            width: width as u8,
                            scale: scale as u8,
                        }
                    } else {
                        Content::Covered {
                            left: dx as u8,
                            up: dy as u8,
                        }
                    },
                    style,
                };
            }
        }
        true
    }

    /// Empty every cell of the grapheme that covers this cell, keeping washes.
    fn erase(&mut self, x: usize, y: usize) {
        let (x, y) = match self.cells[y * self.columns + x].content {
            Content::Blank => return,
            Content::Lead { .. } => (x, y),
            Content::Covered { left, up } => (x - usize::from(left), y - usize::from(up)),
        };
        let Content::Lead { width, scale, .. } = self.cells[y * self.columns + x].content else {
            return;
        };
        let (across, down) = (usize::from(width) * usize::from(scale), usize::from(scale));
        for dy in 0..down {
            for dx in 0..across {
                let cell = &mut self.cells[(y + dy) * self.columns + x + dx];
                *cell = Cell::blank(cell.style.wash);
            }
        }
    }

    /// Lay a wash behind a rectangle, under whatever is or will be drawn there.
    pub fn wash(&mut self, x: usize, y: usize, width: usize, height: usize, wash: Wash) {
        for row in y..(y + height).min(self.rows) {
            for column in x..(x + width).min(self.columns) {
                self.cells[row * self.columns + column].style.wash = wash;
            }
        }
    }

    /// The screen as plain text: one line per row, trailing blanks removed.
    ///
    /// A scaled grapheme is written once, in its top-left cell, and the rest
    /// of its block is blank, so the text shows where each block starts.
    #[must_use]
    pub fn text(&self) -> String {
        let mut out = String::new();
        for y in 0..self.rows {
            let mut line = String::new();
            let mut skip = 0;
            for x in 0..self.columns {
                if skip > 0 {
                    skip -= 1;
                    continue;
                }
                match &self.cells[y * self.columns + x].content {
                    Content::Lead { text, width, .. } => {
                        line.push_str(text);
                        skip = usize::from(*width) - 1;
                    }
                    Content::Blank | Content::Covered { .. } => line.push(' '),
                }
            }
            out.push_str(line.trim_end());
            out.push('\n');
        }
        out
    }

    /// The screen as one styled line per row, for a capture that is looked at
    /// with `cat` rather than replayed. Scaled text is written at ordinary
    /// size.
    #[must_use]
    pub fn lines(&self, theme: Theme) -> Vec<String> {
        (0..self.rows)
            .map(|y| {
                let mut line = String::new();
                let mut pen = None;
                let mut skip = 0;
                for x in 0..self.columns {
                    if skip > 0 {
                        skip -= 1;
                        continue;
                    }
                    let cell = &self.cells[y * self.columns + x];
                    if theme.color && pen != Some(cell.style) {
                        line.push_str(&sgr(cell.style, theme));
                        pen = Some(cell.style);
                    }
                    match &cell.content {
                        Content::Lead { text, width, .. } => {
                            line.push_str(text);
                            skip = usize::from(*width) - 1;
                        }
                        Content::Blank | Content::Covered { .. } => line.push(' '),
                    }
                }
                if theme.color {
                    line.push_str("\x1b[0m");
                    line
                } else {
                    line.trim_end().to_owned()
                }
            })
            .collect()
    }

    /// The bytes that make a terminal showing `shown` show this buffer.
    ///
    /// Only cells that differ are written, each addressed where it is, so
    /// nothing scrolls and nothing between two changes is touched. With no
    /// `shown`, or one of another size, the screen is cleared first and every
    /// cell that is not empty is written. The caller brackets the bytes in
    /// synchronized output; [`Buffer::present`] does.
    ///
    /// A scaled grapheme is written with the text sizing sequence, so a buffer
    /// that holds one must only be painted on a terminal that reported it —
    /// see [`crate::scale`].
    ///
    /// Where scaled text changes, the whole screen is painted again. Written
    /// cell by cell over what was there, a statement on a real kitty lost its
    /// first words where the larger figure before it had stood, and text from
    /// the screen before stayed on the statement's lower row: the terminal
    /// holds the right cells and does not draw all of them again. A cleared
    /// screen is drawn whole, and the synchronized output it is sent in shows
    /// it as one frame.
    #[must_use]
    pub fn paint(&self, shown: Option<&Self>, theme: Theme) -> String {
        let mut out = String::new();
        let shown = shown
            .filter(|s| s.columns == self.columns && s.rows == self.rows)
            .filter(|shown| !self.scaled_text_moved(shown));
        // What the terminal holds as the writes below land. `None` is a cell a
        // write emptied as a side effect, which must be written again.
        let mut model: Vec<Option<Cell>> = match shown {
            Some(shown) => shown.cells.iter().cloned().map(Some).collect(),
            None => {
                out.push_str("\x1b[0m\x1b[2J");
                vec![Some(Cell::default()); self.cells.len()]
            }
        };
        let mut cursor = None;
        let mut pen = None;
        for y in 0..self.rows {
            for x in 0..self.columns {
                let index = y * self.columns + x;
                let cell = &self.cells[index];
                if matches!(cell.content, Content::Covered { .. })
                    || model[index].as_ref() == Some(cell)
                {
                    continue;
                }
                if cursor != Some((x, y)) {
                    let _ = write!(out, "\x1b[{};{}H", y + 1, x + 1);
                }
                // An empty cell shows only its wash, so whatever ink is set
                // will do for it.
                let set = pen.is_some_and(|pen: Style| match cell.content {
                    Content::Blank => pen.wash == cell.style.wash,
                    _ => pen == cell.style,
                });
                if theme.color && !set {
                    out.push_str(&sgr(cell.style, theme));
                    pen = Some(cell.style);
                }
                let (across, down, simple) = match &cell.content {
                    Content::Lead { text, width, scale } if *scale > 1 => {
                        out.push_str(&crate::scale::sequence(text, *scale, *width));
                        let scale = usize::from(*scale);
                        (usize::from(*width) * scale, scale, false)
                    }
                    Content::Lead { text, width, .. } => {
                        out.push_str(text);
                        (
                            usize::from(*width),
                            1,
                            *width == 1 && text.chars().count() == 1,
                        )
                    }
                    Content::Blank | Content::Covered { .. } => {
                        out.push(' ');
                        (1, 1, true)
                    }
                };
                // A terminal empties every cell of a grapheme that a write
                // lands on, and this write may have landed on several.
                for dy in 0..down {
                    for dx in 0..across {
                        forget(&mut model, self.columns, x + dx, y + dy);
                    }
                }
                for dy in 0..down {
                    for dx in 0..across {
                        let covered = (y + dy) * self.columns + x + dx;
                        model[covered] = Some(self.cells[covered].clone());
                    }
                }
                // After anything but one ordinary character the terminal may
                // not agree where the cursor is, so the next cell is addressed.
                cursor = (simple && x + 1 < self.columns).then_some((x + 1, y));
            }
        }
        if pen.is_some() {
            out.push_str("\x1b[0m");
        }
        out
    }

    /// Whether a cell that differs from `shown`, a buffer of the same size,
    /// is part of scaled text in either of the two.
    pub(crate) fn scaled_text_moved(&self, shown: &Self) -> bool {
        (0..self.cells.len()).any(|index| {
            self.cells[index] != shown.cells[index]
                && (self.scaled_at(index) || shown.scaled_at(index))
        })
    }

    /// Whether the cell at `index` is part of a grapheme drawn larger than
    /// the ordinary size.
    fn scaled_at(&self, index: usize) -> bool {
        let lead = match self.cells[index].content {
            Content::Blank => return false,
            Content::Lead { .. } => index,
            Content::Covered { left, up } => {
                index - usize::from(up) * self.columns - usize::from(left)
            }
        };
        matches!(self.cells[lead].content, Content::Lead { scale, .. } if scale > 1)
    }

    /// Paint inside synchronized output and flush, so a terminal that
    /// supports it shows the new screen in one frame.
    pub fn present(
        &self,
        out: &mut impl std::io::Write,
        shown: Option<&Self>,
        theme: Theme,
    ) -> std::io::Result<()> {
        write!(out, "\x1b[?2026h{}\x1b[?2026l", self.paint(shown, theme))?;
        out.flush()
    }

    /// The screen as a self-contained HTML page.
    #[must_use]
    pub fn html(&self, title: &str) -> String {
        page(title, &[(title, self)])
    }

    /// This screen's rows as HTML, without a document around them.
    fn markup(&self) -> String {
        let mut out = String::new();
        for y in 0..self.rows {
            out.push_str("<div class=\"r\">");
            let mut x = 0;
            while x < self.columns {
                let cell = &self.cells[y * self.columns + x];
                let (text, width) = flow(cell);
                // A grapheme that is not one ordinary character gets a box of
                // its own, so a font that draws it too wide cannot push the
                // rest of the row off the grid.
                let alone = width > 1 || text.chars().count() != 1;
                let look = (cell.style, inked(cell));
                let mut run = String::from(text);
                let mut cells = width;
                x += width;
                while !alone && x < self.columns {
                    let next = &self.cells[y * self.columns + x];
                    let (text, width) = flow(next);
                    let next_look = (next.style, inked(next));
                    let same = next_look == look
                        || (!look.1 && !next_look.1 && next.style.wash == cell.style.wash);
                    if !same || width > 1 || text.chars().count() != 1 {
                        break;
                    }
                    run.push_str(text);
                    cells += width;
                    x += width;
                }
                let _ = write!(
                    out,
                    "<span style=\"width:{cells}ch{}\">{}</span>",
                    css(cell.style, look.1),
                    escape(&run)
                );
            }
            // Scaled text lies over the row it starts in.
            for x in 0..self.columns {
                let cell = &self.cells[y * self.columns + x];
                if let Content::Lead { text, scale, .. } = &cell.content
                    && *scale > 1
                {
                    let _ = write!(
                        out,
                        "<b style=\"left:{x}ch\"><i style=\"font-size:{scale}00%;line-height:{}px{}\">{}</i></b>",
                        usize::from(*scale) * ROW_PIXELS,
                        css(Style::new(cell.style.ink).bold_if(cell.style.bold), true),
                        escape(text)
                    );
                }
            }
            out.push_str("</div>\n");
        }
        out
    }
}

impl Style {
    const fn bold_if(mut self, bold: bool) -> Self {
        self.bold = bold;
        self
    }
}

/// The height of one row in the HTML form, which the design draws at 22px.
const ROW_PIXELS: usize = 22;

/// Several screens as one self-contained HTML page, each under its name.
///
/// Nothing is fetched: the page names the faces the image ships and falls back
/// to whatever monospace face the browser has. Colours are the truecolor
/// palette, on the background the Herda theme gives a terminal.
#[must_use]
pub fn page(title: &str, screens: &[(&str, &Buffer)]) -> String {
    let mut out = format!(
        "<!doctype html>\n<html lang=\"en\">\n<head>\n<meta charset=\"utf-8\">\n\
         <title>{}</title>\n<style>\n\
         body{{margin:0;padding:28px;background:#07090a;color:#8f9496;\
         font:14px/{ROW_PIXELS}px 'JetBrains Mono','RobotoMono Nerd Font Mono',\
         'DejaVu Sans Mono',ui-monospace,Menlo,monospace;font-variant-ligatures:none}}\n\
         h1{{font:inherit;margin:0 0 {ROW_PIXELS}px}}\n\
         .t{{display:inline-block;margin:0 0 44px;background:#131618;\
         border:1px solid #2a2e32;border-radius:8px;overflow:hidden}}\n\
         .r{{position:relative;height:{ROW_PIXELS}px;white-space:pre}}\n\
         .r>span{{display:inline-block;height:{ROW_PIXELS}px;overflow:hidden;\
         vertical-align:top;white-space:pre}}\n\
         .r>b{{position:absolute;top:0;font-weight:inherit}}\n\
         .r>b>i{{display:block;font-style:normal;white-space:pre}}\n\
         </style>\n</head>\n<body>\n",
        escape(title)
    );
    for (name, buffer) in screens {
        let _ = write!(
            out,
            "<h1>{} · {} × {}</h1>\n<div class=\"t\" style=\"width:{}ch\">\n{}</div>\n<br>\n",
            escape(name),
            buffer.columns,
            buffer.rows,
            buffer.columns,
            buffer.markup()
        );
    }
    out.push_str("</body>\n</html>\n");
    out
}

/// Whether a cell puts a character of its own into the flow of its row.
fn inked(cell: &Cell) -> bool {
    matches!(&cell.content, Content::Lead { scale, .. } if *scale <= 1)
}

/// What a cell contributes to the ordinary flow of its row, and how many
/// cells of it that takes up.
fn flow(cell: &Cell) -> (&str, usize) {
    match &cell.content {
        Content::Lead { text, width, scale } if *scale <= 1 => (text, usize::from(*width)),
        _ => (" ", 1),
    }
}

fn css(style: Style, inked: bool) -> String {
    let mut out = String::new();
    if inked {
        if let Some((r, g, b)) = rgb(style.ink.code(true), "38;2;") {
            let _ = write!(out, ";color:#{r:02x}{g:02x}{b:02x}");
        }
        if style.bold {
            out.push_str(";font-weight:700");
        }
    }
    if let Some((r, g, b)) = rgb(style.wash.code(true), "48;2;") {
        let _ = write!(out, ";background:#{r:02x}{g:02x}{b:02x}");
    }
    out
}

/// The colour a truecolor escape body names after `lead`, so the HTML form
/// reads the palette rather than repeating it.
fn rgb(code: &str, lead: &str) -> Option<(u8, u8, u8)> {
    let mut parts = code.strip_prefix(lead)?.split(';');
    let mut next = || parts.next()?.parse().ok();
    Some((next()?, next()?, next()?))
}

fn escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

/// The SGR sequence that selects a style, or nothing with colour off.
fn sgr(style: Style, theme: Theme) -> String {
    if !theme.color {
        return String::new();
    }
    let mut out = String::from("\x1b[0");
    if style.bold {
        out.push_str(";1");
    }
    out.push(';');
    out.push_str(style.ink.code(theme.truecolor));
    let wash = style.wash.code(theme.truecolor);
    if !wash.is_empty() {
        out.push(';');
        out.push_str(wash);
    }
    out.push('m');
    out
}

/// Mark every cell of the grapheme the terminal holds at this cell as unknown.
fn forget(model: &mut [Option<Cell>], columns: usize, x: usize, y: usize) {
    let (x, y) = match model[y * columns + x].as_ref().map(|cell| &cell.content) {
        Some(Content::Lead { .. }) => (x, y),
        Some(Content::Covered { left, up }) => (x - usize::from(*left), y - usize::from(*up)),
        Some(Content::Blank) | None => return,
    };
    let Some(Content::Lead { width, scale, .. }) =
        model[y * columns + x].as_ref().map(|cell| &cell.content)
    else {
        return;
    };
    let (across, down) = (
        usize::from(*width) * usize::from(*scale),
        usize::from(*scale),
    );
    for dy in 0..down {
        for dx in 0..across {
            if let Some(cell) = model.get_mut((y + dy) * columns + x + dx) {
                *cell = None;
            }
        }
    }
}

/// A rectangle of a [`Buffer`] that a piece of the screen draws itself into.
///
/// Coordinates are the rectangle's own, and nothing drawn through a canvas
/// lands outside it, so a piece cannot write over its neighbour.
pub struct Canvas<'a> {
    buffer: &'a mut Buffer,
    x: usize,
    y: usize,
    width: usize,
    height: usize,
}

impl Canvas<'_> {
    #[must_use]
    pub const fn width(&self) -> usize {
        self.width
    }

    #[must_use]
    pub const fn height(&self) -> usize {
        self.height
    }

    /// A rectangle inside this one, clipped to it.
    pub fn part(&mut self, x: usize, y: usize, width: usize, height: usize) -> Canvas<'_> {
        let x = x.min(self.width);
        let y = y.min(self.height);
        Canvas {
            x: self.x + x,
            y: self.y + y,
            width: width.min(self.width - x),
            height: height.min(self.height - y),
            buffer: self.buffer,
        }
    }

    /// Write text from a cell to the right edge, clipped with an ellipsis.
    /// Returns the cells it took.
    pub fn put(&mut self, x: usize, y: usize, text: &str, style: impl Into<Style>) -> usize {
        self.fit(x, y, self.width.saturating_sub(x), text, style)
    }

    /// Write text into at most `width` cells, clipped with an ellipsis.
    /// Returns the cells it took.
    pub fn fit(
        &mut self,
        x: usize,
        y: usize,
        width: usize,
        text: &str,
        style: impl Into<Style>,
    ) -> usize {
        let width = width.min(self.width.saturating_sub(x));
        if y >= self.height || width == 0 {
            return 0;
        }
        self.buffer
            .put(self.x + x, self.y + y, &clip(text, width), style.into())
    }

    /// Write text so that it ends just before `end`, clipped with an ellipsis
    /// if it would start left of `start`. Returns the cell it starts in.
    pub fn end(
        &mut self,
        start: usize,
        end: usize,
        y: usize,
        text: &str,
        style: impl Into<Style>,
    ) -> usize {
        let end = end.min(self.width);
        let text = clip(text, end.saturating_sub(start));
        let x = end - text.width();
        self.fit(x, y, end - x, &text, style);
        x
    }

    /// Write text `scale` times the ordinary size. Returns the cells it took
    /// across; text that does not fit is cut at the last whole grapheme.
    pub fn scaled(
        &mut self,
        x: usize,
        y: usize,
        text: &str,
        style: impl Into<Style>,
        scale: u8,
    ) -> usize {
        let rows = usize::from(scale.max(1));
        if x >= self.width || y + rows > self.height {
            return 0;
        }
        let room = (self.width - x) / rows;
        let mut fitting = String::new();
        let mut used = 0;
        for grapheme in clean(&plain(text)).graphemes(true) {
            if used + grapheme.width() > room {
                break;
            }
            used += grapheme.width();
            fitting.push_str(grapheme);
        }
        self.buffer
            .put_scaled(self.x + x, self.y + y, &fitting, style.into(), scale)
    }

    /// Fill a run of cells with one character.
    pub fn repeat(
        &mut self,
        x: usize,
        y: usize,
        width: usize,
        character: char,
        style: impl Into<Style>,
    ) {
        let width = width.min(self.width.saturating_sub(x));
        if y >= self.height || width == 0 {
            return;
        }
        let text: String = std::iter::repeat_n(character, width).collect();
        self.buffer.put(self.x + x, self.y + y, &text, style.into());
    }

    /// Lay a wash behind part of the rectangle.
    pub fn wash(&mut self, x: usize, y: usize, width: usize, height: usize, wash: Wash) {
        let x = x.min(self.width);
        let y = y.min(self.height);
        self.buffer.wash(
            self.x + x,
            self.y + y,
            width.min(self.width - x),
            height.min(self.height - y),
            wash,
        );
    }
}

#[cfg(test)]
pub(crate) mod replay {
    //! A terminal small enough to read, for checking what [`Buffer::paint`]
    //! leaves on one.
    //!
    //! It follows the rules a real one does for the sequences the painter
    //! emits: an address, a clear, a style, a character, and a scaled
    //! grapheme — including that writing onto any cell of a wide or scaled
    //! grapheme empties all of it.

    use unicode_segmentation::UnicodeSegmentation;
    use unicode_width::UnicodeWidthStr;

    #[derive(Clone, Debug, Default, PartialEq, Eq)]
    pub(crate) struct Shown {
        pub text: String,
        pub scale: usize,
        /// The style sequence in force when the cell was written.
        pub pen: String,
        owner: Option<(usize, usize)>,
        extent: (usize, usize),
    }

    pub(crate) struct Terminal {
        pub columns: usize,
        pub rows: usize,
        pub cells: Vec<Shown>,
        cursor: (usize, usize),
        pen: String,
        /// Sequences the painter has no business sending.
        pub unknown: Vec<String>,
    }

    impl Terminal {
        pub(crate) fn new(columns: usize, rows: usize) -> Self {
            Self {
                columns,
                rows,
                cells: vec![Shown::default(); columns * rows],
                cursor: (0, 0),
                pen: String::new(),
                unknown: Vec::new(),
            }
        }

        fn empty(&mut self, x: usize, y: usize) {
            let Some((ox, oy)) = self.cells[y * self.columns + x].owner else {
                return;
            };
            let (across, down) = self.cells[oy * self.columns + ox].extent;
            for dy in 0..down {
                for dx in 0..across {
                    self.cells[(oy + dy) * self.columns + ox + dx] = Shown::default();
                }
            }
        }

        fn draw(&mut self, grapheme: &str, width: usize, scale: usize) {
            let (x, y) = self.cursor;
            let (across, down) = (width * scale, scale);
            assert!(
                x + across <= self.columns && y + down <= self.rows,
                "{grapheme:?} at {x},{y} leaves the screen"
            );
            for dy in 0..down {
                for dx in 0..across {
                    self.empty(x + dx, y + dy);
                }
            }
            for dy in 0..down {
                for dx in 0..across {
                    self.cells[(y + dy) * self.columns + x + dx] = Shown {
                        text: if dx == 0 && dy == 0 {
                            grapheme.to_owned()
                        } else {
                            String::new()
                        },
                        scale,
                        pen: self.pen.clone(),
                        owner: Some((x, y)),
                        extent: (across, down),
                    };
                }
            }
            self.cursor.0 += across;
        }

        pub(crate) fn feed(&mut self, bytes: &str) {
            let mut rest = bytes;
            while !rest.is_empty() {
                if let Some(after) = rest.strip_prefix("\x1b[") {
                    let end = after
                        .find(|c: char| ('@'..='~').contains(&c))
                        .expect("an unterminated control sequence");
                    let (body, last) = (&after[..end], &after[end..=end]);
                    match last {
                        "H" => {
                            let (row, column) = body.split_once(';').expect("an address");
                            self.cursor = (
                                column.parse::<usize>().unwrap() - 1,
                                row.parse::<usize>().unwrap() - 1,
                            );
                        }
                        "J" if body == "2" => {
                            self.cells.fill(Shown::default());
                        }
                        "m" => {
                            self.pen = if body == "0" {
                                String::new()
                            } else {
                                body.to_owned()
                            };
                        }
                        _ => self.unknown.push(format!("CSI {body}{last}")),
                    }
                    rest = &after[end + 1..];
                } else if let Some(after) = rest.strip_prefix("\x1b]66;") {
                    let end = after
                        .find("\x1b\\")
                        .expect("an unterminated sizing sequence");
                    let (keys, text) = after[..end].split_once(';').expect("sizing metadata");
                    let key = |name: &str| {
                        keys.split(':')
                            .find_map(|pair| pair.strip_prefix(name)?.strip_prefix('='))
                            .map_or(0, |value| value.parse::<usize>().unwrap())
                    };
                    assert_eq!(text.graphemes(true).count(), 1, "one grapheme: {text:?}");
                    self.draw(text, key("w"), key("s"));
                    rest = &after[end + 2..];
                } else if rest.starts_with('\x1b') {
                    self.unknown.push(rest.chars().take(12).collect());
                    rest = &rest[1..];
                } else {
                    let grapheme = rest.graphemes(true).next().unwrap();
                    self.draw(grapheme, grapheme.width().max(1), 1);
                    rest = &rest[grapheme.len()..];
                }
            }
        }

        /// The screen as text, in the form [`super::Buffer::text`] writes.
        pub(crate) fn text(&self) -> String {
            let mut out = String::new();
            for y in 0..self.rows {
                let mut line = String::new();
                let mut skip = 0;
                for x in 0..self.columns {
                    if skip > 0 {
                        skip -= 1;
                        continue;
                    }
                    let cell = &self.cells[y * self.columns + x];
                    if cell.text.is_empty() {
                        line.push(' ');
                    } else {
                        line.push_str(&cell.text);
                        skip = cell.text.width().max(1) - 1;
                    }
                }
                out.push_str(line.trim_end());
                out.push('\n');
            }
            out
        }
    }
}

#[cfg(test)]
mod tests {
    use super::replay::Terminal;
    use super::*;

    fn shown(buffer: &Buffer, theme: Theme) -> Terminal {
        let mut terminal = Terminal::new(buffer.columns(), buffer.rows());
        terminal.feed(&buffer.paint(None, theme));
        terminal
    }

    #[test]
    fn text_lands_in_the_cells_it_was_put_in() {
        let mut buffer = Buffer::new(12, 3);
        assert_eq!(buffer.put(2, 1, "pkg", Style::new(Ink::Gold).bold()), 3);
        assert_eq!(buffer.text(), "\n  pkg\n\n");
        // The right edge ends the text, in whole graphemes.
        assert_eq!(buffer.put(9, 0, "abcdef", Ink::Text.into()), 3);
        assert_eq!(buffer.put(10, 2, "a界b", Ink::Text.into()), 1);
        assert_eq!(buffer.text(), "         abc\n  pkg\n          a\n");
    }

    #[test]
    fn untrusted_text_cannot_carry_a_control_into_the_buffer() {
        let mut buffer = Buffer::new(40, 1);
        buffer.put(
            0,
            0,
            "name\x1b[2J\x1b]52;c;x\x07\r\u{202e}end",
            Ink::Text.into(),
        );
        for theme in [Theme::PLAIN, Theme::FULL, Theme::INDEXED] {
            let bytes = buffer.paint(None, theme);
            assert!(!bytes.contains("\x1b]52"), "{bytes:?}");
            assert!(!bytes.contains(['\r', '\u{202e}', '\x07']), "{bytes:?}");
            assert_eq!(
                bytes.matches("\x1b[2J").count(),
                1,
                "only the painter's clear"
            );
        }
        assert!(buffer.html("x").contains("name"));
        assert!(!buffer.html("<x>").contains("<x>"));
    }

    /// Wide and combining graphemes keep the cells after them where they were
    /// measured.
    #[test]
    fn wide_and_combining_graphemes_keep_the_grid() {
        let mut buffer = Buffer::new(20, 2);
        assert_eq!(buffer.put(0, 0, "界面e\u{301}x", Ink::Text.into()), 6);
        buffer.put(6, 0, "|", Ink::Rule.into());
        assert_eq!(buffer.put(0, 1, "👩‍💻ab", Ink::Text.into()), 4);
        buffer.put(6, 1, "|", Ink::Rule.into());
        assert_eq!(buffer.text(), "界面e\u{301}x|\n👩‍💻ab  |\n");
        let terminal = shown(&buffer, Theme::PLAIN);
        assert_eq!(terminal.text(), buffer.text());
        assert_eq!(terminal.cells[6].text, "|");
        assert_eq!(terminal.cells[20 + 6].text, "|");
        // A lone combining mark has no cell.
        assert_eq!(buffer.put(10, 0, "\u{301}", Ink::Text.into()), 0);
    }

    #[test]
    fn writing_over_part_of_a_grapheme_removes_all_of_it() {
        let mut buffer = Buffer::new(10, 3);
        buffer.wash(0, 0, 10, 3, Wash::Plate);
        buffer.put_scaled(0, 0, "A界", Ink::Text.into(), 2);
        assert_eq!(buffer.text(), "A 界\n\n\n");
        buffer.put(1, 1, "x", Ink::Text.into());
        assert_eq!(buffer.text(), "  界\n x\n\n");
        buffer.put(5, 0, "y", Ink::Text.into());
        assert_eq!(buffer.text(), "     y\n x\n\n");
        // The plate is still whole under what was removed.
        assert!(
            buffer
                .cells
                .iter()
                .all(|cell| cell.style.wash == Wash::Plate)
        );
    }

    #[test]
    fn colour_off_paints_the_same_cells_with_no_style_at_all() {
        let mut buffer = Buffer::new(16, 2);
        buffer.wash(0, 0, 7, 1, Wash::Accent);
        buffer.put(1, 0, "Enter", Style::new(Ink::Gold).bold());
        buffer.put(8, 0, "Install", Ink::Text.into());
        // A reset before the clear, and after that addresses and characters.
        assert_eq!(
            buffer.paint(None, Theme::PLAIN),
            "\x1b[0m\x1b[2J\x1b[1;1H Enter \x1b[1;9HInstall"
        );
        let full = shown(&buffer, Theme::FULL);
        let indexed = shown(&buffer, Theme::INDEXED);
        let bare = shown(&buffer, Theme::PLAIN);
        assert_eq!(full.text(), buffer.text());
        assert_eq!(indexed.text(), buffer.text());
        assert_eq!(bare.text(), buffer.text());
        assert_eq!(full.cells[1].pen, "0;1;38;2;255;188;80;48;2;57;49;33");
        assert_eq!(indexed.cells[1].pen, "0;1;38;5;215;48;5;238");
        assert_eq!(full.cells[8].pen, "0;38;2;242;242;239");
        for terminal in [full, indexed, bare] {
            assert!(terminal.unknown.is_empty(), "{:?}", terminal.unknown);
        }
    }

    /// A second paint writes only what moved, and leaves the terminal holding
    /// exactly the new buffer. Where scaled text came, went or changed, it is
    /// the whole screen, from a clear.
    #[test]
    fn a_repaint_touches_only_what_changed() {
        let mut first = Buffer::new(30, 6);
        first.put_scaled(0, 0, "67%", Style::new(Ink::Text).bold(), 3);
        first.put(12, 2, "22.88 of 33.78 MiB", Ink::Text.into());
        first.put(0, 4, "inkscape", Ink::Text.into());
        let mut second = first.clone();
        second.put_scaled(3, 0, "8", Style::new(Ink::Text).bold(), 3);
        second.put(12, 2, "23.10", Ink::Text.into());

        let mut terminal = shown(&first, Theme::FULL);
        let mut moved = first.clone();
        moved.put(12, 2, "23.10", Ink::Text.into());
        let bytes = moved.paint(Some(&first), Theme::FULL);
        assert!(!bytes.contains("\x1b[2J"), "{bytes:?}");
        assert!(
            !bytes.contains("inkscape") && !bytes.contains("\x1b]66;"),
            "{bytes:?}"
        );
        terminal.feed(&bytes);
        assert_eq!(terminal.text(), moved.text());

        // Scaled text that changes paints the screen whole.
        let bytes = second.paint(Some(&moved), Theme::FULL);
        assert!(bytes.starts_with("\x1b[0m\x1b[2J"), "{bytes:?}");
        assert_eq!(bytes.matches("\x1b]66;").count(), 3, "{bytes:?}");
        terminal.feed(&bytes);
        assert_eq!(terminal.text(), second.text());
        assert_eq!(terminal.cells[3].scale, 3);

        // Scaled text gives way to ordinary text, and the reverse.
        let mut third = Buffer::new(30, 6);
        third.put(0, 0, "Verifying packages", Style::new(Ink::Text).bold());
        third.put(1, 1, "x", Ink::Muted.into());
        third.put(0, 4, "inkscape", Ink::Text.into());
        terminal.feed(&third.paint(Some(&second), Theme::FULL));
        assert_eq!(terminal.text(), third.text());
        terminal.feed(&first.paint(Some(&third), Theme::FULL));
        assert_eq!(terminal.text(), first.text());
        assert!(first.paint(Some(&first), Theme::FULL).is_empty());

        // Another size cannot be patched: it starts from a cleared screen.
        assert!(
            Buffer::new(31, 6)
                .paint(Some(&first), Theme::PLAIN)
                .starts_with("\x1b[0m\x1b[2J")
        );
    }

    /// The ink is set once for a run of words, not again for each space
    /// between them.
    #[test]
    fn an_empty_cell_is_painted_in_the_ink_already_set() {
        let mut first = Buffer::new(24, 1);
        first.put(0, 0, "xxxxxxxxxxxxxxxxxxxxxxxx", Ink::Faint.into());
        let mut second = Buffer::new(24, 1);
        second.put(0, 0, "Log", Ink::Muted.into());
        second.put(6, 0, "Help", Ink::Muted.into());
        second.wash(12, 0, 3, 1, Wash::Key);
        let bytes = second.paint(Some(&first), Theme::FULL);
        assert_eq!(
            bytes,
            "\x1b[1;1H\x1b[0;38;2;180;185;187mLog   Help  \
             \x1b[0;38;2;242;242;239;48;2;37;40;41m   \
             \x1b[0;38;2;242;242;239m         \x1b[0m"
        );
        let mut terminal = shown(&first, Theme::FULL);
        terminal.feed(&bytes);
        assert_eq!(terminal.text(), second.text());
    }

    #[test]
    fn a_frame_is_presented_inside_synchronized_output() {
        let mut buffer = Buffer::new(4, 1);
        buffer.put(0, 0, "ok", Ink::Green.into());
        let mut out = Vec::new();
        buffer.present(&mut out, None, Theme::PLAIN).unwrap();
        let bytes = String::from_utf8(out).unwrap();
        assert!(
            bytes.starts_with("\x1b[?2026h") && bytes.ends_with("\x1b[?2026l"),
            "{bytes:?}"
        );
    }

    #[test]
    fn a_canvas_keeps_a_piece_inside_its_rectangle() {
        let mut buffer = Buffer::new(20, 4);
        let mut canvas = buffer.canvas();
        let mut part = canvas.part(4, 1, 8, 2);
        assert_eq!((part.width(), part.height()), (8, 2));
        assert_eq!(part.put(0, 0, "a long package name", Ink::Text), 8);
        assert_eq!(part.put(0, 5, "below", Ink::Text), 0);
        assert_eq!(part.end(0, 8, 1, "9.9s", Ink::Faint), 4);
        part.repeat(0, 1, 3, '─', Ink::Rule);
        part.wash(0, 0, 99, 99, Wash::Plate);
        assert_eq!(part.scaled(0, 0, "ABCDE", Ink::Text, 2), 8);
        assert_eq!(buffer.text(), "\n    A B C D\n\n\n");
        let washed = buffer
            .cells
            .iter()
            .filter(|cell| cell.style.wash == Wash::Plate)
            .count();
        assert_eq!(washed, 16);
    }

    #[test]
    fn the_page_is_self_contained_and_draws_scaled_text_large() {
        let mut buffer = Buffer::new(24, 3);
        buffer.wash(0, 2, 7, 1, Wash::Key);
        buffer.put_scaled(0, 0, "Ok", Style::new(Ink::Gold).bold(), 2);
        buffer.put(1, 2, "界 <q>", Ink::Text.into());
        let html = buffer.html("review");
        assert!(!html.contains("http"), "nothing is fetched");
        assert!(html.contains("font-size:200%"), "{html}");
        assert!(html.contains("color:#ffbc50"), "{html}");
        assert!(html.contains("background:#252829"), "{html}");
        assert!(html.contains("&lt;q&gt;"), "{html}");
        assert!(
            html.contains("<span style=\"width:2ch"),
            "a wide grapheme has its own box"
        );
        assert_eq!(html.matches("<div class=\"r\">").count(), 3);
    }

    #[test]
    fn styled_lines_are_the_plain_rows_when_colour_is_off() {
        let mut buffer = Buffer::new(10, 2);
        buffer.put(0, 0, "ok", Ink::Green.into());
        assert_eq!(buffer.lines(Theme::PLAIN), ["ok", ""]);
        assert!(buffer.lines(Theme::FULL)[0].contains(Ink::Green.code(true)));
    }
}

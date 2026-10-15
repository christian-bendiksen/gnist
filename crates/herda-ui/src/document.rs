//! The scrollback document: what a command leaves behind when it is done.
//!
//! One shape for every Herda tool. A heading naming the tool and what it did,
//! the seam under it, then sections of text, fields and tables, and a footer
//! behind a flat rule. Rendering cannot run or approve anything — a `Document`
//! is a pure function of what it was told.

use unicode_width::UnicodeWidthStr;

use crate::ink::{Ink, glyph};
use crate::terminal::{Theme, clean, fold};
use crate::width::Layout;

/// Which edge a column's cells sit against.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Align {
    /// Names, versions, states.
    #[default]
    Left,
    /// Figures, so their digits line up and the eye can compare them.
    Right,
}

/// One column of a table.
#[derive(Clone, Debug)]
pub struct Column {
    /// The heading, rendered in faint capitals.
    pub label: String,
    /// Which edge the cells sit against.
    pub align: Align,
    /// The narrowest terminal that still shows this column.
    ///
    /// The ladder gives up columns in a stated order rather than squeezing
    /// every column until none of them is readable: [`crate::width::SIZE_COLUMN`]
    /// here drops SIZE first, because it is the widest optional column and its
    /// total is in the footer anyway.
    pub keep_above: usize,
}

impl Column {
    /// A column that is always shown.
    #[must_use]
    pub fn new(label: &str) -> Self {
        Self {
            label: label.to_ascii_uppercase(),
            align: Align::Left,
            keep_above: 0,
        }
    }

    /// Right-align this column's cells.
    #[must_use]
    pub fn right(mut self) -> Self {
        self.align = Align::Right;
        self
    }

    /// Drop this column on terminals narrower than `columns`.
    #[must_use]
    pub fn keep_above(mut self, columns: usize) -> Self {
        self.keep_above = columns;
        self
    }
}

/// One row of a table: an optional marker, then a cell per column.
#[derive(Clone, Debug, Default)]
pub struct Row {
    /// `+`, `-`, `~` or a status glyph, in the gutter to the left.
    pub marker: Option<(String, Ink)>,
    /// One per [`Column`], in order. A short row is padded with blanks.
    pub cells: Vec<(String, Ink)>,
}

impl Row {
    /// A row with no marker.
    #[must_use]
    pub fn new(cells: Vec<(String, Ink)>) -> Self {
        Self {
            marker: None,
            cells,
        }
    }

    /// Put a marker in the left gutter.
    #[must_use]
    pub fn marked(mut self, marker: &str, ink: Ink) -> Self {
        self.marker = Some((marker.to_owned(), ink));
        self
    }
}

/// A bounded scrollback document. Rendering cannot run or approve an operation.
pub struct Document {
    pub theme: Theme,
    /// The usable width inside the gutter. See [`Layout::content`].
    pub width: usize,
    layout: Layout,
    lines: Vec<String>,
}

impl Document {
    /// Open a document. `columns` is the terminal's full width; the gutter
    /// and the ladder are applied here so no caller subtracts its own.
    pub fn new(app: &str, title: &str, badge: &str, columns: usize, theme: Theme) -> Self {
        let layout = Layout::of(columns, Layout::detect().rows());
        let width = layout.content();
        let title = clean(title);
        let mut lines = vec![String::new()];
        let prefix = app.width() + 2;
        if title.width() + prefix <= width {
            lines.push(theme.heading_for(app, &title, badge, width));
        } else {
            lines.push(theme.heading_for(app, "", badge, width));
            for line in fold(&title, width) {
                lines.push(theme.paint(&line, Ink::Text, true));
            }
        }
        lines.extend([String::new(), theme.seam(width)]);
        Self {
            theme,
            width,
            layout,
            lines,
        }
    }

    pub fn section(&mut self, title: &str) {
        self.lines.push(String::new());
        for line in fold(title, self.width) {
            self.lines.push(self.theme.paint(&line, Ink::Text, true));
        }
        self.lines.push(String::new());
    }

    pub fn text(&mut self, text: &str, ink: Ink) {
        for line in fold(text, self.width) {
            self.lines.push(self.theme.paint(&line, ink, false));
        }
    }

    pub fn field(&mut self, label: &str, value: &str, ink: Ink) {
        let label_width = 18.min(self.width / 3);
        if self.width < 48 || label.width() > label_width {
            self.text(label, Ink::Muted);
            self.text(value, ink);
        } else {
            for (index, line) in fold(value, self.width - label_width - 2).iter().enumerate() {
                self.lines.push(format!(
                    "{}  {}",
                    self.theme.cell(
                        if index == 0 { label } else { "" },
                        label_width,
                        Ink::Muted,
                        false
                    ),
                    self.theme.paint(line, ink, false)
                ));
            }
        }
    }

    pub fn item(&mut self, title: &str, detail: &str, status: &str, ink: Ink) {
        let status = clean(status);
        if self.lines.last().is_some_and(|line| !line.is_empty()) {
            self.lines.push(String::new());
        }
        let available = self.width.saturating_sub(status.width() + 2);
        if !status.is_empty() && available >= 24 && title.width() <= available {
            self.lines.push(format!(
                "{}  {}",
                self.theme.cell(title, available, Ink::Text, true),
                self.theme.paint(&status, ink, false)
            ));
        } else {
            for line in fold(title, self.width) {
                self.lines.push(self.theme.paint(&line, Ink::Text, true));
            }
            if !status.is_empty() {
                self.text(&status, ink);
            }
        }
        if !detail.is_empty() {
            self.text(detail, Ink::Muted);
        }
    }

    pub fn footer(&mut self, text: &str) {
        self.lines
            .extend([String::new(), self.theme.rule(self.width)]);
        self.text(text, Ink::Muted);
    }

    pub fn render(self) -> String {
        if self.layout.columns() <= crate::width::GUTTER {
            return String::new();
        }
        self.lines
            .into_iter()
            .map(|line| {
                if line.is_empty() {
                    line
                } else {
                    format!("  {line}")
                }
            })
            .collect::<Vec<_>>()
            .join("\n")
            + "\n\n"
    }
}

impl Document {
    /// The gold advisory rail: what the command is about to do, beside the
    /// choice it is about to offer.
    ///
    /// This is what replaces the retired `!` glyph. A warning used to be
    /// painted in the same sequence as a heading — literally pixel-identical,
    /// because two tones shared one escape code — so it was not distinguishable
    /// as a warning at all. The rail is a shape rather than a colour, and it
    /// survives `NO_COLOR`.
    pub fn rail(&mut self, text: &str) {
        if self.width < 4 {
            self.text(text, Ink::Gold);
            return;
        }
        if self.lines.last().is_some_and(|line| !line.is_empty()) {
            self.lines.push(String::new());
        }
        let width = self.width.saturating_sub(3).max(1);
        for line in fold(text, width) {
            self.lines.push(format!(
                "{}  {}",
                self.theme.paint(glyph::RAIL, Ink::Gold, false),
                self.theme.paint(&line, Ink::Text, false)
            ));
        }
    }

    /// A flat hairline: a break within a section, never under a heading.
    pub fn divide(&mut self) {
        self.lines.push(self.theme.rule(self.width));
    }

    /// A table, with faint capitalised column headings.
    ///
    /// Columns below their [`Column::keep_above`] width are dropped, and if the
    /// terminal is too narrow to be tabular at all each row becomes a labelled
    /// block instead — the labels are the column headings, so nothing is lost
    /// but the alignment.
    pub fn table(&mut self, columns: &[Column], rows: &[Row]) {
        if self.layout.tabular() {
            if self.lines.last().is_some_and(|line| !line.is_empty()) {
                self.lines.push(String::new());
            }
            self.lines
                .extend(table(columns, rows, self.layout, self.theme));
        } else {
            let kept = kept(columns, self.layout);
            self.stack(columns, rows, &kept);
        }
    }

    /// The sub-[`crate::width::TABLE`] fallback: one labelled block per row.
    fn stack(&mut self, columns: &[Column], rows: &[Row], kept: &[usize]) {
        for row in rows {
            self.lines.push(String::new());
            let head = row
                .cells
                .first()
                .map_or_else(String::new, |(text, _)| text.clone());
            let marker = row
                .marker
                .as_ref()
                .map_or_else(String::new, |(marker, _)| format!("{marker} "));
            let ink = row.cells.first().map_or(Ink::Text, |(_, ink)| *ink);
            for line in fold(&format!("{marker}{head}"), self.width) {
                self.lines.push(self.theme.paint(&line, ink, true));
            }
            for &index in kept.iter().skip(1) {
                let Some((text, ink)) = row.cells.get(index) else {
                    continue;
                };
                if text.is_empty() {
                    continue;
                }
                self.field(&columns[index].label, text, field_ink(*ink));
            }
        }
    }
}

/// Which columns this terminal is wide enough for.
fn kept(columns: &[Column], layout: Layout) -> Vec<usize> {
    (0..columns.len())
        .filter(|&index| layout.columns() >= columns[index].keep_above)
        .collect()
}

/// The table grammar on its own, without a document around it.
///
/// Streamed output — a build log, a pipe — wants the columns without a heading
/// and a seam above them, so the grammar is a function and [`Document::table`]
/// is one of its callers. Before this, that path hand-padded its own label
/// columns and drifted from the table beside it.
#[must_use]
pub fn table(columns: &[Column], rows: &[Row], layout: Layout, theme: Theme) -> Vec<String> {
    if layout.columns() <= crate::width::GUTTER {
        return Vec::new();
    }
    if !layout.tabular() {
        let mut document = Document {
            theme,
            width: layout.content(),
            layout,
            lines: Vec::new(),
        };
        document.stack(columns, rows, &kept(columns, layout));
        return document.lines;
    }
    let kept = kept(columns, layout);
    if kept.is_empty() || rows.is_empty() {
        return Vec::new();
    }
    let marked = rows.iter().any(|row| row.marker.is_some());
    let gutter = if marked { 3 } else { 0 };
    let width = layout.content();

    // Every column is as wide as its widest cell; the first absorbs the slack,
    // because it holds the name a reader scans down.
    let mut widths: Vec<usize> = kept
        .iter()
        .map(|&index| {
            rows.iter()
                .filter_map(|row| row.cells.get(index))
                .map(|(text, _)| clean(text).width())
                .chain(std::iter::once(columns[index].label.width()))
                .max()
                .unwrap_or(0)
        })
        .collect();
    fit(
        &mut widths,
        width,
        2 * kept.len().saturating_sub(1) + gutter,
    );

    let mut lines = Vec::with_capacity(rows.len() + 1);
    let mut heading = " ".repeat(gutter);
    for (slot, &index) in kept.iter().enumerate() {
        if slot > 0 {
            heading.push_str("  ");
        }
        heading.push_str(&theme.paint(
            &place(&columns[index].label, widths[slot], columns[index].align),
            Ink::Faint,
            false,
        ));
    }
    lines.push(heading.trim_end().to_owned());

    for row in rows {
        let mut line = String::new();
        if marked {
            match &row.marker {
                Some((marker, ink)) => {
                    line.push_str(&theme.paint(&crate::terminal::pad(marker, 1), *ink, true));
                    line.push_str("  ");
                }
                None => line.push_str("   "),
            }
        }
        for (slot, &index) in kept.iter().enumerate() {
            if slot > 0 {
                line.push_str("  ");
            }
            let (text, ink) = row
                .cells
                .get(index)
                .cloned()
                .unwrap_or_else(|| (String::new(), Ink::Muted));
            let ink = if slot == 0 { ink } else { field_ink(ink) };
            line.push_str(&theme.paint(
                &place(&text, widths[slot], columns[index].align),
                ink,
                slot == 0,
            ));
        }
        lines.push(line.trim_end().to_owned());
    }
    lines
}

/// Shrink the natural column widths until the row holds them.
///
/// A table is as wide as its content and no wider: slack is left as empty
/// space to the right, which is how the design draws it and what keeps a
/// right-aligned column of figures next to the words it belongs to rather than
/// flung against the far edge of a wide terminal.
///
/// Overflow comes off the *widest* column, one cell at a time — not off the
/// first, which is how a short `DIRECTORY` column came to be clipped to
/// `user-templa…` beside a `PATH` column with room to spare. Ties go to the
/// later column: the first holds the name a reader scans down.
fn fit(widths: &mut [usize], row: usize, overhead: usize) {
    let natural: usize = widths.iter().sum::<usize>() + overhead;
    if natural <= row {
        return;
    }
    let mut over = natural - row;
    while over > 0 {
        let (index, widest) = widths
            .iter()
            .copied()
            .enumerate()
            .max_by_key(|(index, width)| (*width, *index))
            .expect("a table has at least one column");
        // Nothing left to give: the terminal is narrower than the columns can
        // represent, and clipping every one of them equally is the least bad
        // remaining answer.
        if widest <= 1 {
            break;
        }
        let take = over.min(widest - 1);
        widths[index] -= take;
        over -= take;
    }
}

/// Pad or clip `text` to `width`, against the given edge.
fn place(text: &str, width: usize, align: Align) -> String {
    match align {
        Align::Left => crate::terminal::pad(text, width),
        Align::Right => {
            let text = crate::terminal::clip(text, width);
            format!("{}{text}", " ".repeat(width.saturating_sub(text.width())))
        }
    }
}

/// Secondary columns never carry the primary ink: a version beside a name is
/// support, and painting it `Text` would give it the same weight as the name.
fn field_ink(ink: Ink) -> Ink {
    match ink {
        Ink::Text => Ink::Muted,
        other => other,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::terminal::plain;

    fn review() -> (Vec<Column>, Vec<Row>) {
        let columns = vec![
            Column::new("package"),
            Column::new("version"),
            Column::new("repo"),
            Column::new("size")
                .right()
                .keep_above(crate::width::SIZE_COLUMN),
            Column::new("state"),
        ];
        let rows = vec![
            Row::new(vec![
                ("inkscape".into(), Ink::Text),
                ("1.4.2-1".into(), Ink::Muted),
                ("extra".into(), Ink::repository("extra")),
                ("32.4 MiB".into(), Ink::Muted),
                ("new".into(), Ink::Green),
            ])
            .marked(glyph::ADD, Ink::Green),
            Row::new(vec![
                ("hyprland-git".into(), Ink::Text),
                ("0.52.1-1".into(), Ink::Muted),
                ("aur".into(), Ink::repository("aur")),
                ("18.9 MiB".into(), Ink::Muted),
                ("build".into(), Ink::Gold),
            ])
            .marked(glyph::CHANGE, Ink::Gold),
        ];
        (columns, rows)
    }

    fn render(width: usize) -> String {
        let (columns, rows) = review();
        let mut doc = Document::new("pkg", "Install software", "REVIEW", width, Theme::PLAIN);
        doc.section("REQUESTED");
        doc.table(&columns, &rows);
        doc.rail("A new revision is created. r-0241 stays.");
        doc.render()
    }

    /// The table is a table, and every column is there at the design's width.
    #[test]
    fn the_full_grammar_shows_every_column() {
        let out = render(crate::width::REFERENCE);
        assert!(out.contains("PACKAGE"), "{out}");
        assert!(out.contains("SIZE"), "{out}");
        let row = out
            .lines()
            .find(|line| line.contains("inkscape"))
            .expect("the row is there");
        // Marker, name, version, repo, size and state all on one line.
        assert!(
            row.contains('+') && row.contains("1.4.2-1") && row.contains("32.4 MiB"),
            "{row}"
        );
    }

    /// SIZE is the first thing given up, and nothing else goes with it.
    #[test]
    fn size_is_dropped_before_anything_else() {
        let out = render(crate::width::SIZE_COLUMN - 1);
        assert!(!out.contains("SIZE"), "{out}");
        assert!(!out.contains("32.4 MiB"), "{out}");
        assert!(out.contains("PACKAGE") && out.contains("VERSION"), "{out}");
        assert!(out.contains("inkscape") && out.contains("1.4.2-1"), "{out}");
    }

    /// Below the table width every value survives, as labelled fields.
    #[test]
    fn a_narrow_terminal_keeps_every_value() {
        let out = render(crate::width::TABLE - 1);
        assert!(!out.contains("PACKAGE"), "no column headings: {out}");
        for value in ["inkscape", "1.4.2-1", "extra", "hyprland-git", "aur"] {
            assert!(out.contains(value), "lost {value}: {out}");
        }
        assert!(
            out.contains("VERSION"),
            "the heading became the label: {out}"
        );
    }

    /// No rendered line ever runs past the terminal, at any width.
    #[test]
    fn nothing_overruns_the_terminal() {
        for width in 0..=140 {
            for line in render(width).lines() {
                assert!(
                    plain(line).width() <= width,
                    "width {width}: {:?} is {} wide",
                    line,
                    plain(line).width()
                );
            }
        }
    }

    /// The rail is a shape, so it survives with colour off.
    #[test]
    fn the_rail_reads_without_colour() {
        let out = render(crate::width::REFERENCE);
        let rail = out
            .lines()
            .find(|line| line.contains(glyph::RAIL))
            .expect("the advisory is railed");
        assert!(rail.contains("A new revision is created"), "{rail}");
        assert!(!out.contains('\x1b'));
    }

    /// Overflow comes off the column that has the most to give.
    #[test]
    fn a_long_column_gives_ground_before_a_short_one() {
        let columns = vec![Column::new("directory"), Column::new("path")];
        let rows = vec![Row::new(vec![
            ("user-templates".into(), Ink::Text),
            (
                "/home/someone/.config/gnist/themes/user-templates".into(),
                Ink::Muted,
            ),
        ])];
        // Narrow enough that something has to give: the two columns and the
        // space between them want 64, and there are 56.
        let mut doc = Document::new("gnist", "Prepare", "INIT", 60, Theme::PLAIN);
        doc.table(&columns, &rows);
        let out = doc.render();
        assert!(
            out.contains("user-templates"),
            "the short column is intact: {out}"
        );
        assert!(out.contains('…'), "the long column gave the ground: {out}");
        // With room to spare the table is its own width, not the row's: a
        // column of figures stays beside the words it belongs to.
        let mut wide = Document::new("gnist", "Prepare", "INIT", 140, Theme::PLAIN);
        wide.table(&columns, &rows);
        let wide = wide.render();
        assert!(!wide.contains('…'), "{wide}");
        let row = wide
            .lines()
            .find(|line| line.contains("user-templates"))
            .expect("the row is there");
        assert!(
            row.trim_end().len() < 100,
            "a wide terminal should not stretch the table: {row:?}"
        );
    }

    /// The seam sits under the heading; section breaks stay flat.
    #[test]
    fn the_heading_is_seamed_and_only_the_heading() {
        let mut doc = Document::new("pkg", "Install", "REVIEW", 96, Theme::FULL);
        doc.section("REQUESTED");
        doc.divide();
        let out = doc.render();
        let seams = out
            .lines()
            .filter(|line| line.contains(glyph::RULE) && line.contains("38;2;230;191;139"))
            .count();
        assert_eq!(seams, 1, "exactly one gradient, under the heading: {out}");
    }
}

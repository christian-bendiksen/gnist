//! One width ladder, replacing five floors and eleven hand-written gutters.
//!
//! Ninety-six columns is the width the design is drawn at, and it is a
//! *reference*, not a constant: the ladder below says what to drop, in what
//! order, as a real terminal falls short of it. Every threshold in the crate
//! and in its consumers is named here, so widening a column is one edit rather
//! than a search for every `saturating_sub(4)`.
//!
//! Before this there were five unrelated minimum widths — 1, 12, 16, 28 and 44
//! — and two different ideas of how wide the terminal even was: the Ledger half
//! read `$COLUMNS`, which no common shell exports, and so rendered every
//! hairline at exactly 72 while the Document half beside it asked the kernel.

use unicode_width::UnicodeWidthStr;

/// Two columns of breathing room on each side of the content.
pub const GUTTER: usize = 4;

/// The width the design is drawn at. Above this nothing more appears — the
/// content stops growing and the gutter takes the slack — so a very wide
/// terminal shows the same document as a 96-column one rather than a table
/// stretched across half a metre of screen.
pub const REFERENCE: usize = 96;

/// At or above this, every column of the grammar is present.
pub const FULL: usize = REFERENCE;

/// At or above this, the browser splits into a list and a detail pane.
pub const TWO_PANE: usize = 84;

/// Below this, the SIZE column goes. It is the widest optional column and the
/// one a reader can most easily do without — the total is still in the footer.
pub const SIZE_COLUMN: usize = 64;

/// Below this, a table stops being a table: one field per line, labelled.
pub const TABLE: usize = 60;

/// Below this — or with fewer rows than [`BROWSER_ROWS`] — the interactive
/// browser refuses to open and the command prints a plain list instead.
/// Refusing is the honest outcome: a full-screen pane that cannot show a
/// revision identity and its date on one line is worse than scrollback.
pub const BROWSER: usize = 44;

/// The browser needs this many rows for a header, a page of rows and a footer.
pub const BROWSER_ROWS: usize = 24;

/// Below this the staged progress region collapses to a single line.
pub const STAGED: usize = 28;

/// The staged region needs this many rows before it is worth holding open.
pub const STAGED_ROWS: usize = 10;

/// What a terminal of a given size can show.
///
/// Constructed from real dimensions with [`Layout::detect`] or from stated ones
/// with [`Layout::of`], so every renderer is a pure function of a `Layout` and
/// a test can sweep the ladder without a pty.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Layout {
    columns: usize,
    rows: usize,
}

impl Layout {
    /// Ask the kernel.
    #[must_use]
    pub fn detect() -> Self {
        let (columns, rows) = crate::terminal::size();
        Self::of(columns, rows)
    }

    /// A layout for stated dimensions.
    #[must_use]
    pub fn of(columns: usize, rows: usize) -> Self {
        Self { columns, rows }
    }

    /// The reference layout, for fixtures and for anything rendering to a file
    /// rather than a screen.
    #[must_use]
    pub const fn reference() -> Self {
        Self {
            columns: REFERENCE,
            rows: BROWSER_ROWS,
        }
    }

    /// The terminal's full width, gutter included. Renderers want
    /// [`Layout::content`]; this is for things that address the screen itself,
    /// like clearing a line.
    #[must_use]
    pub const fn columns(self) -> usize {
        self.columns
    }

    /// The terminal's height.
    #[must_use]
    pub const fn rows(self) -> usize {
        self.rows
    }

    /// The usable width inside the gutter, capped at the reference width.
    ///
    /// Never zero: a renderer that divides by this or subtracts a marker from
    /// it should not have to carry its own floor, which is how five different
    /// floors came to exist.
    #[must_use]
    pub const fn content(self) -> usize {
        let inside = self.columns.saturating_sub(GUTTER);
        let capped = if inside > REFERENCE {
            REFERENCE
        } else {
            inside
        };
        if capped == 0 { 1 } else { capped }
    }

    /// Whether the full grammar fits: every column, at the drawn width.
    #[must_use]
    pub const fn full(self) -> bool {
        self.columns >= FULL
    }

    /// Whether the browser shows a list and a detail pane side by side.
    #[must_use]
    pub const fn two_pane(self) -> bool {
        self.columns >= TWO_PANE
    }

    /// Whether a table can afford its SIZE column.
    #[must_use]
    pub const fn shows_size(self) -> bool {
        self.columns >= SIZE_COLUMN
    }

    /// Whether rows can be laid out in columns at all, or must become one
    /// labelled field per line.
    #[must_use]
    pub const fn tabular(self) -> bool {
        self.columns >= TABLE
    }

    /// Whether the interactive browser can open here.
    #[must_use]
    pub const fn browsable(self) -> bool {
        self.columns >= BROWSER && self.rows >= BROWSER_ROWS
    }

    /// Whether the staged progress region can be held open here.
    #[must_use]
    pub const fn staged(self) -> bool {
        self.columns >= STAGED && self.rows >= STAGED_ROWS
    }

    /// Split the content width into a list pane and a detail pane.
    ///
    /// `None` below [`TWO_PANE`] — the caller draws one pane at
    /// [`Layout::content`]. The list takes the smaller share: it holds short
    /// identities, the detail pane holds prose.
    #[must_use]
    pub const fn panes(self) -> Option<(usize, usize)> {
        if !self.two_pane() {
            return None;
        }
        let content = self.content();
        let list = content * 2 / 5;
        Some((list, content - list - 3))
    }

    /// How many cells a progress bar gets, given the text beside it.
    #[must_use]
    pub fn bar_cells(self, beside: &str) -> usize {
        self.content()
            .saturating_sub(beside.width() + 2)
            .clamp(1, 24)
    }
}

impl Default for Layout {
    fn default() -> Self {
        Self::reference()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The ladder only ever gives up things, and always in the same order.
    #[test]
    fn the_ladder_degrades_monotonically() {
        let mut lost: Vec<&str> = Vec::new();
        let mut previous = [true; 5];
        for columns in (20..=200).rev() {
            let layout = Layout::of(columns, 40);
            let now = [
                layout.full(),
                layout.two_pane(),
                layout.shows_size(),
                layout.tabular(),
                layout.browsable(),
            ];
            for (index, (was, is)) in previous.iter().zip(now.iter()).enumerate() {
                assert!(
                    *was || !*is,
                    "column {columns}: rung {index} came back after being dropped"
                );
                if *was && !*is {
                    lost.push(["full", "two_pane", "size", "tabular", "browser"][index]);
                }
            }
            previous = now;
        }
        assert_eq!(lost, ["full", "two_pane", "size", "tabular", "browser"]);
    }

    /// A very wide terminal renders the document the design was drawn at.
    #[test]
    fn content_stops_at_the_reference_width() {
        assert_eq!(Layout::of(100, 40).content(), REFERENCE);
        assert_eq!(Layout::of(400, 40).content(), REFERENCE);
        assert_eq!(Layout::of(96, 40).content(), 96 - GUTTER);
    }

    /// Every renderer used to carry its own floor because this could be zero.
    #[test]
    fn content_is_never_zero() {
        for columns in 0..=GUTTER {
            assert_eq!(Layout::of(columns, 24).content(), 1, "columns {columns}");
        }
    }

    /// Two panes plus the space between them are exactly the content width.
    #[test]
    fn the_panes_account_for_every_column() {
        for columns in TWO_PANE..=200 {
            let layout = Layout::of(columns, 40);
            let (list, detail) = layout.panes().expect("wide enough for two panes");
            assert_eq!(list + detail + 3, layout.content(), "columns {columns}");
            assert!(list > 0 && detail > 0, "columns {columns}");
        }
        assert_eq!(Layout::of(TWO_PANE - 1, 40).panes(), None);
    }

    /// Height matters as much as width for anything that takes the screen.
    #[test]
    fn a_short_terminal_is_not_browsable_however_wide() {
        assert!(!Layout::of(200, BROWSER_ROWS - 1).browsable());
        assert!(Layout::of(200, BROWSER_ROWS).browsable());
        assert!(!Layout::of(BROWSER - 1, 80).browsable());
        assert!(!Layout::of(200, STAGED_ROWS - 1).staged());
    }

    #[test]
    fn a_bar_always_has_at_least_one_cell() {
        let narrow = Layout::of(20, 24);
        assert!(narrow.bar_cells("a very long label indeed 1234/5678") >= 1);
        assert_eq!(Layout::reference().bar_cells("12/42"), 24);
    }
}

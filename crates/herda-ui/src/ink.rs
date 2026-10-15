//! The palette, the glyphs and the seam — one table each.
//!
//! This is the whole of the Herda terminal look, and it is deliberately the
//! only place any of it is written down. The crate used to carry two
//! unconnected palettes (`Tone` here in the crate root, `Ink` in `terminal`),
//! two capability detections that disagreed about `NO_COLOR` and `FORCE_COLOR`,
//! and two glyph sets that disagreed about what `!` meant. A reader could not
//! tell which one an output line came from, and neither could the code: the
//! output boundary in `pkg-cli` sniffed its own text for a `──` prefix to work
//! out which system had produced it.
//!
//! The colours are the desktop's. Every value here appears in the Fold design
//! with the same role, so a terminal and a settings panel on the same screen
//! are recognisably one system.

/// A colour role. Never a colour — the same `Ink` is a different number of
/// bits on a truecolor terminal, a 256-colour terminal and no terminal at all.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Ink {
    /// Primary text.
    #[default]
    Text,
    /// Secondary text: values beside a label, a version beside a name.
    Muted,
    /// Tertiary text: column headings, units, annotations, anything that is
    /// there to be skipped over.
    Faint,
    /// Hairlines and the unfilled half of a bar. Never text.
    Rule,
    /// The brand. Headings, the command's own name, an advisory rail, the
    /// figure a reader is waiting for.
    Gold,
    /// A step that finished, a package that verified.
    Green,
    /// A step that failed and a command that could not continue.
    ///
    /// Named for what it means rather than what it looks like: it was `Red`
    /// and the colour is orange.
    Danger,
    /// The core repository.
    Blue,
    /// The AUR — built from source, from a live service rather than the pin.
    Magenta,
    /// The extra repository.
    Cyan,
    /// The row the cursor is on. The only ink that sets a background.
    Selected,
}

impl Ink {
    /// The 24-bit form: what the design specifies.
    const fn truecolor(self) -> &'static str {
        match self {
            Self::Text => "38;2;242;242;239",
            Self::Muted => "38;2;180;185;187",
            Self::Faint => "38;2;143;148;150",
            Self::Rule => "38;2;58;62;65",
            Self::Gold => "38;2;255;188;80",
            Self::Green => "38;2;106;213;139",
            Self::Danger => "38;2;255;154;118",
            Self::Blue => "38;2;115;181;255",
            Self::Magenta => "38;2;227;149;220",
            Self::Cyan => "38;2;110;217;228",
            Self::Selected => "38;2;242;242;239;48;2;42;46;50",
        }
    }

    /// The nearest 256-colour index, for terminals that cannot do better.
    ///
    /// These are chosen to keep the *relationships* — text above muted above
    /// faint, rule below all three — rather than to be the closest match to
    /// each hex in isolation.
    const fn indexed(self) -> &'static str {
        match self {
            Self::Text => "38;5;255",
            Self::Muted => "38;5;249",
            Self::Faint => "38;5;246",
            Self::Rule => "38;5;237",
            Self::Gold => "38;5;215",
            Self::Green => "38;5;114",
            Self::Danger => "38;5;209",
            Self::Blue => "38;5;111",
            Self::Magenta => "38;5;176",
            Self::Cyan => "38;5;80",
            Self::Selected => "38;5;255;48;5;236",
        }
    }

    /// The escape body for this ink at the depth the terminal supports.
    ///
    /// Public so a test can ask what an ink renders as rather than writing the
    /// sequence out by hand — which is how a palette change came to break
    /// pinned tests in two repositories at once.
    #[must_use]
    pub const fn code(self, truecolor: bool) -> &'static str {
        match truecolor {
            true => self.truecolor(),
            false => self.indexed(),
        }
    }

    /// The ink a package repository is named in.
    ///
    /// The design gives the three sources distinct colours so a reader can see
    /// at a glance that one line in a list came from somewhere else — the AUR
    /// in particular, which is built from source against a live service rather
    /// than the machine's pinned snapshot.
    pub fn repository(name: &str) -> Self {
        match name {
            "core" => Self::Blue,
            "extra" => Self::Cyan,
            "aur" => Self::Magenta,
            _ => Self::Faint,
        }
    }
}

/// A background role: a plate behind text, or the row or key that is picked out.
///
/// A wash is never a saturated block. Each one is the terminal's graphite
/// background lifted a few percent toward the text or toward an accent, the
/// way the Fold design lays a translucent plate over a window, so the text on
/// it keeps its own ink and its contrast. A cell carries one beside its
/// [`Ink`]; with colour off a wash is not drawn at all, which is why nothing
/// may be told by a wash alone.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Wash {
    /// No plate: the terminal's own background.
    #[default]
    None,
    /// A fact tile, the log pane, the command line.
    Plate,
    /// A key that can be pressed now.
    Key,
    /// The key that moves the command forward.
    Accent,
    /// The row the cursor is on.
    Cursor,
    /// The plate behind what went wrong.
    Alarm,
}

impl Wash {
    /// The 24-bit form: each overlay of the design composited on `#131618`.
    const fn truecolor(self) -> &'static str {
        match self {
            Self::None => "",
            Self::Plate => "48;2;28;31;33",
            Self::Key => "48;2;37;40;41",
            Self::Accent => "48;2;57;49;33",
            Self::Cursor => "48;2;36;34;28",
            Self::Alarm => "48;2;36;31;31",
        }
    }

    /// The 256-colour form.
    ///
    /// The cube has no dark amber and no dark orange that would pass for a
    /// wash, so these are steps of the grey ramp: what survives is that a key
    /// stands off a plate and the forward key stands off the other keys. The
    /// hue is carried by the ink on top, as it is with colour off.
    const fn indexed(self) -> &'static str {
        match self {
            Self::None => "",
            Self::Plate => "48;5;234",
            Self::Cursor => "48;5;235",
            Self::Alarm => "48;5;236",
            Self::Key => "48;5;237",
            Self::Accent => "48;5;238",
        }
    }

    /// The escape body for this wash at the depth the terminal supports, empty
    /// for [`Wash::None`].
    #[must_use]
    pub const fn code(self, truecolor: bool) -> &'static str {
        match truecolor {
            true => self.truecolor(),
            false => self.indexed(),
        }
    }
}

/// Every glyph the terminal uses, and the one meaning each carries.
///
/// There were two of these tables, in different crates, and they contradicted
/// each other: `!` was a gold "Review" advisory in one renderer and a red
/// "Stopped" failure in the other, inside the same binary. Failure had two
/// glyphs, waiting had two, and there were two progress bars with different
/// fill arithmetic. `!` is retired here — a failure is `FAILED`, an advisory is
/// the `RAIL`.
pub mod glyph {
    /// A step that finished.
    pub const DONE: &str = "✓";
    /// The step that is running now.
    pub const WORKING: &str = "●";
    /// A step that has not started.
    pub const WAITING: &str = "○";
    /// A step that failed, and the command that could not continue.
    pub const FAILED: &str = "✗";
    /// Neutral: a status with nothing to report. Never a separator — use
    /// [`SEPARATOR`] for that, which is the same character doing a different
    /// job and is worth being able to grep apart.
    pub const NEUTRAL: &str = "·";
    /// Between fields on one line.
    pub const SEPARATOR: &str = "·";
    /// A package added.
    pub const ADD: &str = "+";
    /// A package removed.
    pub const REMOVE: &str = "-";
    /// A package whose version moved.
    pub const CHANGE: &str = "~";
    /// The advisory rail: what the command is about to do, beside the choice.
    pub const RAIL: &str = "│";
    /// A filled cell of a progress bar.
    pub const BAR_FULL: char = '▰';
    /// An unfilled cell of a progress bar.
    pub const BAR_EMPTY: char = '▱';
    /// One cell of a hairline.
    pub const RULE: char = '─';
    /// The cursor, where a prompt waits for an answer.
    pub const CURSOR: &str = "▌";
    /// The row the cursor is on, so it is findable with colour off.
    pub const ROW: &str = "›";
    /// The revision the machine runs. The same character as [`WORKING`], doing
    /// a different job: on the revision line it is a point, not a state.
    pub const POINT_SELECTED: &str = "●";
    /// A retained revision the machine does not run.
    pub const POINT_KEPT: &str = "○";
    /// A revision that does not exist yet.
    pub const POINT_AHEAD: &str = "◌";
    /// Where the files are when they have moved away from every revision.
    pub const POINT_AWAY: &str = "◇";
    /// One cell of the part of a change that is done, and of a filled thin bar.
    pub const LINE_DONE: char = '━';
    /// The head of that part when it ends in the first half of a cell.
    pub const LINE_HEAD: char = '╸';
    /// One cell of the part of a change that is still ahead.
    pub const LINE_AHEAD: char = '╌';
    /// Where the line comes in from revisions there is no room to draw.
    pub const LINE_FROM: char = '╶';
    /// Where the line goes on to revisions there is no room to draw.
    pub const LINE_ON: char = '╴';
    /// A graph column's eight heights within one cell, lowest first.
    pub const LEVELS: [char; 8] = ['▁', '▂', '▃', '▄', '▅', '▆', '▇', '█'];
}

/// The seam's colour stops: the rule grey lifted toward the brand amber and
/// back, mirrored around the brightest point.
///
/// Eight stops rather than a smooth ramp, because a per-column gradient across
/// ninety-six columns costs ninety-six escape sequences for a single rule. This
/// costs fifteen runs and is indistinguishable at a terminal's resolution.
const SEAM: [&str; 8] = [
    "38;2;58;62;65",
    "38;2;83;80;76",
    "38;2;107;99;86",
    "38;2;132;118;97",
    "38;2;156;136;108",
    "38;2;181;154;118",
    "38;2;206;173;129",
    "38;2;230;191;139",
];

/// The gradient rule that sits under every heading.
///
/// Truecolor only. On a 256-colour terminal the stops would band visibly and
/// cost more than they are worth, so both it and `NO_COLOR` fall back to the
/// flat rule — which is why this is a function that is given the theme rather
/// than a string a caller can assemble.
pub(crate) fn seam(width: usize, color: bool, truecolor: bool) -> String {
    let flat: String = std::iter::repeat_n(glyph::RULE, width).collect();
    if !color {
        return flat;
    }
    if !truecolor {
        return format!("\x1b[22;{}m{flat}\x1b[0m", Ink::Rule.indexed());
    }
    // Up through the stops and back down, so the brightest point sits a third
    // of the way in the way the design draws it rather than in the middle.
    let stops: Vec<&str> = SEAM
        .iter()
        .copied()
        .chain(SEAM.iter().rev().skip(1).copied())
        .collect();
    let mut out = String::new();
    let mut placed = 0;
    for (index, code) in stops.iter().enumerate() {
        let end = (index + 1) * width / stops.len();
        let run = end.saturating_sub(placed);
        if run == 0 {
            continue;
        }
        out.push_str("\x1b[22;");
        out.push_str(code);
        out.push('m');
        for _ in 0..run {
            out.push(glyph::RULE);
        }
        placed = end;
    }
    for _ in placed..width {
        out.push(glyph::RULE);
    }
    out.push_str("\x1b[0m");
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every ink is distinct at both depths.
    ///
    /// This is the regression that matters most: `Tone::Accent` and
    /// `Tone::Attention` used to emit the identical sequence `1;38;5;180`, so a
    /// warning was pixel-identical to a heading — and a test asserted that,
    /// pinning the bug in place.
    #[test]
    fn no_two_inks_render_alike() {
        let inks = [
            Ink::Text,
            Ink::Muted,
            Ink::Faint,
            Ink::Rule,
            Ink::Gold,
            Ink::Green,
            Ink::Danger,
            Ink::Blue,
            Ink::Magenta,
            Ink::Cyan,
            Ink::Selected,
        ];
        for truecolor in [true, false] {
            let mut seen = std::collections::BTreeMap::new();
            for ink in inks {
                let code = ink.code(truecolor);
                if let Some(other) = seen.insert(code, ink) {
                    panic!("{ink:?} and {other:?} both emit {code} (truecolor {truecolor})");
                }
            }
        }
    }

    /// A wash that rendered like another would make a key look like a plate.
    #[test]
    fn no_two_washes_render_alike() {
        let washes = [
            Wash::Plate,
            Wash::Key,
            Wash::Accent,
            Wash::Cursor,
            Wash::Alarm,
        ];
        for truecolor in [true, false] {
            let mut seen = std::collections::BTreeMap::new();
            for wash in washes {
                let code = wash.code(truecolor);
                assert!(code.starts_with("48;"), "{wash:?} sets only a background");
                if let Some(other) = seen.insert(code, wash) {
                    panic!("{wash:?} and {other:?} both emit {code} (truecolor {truecolor})");
                }
            }
            assert_eq!(Wash::None.code(truecolor), "");
        }
    }

    /// Every character the revision line and the graph are built from takes
    /// one cell, or the line would not land where it was measured.
    #[test]
    fn the_line_characters_are_one_cell_each() {
        use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};
        for text in [
            glyph::POINT_SELECTED,
            glyph::POINT_KEPT,
            glyph::POINT_AHEAD,
            glyph::POINT_AWAY,
            glyph::ROW,
        ] {
            assert_eq!(text.width(), 1, "{text}");
        }
        for character in [
            glyph::LINE_DONE,
            glyph::LINE_HEAD,
            glyph::LINE_AHEAD,
            glyph::LINE_FROM,
            glyph::LINE_ON,
            glyph::RULE,
        ]
        .into_iter()
        .chain(glyph::LEVELS)
        {
            assert_eq!(character.width(), Some(1), "{character}");
        }
    }

    #[test]
    fn a_repository_is_told_apart_from_the_pin() {
        assert_eq!(Ink::repository("aur"), Ink::Magenta);
        assert_eq!(Ink::repository("core"), Ink::Blue);
        assert_eq!(Ink::repository("extra"), Ink::Cyan);
        // Anything else is not a claim about where it came from.
        assert_eq!(Ink::repository("local"), Ink::Faint);
    }

    #[test]
    fn the_seam_is_exactly_as_wide_as_it_is_asked_for() {
        for width in [0, 1, 7, 8, 9, 44, 80, 96, 200] {
            for (color, truecolor) in [(false, false), (true, false), (true, true)] {
                let drawn = seam(width, color, truecolor);
                let bare: String = crate::terminal::plain(&drawn);
                assert_eq!(
                    bare.chars().count(),
                    width,
                    "width {width}, color {color}, truecolor {truecolor}: {drawn:?}"
                );
            }
        }
    }

    /// Only a truecolor terminal gets the gradient; the rest get the rule.
    #[test]
    fn the_seam_bands_only_where_it_can() {
        let plain = seam(96, false, false);
        assert!(!plain.contains('\x1b'), "{plain:?}");

        let indexed = seam(96, true, false);
        assert!(indexed.contains("38;5;237"), "{indexed:?}");
        assert_eq!(indexed.matches("\x1b[").count(), 2, "one colour, one reset");

        let full = seam(96, true, true);
        assert!(full.contains("38;2;58;62;65"), "starts at the rule grey");
        assert!(full.contains("38;2;230;191;139"), "reaches the brand amber");
        assert!(full.matches("\x1b[").count() > 8, "banded: {full:?}");
    }
}

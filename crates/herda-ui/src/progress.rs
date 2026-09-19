//! The staged progress region: what is happening now, and what is left.
//!
//! A bounded block of the terminal, rewritten in place, that says which step of
//! a long operation is running, what it has moved so far and what it is still
//! waiting on. It is bounded on purpose: it never scrolls, so the command's
//! real output — the document, or the error — is what the person is left
//! looking at when it is over.
//!
//! What is here is the shape. The steps, the words on them and the note along
//! the bottom belong to the tool: `pkg` resolves, downloads, verifies, creates
//! a revision and activates; Malm does something else entirely; neither
//! sequence belongs in a shared crate.

use std::io::Write;
use std::time::{Duration, Instant};

use crate::ink::{Ink, glyph};
use crate::terminal::{Theme, clip, size_label, wrap};
use crate::width::Layout;

/// Where one step of an operation has got to.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Step {
    /// Not started.
    #[default]
    Waiting,
    /// Running now.
    Working,
    /// Finished.
    Done,
    /// Stopped here, and the operation did not continue.
    Failed,
}

impl Step {
    /// The glyph, from the crate's one table.
    #[must_use]
    pub const fn glyph(self) -> &'static str {
        match self {
            Self::Waiting => glyph::WAITING,
            Self::Working => glyph::WORKING,
            Self::Done => glyph::DONE,
            Self::Failed => glyph::FAILED,
        }
    }

    /// The word beside the glyph, so the state survives a monochrome terminal
    /// and a screen reader.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Waiting => "Waiting",
            Self::Working => "Working",
            Self::Done => "Complete",
            Self::Failed => "Stopped",
        }
    }

    /// The ink. `Failed` is the only one that used to be ambiguous: it was
    /// drawn with `!`, which the other renderer in the same binary used for a
    /// gold advisory.
    #[must_use]
    pub const fn ink(self) -> Ink {
        match self {
            Self::Waiting => Ink::Muted,
            Self::Working => Ink::Gold,
            Self::Done => Ink::Green,
            Self::Failed => Ink::Danger,
        }
    }
}

/// One step of the operation.
#[derive(Clone, Debug, Default)]
pub struct Stage {
    /// What this step does, as a person would say it.
    pub label: String,
    /// Where it has got to.
    pub state: Step,
}

/// One thing being moved, for the table under the steps.
#[derive(Clone, Debug, Default)]
pub struct Transfer {
    /// What is being moved.
    pub name: String,
    /// How much has arrived.
    pub bytes: u64,
    /// How much there is, once that is known. Unknown is not zero, and is not
    /// rendered as a percentage of anything.
    pub total: Option<u64>,
    /// Arrived in full.
    pub complete: bool,
    /// Was already here, so nothing was moved for it.
    pub cached: bool,
}

impl Transfer {
    /// The word in the STATUS column.
    #[must_use]
    pub const fn status(&self) -> &'static str {
        if self.cached {
            "Cached"
        } else if self.complete {
            "Complete"
        } else if self.bytes > 0 {
            "Working"
        } else {
            "Waiting"
        }
    }

    const fn ink(&self) -> Ink {
        if self.complete {
            Ink::Green
        } else if self.bytes > 0 {
            Ink::Gold
        } else {
            Ink::Muted
        }
    }
}

/// How far along the whole operation is, when that is measurable.
#[derive(Clone, Copy, Debug, Default)]
pub struct Measure {
    /// Moved so far.
    pub bytes: u64,
    /// The total, once it is known.
    pub total: Option<u64>,
    /// Bytes per second, once there has been long enough to mean anything.
    pub rate: Option<u64>,
    /// Seconds left, if both the total and the rate are known.
    pub eta: Option<u64>,
}

/// Everything the region shows, as values. Rendering reads nothing else.
#[derive(Clone, Debug, Default)]
pub struct Region {
    /// The heading.
    pub title: String,
    /// The line under it.
    pub subtitle: String,
    /// The top right of the heading — usually the clock.
    pub badge: String,
    /// The steps, in order.
    pub stages: Vec<Stage>,
    /// What the current step is doing, when there is nothing to measure.
    pub detail: String,
    /// What there is to measure, when there is.
    pub measure: Option<Measure>,
    /// The table under the steps.
    pub transfers: Vec<Transfer>,
    /// The line along the bottom: what this means for the machine.
    pub note: String,
}

impl Region {
    /// Which step is running, or the one that stopped.
    #[must_use]
    pub fn current(&self) -> usize {
        self.stages
            .iter()
            .position(|stage| matches!(stage.state, Step::Working | Step::Failed))
            .unwrap_or(self.stages.len().saturating_sub(1))
    }

    /// Lay the region out for a terminal of this size.
    ///
    /// Pure, so the pty suite and the design fixtures render the real region
    /// without waiting for a download.
    #[must_use]
    pub fn lines(&self, layout: Layout, theme: Theme) -> Vec<String> {
        let rows = layout.rows();
        let width = layout.content();
        let mut lines = vec![theme.heading(&self.title, &self.badge, width)];
        if !self.subtitle.is_empty() {
            lines.push(theme.paint(&clip(&self.subtitle, width), Ink::Muted, false));
        }
        lines.extend([theme.seam(width), String::new()]);

        let current = self.current();
        if rows >= crate::width::BROWSER_ROWS {
            for stage in &self.stages {
                let left = format!("{}  {}", stage.state.glyph(), stage.label);
                lines.push(format!(
                    "{} {}",
                    theme.cell(
                        &left,
                        width.saturating_sub(12),
                        stage.state.ink(),
                        stage.state == Step::Working
                    ),
                    theme.cell(stage.state.label(), 10, stage.state.ink(), false)
                ));
            }
        } else if let Some(stage) = self.stages.get(current) {
            // Too short for the list: one line saying where it is.
            lines.push(theme.paint(
                &clip(&format!("{}  {}", stage.state.glyph(), stage.label), width),
                stage.state.ink(),
                true,
            ));
        }
        lines.push(String::new());

        match &self.measure {
            Some(measure) => lines.extend(self.measured(*measure, layout, theme)),
            None => lines.push(theme.paint(&clip(&self.detail, width), Ink::Muted, false)),
        }

        if !self.transfers.is_empty() && rows >= 20 {
            lines.extend(self.table(layout, theme, &mut { lines.len() }));
        }

        while lines.len() < rows.saturating_sub(6).min(24) {
            lines.push(String::new());
        }
        lines.extend([String::new(), theme.rule(width)]);
        lines.extend(
            wrap(&self.note, width)
                .into_iter()
                .map(|line| theme.paint(&line, Ink::Muted, false)),
        );
        lines.truncate(rows.saturating_sub(2).max(1));
        lines
    }

    /// The figures and the bar.
    fn measured(&self, measure: Measure, layout: Layout, theme: Theme) -> Vec<String> {
        let width = layout.content();
        let mut lines = Vec::new();
        let total = measure.total.filter(|n| *n > 0);
        let figures = match total {
            Some(total) => format!(
                "{} / {}  {}  {}%",
                size_label(measure.bytes),
                size_label(total),
                glyph::SEPARATOR,
                (measure.bytes as f64 / total as f64 * 100.0).min(100.0) as u64
            ),
            // An unknown total is not a zero total, and never a percentage of
            // one: a bar that fills to 100% and then keeps going is a lie.
            None => format!(
                "{} received {} total not yet known",
                size_label(measure.bytes),
                glyph::SEPARATOR
            ),
        };
        lines.push(theme.paint(&clip(&figures, width), Ink::Text, false));
        if let Some(total) = total {
            let cells = width.saturating_sub(2);
            let filled = ((measure.bytes as f64 / total as f64).min(1.0) * cells as f64) as usize;
            lines.push(format!(
                "{}{}",
                theme.paint(
                    &glyph::BAR_FULL.to_string().repeat(filled),
                    Ink::Gold,
                    false
                ),
                theme.paint(
                    &glyph::BAR_EMPTY.to_string().repeat(cells - filled),
                    Ink::Rule,
                    false
                )
            ));
        }
        if let Some(rate) = measure.rate {
            let eta = measure
                .eta
                .map(|seconds| format!(" {} About {seconds}s remaining", glyph::SEPARATOR))
                .unwrap_or_default();
            lines.push(theme.paint(
                &clip(&format!("{}/s{eta}", size_label(rate)), width),
                Ink::Muted,
                false,
            ));
        }
        lines
    }

    /// The transfer table, sized to what is left of the screen.
    fn table(&self, layout: Layout, theme: Theme, used: &mut usize) -> Vec<String> {
        let width = layout.content();
        let wide = layout.shows_size();
        let name_width = if wide {
            width.saturating_sub(40)
        } else {
            width.saturating_sub(17)
        }
        .max(1);
        let mut lines = vec![String::new(), theme.rule(width)];
        lines.push(if wide {
            format!(
                "{}  {}  {}",
                theme.cell("PACKAGE", name_width, Ink::Faint, false),
                theme.cell("TRANSFER", 22, Ink::Faint, false),
                theme.cell("STATUS", 14, Ink::Faint, false)
            )
        } else {
            format!(
                "{}  {}",
                theme.cell("PACKAGE", name_width, Ink::Faint, false),
                theme.cell("STATUS", 14, Ink::Faint, false)
            )
        });
        let capacity = layout.rows().saturating_sub(*used + lines.len() + 6).min(6);
        let mut transfers: Vec<&Transfer> = self.transfers.iter().collect();
        transfers.sort_by(|a, b| (a.complete, &a.name).cmp(&(b.complete, &b.name)));
        for transfer in transfers.iter().take(capacity) {
            let name = theme.cell(&transfer.name, name_width, Ink::Text, true);
            let status = theme.cell(transfer.status(), 14, transfer.ink(), false);
            if wide {
                lines.push(format!(
                    "{name}  {}  {status}",
                    theme.cell(&moved(transfer), 22, Ink::Muted, false)
                ));
            } else {
                lines.push(format!("{name}  {status}"));
            }
        }
        if transfers.len() > capacity {
            lines.push(theme.paint(
                &format!("… {} more packages", transfers.len() - capacity),
                Ink::Muted,
                false,
            ));
        }
        lines
    }
}

/// `22.5 / 32.4 MiB` — the unit is written once when both halves share it.
fn moved(transfer: &Transfer) -> String {
    let bytes = size_label(transfer.bytes);
    let total = transfer.total.map_or_else(|| "?".to_owned(), size_label);
    let unit = |text: &str| text.split_once(' ').map(|(_, unit)| unit.to_owned());
    if unit(&bytes) == unit(&total)
        && let Some((number, _)) = bytes.split_once(' ')
    {
        return format!("{number} / {total}");
    }
    format!("{bytes} / {total}")
}

/// Holds one region of the terminal open and rewrites it in place.
pub struct Painter {
    drawn: usize,
    columns: usize,
    last: Instant,
    closed: bool,
}

impl Default for Painter {
    fn default() -> Self {
        Self::new()
    }
}

impl Painter {
    /// A painter that has not drawn yet.
    #[must_use]
    pub fn new() -> Self {
        Self {
            drawn: 0,
            columns: 0,
            last: Instant::now() - Duration::from_secs(1),
            closed: false,
        }
    }

    /// Draw the region, unless it is too soon or the terminal is too small.
    ///
    /// `force` draws whatever the rate limit says, for the moments that matter:
    /// a step changing, and the end.
    pub fn draw(&mut self, region: &Region, theme: Theme, force: bool) {
        if self.closed || (!force && self.last.elapsed() < Duration::from_millis(80)) {
            return;
        }
        let layout = Layout::detect();
        if !layout.staged() {
            return;
        }
        let lines = region.lines(layout, theme);
        let mut out = std::io::stdout().lock();
        let _ = write!(out, "\x1b[?2026h");
        // A width change reflows the rows already drawn. Start a fresh region
        // rather than moving the cursor into somebody's shell scrollback.
        if self.drawn > 0 && layout.columns() == self.columns {
            let _ = write!(
                out,
                "\x1b[{}F",
                self.drawn.min(layout.rows().saturating_sub(1))
            );
        }
        let _ = write!(out, "\x1b[J");
        for line in &lines {
            let _ = writeln!(out, "  {line}");
        }
        let _ = write!(out, "\x1b[?2026l");
        let _ = out.flush();
        self.drawn = lines.len();
        self.columns = layout.columns();
        self.last = Instant::now();
    }

    /// Draw once more and stop. Later draws do nothing, so the command's own
    /// output is not overwritten by a worker that had not noticed the end.
    pub fn close(&mut self, region: &Region, theme: Theme) {
        if !self.closed {
            self.draw(region, theme, true);
            self.closed = true;
        }
    }
}

/// A single progress line, rewritten in place: `── label ──── ▰▰▱▱ 12/42 tail`.
///
/// The small sibling of [`Region`], for work that has a count but no steps.
/// On a terminal the line is held open and rewritten; anywhere else — a pipe, a
/// log, a serial console — the header prints once and every update is a no-op,
/// so a caller can update per item without flooding a build log.
pub struct Line {
    theme: Theme,
    interactive: bool,
    label: String,
}

impl Line {
    /// Open the line for `label`.
    #[must_use]
    pub fn start(label: &str) -> Self {
        let theme = crate::presentation::theme();
        let interactive = crate::terminal::interactive_output();
        let line = theme.hairline(label, "", Layout::detect().content());
        if interactive {
            print!("{line}");
            let _ = std::io::stdout().flush();
        } else {
            println!("{line}");
        }
        Self {
            theme,
            interactive,
            label: label.to_owned(),
        }
    }

    /// Whether updates render in place. When false the caller decides what — if
    /// anything — to print per item.
    #[must_use]
    pub fn interactive(&self) -> bool {
        self.interactive
    }

    /// Rewrite the line with a bar, the counter, and any `tail`.
    pub fn update(&self, done: usize, total: usize, tail: &str) {
        if !self.interactive {
            return;
        }
        let layout = Layout::detect();
        let mut rendered = format!("{} {done}/{total}", self.theme.bar(done, total, 16));
        if !tail.is_empty() {
            rendered.push(' ');
            rendered.push_str(glyph::SEPARATOR);
            rendered.push(' ');
            rendered.push_str(tail);
        }
        let line = self
            .theme
            .hairline(&self.label, &rendered, layout.content());
        print!("\r\x1b[2K{line}");
        let _ = std::io::stdout().flush();
    }

    /// Close the line. A finished bar has already turned green; this ends the
    /// row. Off a terminal it is a no-op — the header is already complete.
    pub fn finish(&self) {
        if self.interactive {
            println!();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::terminal::plain;
    use unicode_width::UnicodeWidthStr;

    fn region() -> Region {
        Region {
            title: "Installing software".into(),
            subtitle: "Inkscape and 2 dependencies".into(),
            badge: "00:08".into(),
            stages: vec![
                Stage {
                    label: "Resolve packages".into(),
                    state: Step::Done,
                },
                Stage {
                    label: "Download packages".into(),
                    state: Step::Working,
                },
                Stage {
                    label: "Verify packages".into(),
                    state: Step::Waiting,
                },
            ],
            detail: String::new(),
            measure: Some(Measure {
                bytes: 24_000_000,
                total: Some(35_416_473),
                rate: Some(6_000_000),
                eta: Some(2),
            }),
            transfers: vec![
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
            ],
            note: "Your current software revision is still active.".into(),
        }
    }

    /// The region is bounded: it never scrolls, at any size.
    #[test]
    fn the_region_always_fits_the_screen() {
        let region = region();
        for columns in [32, 48, 64, 80, 96, 160] {
            for rows in [12, 20, 24, 36] {
                let layout = Layout::of(columns, rows);
                let lines = region.lines(layout, Theme::PLAIN);
                assert!(lines.len() < rows, "{columns}x{rows}: {}", lines.len());
                for line in lines {
                    assert!(
                        plain(&line).width() + 2 < columns,
                        "{columns}x{rows}: {line:?}"
                    );
                }
            }
        }
    }

    /// An unknown total never becomes a percentage or a bar.
    #[test]
    fn an_unknown_total_is_never_dressed_as_progress() {
        let mut region = region();
        region.measure = Some(Measure {
            bytes: 10,
            total: None,
            rate: None,
            eta: None,
        });
        let text = region.lines(Layout::of(96, 36), Theme::PLAIN).join("\n");
        assert!(text.contains("total not yet known"), "{text}");
        assert!(!text.contains('%'), "{text}");
        assert!(!text.contains(glyph::BAR_FULL), "{text}");
    }

    /// Failure is `✗`, and it says so in a word as well.
    #[test]
    fn a_stopped_step_says_stopped() {
        let mut region = region();
        region.stages[1].state = Step::Failed;
        let text = region.lines(Layout::of(96, 36), Theme::PLAIN).join("\n");
        assert!(text.contains("Stopped"), "{text}");
        assert!(text.contains(glyph::FAILED), "{text}");
        // The retired glyph, which meant a gold advisory in the other renderer.
        assert!(!text.contains('!'), "{text}");
    }

    /// The bar is the crate's bar, and it fills in proportion.
    #[test]
    fn the_bar_is_the_one_bar() {
        let text = region().lines(Layout::of(96, 36), Theme::PLAIN).join("\n");
        let bar = text
            .lines()
            .find(|line| line.contains(glyph::BAR_FULL))
            .expect("there is a bar");
        let full = bar.matches(glyph::BAR_FULL).count();
        let empty = bar.matches(glyph::BAR_EMPTY).count();
        assert_eq!(full + empty, Layout::of(96, 36).content() - 2);
        // 24.0 of 35.4 MiB is a shade over two thirds.
        assert!(
            (0.6..0.75).contains(&(full as f64 / (full + empty) as f64)),
            "{bar}"
        );
        assert!(!text.contains('█'), "the other bar is gone: {text}");
    }

    /// Off a terminal a progress line is one printed header and every update
    /// is a no-op: a pipe must never receive carriage returns or rewrites.
    #[test]
    fn a_progress_line_without_a_terminal_prints_once_and_stays_quiet() {
        // The harness captures stdout, so this is the non-terminal path; the
        // interactive path rewrites in place and cannot be asserted against a
        // captured stream.
        let line = Line::start("acquiring packages");
        assert!(!line.interactive());
        line.update(1, 4, "one.pkg");
        line.finish();
    }

    /// The bar fills in proportion, and an unknown total is not a stall.
    #[test]
    fn the_bar_fills_in_proportion() {
        let theme = Theme::PLAIN;
        assert_eq!(theme.bar(0, 4, 10), "▱▱▱▱▱▱▱▱▱▱");
        assert_eq!(theme.bar(1, 4, 10), "▰▰▰▱▱▱▱▱▱▱");
        assert_eq!(theme.bar(4, 4, 10), "▰▰▰▰▰▰▰▰▰▰");
        assert_eq!(theme.bar(9, 4, 10), "▰▰▰▰▰▰▰▰▰▰", "overrun clamps");
        assert_eq!(theme.bar(0, 0, 4), "▰▰▰▰", "no total means done");
    }

    /// A phase line is exactly one row, with the moving number at its end.
    #[test]
    fn a_hairline_fills_to_a_right_aligned_tail() {
        let line = Theme::PLAIN.hairline("retrieving packages", "42/42", 72);
        assert!(line.starts_with("── retrieving packages "), "{line}");
        assert!(line.ends_with("42/42"), "{line}");
        assert_eq!(line.width(), 72, "{line}");
        let bare = Theme::PLAIN.hairline("resolving dependencies", "", 72);
        assert!(bare.ends_with(glyph::RULE), "{bare}");
        assert_eq!(bare.width(), 72, "{bare}");
    }

    /// A short terminal drops the step list rather than the step it is on.
    #[test]
    fn a_short_terminal_still_says_what_is_happening() {
        let text = region().lines(Layout::of(96, 12), Theme::PLAIN).join("\n");
        assert!(text.contains("Download packages"), "{text}");
        assert!(!text.contains("Verify packages"), "{text}");
    }
}

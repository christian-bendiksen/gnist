//! The work a command does once it has what it needs, as one table that is
//! filled in where it stands.
//!
//! A command that showed a page for each of its steps turned those pages
//! faster than they could be read, and the list on each grew as work started
//! that nobody had said would run. The table is laid out once, with every row
//! the work can have, in the order the work reaches them. A row is waiting,
//! working, complete or not needed; it never moves and never goes back to
//! working once it is complete. What took the time is the [`Timeline`] above
//! it, one segment a row.

use std::time::Duration;

use crate::cells::{Canvas, Style};
use crate::frame::{Fit, Piece, Rows};
use crate::ink::{Ink, glyph};
use crate::revision_line::took_label;
use crate::terminal::clip;

/// Where one row of the work has got to.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum WorkState {
    /// Not started, and it still may.
    #[default]
    Waiting,
    /// Running now.
    Working,
    /// Finished.
    Complete,
    /// The command went past it without needing it.
    Skipped,
    /// The command stopped here.
    Failed,
}

impl WorkState {
    /// The word in the status column, so the state survives a monochrome
    /// terminal and a screen reader.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Waiting => "Waiting",
            Self::Working => "Working",
            Self::Complete => "Complete",
            Self::Skipped => "Skipped",
            Self::Failed => "Stopped",
        }
    }

    const fn ink(self) -> Ink {
        match self {
            Self::Waiting | Self::Skipped => Ink::Faint,
            Self::Working => Ink::Gold,
            Self::Complete => Ink::Green,
            Self::Failed => Ink::Danger,
        }
    }

    /// Whether nothing more happens in this row.
    const fn settled(self) -> bool {
        matches!(self, Self::Complete | Self::Skipped)
    }
}

/// One row of a [`WorkTable`].
#[derive(Clone, Debug, Default)]
pub struct WorkRow {
    /// The step of the command the row is under. The table names it on the
    /// first row of each run of rows with the same step.
    pub step: String,
    pub work: String,
    pub state: WorkState,
    /// How much of a working row is done, where that is known.
    pub done: Option<f64>,
    /// Where the marker of a working row whose amount is unknown stands: any
    /// count that moves with time, so the row is seen to be alive.
    pub pulse: usize,
    /// A short fact about the row: how many, which.
    pub detail: String,
    pub took: Option<Duration>,
}

/// One line the table draws: a row, or a step folded to one line.
#[derive(Clone, Debug)]
struct Line {
    step: String,
    /// The step's name even where the line does not show it, so a line that
    /// becomes the first one shown can name it.
    under: String,
    work: String,
    state: WorkState,
    done: Option<f64>,
    pulse: usize,
    detail: String,
    took: Option<Duration>,
    /// Folded lines are muted: they stand for rows, not for one.
    folded: bool,
}

impl From<&WorkRow> for Line {
    fn from(row: &WorkRow) -> Self {
        Self {
            step: row.step.clone(),
            under: row.step.clone(),
            work: row.work.clone(),
            state: row.state,
            done: row.done,
            pulse: row.pulse,
            detail: row.detail.clone(),
            took: row.took,
            folded: false,
        }
    }
}

/// The rows of the work, under the step each belongs to.
///
/// When there are more rows than room, whole steps fold to one line each:
/// first the finished ones, oldest first, then the ones that have not
/// started, last first. The step at work keeps its rows as long as anything
/// is drawn.
#[derive(Clone, Debug, Default)]
pub struct WorkTable(pub Vec<WorkRow>);

/// The columns at a width: where each starts and how wide it is.
struct Layout {
    work: (usize, usize),
    bar: (usize, usize),
    detail: (usize, usize),
    time_end: usize,
    status: usize,
}

const STEP_WIDTH: usize = 8;
/// The narrowest the work column is made, so tables of short names still
/// line up with each other.
const WORK_WIDTH: usize = 26;
const STATUS_WIDTH: usize = 8;
const TIME_WIDTH: usize = 6;
/// The widest the detail column is made.
const DETAIL_WIDTH: usize = 18;
/// The shortest bar worth drawing.
const BAR_WIDTH: usize = 10;

impl Layout {
    /// The columns at `width`, for work names up to `longest` cells and
    /// details up to `detail` cells.
    fn at(width: usize, longest: usize, detail: usize) -> Self {
        let status = width.saturating_sub(STATUS_WIDTH);
        let time_end = status.saturating_sub(2);
        let time = time_end.saturating_sub(TIME_WIDTH);
        let work_x = STEP_WIDTH + 2;
        let detail = detail.min(DETAIL_WIDTH);
        // The work names what is done: as wide as its longest name, as long
        // as a bar and the details still fit beside it.
        let beside = if detail > 0 {
            BAR_WIDTH + 2 + detail + 2
        } else {
            BAR_WIDTH + 2
        };
        let work_width = longest
            .max(WORK_WIDTH)
            .min(time.saturating_sub(work_x + 2 + beside).max(WORK_WIDTH))
            .min(time.saturating_sub(work_x + 2));
        let bar_x = work_x + work_width + 2;
        let rest = time.saturating_sub(bar_x + 2);
        let (bar, detail) = if detail > 0 && rest >= detail + 2 + BAR_WIDTH {
            (rest - detail - 2, detail)
        } else if rest >= 6 {
            (rest, 0)
        } else {
            (0, 0)
        };
        Self {
            work: (work_x, work_width),
            bar: (bar_x, bar),
            detail: (bar_x + bar + 2, detail),
            time_end,
            status,
        }
    }
}

impl WorkTable {
    /// The runs of rows with the same step, as ranges of indices.
    fn steps(&self) -> Vec<std::ops::Range<usize>> {
        let mut runs: Vec<std::ops::Range<usize>> = Vec::new();
        for (index, row) in self.0.iter().enumerate() {
            match runs.last_mut() {
                Some(run) if self.0[run.start].step == row.step => run.end = index + 1,
                _ => runs.push(index..index + 1),
            }
        }
        runs
    }

    /// The lines to draw in `room` rows below the heading.
    fn lines(&self, room: usize) -> Vec<Line> {
        let runs = self.steps();
        // Each run is drawn whole or folded.
        let mut folded = vec![false; runs.len()];
        let count = |folded: &[bool]| -> usize {
            runs.iter()
                .zip(folded)
                .map(|(run, folded)| if *folded { 1 } else { run.len() })
                .sum()
        };
        let finished: Vec<bool> = runs
            .iter()
            .map(|run| self.0[run.clone()].iter().all(|row| row.state.settled()))
            .collect();
        let waiting: Vec<bool> = runs
            .iter()
            .map(|run| {
                self.0[run.clone()]
                    .iter()
                    .all(|row| row.state == WorkState::Waiting)
            })
            .collect();
        for index in 0..runs.len() {
            if count(&folded) <= room {
                break;
            }
            if finished[index] && runs[index].len() > 1 {
                folded[index] = true;
            }
        }
        for index in (0..runs.len()).rev() {
            if count(&folded) <= room {
                break;
            }
            if waiting[index] && runs[index].len() > 1 {
                folded[index] = true;
            }
        }
        let mut lines = Vec::new();
        for (run, folded) in runs.iter().zip(&folded) {
            let rows = &self.0[run.clone()];
            if *folded {
                let total = rows.len();
                let complete = rows
                    .iter()
                    .filter(|row| row.state == WorkState::Complete)
                    .count();
                let finished = rows.iter().all(|row| row.state.settled());
                let took: Duration = rows.iter().filter_map(|row| row.took).sum();
                lines.push(Line {
                    step: rows[0].step.clone(),
                    under: rows[0].step.clone(),
                    work: if finished {
                        format!("{complete} of {total} done")
                    } else {
                        format!("{total} to do")
                    },
                    state: if finished {
                        WorkState::Complete
                    } else {
                        WorkState::Waiting
                    },
                    done: None,
                    pulse: 0,
                    detail: String::new(),
                    took: finished.then_some(took),
                    folded: true,
                });
            } else {
                for (offset, row) in rows.iter().enumerate() {
                    let mut line = Line::from(row);
                    if offset > 0 {
                        line.step.clear();
                    }
                    lines.push(line);
                }
            }
        }
        // Still too many: keep the row at work in view, and the steps folded
        // after it, and say how many rows are not shown.
        if lines.len() > room && room > 0 {
            let at = lines
                .iter()
                .position(|line| line.state == WorkState::Working)
                .unwrap_or(0);
            let tail = lines
                .iter()
                .skip(at + 1)
                .rev()
                .take_while(|line| line.folded)
                .count();
            let tail = if room >= tail + 3 { tail } else { 0 };
            let body = lines.len() - tail;
            let shown = room.saturating_sub(1 + tail).max(1).min(body);
            let first = at.saturating_sub(shown / 2).min(body - shown);
            let hidden = body - shown;
            let folded_tail: Vec<Line> = lines.drain(body..).collect();
            let mut kept: Vec<Line> = lines.drain(first..first + shown).collect();
            if let Some(line) = kept.first_mut()
                && line.step.is_empty()
            {
                line.step = line.under.clone();
            }
            kept.push(Line {
                step: String::new(),
                under: String::new(),
                work: format!("… {hidden} more"),
                state: WorkState::Waiting,
                done: None,
                pulse: 0,
                detail: String::new(),
                took: None,
                folded: true,
            });
            kept.extend(folded_tail);
            lines = kept;
        }
        lines
    }
}

/// A thin bar: the done part heavy in `ink`, the rest a hairline. Half a cell
/// at a time, as the transfer table draws it.
fn bar(canvas: &mut Canvas<'_>, x: usize, y: usize, width: usize, done: f64, ink: Ink) {
    let halves = (done.clamp(0.0, 1.0) * (width * 2) as f64) as usize;
    let full = halves / 2;
    canvas.repeat(x, y, full, glyph::LINE_DONE, ink);
    let mut drawn = full;
    if halves % 2 == 1 {
        canvas.repeat(x + full, y, 1, glyph::LINE_HEAD, ink);
        drawn += 1;
    }
    canvas.repeat(x + drawn, y, width - drawn, glyph::RULE, Ink::Rule);
}

/// A hairline with a short heavy marker travelling along it: work whose
/// amount nobody knows, seen to be moving.
fn sweep(canvas: &mut Canvas<'_>, x: usize, y: usize, width: usize, pulse: usize) {
    const MARKER: usize = 4;
    canvas.repeat(x, y, width, glyph::RULE, Ink::Rule);
    if width <= MARKER {
        return;
    }
    let travel = width - MARKER;
    // There and back, so the marker does not jump from one end to the other.
    let step = pulse % (travel * 2);
    let at = if step <= travel {
        step
    } else {
        travel * 2 - step
    };
    canvas.repeat(x + at, y, MARKER, glyph::LINE_DONE, Ink::Gold);
}

impl Piece for WorkTable {
    fn rows(&self, _: usize, _: Fit) -> Option<Rows> {
        Some(Rows {
            least: 1 + self.steps().len().min(self.0.len()),
            most: 1 + self.0.len(),
            grows: false,
        })
    }

    fn draw(&self, canvas: &mut Canvas<'_>, _: Fit) {
        let width = canvas.width();
        let longest = self
            .0
            .iter()
            .map(|row| unicode_width::UnicodeWidthStr::width(row.work.as_str()))
            .max()
            .unwrap_or(0);
        let detail = self
            .0
            .iter()
            .map(|row| unicode_width::UnicodeWidthStr::width(row.detail.as_str()))
            .max()
            .unwrap_or(0);
        let layout = Layout::at(width, longest, detail);
        canvas.fit(0, 0, STEP_WIDTH, "STEP", Ink::Faint);
        canvas.fit(layout.work.0, 0, layout.work.1, "WORK", Ink::Faint);
        if layout.bar.1 > 0 {
            canvas.fit(layout.bar.0, 0, layout.bar.1, "PROGRESS", Ink::Faint);
        }
        canvas.end(0, layout.time_end, 0, "TIME", Ink::Faint);
        canvas.put(layout.status, 0, "STATUS", Ink::Faint);
        let room = canvas.height().saturating_sub(1);
        for (index, line) in self.lines(room).iter().enumerate() {
            let y = index + 1;
            canvas.fit(0, y, STEP_WIDTH, &line.step, Ink::Muted);
            let style = match (line.state, line.folded) {
                (_, true) => Style::new(Ink::Muted),
                (WorkState::Working, _) => Style::new(Ink::Text).bold(),
                (WorkState::Failed, _) => Style::new(Ink::Danger).bold(),
                (WorkState::Complete, _) => Style::new(Ink::Text),
                (WorkState::Waiting | WorkState::Skipped, _) => Style::new(Ink::Faint),
            };
            canvas.fit(layout.work.0, y, layout.work.1, &line.work, style);
            let (bar_x, bar_width) = layout.bar;
            match line.state {
                WorkState::Complete if bar_width > 0 => {
                    bar(canvas, bar_x, y, bar_width, 1.0, Ink::Green);
                }
                WorkState::Working if bar_width > 0 => match line.done {
                    Some(done) => bar(canvas, bar_x, y, bar_width, done, Ink::Gold),
                    None => sweep(canvas, bar_x, y, bar_width, line.pulse),
                },
                WorkState::Skipped => {
                    canvas.fit(bar_x, y, bar_width.max(10), "not needed", Ink::Faint);
                }
                _ => {}
            }
            if layout.detail.1 > 0 && !line.detail.is_empty() {
                let ink = if line.state == WorkState::Working {
                    Ink::Muted
                } else {
                    Ink::Faint
                };
                canvas.fit(layout.detail.0, y, layout.detail.1, &line.detail, ink);
            }
            if let Some(took) = line.took {
                let ink = if line.state == WorkState::Working {
                    Ink::Text
                } else {
                    Ink::Muted
                };
                canvas.end(0, layout.time_end, y, &took_label(took), ink);
            }
            if !(line.folded && line.state == WorkState::Waiting && line.work.starts_with('…')) {
                canvas.put(layout.status, y, line.state.label(), line.state.ink());
            }
        }
    }
}

/// One segment of a [`Timeline`].
#[derive(Clone, Debug, Default)]
pub struct Segment {
    pub took: Duration,
    pub ink: Ink,
    /// Written under the segment where it fits.
    pub label: String,
}

/// Where the time went: one bar across the width, a segment for each piece
/// of work as long as it took, and a figure at the right.
#[derive(Clone, Debug, Default)]
pub struct Timeline {
    pub segments: Vec<Segment>,
    pub figure: String,
    /// Whether the labels are written on a row under the bar.
    pub labels: bool,
}

impl Timeline {
    /// Each segment's first cell and width, in `width` cells.
    fn spans(&self, width: usize) -> Vec<(usize, usize)> {
        let count = self.segments.len();
        if count == 0 || width < count * 2 {
            return Vec::new();
        }
        // One cell between segments; every segment is at least one cell.
        let room = width - (count - 1) - count;
        let total: f64 = self
            .segments
            .iter()
            .map(|segment| segment.took.as_secs_f64())
            .sum();
        let mut spans = Vec::with_capacity(count);
        let mut x = 0;
        let mut before = 0.0;
        let mut used = 0;
        for segment in &self.segments {
            before += segment.took.as_secs_f64();
            let end = if total > 0.0 {
                (before / total * room as f64).round() as usize
            } else {
                0
            };
            let size = 1 + end.saturating_sub(used);
            used = end.max(used);
            spans.push((x, size));
            x += size + 1;
        }
        spans
    }
}

impl Piece for Timeline {
    fn rows(&self, _: usize, _: Fit) -> Option<Rows> {
        Some(Rows::exactly(1 + usize::from(self.labels)))
    }

    fn draw(&self, canvas: &mut Canvas<'_>, _: Fit) {
        let width = canvas.width();
        let figure = clip(&self.figure, width / 3);
        let figure_width = unicode_width::UnicodeWidthStr::width(figure.as_str());
        let room = width.saturating_sub(figure_width + if figure_width > 0 { 3 } else { 0 });
        let spans = self.spans(room);
        let mut end = 0;
        for (segment, (x, size)) in self.segments.iter().zip(&spans) {
            canvas.repeat(*x, 0, *size, glyph::LEVELS[7], segment.ink);
            end = x + size;
            if self.labels {
                let label = &segment.label;
                if !label.is_empty()
                    && unicode_width::UnicodeWidthStr::width(label.as_str()) < *size
                {
                    canvas.put(*x, 1, label, Ink::Faint);
                }
            }
        }
        if end + 1 < room {
            canvas.repeat(end + 1, 0, room - end - 1, glyph::LEVELS[0], Ink::Rule);
        }
        if figure_width > 0 {
            canvas.end(0, width, 0, &figure, Ink::Text);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cells::Buffer;
    use crate::frame::Rung;
    use crate::scale::Sizing;

    fn fit() -> Fit {
        Fit {
            sizing: Sizing::Bold,
            rung: Rung::Full,
            squeeze: 0,
        }
    }

    fn drawn(piece: &impl Piece, width: usize, height: usize) -> Vec<String> {
        let mut buffer = Buffer::new(width, height);
        piece.draw(&mut buffer.canvas(), fit());
        buffer
            .text()
            .lines()
            .map(|line| line.trim_end().to_owned())
            .collect()
    }

    fn row(step: &str, work: &str, state: WorkState, took: Option<f64>) -> WorkRow {
        WorkRow {
            step: step.into(),
            work: work.into(),
            state,
            took: took.map(Duration::from_secs_f64),
            ..WorkRow::default()
        }
    }

    fn table() -> WorkTable {
        WorkTable(vec![
            row("Verify", "Check signatures", WorkState::Complete, Some(0.5)),
            row(
                "Verify",
                "Unpack in the sandbox",
                WorkState::Complete,
                Some(4.1),
            ),
            row(
                "Create",
                "Merge into the system",
                WorkState::Complete,
                Some(2.5),
            ),
            row(
                "Create",
                "Build the generation",
                WorkState::Working,
                Some(4.4),
            ),
            row("Create", "Build the system image", WorkState::Waiting, None),
            row("Activate", "Commit the selection", WorkState::Waiting, None),
            row("Activate", "Finish", WorkState::Waiting, None),
        ])
    }

    /// Every row stands where the table put it, under its step's name once.
    #[test]
    fn rows_are_listed_in_order_with_their_step_named_once() {
        let lines = drawn(&table(), 96, 10);
        assert!(lines[0].starts_with("STEP"), "{lines:?}");
        assert!(
            lines[1].starts_with("Verify    Check signatures"),
            "{lines:?}"
        );
        assert!(
            lines[2].starts_with("          Unpack in the sandbox"),
            "{lines:?}"
        );
        assert!(lines[4].contains("Build the generation"), "{lines:?}");
        assert!(lines[4].ends_with("4.4s  Working"), "{lines:?}");
        assert!(lines[1].ends_with("0.5s  Complete"), "{lines:?}");
        assert!(lines[5].ends_with("Waiting"), "{lines:?}");
    }

    /// A short terminal folds the finished step first and the waiting one
    /// next; the step at work keeps every row.
    #[test]
    fn a_short_terminal_folds_finished_steps_then_waiting_ones() {
        let lines = drawn(&table(), 96, 6);
        assert!(
            lines[1].starts_with("Verify    2 of 2 done"),
            "the finished step folds: {lines:?}"
        );
        assert!(lines[1].ends_with("4.6s  Complete"), "{lines:?}");
        assert!(lines[2].contains("Merge into the system"), "{lines:?}");
        assert!(lines[4].contains("Build the system image"), "{lines:?}");
        assert!(
            lines[5].starts_with("Activate  2 to do"),
            "the waiting step folds: {lines:?}"
        );
    }

    #[test]
    fn a_row_the_command_did_not_need_says_so() {
        let mut table = table();
        table.0[4].state = WorkState::Skipped;
        let lines = drawn(&table, 96, 10);
        assert!(lines[5].contains("not needed"), "{lines:?}");
        assert!(lines[5].ends_with("Skipped"), "{lines:?}");
    }

    /// Each segment is as long as its share of the time, and at least one
    /// cell, with the figure at the right.
    #[test]
    fn segments_are_as_long_as_their_share() {
        let timeline = Timeline {
            segments: vec![
                Segment {
                    took: Duration::from_secs(1),
                    ink: Ink::Green,
                    label: "Resolve".into(),
                },
                Segment {
                    took: Duration::from_secs(30),
                    ink: Ink::Gold,
                    label: "Download".into(),
                },
            ],
            figure: "31.0s".into(),
            labels: true,
        };
        let lines = drawn(&timeline, 60, 2);
        let runs: Vec<usize> = lines[0]
            .split(|c: char| c != '█')
            .filter(|run| !run.is_empty())
            .map(|run| run.chars().count())
            .collect();
        assert_eq!(runs.len(), 2, "{lines:?}");
        assert!(runs[0] <= 3 && runs[1] > 40, "{lines:?}");
        // The bar, one cell between its segments, and the figure fill the row.
        assert_eq!(runs[0] + 1 + runs[1] + 3 + 5, 60, "{lines:?}");
        assert!(lines[0].ends_with("31.0s"), "{lines:?}");
        assert_eq!(lines[1].trim_start(), "Download", "{lines:?}");
    }
}

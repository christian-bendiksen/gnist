//! The revision line: every retained revision as a point on one row.
//!
//! The gold point is the revision the machine runs. A change is the line
//! growing from the revision it starts at towards a new point, divided into
//! the command's steps and filling as they happen. The line is given the
//! machine's state and nothing else, and it draws no more than that state
//! says: the gold point is drawn in one place in this file, on the revision
//! named as selected, so a change that has not committed cannot move it.
//!
//! Four rows, in either of two forms. With a stretch: revision identities, the
//! line, the steps with their marks, and a figure under each step. Without
//! one: identities, the line, what each revision was, and what state it is in.
//!
//! Without a stretch the line may end open, towards a change that has not
//! been made, or towards where the files are now when they have moved away
//! from the selected revision: that end is a diamond reached in gold, since
//! it is what the machine has. And a cursor may stand on one revision, for a
//! person who walks the line to read it.
//!
//! The line shortens itself. With a stretch it shows as many older revisions
//! as leave the steps their room, and in its short form only the revision the
//! change starts at and the stretch. Where revisions are left out, the line
//! comes in from the edge to say so.

use std::time::Duration;

use unicode_width::UnicodeWidthStr;

use crate::cells::{Canvas, Style};
use crate::ink::{Ink, Wash, glyph};
use crate::terminal::{clean, plain};

/// One retained revision.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Point {
    /// The identity a person can quote back.
    pub id: String,
    /// What the revision was, in a few words. Shown when there is no stretch.
    pub title: String,
    /// A state word: `Selected`, `Kept`. Shown when there is no stretch.
    pub badge: String,
}

impl Point {
    #[must_use]
    pub fn new(id: &str) -> Self {
        Self {
            id: id.to_owned(),
            ..Self::default()
        }
    }

    #[must_use]
    pub fn titled(mut self, title: &str) -> Self {
        self.title = title.to_owned();
        self
    }

    #[must_use]
    pub fn badged(mut self, badge: &str) -> Self {
        self.badge = badge.to_owned();
        self
    }
}

/// Where one step of a change has got to.
#[derive(Clone, Debug, Default, PartialEq)]
pub enum StepState {
    /// Will run.
    #[default]
    Waiting,
    /// Will run as soon as the person answers.
    Held,
    /// Running. `fraction` is how much of it is done, when that is known;
    /// `figure` is the live figure under it.
    Working {
        fraction: Option<f32>,
        figure: String,
    },
    /// Finished, in this long.
    Done { took: Duration },
    /// Stopped here. `reached` is how far it had got, when that is known.
    Failed { reached: Option<f32> },
    /// Never ran, because an earlier step stopped.
    NotStarted,
}

/// One step of a change.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct StretchStep {
    pub label: String,
    pub state: StepState,
}

impl StretchStep {
    #[must_use]
    pub fn new(label: &str, state: StepState) -> Self {
        Self {
            label: label.to_owned(),
            state,
        }
    }
}

/// What is at the far end of a change.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum StretchEnd {
    /// No revision: the words that stand over the place one would be, such as
    /// `new revision` or `not created`.
    Open(String),
    /// The revision at this index of the line's points.
    Reached(usize),
}

/// A change: the stretch of line between the revision it starts at and what
/// it produces.
#[derive(Clone, Debug, PartialEq)]
pub struct Stretch {
    /// The index of the revision the change starts at.
    pub origin: usize,
    /// The command's steps, in the order they run.
    pub steps: Vec<StretchStep>,
    pub end: StretchEnd,
}

/// The line.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct RevisionLine {
    /// The retained revisions, oldest first.
    pub points: Vec<Point>,
    /// The index of the revision the machine runs.
    pub selected: Option<usize>,
    /// The change in progress, finished or stopped, if there is one.
    pub stretch: Option<Stretch>,
    /// With no stretch: the words over the open end of the line, such as
    /// `your next change`. Empty for a line that ends at its last point.
    pub open_end: String,
    /// With no stretch: the words under the open end, such as `not planned
    /// yet`.
    pub open_end_note: String,
    /// Whether the open end is where the files are now, having moved away
    /// from the selected revision, and not a change still to come.
    pub moved_away: bool,
    /// With no stretch: the index of the revision under the cursor. The line
    /// shows the revisions from it on when the newest would leave it out.
    pub cursor: Option<usize>,
}

/// The cells one older revision takes: its eight-character identity and a gap.
const OLDER: usize = 10;
/// The cells a step is given when older revisions are shown beside it.
const STEP: usize = 13;
/// The cells the line comes in over when revisions are left out.
const LEAD: usize = 3;
/// The most older revisions shown beside a stretch.
const MOST_OLDER: usize = 4;
/// The cells between two points when there is no stretch.
const STRIDE: usize = 20;
/// The most revisions shown when there is no stretch.
const MOST_POINTS: usize = 6;

/// Where everything on the line is, for one width.
enum Plan {
    Change {
        /// Cells the line comes in over at the left, when revisions are left out.
        lead: usize,
        /// Older revisions shown: index and column.
        older: Vec<(usize, usize)>,
        /// The column of the revision the change starts at, if it is a point.
        origin: Option<usize>,
        /// Each step's first column and its cells.
        segments: Vec<(usize, usize)>,
    },
    History {
        lead: usize,
        /// Revisions shown: index and column.
        shown: Vec<(usize, usize)>,
        /// Whether the open end is drawn.
        open: bool,
        /// Whether newer revisions are left out at the right.
        on: bool,
    },
}

impl RevisionLine {
    /// The rows the line takes, in every form.
    pub const ROWS: usize = 4;

    /// Whether the line can be drawn `width` cells wide: in its short form, or
    /// with older revisions beside the stretch.
    #[must_use]
    pub fn fits(&self, width: usize, short: bool) -> bool {
        self.plan(width, short).is_some()
    }

    fn plan(&self, width: usize, short: bool) -> Option<Plan> {
        match self.stretch.as_ref().filter(|s| !s.steps.is_empty()) {
            Some(stretch) => self.plan_change(stretch, width, short),
            None => self.plan_history(width, short),
        }
    }

    fn plan_change(&self, stretch: &Stretch, width: usize, short: bool) -> Option<Plan> {
        let steps = stretch.steps.len();
        // A mark, a space, the name, and a space before the next step.
        let need = stretch
            .steps
            .iter()
            .map(|step| cells(&step.label) + 3)
            .max()
            .unwrap_or(0);
        let origin = (stretch.origin < self.points.len()).then_some(stretch.origin);
        let before = origin.unwrap_or(0);
        // Segments fill from the cell after the origin to the right edge; the
        // far end is the last cell of the last one.
        let segments = |start: usize| -> Option<Vec<(usize, usize)>> {
            let room = width.checked_sub(start)?;
            let (base, spare) = (room / steps, room % steps);
            let mut at = start;
            Some(
                (0..steps)
                    .map(|index| {
                        let size = base + usize::from(index < spare);
                        at += size;
                        (at - size, size)
                    })
                    .collect(),
            )
        };
        let own = usize::from(origin.is_some());
        if short || before == 0 {
            let segments = segments(own)?;
            return (segments[steps - 1].1 >= need.max(2)).then_some(Plan::Change {
                lead: 0,
                older: Vec::new(),
                origin: origin.map(|_| 0),
                segments,
            });
        }
        (1..=before.min(MOST_OLDER)).rev().find_map(|count| {
            let lead = if before > count { LEAD } else { 0 };
            let at = lead + count * OLDER;
            let segments = segments(at + own)?;
            (segments[steps - 1].1 >= need.max(STEP)).then(|| Plan::Change {
                lead,
                older: (0..count)
                    .map(|slot| (before - count + slot, lead + slot * OLDER))
                    .collect(),
                origin: Some(at),
                segments,
            })
        })
    }

    fn plan_history(&self, width: usize, short: bool) -> Option<Plan> {
        let points = self.points.len();
        let wants_open = !self.open_end.is_empty();
        if width < 2 {
            return None;
        }
        if points == 0 {
            return Some(Plan::History {
                lead: 0,
                shown: Vec::new(),
                open: wants_open,
                on: false,
            });
        }
        let most = if short { 1 } else { points.min(MOST_POINTS) };
        (1..=most).rev().find_map(|count| {
            // The newest revisions, unless that would leave out the one
            // under the cursor, or the selected one where there is no cursor:
            // then the ones from it on, and the line goes on past them.
            let first = match self
                .cursor
                .filter(|cursor| *cursor < points)
                .or(self.selected)
            {
                Some(kept) if kept < points - count => kept,
                _ => points - count,
            };
            let on = first + count < points;
            let open = wants_open && !on;
            let lead = if first > 0 { LEAD } else { 0 };
            let spans = count - 1 + usize::from(open || on);
            let stride = match spans {
                0 => 0,
                spans => width.checked_sub(lead + 1)? / spans,
            };
            (count == 1 || stride >= STRIDE).then(|| Plan::History {
                lead,
                shown: (0..count)
                    .map(|slot| (first + slot, lead + slot * stride))
                    .collect(),
                open,
                on,
            })
        })
    }

    /// Draw the line into a canvas [`RevisionLine::ROWS`] high.
    ///
    /// Draws nothing where [`RevisionLine::fits`] is false for this width.
    pub fn draw(&self, canvas: &mut Canvas<'_>, short: bool) {
        match self.plan(canvas.width(), short) {
            Some(Plan::Change {
                lead,
                older,
                origin,
                segments,
            }) => {
                if let Some(stretch) = &self.stretch {
                    self.draw_change(canvas, stretch, lead, &older, origin, &segments);
                }
            }
            Some(Plan::History {
                lead,
                shown,
                open,
                on,
            }) => self.draw_history(canvas, lead, &shown, open, on),
            None => {}
        }
    }

    /// The point of a revision, and the style of its identity above it.
    ///
    /// The only place a gold point comes from.
    fn point(&self, index: usize) -> (&'static str, Style, Style) {
        if self.selected == Some(index) {
            (
                glyph::POINT_SELECTED,
                Style::new(Ink::Gold),
                Style::new(Ink::Text).bold(),
            )
        } else {
            (
                glyph::POINT_KEPT,
                Style::new(Ink::Faint),
                Style::new(Ink::Faint),
            )
        }
    }

    fn draw_change(
        &self,
        canvas: &mut Canvas<'_>,
        stretch: &Stretch,
        lead: usize,
        older: &[(usize, usize)],
        origin: Option<usize>,
        segments: &[(usize, usize)],
    ) {
        let width = canvas.width();
        let end = width - 1;
        let failed = stretch
            .steps
            .iter()
            .any(|step| matches!(step.state, StepState::Failed { .. }));
        let finished = stretch
            .steps
            .iter()
            .all(|step| matches!(step.state, StepState::Done { .. }));
        let reached = match stretch.end {
            StretchEnd::Reached(index) if index < self.points.len() => Some(index),
            _ => None,
        };
        // A stretch that stopped, or that ended in a revision, is history: a
        // thin rule. One that is still being walked is gold where it is done.
        let over = failed || (finished && reached.is_some());

        // The line row.
        if lead > 0 {
            canvas.repeat(0, 1, 1, glyph::LINE_FROM, Ink::Rule);
            canvas.repeat(1, 1, lead - 1, glyph::RULE, Ink::Rule);
        }
        for (slot, &(index, x)) in older.iter().enumerate() {
            let (mark, style, id) = self.point(index);
            canvas.put(x, 1, mark, style);
            let next = older.get(slot + 1).map(|&(_, x)| x).or(origin).unwrap_or(x);
            canvas.repeat(x + 1, 1, next.saturating_sub(x + 1), glyph::RULE, Ink::Rule);
            canvas.fit(x, 0, OLDER - 1, &self.points[index].id, id);
        }
        let mut stopped = false;
        for (index, (step, &(start, size))) in stretch.steps.iter().zip(segments).enumerate() {
            // The far end takes the last cell of the last step.
            let line = if index + 1 == segments.len() {
                size - 1
            } else {
                size
            };
            match &step.state {
                _ if stopped => {}
                StepState::Failed { reached } => {
                    let at = (reached.unwrap_or(0.5).clamp(0.0, 1.0)
                        * line.saturating_sub(1) as f32)
                        .round() as usize;
                    canvas.repeat(start, 1, at, glyph::RULE, Ink::Faint);
                    canvas.put(start + at, 1, glyph::FAILED, Style::new(Ink::Danger).bold());
                    stopped = true;
                }
                _ if over => canvas.repeat(start, 1, line, glyph::RULE, Ink::Faint),
                StepState::Done { .. } => {
                    canvas.repeat(start, 1, line, glyph::LINE_DONE, Ink::Gold);
                }
                StepState::Working {
                    fraction: Some(fraction),
                    ..
                } => {
                    // Half a cell at a time: the head is the last gold cell.
                    let halves = (fraction.clamp(0.0, 1.0) * (line * 2) as f32).round() as usize;
                    let full = halves / 2;
                    canvas.repeat(start, 1, full, glyph::LINE_DONE, Ink::Gold);
                    let mut drawn = full;
                    if halves % 2 == 1 {
                        canvas.repeat(start + full, 1, 1, glyph::LINE_HEAD, Ink::Gold);
                        drawn += 1;
                    }
                    canvas.repeat(start + drawn, 1, line - drawn, glyph::LINE_AHEAD, Ink::Rule);
                }
                _ => canvas.repeat(start, 1, line, glyph::LINE_AHEAD, Ink::Rule),
            }
        }

        // The two ends, and the words over them.
        let (far, far_style) = match reached {
            Some(index) => {
                let (mark, style, id) = self.point(index);
                canvas.put(end, 1, mark, style);
                (self.points[index].id.as_str(), id)
            }
            None => {
                let ink = if failed { Ink::Rule } else { Ink::Faint };
                canvas.put(end, 1, glyph::POINT_AHEAD, ink);
                let label = match &stretch.end {
                    StretchEnd::Open(label) => label.as_str(),
                    StretchEnd::Reached(_) => "",
                };
                (label, Style::new(Ink::Faint))
            }
        };
        let own = origin.map_or(0, |x| x + 2);
        let far_at = canvas.end(own.min(width), width, 0, far, far_style);
        if let Some(x) = origin {
            let (mark, style, id) = self.point(stretch.origin);
            canvas.put(x, 1, mark, style);
            let room = far_at.saturating_sub(x + 2).max(1);
            canvas.fit(x, 0, room, &self.points[stretch.origin].id, id);
        }

        // The steps and their figures.
        let mut free = 0;
        for (index, (step, &(start, size))) in stretch.steps.iter().zip(segments).enumerate() {
            let last = index + 1 == segments.len();
            let (mark, mark_style, label_style) = match step.state {
                StepState::Done { .. } => (
                    glyph::DONE,
                    Style::new(Ink::Green),
                    Style::new(if over { Ink::Muted } else { Ink::Text }),
                ),
                StepState::Working { .. } => (
                    glyph::WORKING,
                    Style::new(Ink::Gold).bold(),
                    Style::new(Ink::Gold).bold(),
                ),
                StepState::Failed { .. } => (
                    glyph::FAILED,
                    Style::new(Ink::Danger).bold(),
                    Style::new(Ink::Danger).bold(),
                ),
                StepState::NotStarted => (
                    glyph::WAITING,
                    Style::new(Ink::Faint),
                    Style::new(Ink::Faint),
                ),
                StepState::Waiting | StepState::Held => (
                    glyph::WAITING,
                    Style::new(Ink::Muted),
                    Style::new(Ink::Muted),
                ),
            };
            canvas.put(start, 2, mark, mark_style);
            let room = size.saturating_sub(if last { 2 } else { 3 });
            canvas.fit(start + 2, 2, room, &step.label, label_style);

            let (figure, ink) = match &step.state {
                StepState::Done { took } => (took_label(*took), Ink::Faint),
                StepState::Working { figure, .. } => (figure.clone(), Ink::Muted),
                StepState::Held => ("waits for you".to_owned(), Ink::Gold),
                StepState::Failed { .. } => ("Stopped".to_owned(), Ink::Danger),
                StepState::NotStarted => ("Not started".to_owned(), Ink::Faint),
                StepState::Waiting => (String::new(), Ink::Faint),
            };
            let size = cells(&figure);
            if size == 0 {
                continue;
            }
            // Under the step's name; moved left if it would leave the line,
            // and never onto the figure before it.
            let x = (start + 2).min(width.saturating_sub(size)).max(free);
            free = x + canvas.put(x, 3, &figure, ink) + 1;
        }
    }

    fn draw_history(
        &self,
        canvas: &mut Canvas<'_>,
        lead: usize,
        shown: &[(usize, usize)],
        open: bool,
        on: bool,
    ) {
        let width = canvas.width();
        if lead > 0 {
            canvas.repeat(0, 1, 1, glyph::LINE_FROM, Ink::Rule);
            canvas.repeat(1, 1, lead - 1, glyph::RULE, Ink::Rule);
        }
        let mut last = lead;
        for (slot, &(index, x)) in shown.iter().enumerate() {
            let point = &self.points[index];
            let (mark, style, id) = self.point(index);
            let selected = self.selected == Some(index);
            let next = shown.get(slot + 1).map_or(width, |&(_, x)| x);
            let room = next.saturating_sub(x + 2).max(1);
            canvas.put(x, 1, mark, style);
            if self.cursor == Some(index) {
                // A wash and a mark, as a table's cursor row has: the mark is
                // what says so with colour off.
                let name = room.saturating_sub(2).min(cells(&point.id));
                canvas.wash(x, 0, (name + 3).min(room), 1, Wash::Cursor);
                canvas.put(x, 0, glyph::ROW, Style::new(Ink::Gold).bold());
                canvas.fit(
                    x + 2,
                    0,
                    room.saturating_sub(2),
                    &point.id,
                    Style::new(Ink::Text).bold(),
                );
            } else {
                canvas.fit(x, 0, room, &point.id, id);
            }
            canvas.fit(
                x,
                2,
                room,
                &point.title,
                if selected { Ink::Text } else { Ink::Muted },
            );
            canvas.fit(
                x,
                3,
                room,
                &point.badge,
                if selected { Ink::Gold } else { Ink::Faint },
            );
            if slot + 1 < shown.len() {
                canvas.repeat(x + 1, 1, next - x - 1, glyph::RULE, Ink::Rule);
            }
            last = x;
        }
        let from = if shown.is_empty() { 0 } else { last + 1 };
        if open {
            // What is still to come is faint. Where the files have gone is
            // what the machine has, so it is reached in the selected ink.
            let (line, end, over) = if self.moved_away {
                (Ink::Gold, glyph::POINT_AWAY, Ink::Gold)
            } else {
                (Ink::Rule, glyph::POINT_AHEAD, Ink::Faint)
            };
            canvas.repeat(
                from,
                1,
                (width - 1).saturating_sub(from),
                glyph::LINE_AHEAD,
                line,
            );
            canvas.put(
                width - 1,
                1,
                end,
                if self.moved_away {
                    Ink::Gold
                } else {
                    Ink::Faint
                },
            );
            // Clear of the last revision's own words, which may be longer
            // than its identity where the cursor stands on it.
            let own = |text: &str, extra: usize| {
                shown
                    .last()
                    .map_or(0, |&(_, x)| x + cells(text) + extra + 2)
                    .min(width)
            };
            let last = shown.last().map(|&(index, _)| &self.points[index]);
            let marked = shown
                .last()
                .is_some_and(|&(index, _)| self.cursor == Some(index));
            canvas.end(
                own(
                    last.map_or("", |point| &point.id),
                    if marked { 2 } else { 0 },
                ),
                width,
                0,
                &self.open_end,
                over,
            );
            canvas.end(
                own(last.map_or("", |point| &point.title), 0),
                width,
                2,
                &self.open_end_note,
                Ink::Faint,
            );
        } else if on {
            canvas.repeat(
                from,
                1,
                (width - 1).saturating_sub(from),
                glyph::RULE,
                Ink::Rule,
            );
            canvas.repeat(width - 1, 1, 1, glyph::LINE_ON, Ink::Rule);
        }
    }
}

/// The cells a piece of untrusted text takes once it is sanitized.
fn cells(text: &str) -> usize {
    clean(&plain(text)).width()
}

/// How long a step took, as it stands under the step: `0.4s`, `1m 05s`.
#[must_use]
pub fn took_label(took: Duration) -> String {
    let tenths = (took.as_secs_f64() * 10.0).round() as u64;
    if tenths < 600 {
        format!("{}.{}s", tenths / 10, tenths % 10)
    } else {
        let seconds = (tenths + 5) / 10;
        format!("{}m {:02}s", seconds / 60, seconds % 60)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cells::Buffer;

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

    const NAMES: [&str; 5] = ["Resolve", "Download", "Verify", "Create", "Activate"];

    fn done(tenths: u64) -> StepState {
        StepState::Done {
            took: Duration::from_millis(tenths * 100),
        }
    }

    fn working(fraction: f32, figure: &str) -> StepState {
        StepState::Working {
            fraction: Some(fraction),
            figure: figure.into(),
        }
    }

    fn change(states: [StepState; 5], end: StretchEnd) -> RevisionLine {
        RevisionLine {
            points: points(),
            selected: Some(2),
            stretch: Some(Stretch {
                origin: 2,
                steps: NAMES
                    .iter()
                    .zip(states)
                    .map(|(name, state)| StretchStep::new(name, state))
                    .collect(),
                end,
            }),
            ..RevisionLine::default()
        }
    }

    fn drawn(line: &RevisionLine, width: usize, short: bool) -> Buffer {
        let mut buffer = Buffer::new(width, RevisionLine::ROWS);
        line.draw(&mut buffer.canvas(), short);
        buffer
    }

    fn open() -> StretchEnd {
        StretchEnd::Open("new revision".into())
    }

    #[test]
    fn the_review_line_waits_at_the_end_of_what_is_done() {
        let line = change(
            [
                done(4),
                StepState::Held,
                StepState::Waiting,
                StepState::Waiting,
                StepState::Waiting,
            ],
            open(),
        );
        assert_eq!(
            drawn(&line, 96, false).text(),
            "\
4cd890a6  5d02b7e1  a83f19c2                                                        new revision
○─────────○─────────●━━━━━━━━━━━━━━━╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌◌
                     ✓ Resolve      ○ Download     ○ Verify       ○ Create       ○ Activate
                       0.4s           waits for you
"
        );
    }

    #[test]
    fn a_working_step_fills_by_half_cells() {
        let states = |fraction| {
            [
                done(4),
                working(fraction, "4.0s"),
                StepState::Waiting,
                StepState::Waiting,
                StepState::Waiting,
            ]
        };
        let row = |fraction| {
            let text = drawn(&change(states(fraction), open()), 96, false).text();
            text.lines().nth(1).unwrap().to_owned()
        };
        // Fifteen cells: 0.67 is twenty halves, 0.7 is twenty-one.
        assert!(
            row(0.67).contains("●━━━━━━━━━━━━━━━━━━━━━━━━━╌╌╌╌╌"),
            "{}",
            row(0.67)
        );
        assert!(
            row(0.70).contains("●━━━━━━━━━━━━━━━━━━━━━━━━━╸╌╌╌╌"),
            "{}",
            row(0.70)
        );
        assert!(row(0.0).contains("●━━━━━━━━━━━━━━━╌"), "{}", row(0.0));
        assert!(
            row(1.0).contains(&format!("●{}╌", "━".repeat(30))),
            "{}",
            row(1.0)
        );
        // A fraction outside its range claims neither more nor less.
        assert_eq!(row(7.0), row(1.0));
        assert_eq!(row(-1.0), row(0.0));
        // A step with nothing to measure fills nothing.
        let unmeasured = change(
            [
                done(4),
                StepState::Working {
                    fraction: None,
                    figure: "1.4s".into(),
                },
                StepState::Waiting,
                StepState::Waiting,
                StepState::Waiting,
            ],
            open(),
        );
        let text = drawn(&unmeasured, 96, false).text();
        assert_eq!(text.lines().nth(1).unwrap(), row(0.0));
    }

    #[test]
    fn a_finished_change_is_history_and_its_revision_is_gold() {
        let mut line = change(
            [done(4), done(59), done(10), done(26), done(8)],
            StretchEnd::Reached(3),
        );
        line.points.push(Point::new("7be204d1"));
        line.selected = Some(3);
        assert_eq!(
            drawn(&line, 96, false).text(),
            "\
4cd890a6  5d02b7e1  a83f19c2                                                            7be204d1
○─────────○─────────○──────────────────────────────────────────────────────────────────────────●
                     ✓ Resolve      ✓ Download     ✓ Verify       ✓ Create       ✓ Activate
                       0.4s           5.9s           1.0s           2.6s           0.8s
"
        );
    }

    #[test]
    fn a_stopped_change_ends_at_a_cross_and_draws_nothing_after_it() {
        let line = change(
            [
                done(4),
                done(59),
                done(10),
                StepState::Failed { reached: Some(0.7) },
                StepState::NotStarted,
            ],
            StretchEnd::Open("not created".into()),
        );
        assert_eq!(
            drawn(&line, 96, false).text(),
            "\
4cd890a6  5d02b7e1  a83f19c2                                                         not created
○─────────○─────────●───────────────────────────────────────────────────────✗                  ◌
                     ✓ Resolve      ✓ Download     ✓ Verify       ✗ Create       ○ Activate
                       0.4s           5.9s           1.0s           Stopped        Not started
"
        );
    }

    #[test]
    fn the_short_form_is_the_selected_point_and_the_stretch() {
        let line = change(
            [
                done(4),
                done(59),
                done(10),
                StepState::Failed { reached: None },
                StepState::NotStarted,
            ],
            StretchEnd::Open("not created".into()),
        );
        // Too narrow for an older revision beside five steps.
        assert!(!line.fits(76, false));
        assert!(line.fits(76, true));
        assert_eq!(
            drawn(&line, 56, true).text(),
            "\
a83f19c2                                     not created
●──────────────────────────────────────✗               ◌
 ✓ Resolve  ✓ Download ✓ Verify   ✗ Create   ○ Activate
   0.4s       5.9s       1.0s       Stopped  Not started
"
        );
        // One cell narrower and a step name would run into the next.
        assert!(!line.fits(55, true));
        assert_eq!(drawn(&line, 55, true).text().trim(), "");
    }

    #[test]
    fn revisions_that_do_not_fit_are_left_out_and_the_line_says_so() {
        let mut line = change(
            [
                done(4),
                working(0.5, "2.0s"),
                StepState::Waiting,
                StepState::Waiting,
                StepState::Waiting,
            ],
            open(),
        );
        let text = drawn(&line, 80, false).text();
        assert!(text.starts_with("   5d02b7e1  a83f19c2"), "{text}");
        assert!(
            text.lines().nth(1).unwrap().starts_with("╶──○─────────●━"),
            "{text}"
        );
        assert!(!text.contains("4cd890a6"), "{text}");
        // Revisions newer than the one the change starts at are not between
        // it and its stretch.
        line.points.push(Point::new("ffffffff"));
        let text = drawn(&line, 96, false).text();
        assert!(!text.contains("ffffffff"), "{text}");
    }

    #[test]
    fn history_spreads_the_points_and_names_them() {
        let line = RevisionLine {
            points: points(),
            selected: Some(2),
            stretch: None,
            open_end: "your next change".into(),
            ..RevisionLine::default()
        };
        assert_eq!(
            drawn(&line, 96, false).text(),
            "\
4cd890a6                       5d02b7e1                       a83f19c2          your next change
○──────────────────────────────○──────────────────────────────●╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌◌
Initial system                 Installed software             Updated software
Kept                                                          Selected
"
        );
        assert_eq!(
            drawn(&line, 56, true).text(),
            "   a83f19c2                             your next change
╶──●╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌◌
   Updated software
   Selected
"
        );
        // An older revision can be the selected one: it stays in view, and the
        // line goes on past the ones that follow.
        let mut returned = line.clone();
        returned.selected = Some(0);
        let text = drawn(&returned, 40, false).text();
        assert!(text.starts_with("4cd890a6"), "{text}");
        assert!(text.lines().nth(1).unwrap().starts_with('●'), "{text}");
        assert!(text.lines().nth(1).unwrap().ends_with('╴'), "{text}");
        assert!(!text.contains("your next change"), "{text}");
    }

    fn history() -> RevisionLine {
        RevisionLine {
            points: points(),
            selected: Some(2),
            open_end: "your next change".into(),
            ..RevisionLine::default()
        }
    }

    fn rows(line: &RevisionLine, width: usize, short: bool) -> Vec<String> {
        let text = drawn(line, width, short).text();
        text.lines().map(str::to_owned).collect()
    }

    #[test]
    fn the_open_end_may_have_words_under_it_as_well_as_over_it() {
        let mut line = history();
        line.open_end = "glo".into();
        line.open_end_note = "not planned yet".into();
        let drawn = rows(&line, 96, false);
        assert!(drawn[0].ends_with("  glo"), "{drawn:#?}");
        assert!(drawn[1].ends_with("╌╌◌"), "{drawn:#?}");
        assert!(drawn[2].contains("Updated software  "), "{drawn:#?}");
        assert!(drawn[2].ends_with("  not planned yet"), "{drawn:#?}");
        assert_eq!(drawn[2].chars().count(), 96, "{drawn:#?}");
        // Where the note and what the last revision was cannot both stand,
        // the note gives way.
        let short = rows(&line, 30, true);
        assert!(short[2].contains("Updated software  "), "{short:#?}");
        assert!(short[2].ends_with('…'), "{short:#?}");
    }

    #[test]
    fn files_that_moved_away_are_a_diamond_reached_in_gold() {
        let mut line = history();
        line.open_end = "2 files moved away".into();
        line.open_end_note = "your files now".into();
        line.moved_away = true;
        let buffer = drawn(&line, 96, false);
        let drawn_rows = rows(&line, 96, false);
        assert!(
            drawn_rows[0].ends_with("  2 files moved away"),
            "{drawn_rows:#?}"
        );
        assert!(
            drawn_rows[1].ends_with("●╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌◇"),
            "{drawn_rows:#?}"
        );
        assert!(
            drawn_rows[2].ends_with("  your files now"),
            "{drawn_rows:#?}"
        );
        // The diamond and the way to it are in the ink of what the machine
        // has; the gold point itself has not moved.
        for (x, y) in [(95, 1), (80, 1), (95, 0)] {
            assert_eq!(buffer.cell(x, y).unwrap().1.ink, Ink::Gold, "{x}, {y}");
        }
        assert_gold(&buffer, &line, "moved away");
        // A change still to come is drawn as it always was.
        let ahead = drawn(&history(), 96, false);
        assert_eq!(ahead.cell(95, 1).unwrap(), ("◌", Style::new(Ink::Faint)));
        assert_eq!(ahead.cell(80, 1).unwrap().1.ink, Ink::Rule);
    }

    #[test]
    fn the_cursor_is_a_mark_and_a_wash_on_one_revision() {
        let mut line = history();
        line.cursor = Some(1);
        let buffer = drawn(&line, 96, false);
        let marked = rows(&line, 96, false);
        assert!(marked[0].starts_with("4cd890a6 "), "{marked:#?}");
        assert!(marked[0].contains("› 5d02b7e1 "), "{marked:#?}");
        // The line itself is as it is without a cursor.
        assert_eq!(marked[1], rows(&history(), 96, false)[1]);
        let column = marked[0].chars().position(|c| c == '›').unwrap();
        assert_eq!(buffer.cell(column, 1).unwrap().0, "○");
        let (mark, style) = buffer.cell(column, 0).unwrap();
        assert_eq!(
            (mark, style.ink, style.wash),
            ("›", Ink::Gold, Wash::Cursor)
        );
        let (_, name) = buffer.cell(column + 2, 0).unwrap();
        assert_eq!((name.wash, name.bold), (Wash::Cursor, true));
        assert_eq!(buffer.cell(column + 9, 0).unwrap().1.wash, Wash::Cursor);
        assert_gold(&buffer, &line, "cursor");
        // On the last revision the words over the open end keep clear of it.
        line.cursor = Some(2);
        let last = rows(&line, 96, false);
        assert!(last[0].contains("› a83f19c2 "), "{last:#?}");
        assert!(last[0].ends_with("  your next change"), "{last:#?}");
    }

    #[test]
    fn the_line_shows_the_revisions_from_the_cursor_on() {
        let mut line = RevisionLine {
            points: (0..9)
                .map(|n| Point::new(&n.to_string().repeat(8)))
                .collect(),
            selected: Some(8),
            open_end: "your next change".into(),
            ..RevisionLine::default()
        };
        let ids = |line: &RevisionLine| {
            rows(line, 96, false)[0]
                .split_whitespace()
                .filter(|word| word.len() == 8 && word.chars().all(|c| c.is_ascii_digit()))
                .map(|word| word[..1].to_owned())
                .collect::<String>()
        };
        // The newest that fit, as without a cursor.
        assert_eq!(ids(&line), "5678");
        line.cursor = Some(6);
        assert_eq!(ids(&line), "5678");
        // Walking left of them brings the line along, and it goes on past
        // the revisions that no longer fit.
        line.cursor = Some(3);
        assert_eq!(ids(&line), "3456");
        let walked = rows(&line, 96, false);
        assert!(walked[1].starts_with('╶'), "{walked:#?}");
        assert!(walked[1].ends_with('╴'), "{walked:#?}");
        assert!(!walked[0].contains("your next change"), "{walked:#?}");
        line.cursor = Some(0);
        assert_eq!(ids(&line), "0123");
        // A cursor that points at nothing is no cursor.
        line.cursor = Some(40);
        assert_eq!(ids(&line), "5678");
    }

    /// Every combination of step states, both ends, and both revisions the
    /// machine could be running: the gold point is on the selected revision
    /// and nowhere else, and it is the only filled point on the line.
    #[test]
    fn the_gold_point_is_only_ever_on_the_selected_revision() {
        let states = [
            StepState::Waiting,
            StepState::Held,
            working(0.0, "0.1s"),
            working(1.0, "9.9s"),
            StepState::Working {
                fraction: None,
                figure: String::new(),
            },
            done(4),
            StepState::Failed { reached: None },
            StepState::Failed { reached: Some(1.0) },
            StepState::NotStarted,
        ];
        let mut cases = 0;
        for first in &states {
            for second in &states {
                for third in &states {
                    for end in [open(), StretchEnd::Reached(3), StretchEnd::Reached(9)] {
                        for selected in [None, Some(0), Some(2), Some(3)] {
                            for (width, short) in [(96, false), (80, false), (56, true)] {
                                let mut line = change(
                                    [
                                        first.clone(),
                                        second.clone(),
                                        third.clone(),
                                        StepState::Waiting,
                                        StepState::Waiting,
                                    ],
                                    end.clone(),
                                );
                                line.points.push(Point::new("7be204d1"));
                                line.selected = selected;
                                let buffer = drawn(&line, width, short);
                                assert_gold(&buffer, &line, &format!("{line:?} at {width}"));
                                cases += 1;
                            }
                        }
                    }
                }
            }
        }
        assert_eq!(cases, 9 * 9 * 9 * 3 * 4 * 3);

        // With no stretch, and with the selected revision anywhere.
        for selected in [None, Some(0), Some(1), Some(2), Some(7)] {
            for width in [96, 60, 30] {
                let line = RevisionLine {
                    points: points(),
                    selected,
                    stretch: None,
                    open_end: "your next change".into(),
                    ..RevisionLine::default()
                };
                assert_gold(
                    &drawn(&line, width, false),
                    &line,
                    &format!("{selected:?} at {width}"),
                );
            }
        }
    }

    /// Find every point on the line row and check it against the selection.
    fn assert_gold(buffer: &Buffer, line: &RevisionLine, context: &str) {
        let selected = line.selected.and_then(|index| line.points.get(index));
        let mut gold = Vec::new();
        for x in 0..buffer.columns() {
            let Some((text, style)) = buffer.cell(x, 1) else {
                continue;
            };
            if text == glyph::POINT_SELECTED {
                assert_eq!(
                    style.ink,
                    Ink::Gold,
                    "a filled point that is not gold: {context}"
                );
                gold.push(x);
            } else {
                assert!(
                    style.ink != Ink::Gold || text != glyph::POINT_KEPT,
                    "a gold hollow point: {context}"
                );
            }
        }
        assert!(gold.len() <= 1, "{} gold points: {context}", gold.len());
        let Some(&x) = gold.first() else {
            return;
        };
        let selected = selected.unwrap_or_else(|| panic!("gold with nothing selected: {context}"));
        // The identity that stands over the gold point is the selected one,
        // whichever end of the row it is aligned to.
        let row = buffer.text();
        let ids = row.lines().next().unwrap();
        let over: String = ids.chars().skip(x).take(8).collect();
        let ending: String = ids.chars().take(x + 1).collect();
        assert!(
            over == selected.id || ending.ends_with(&selected.id),
            "gold at {x} is not over {}: {context}\n{row}",
            selected.id
        );
    }

    #[test]
    fn a_stretch_whose_selection_has_not_committed_keeps_gold_where_it_was() {
        // The new revision exists and activation is running.
        let mut line = change(
            [done(4), done(59), done(10), done(26), working(0.5, "0.4s")],
            StretchEnd::Reached(3),
        );
        line.points.push(Point::new("7be204d1"));
        let text = drawn(&line, 96, false).text();
        let row = text.lines().nth(1).unwrap();
        assert!(row.contains("○─────────●━"), "{row}");
        assert!(
            row.ends_with('○'),
            "the new revision is kept, not selected: {row}"
        );
        assert!(text.lines().next().unwrap().ends_with("7be204d1"), "{text}");
    }

    #[test]
    fn untrusted_names_cannot_leave_their_cells() {
        let mut line = change(
            [
                done(4),
                working(0.5, "界面\x1b[2J a very long live figure indeed"),
                StepState::Waiting,
                StepState::Waiting,
                StepState::Waiting,
            ],
            StretchEnd::Open("a far end label that is much too long to fit anywhere".into()),
        );
        line.points[2].id = "an-identity-that-is-far-longer-than-eight".into();
        if let Some(stretch) = &mut line.stretch {
            stretch.steps[2].label = "界面界面".into();
        }
        for width in [56, 60, 76, 96] {
            for short in [true, false] {
                let text = drawn(&line, width, short).text();
                for row in text.lines() {
                    assert!(row.width() <= width, "{width}: {row:?}");
                }
                assert!(!text.contains('\x1b'), "{text:?}");
            }
        }
    }

    #[test]
    fn times_read_as_a_person_would_say_them() {
        assert_eq!(took_label(Duration::from_millis(400)), "0.4s");
        assert_eq!(took_label(Duration::from_millis(5_940)), "5.9s");
        assert_eq!(took_label(Duration::from_millis(59_940)), "59.9s");
        assert_eq!(took_label(Duration::from_secs(65)), "1m 05s");
        assert_eq!(took_label(Duration::ZERO), "0.0s");
    }
}

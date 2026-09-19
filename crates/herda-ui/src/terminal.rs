//! Shared Herda terminal typography and content layouts.
//!
//! The palette, the glyphs and the seam are in [`crate::ink`]; the widths are
//! in [`crate::width`]. What lives here is the typography that assembles them:
//! headings, rules, cells, and the bounded [`Document`] that a command's
//! scrollback output is built into.
use std::io::IsTerminal;
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

pub use crate::document::Document;
pub use crate::ink::{Ink, glyph};
pub use crate::width::Layout;

/// What this terminal can render, decided once and then carried.
///
/// Every renderer in the crate is a pure function of its inputs plus a `Theme`,
/// and returns byte-for-byte plain text when `color` is off — so a harness, a
/// pipe and a serial console all get exactly the text they always got.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Theme {
    pub color: bool,
    pub truecolor: bool,
}

impl Theme {
    /// Never styled: machine output, tests, and anything being piped.
    pub const PLAIN: Self = Self {
        color: false,
        truecolor: false,
    };

    /// Always styled, at full depth. For fixtures and for tests that pin what
    /// a real terminal sees.
    pub const FULL: Self = Self {
        color: true,
        truecolor: true,
    };

    /// Always styled, at 256 colours. The fallback path, which has its own
    /// palette and no seam, and so needs its own fixtures.
    pub const INDEXED: Self = Self {
        color: true,
        truecolor: false,
    };

    /// Decide from the environment and the terminal.
    #[must_use]
    pub fn detect() -> Self {
        Self::detect_for(false)
    }

    /// Decide, knowing whether this output is for a machine.
    ///
    /// Machine output is never styled, whatever the environment says: a reader
    /// parsing stdout must receive one clean document.
    ///
    /// The precedence below is the whole of the policy, and it exists because
    /// the crate used to hold two detections that disagreed about it —
    /// `FORCE_COLOR=1` beat `NO_COLOR` in one and lost in the other, and an
    /// unset `TERM` disabled colour in one only, so a tty with no `TERM` was
    /// styled or not depending on which half of the crate had drawn the line:
    ///
    /// 1. machine output is plain;
    /// 2. `NO_COLOR` is plain, whatever else is set;
    /// 3. `FORCE_COLOR` decides, beating both the tty check and `TERM=dumb`;
    /// 4. otherwise: a tty that is not `TERM=dumb`. An *unset* `TERM` is not
    ///    `dumb` — it is a terminal that did not say, and the escapes it
    ///    cannot use are invisible either way.
    #[must_use]
    pub fn detect_for(machine: bool) -> Self {
        if machine || std::env::var_os("NO_COLOR").is_some() {
            return Self::PLAIN;
        }
        let color = match std::env::var("FORCE_COLOR").as_deref() {
            Ok("0") => false,
            Ok(_) => true,
            Err(_) => std::io::stdout().is_terminal() && !dumb(),
        };
        Self {
            color,
            truecolor: color && deep(),
        }
    }

    /// Wrap `text` in the escapes for `ink`, or return it untouched.
    #[must_use]
    pub fn paint(self, text: &str, ink: Ink, bold: bool) -> String {
        if !self.color {
            return text.to_owned();
        }
        let code = ink.code(self.truecolor);
        format!("\x1b[{};{code}m{text}\x1b[0m", if bold { 1 } else { 22 })
    }

    /// A flat hairline: a break between sections, and the unstyled fallback
    /// for the seam.
    #[must_use]
    pub fn rule(self, width: usize) -> String {
        self.paint(&glyph::RULE.to_string().repeat(width), Ink::Rule, false)
    }

    /// A verdict line: a glyph, then the text. `✓ done`, `✗ nope`, `· held`.
    ///
    /// The glyph carries the meaning when colour is off, which is the case a
    /// serial console and a log file are always in.
    #[must_use]
    pub fn mark(self, glyph: &str, ink: Ink, text: &str) -> String {
        format!("{} {text}", self.paint(glyph, ink, true))
    }

    /// A finished thing: `✓ text`.
    #[must_use]
    pub fn ok(self, text: &str) -> String {
        self.mark(glyph::DONE, Ink::Green, text)
    }

    /// Something neutral or still pending: `· text`.
    #[must_use]
    pub fn pending(self, text: &str) -> String {
        self.mark(glyph::NEUTRAL, Ink::Muted, text)
    }

    /// Something that failed: `✗ text`.
    #[must_use]
    pub fn fail(self, text: &str) -> String {
        self.mark(glyph::FAILED, Ink::Danger, text)
    }

    /// A caution the person should read before answering: `│ text`.
    ///
    /// This is the collision the restructure exists to fix. A warning used to
    /// be painted `1;38;5;180` — the identical sequence a heading was painted
    /// in — so it was not distinguishable as a warning at all, and a test
    /// asserted that. It is now a shape as well as a colour, which is also what
    /// makes it survive `NO_COLOR`.
    #[must_use]
    pub fn warn(self, text: &str) -> String {
        self.mark(glyph::RAIL, Ink::Gold, text)
    }

    /// A phase line: `── label ──────────────────────── tail`.
    ///
    /// The rule fills the row from the label to a right-aligned `tail` — a bar,
    /// a counter, a size, an elapsed time — so every phase line ends at the
    /// same column and the moving number stays in one place. An empty `tail`
    /// closes the rule at the row's end, which is the shape a phase prints once
    /// when the output is not a terminal.
    ///
    /// `width` is the row to fill. It used to be read from `$COLUMNS`, which no
    /// common shell exports, so this rendered at exactly 72 columns however
    /// wide the terminal was — above a table in the same command that had asked
    /// the kernel and got the real answer.
    #[must_use]
    pub fn hairline(self, label: &str, tail: &str, width: usize) -> String {
        let lead = 3 + label.width(); // "── " + label + the joining space
        // The space before the tail exists only when there is a tail, so an
        // empty one leaves the rule running to the last column rather than one
        // short of it.
        let tail_width = plain(tail).width();
        let gap = usize::from(!tail.is_empty());
        let rule_width = width.saturating_sub(lead + tail_width + gap + 1).max(2);
        let rule: String = std::iter::repeat_n(glyph::RULE, rule_width).collect();
        format!(
            "{} {} {}{}{tail}",
            self.paint("──", Ink::Rule, false),
            self.paint(label, Ink::Gold, true),
            self.paint(&rule, Ink::Rule, false),
            " ".repeat(gap),
        )
    }

    /// A bar of `width` cells, filled in proportion to `done`/`total`.
    ///
    /// A finished bar turns green. No total means there is nothing left to
    /// wait for, so the bar is full — not empty, which would read as stalled.
    #[must_use]
    pub fn bar(self, done: usize, total: usize, width: usize) -> String {
        let filled = if total == 0 {
            width
        } else {
            (done.min(total) * width).div_ceil(total)
        };
        let ink = if total > 0 && done >= total {
            Ink::Green
        } else {
            Ink::Gold
        };
        format!(
            "{}{}",
            self.paint(
                &std::iter::repeat_n(glyph::BAR_FULL, filled).collect::<String>(),
                ink,
                false
            ),
            self.paint(
                &std::iter::repeat_n(glyph::BAR_EMPTY, width - filled).collect::<String>(),
                Ink::Rule,
                false
            )
        )
    }

    /// The gradient rule that sits under a heading.
    ///
    /// The one place the two halves of the design meet: the rule grey lifted
    /// toward the brand amber and back. Truecolor only — see [`crate::ink`] for
    /// why it degrades to [`Theme::rule`] rather than banding.
    #[must_use]
    pub fn seam(self, width: usize) -> String {
        crate::ink::seam(width, self.color, self.truecolor)
    }

    pub fn title(self, title: &str) -> String {
        self.title_for("pkg", title)
    }

    pub fn title_for(self, app: &str, title: &str) -> String {
        format!(
            "{}  {}",
            self.paint(app, Ink::Gold, true),
            self.paint(title, Ink::Text, true)
        )
    }

    pub fn heading(self, title: &str, tail: &str, width: usize) -> String {
        self.heading_for("pkg", title, tail, width)
    }

    pub fn heading_for(self, app: &str, title: &str, tail: &str, width: usize) -> String {
        let prefix = app.width() + 2;
        if width <= prefix {
            return self.paint(&clip(app, width), Ink::Gold, true);
        }
        if width < title.width() + tail.width() + prefix + 4 {
            return self.title_for(app, &clip(title, width.saturating_sub(prefix)));
        }
        format!(
            "{}{}{}",
            self.title_for(app, title),
            " ".repeat(width - title.width() - tail.width() - prefix),
            self.paint(tail, Ink::Muted, false)
        )
    }

    pub fn cell(self, text: &str, width: usize, ink: Ink, bold: bool) -> String {
        self.paint(&pad(text, width), ink, bold)
    }
}

/// Whether stdout is a terminal a person is watching.
///
/// Not the same question as "should this be styled" — see [`Theme::detect_for`]
/// — and not the same as "is this for a machine", which only the presentation
/// layer knows. This is the one a progress region asks before it holds a line
/// open for rewriting.
#[must_use]
pub fn interactive_output() -> bool {
    std::io::stdout().is_terminal() && !dumb()
}

/// A terminal that says it cannot render escapes. An unset `TERM` does not say.
pub(crate) fn dumb() -> bool {
    std::env::var("TERM").is_ok_and(|term| term == "dumb")
}

/// Whether the terminal can take 24-bit colour, and so the seam.
fn deep() -> bool {
    if matches!(
        std::env::var("COLORTERM").as_deref(),
        Ok("truecolor" | "24bit")
    ) {
        return true;
    }
    std::env::var("TERM").is_ok_and(|term| {
        term.contains("kitty") || term.contains("truecolor") || term.ends_with("-direct")
    })
}

pub fn size() -> (usize, usize) {
    crossterm::terminal::size()
        .map(|(w, h)| (usize::from(w), usize::from(h)))
        .unwrap_or((80, 24))
}

pub fn size_label(bytes: u64) -> String {
    let text = if bytes >= 1 << 30 {
        format!("{:.2} GiB", bytes as f64 / (1u64 << 30) as f64)
    } else if bytes >= 1 << 20 {
        format!("{:.2} MiB", bytes as f64 / (1u64 << 20) as f64)
    } else if bytes >= 1 << 10 {
        format!("{:.2} KiB", bytes as f64 / (1u64 << 10) as f64)
    } else {
        format!("{bytes} B")
    };
    match text.split_once(' ') {
        Some((number, unit)) if number.contains('.') => format!(
            "{} {unit}",
            number.trim_end_matches('0').trim_end_matches('.')
        ),
        _ => text,
    }
}

/// Strip control sequences from external package names/descriptions/errors.
pub fn clean(text: &str) -> String {
    text.chars()
        .map(|c| {
            if c.is_control() || matches!(c, '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}') {
                ' '
            } else {
                c
            }
        })
        .collect()
}

pub fn clip(text: &str, width: usize) -> String {
    let text = clean(text);
    if text.width() <= width {
        return text;
    }
    if width == 0 {
        return String::new();
    }
    let mut result = String::new();
    let mut used = 0;
    for c in text.chars() {
        let n = c.width().unwrap_or(0);
        if used + n > width - 1 {
            break;
        }
        used += n;
        result.push(c);
    }
    result.push('…');
    result
}

pub fn pad(text: &str, width: usize) -> String {
    let text = clip(text, width);
    format!("{text}{}", " ".repeat(width.saturating_sub(text.width())))
}

pub fn wrap(text: &str, width: usize) -> Vec<String> {
    if width == 0 {
        return Vec::new();
    }
    let mut lines = Vec::new();
    let mut line = String::new();
    for word in clean(text).split_whitespace() {
        if !line.is_empty() && line.width() + 1 + word.width() > width {
            lines.push(line);
            line = String::new();
        }
        if !line.is_empty() {
            line.push(' ');
        }
        line.push_str(&clip(word, width));
    }
    if !line.is_empty() {
        lines.push(line);
    }
    lines
}

/// Word wrapping that retains every character, including long paths and digests.
pub fn fold(text: &str, width: usize) -> Vec<String> {
    if width == 0 {
        return Vec::new();
    }
    let mut result = Vec::new();
    let mut line = String::new();
    for word in clean(text).split_whitespace() {
        if !line.is_empty() && line.width() + word.width() + 1 > width {
            result.push(std::mem::take(&mut line));
        }
        if !line.is_empty() {
            line.push(' ');
        }
        for ch in word.chars() {
            let n = ch.width().unwrap_or(0);
            if line.width() + n > width {
                if !line.is_empty() {
                    result.push(std::mem::take(&mut line));
                }
                // A terminal narrower than a wide glyph cannot represent it.
                if n > width {
                    result.push("�".into());
                    continue;
                }
            }
            line.push(ch);
        }
    }
    if !line.is_empty() {
        result.push(line);
    }
    if result.is_empty() {
        result.push(String::new());
    }
    result
}

/// Removes existing ANSI sequences before applying a single trusted style.
pub fn plain(text: &str) -> String {
    let mut chars = text.chars().peekable();
    let mut output = String::new();
    while let Some(ch) = chars.next() {
        if ch == '\x1b' {
            match chars.next() {
                Some('[') => {
                    for ch in chars.by_ref() {
                        if ('@'..='~').contains(&ch) {
                            break;
                        }
                    }
                }
                Some(']') => {
                    while let Some(ch) = chars.next() {
                        if ch == '\x07' || (ch == '\x1b' && chars.next() == Some('\\')) {
                            break;
                        }
                    }
                }
                _ => {}
            }
        } else if ch == '\n' || ch == '\t' {
            output.push(ch);
        } else {
            output.push_str(&clean(&ch.to_string()));
        }
    }
    output
}

/// Applies the shared document layout to existing human report builders.
/// Source headings/sections remain semantic; no JSON or machine frame enters here.
pub fn report(app: &str, body: &str, badge: &str, columns: usize, theme: Theme) -> String {
    let body = plain(body);
    let mut lines = body.trim().lines();
    let Some(title) = lines.next() else {
        return String::new();
    };
    let mut document = Document::new(app, title.trim(), badge, columns, theme);
    let mut section = true;
    for line in lines {
        let text = line.trim();
        if text.is_empty() {
            section = true;
            continue;
        }
        if section && !line.starts_with(char::is_whitespace) {
            document.section(text);
        } else if line.starts_with("  ") && matches!(text.get(..2), Some("+ " | "- " | "~ " | "! "))
        {
            let (marker, content) = text.split_at(2);
            let (title, detail) = content.split_once("  ").unwrap_or((content, ""));
            let (status, ink) = match marker {
                "+ " => ("Add", Ink::Green),
                "- " => ("Remove", Ink::Danger),
                "~ " => ("Change", Ink::Gold),
                _ => ("Review", Ink::Gold),
            };
            document.item(title, detail.trim(), status, ink);
        } else if line.starts_with("  ") {
            let field = text.split_once("  ").or_else(|| text.split_once(": "));
            if let Some((label, value)) =
                field.filter(|(label, value)| label.len() <= 24 && !value.is_empty())
            {
                document.field(label.trim_end_matches(':'), value.trim(), Ink::Text);
            } else {
                document.text(text, Ink::Text);
            }
        } else {
            document.text(text, Ink::Muted);
        }
        section = false;
    }
    document.render()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn documents_fit_and_retain_long_paths_and_untrusted_text() {
        for width in [24, 40, 60, 80, 120] {
            let theme = Theme {
                color: true,
                truecolor: true,
            };
            let path = "/home/界面/a-long-directory/another-very-long-directory/settings.kdl";
            let mut document = Document::new("malm", "Deployment", "REVIEW", width, theme);
            document.field("Target", path, Ink::Text);
            document.item(
                "A package\x1b[2J\u{202e}",
                "Version 1.0",
                "Installed",
                Ink::Green,
            );
            let rendered = document.render();
            assert!(plain(&rendered).lines().all(|line| line.width() <= width));
            assert!(!rendered.contains("\x1b[2J") && !rendered.contains('\u{202e}'));
            assert_eq!(fold(path, width - 4).join(""), path);
        }
    }

    #[test]
    fn reports_retain_complete_error_and_review_text_on_narrow_terminals() {
        let message = "Cannot apply the reviewed plan because /home/example/a-very-long-path/settings.kdl changed. Review again before applying.";
        for width in [8, 24, 40, 80] {
            let text = report(
                "malm",
                message,
                "ERROR",
                width,
                Theme {
                    color: false,
                    truecolor: false,
                },
            );
            assert!(text.lines().all(|line| line.width() <= width));
            let compact = text.split_whitespace().collect::<String>();
            assert!(compact.contains(&message.split_whitespace().collect::<String>()));
        }
    }

    #[test]
    fn brand_and_monochrome_share_the_same_layout() {
        let body = "Source is valid\n  Profiles  12\n\nNext\n  malm deploy\n";
        let color = report("malm", body, "SOURCE CHECK", 80, Theme::FULL);
        let mono = report("malm", body, "SOURCE CHECK", 80, Theme::PLAIN);
        assert_eq!(plain(&color), mono);
        assert!(
            color.contains(Ink::Gold.code(true)),
            "the brand is the brand"
        );
        assert!(mono.contains("malm  Source is valid"));
        assert!(!mono.contains('\x1b'));

        // The same document at 256 colours: same layout, different depth.
        let indexed = report("malm", body, "SOURCE CHECK", 80, Theme::INDEXED);
        assert_eq!(plain(&indexed), mono);
        assert!(indexed.contains(Ink::Gold.code(false)));
        assert!(
            !indexed.contains("38;2;"),
            "no truecolor on an indexed term"
        );
    }

    /// The precedence the two old detections disagreed about.
    ///
    /// Serialised, because it reads and writes process-wide environment.
    #[test]
    fn detection_follows_one_precedence() {
        fn with(vars: &[(&str, Option<&str>)], body: impl FnOnce()) {
            let saved: Vec<_> = vars
                .iter()
                .map(|(k, _)| (*k, std::env::var_os(k)))
                .collect();
            for (key, value) in vars {
                match value {
                    Some(value) => unsafe { std::env::set_var(key, value) },
                    None => unsafe { std::env::remove_var(key) },
                }
            }
            body();
            for (key, value) in saved {
                match value {
                    Some(value) => unsafe { std::env::set_var(key, value) },
                    None => unsafe { std::env::remove_var(key) },
                }
            }
        }

        let _guard = ENV.lock().unwrap_or_else(|poisoned| poisoned.into_inner());

        // Machine output is plain before anything else is consulted.
        with(&[("NO_COLOR", None), ("FORCE_COLOR", Some("1"))], || {
            assert_eq!(Theme::detect_for(true), Theme::PLAIN);
        });
        // NO_COLOR beats FORCE_COLOR. It used to lose here and win there.
        with(
            &[("NO_COLOR", Some("1")), ("FORCE_COLOR", Some("1"))],
            || assert_eq!(Theme::detect(), Theme::PLAIN),
        );
        // FORCE_COLOR beats a pipe, which is the whole point of it, and beats
        // TERM=dumb, because it is an explicit instruction and dumb is a guess.
        with(
            &[
                ("NO_COLOR", None),
                ("FORCE_COLOR", Some("1")),
                ("TERM", Some("dumb")),
                ("COLORTERM", Some("truecolor")),
            ],
            || assert_eq!(Theme::detect(), Theme::FULL),
        );
        with(&[("NO_COLOR", None), ("FORCE_COLOR", Some("0"))], || {
            assert_eq!(Theme::detect(), Theme::PLAIN);
        });
        // An unset TERM is not "dumb"; under a test harness stdout is a pipe,
        // so this asserts the thing that is decidable without a pty.
        with(
            &[("NO_COLOR", None), ("FORCE_COLOR", None), ("TERM", None)],
            || assert!(!dumb()),
        );
    }
}

#[cfg(test)]
static ENV: std::sync::Mutex<()> = std::sync::Mutex::new(());

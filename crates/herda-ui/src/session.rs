//! A command happening inside one full-screen frame: who holds the terminal
//! while it does.
//!
//! [`crate::frame`] is what such a screen looks like and [`crate::surface`]
//! the terminal it is shown on. Here is everything between a tool's command
//! and that terminal which is the same whatever the command is: the thread
//! that draws, the keys, where the process's output goes while the screen is
//! up, and how all of it is put back. A tool supplies what the screen
//! believes and draws, through [`Shown`], and says when its command takes
//! the terminal, asks, and leaves.
//!
//! # Who does what
//!
//! The command runs on the thread it always ran on and is never driven from
//! here. What it reports, the tool gives to its model ([`Session::model`]),
//! which is everything the screen believes. A thread of its own owns the
//! terminal: it draws the model, no more often than the live region redraws,
//! and between frames it waits for a key or a signal, so the process is
//! never deaf to either. Three things cross from the command to that thread
//! and wait for it: taking the terminal when there is first something to
//! show ([`Session::take`]), a question ([`Session::ask`]), and giving the
//! terminal back ([`Session::leave`], and [`Session::lend`] for as long as
//! something else needs it).
//!
//! The one thing that crosses the other way is a request to stop
//! ([`Session::stop_requested`]), which the command reads at points where it
//! already returns errors.
//!
//! # The terminal while the screen is up
//!
//! Nothing but the screen may write to it. The process's standard output and
//! standard error are turned into a pipe for as long as the screen is up,
//! and each line that arrives is given to the model ([`Shown::output`]): a
//! line another crate prints, and whatever a child process says, lands there
//! and not on the frame. The screen itself is written to a descriptor of the
//! terminal kept from before.
//!
//! Ctrl-C, `SIGTERM` and `SIGHUP` mean what they mean without the screen:
//! the terminal is restored and the process ends by that signal.
//!
//! All of this is behind the crate's `session` feature: turning descriptors
//! and signalling the process group are `rustix`, which nothing else in the
//! crate calls.

use std::io::{BufRead as _, IsTerminal as _, Write as _};
use std::os::fd::{AsFd, OwnedFd};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use crate::browser::{KeyCode, KeyEvent, KeyModifiers};
use crate::cells::Buffer;
use crate::scale::Sizing;
use crate::surface::{Input, Surface};
use crate::width::{FRAME, FRAME_ROWS, Layout};

/// What a process is attached to, as far as showing a screen goes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Attached {
    /// `TERM=dumb`.
    pub dumb: bool,
    pub stdin_terminal: bool,
    pub stdout_terminal: bool,
    pub stderr_terminal: bool,
    /// The terminal's columns and rows, when standard output is one.
    pub size: Option<(usize, usize)>,
}

impl Attached {
    /// What this process is attached to.
    #[must_use]
    pub fn detect() -> Self {
        let stdout = std::io::stdout();
        Self {
            dumb: std::env::var("TERM").is_ok_and(|term| term == "dumb"),
            stdin_terminal: std::io::stdin().is_terminal(),
            stdout_terminal: stdout.is_terminal(),
            stderr_terminal: std::io::stderr().is_terminal(),
            size: size_of(&stdout),
        }
    }

    /// Whether what is written to standard output can be drawn over: it is
    /// a terminal, and not a dumb one. Where it cannot, a command prints
    /// lines.
    #[must_use]
    pub const fn redraws(&self) -> bool {
        self.stdout_terminal && !self.dumb
    }

    /// Whether a screen can be shown here: the terminal can be asked for
    /// keys, nothing the command says goes anywhere else, and the frame
    /// fits.
    #[must_use]
    pub fn takes_a_frame(&self) -> bool {
        self.redraws()
            && self.stdin_terminal
            && self.stderr_terminal
            && self
                .size
                .is_some_and(|(columns, rows)| columns >= FRAME && rows >= FRAME_ROWS)
    }
}

/// The columns and rows of the terminal behind `terminal`.
fn size_of(terminal: &impl AsFd) -> Option<(usize, usize)> {
    let size = rustix::termios::tcgetwinsize(terminal).ok()?;
    Some((usize::from(size.ws_col), usize::from(size.ws_row)))
}

/// A key as a model reads it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Pressed {
    Enter,
    Escape,
    Up,
    Down,
    Left,
    Right,
    PageUp,
    PageDown,
    Home,
    End,
    Char(char),
}

/// What a key asks of whoever holds the screen.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    /// Nothing beyond drawing again.
    None,
    /// The answer to the question the command asked.
    Answer(bool),
    /// Ask the working command to stop at its next safe point.
    Cancel,
    /// Put this text on the clipboard.
    Copy(String),
    /// Leave the screen.
    Leave,
}

/// What a session shows: a tool's model of its command.
///
/// Every method is called with the model locked, by the thread that holds
/// the terminal or the one that reads the process's output, so none of them
/// may wait for the command.
pub trait Shown: Send + 'static {
    /// The screen for a terminal of this size at this instant. Where the
    /// frame cannot be drawn this is [`crate::frame::notice`], so that a
    /// terminal made too small says the same in every tool.
    fn draw(&mut self, layout: Layout, sizing: Sizing, now: Instant) -> Buffer;

    /// Forget what was shown only for a while and whose time has passed.
    /// Returns whether anything was: the screen is then drawn again.
    fn settle(&mut self, now: Instant) -> bool;

    /// Whether what is drawn changes with time alone: a clock is on screen.
    fn ticking(&self) -> bool;

    /// Take a key, and say what it asks for.
    ///
    /// A model that returns [`Action::Cancel`] has taken the request and
    /// shows that it has. The session raises [`Session::stop_requested`]
    /// before it lets go of the model, so a command that looks for a request
    /// with the model locked sees one the model took, or none.
    fn key(&mut self, key: Pressed, now: Instant) -> Action;

    /// Whether the screen stays up at the end until a key is pressed.
    fn waits_at_the_end(&self) -> bool;

    /// One line that another crate or a child wrote to standard output or
    /// standard error while the screen held the terminal, without its
    /// escape sequences and control characters.
    fn output(&mut self, line: &str);
}

/// One command's screen.
pub struct Session<M> {
    shared: Arc<Shared<M>>,
}

/// What one command's screen is made of.
struct Shared<M> {
    model: Mutex<M>,
    /// Set whenever the model changed, and cleared by the frame that shows it.
    dirty: AtomicBool,
    /// Whether the screen holds the terminal now.
    up: AtomicBool,
    /// Whether the screen could not be shown, or its thread ended.
    gone: AtomicBool,
    /// Whether the person asked the working command to stop.
    stop: AtomicBool,
    /// Whether the screen ever held the terminal.
    entered: AtomicBool,
    link: Mutex<Option<Sender<Request>>>,
    /// Where the answer to the question being asked arrives.
    answer: Mutex<Option<Receiver<bool>>>,
}

/// What the command's thread asks of the thread that holds the terminal.
/// Each is answered when it is done.
enum Request {
    Ask(Sender<bool>),
    /// Give the terminal back until told to take it again. Answered with
    /// whether it was held.
    Yield(Sender<bool>),
    Retake(Sender<()>),
    Leave(Sender<()>),
}

fn lock<T>(shared: &Mutex<T>) -> MutexGuard<'_, T> {
    shared
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

impl<M: Shown> Session<M> {
    /// A screen for the command `model` is the model of. Nothing is shown,
    /// and the terminal is not touched, before [`Session::take`] or
    /// [`Session::ask`].
    #[must_use]
    pub fn new(model: M) -> Self {
        Self {
            shared: Arc::new(Shared {
                model: Mutex::new(model),
                dirty: AtomicBool::new(true),
                up: AtomicBool::new(false),
                gone: AtomicBool::new(false),
                stop: AtomicBool::new(false),
                entered: AtomicBool::new(false),
                link: Mutex::new(None),
                answer: Mutex::new(None),
            }),
        }
    }

    /// The model, locked. Whoever changes it says so with
    /// [`Session::changed`] once it is let go of.
    pub fn model(&self) -> MutexGuard<'_, M> {
        lock(&self.shared.model)
    }

    /// The model changed: the next frame shows it.
    pub fn changed(&self) {
        self.shared.dirty.store(true, Ordering::Release);
    }

    /// Whether the screen holds the terminal now.
    #[must_use]
    pub fn up(&self) -> bool {
        self.shared.up.load(Ordering::Acquire)
    }

    /// Whether the person asked the working command to stop. A command that
    /// must not miss a request looks with the model locked: see
    /// [`Shown::key`].
    #[must_use]
    pub fn stop_requested(&self) -> bool {
        self.shared.stop.load(Ordering::Acquire)
    }

    /// Have the screen hold the terminal: for the first time, when there is
    /// first something to show, or again after it was lent.
    pub fn take(&self) {
        if self.enter() && !self.up() {
            let (reply, retaken) = mpsc::channel();
            if self.request(Request::Retake(reply)) {
                let _ = retaken.recv();
            }
        }
    }

    /// The command is about to ask a question, which its model then shows.
    ///
    /// The screen is told that an answer is wanted before it can show the
    /// question, so a key pressed the moment the question appears has
    /// somewhere to go.
    pub fn ask(&self) {
        let (reply, answered) = mpsc::channel();
        *lock(&self.shared.answer) =
            (self.enter() && self.request(Request::Ask(reply))).then_some(answered);
    }

    /// The answer a person gave on the screen to the question last asked.
    ///
    /// A screen that could not be shown answers no: nothing is written
    /// before a person has confirmed what they were shown.
    #[must_use]
    pub fn answer(&self) -> bool {
        let answered = lock(&self.shared.answer).take();
        answered.is_some_and(|answered| answered.recv().unwrap_or(false))
    }

    /// Give the terminal to something that needs it to itself, until
    /// [`Session::take`]. Returns whether the screen held it and has now
    /// given it back: what the caller then prints is ordinary scrollback.
    pub fn lend(&self) -> bool {
        if !self.enter() {
            return false;
        }
        let (reply, yielded) = mpsc::channel();
        self.request(Request::Yield(reply)) && yielded.recv().unwrap_or(false)
    }

    /// Give the terminal back for good, once the command has ended, waiting
    /// for a key first where the model says a person should read the result.
    /// Returns whether the screen ever held the terminal.
    pub fn leave(&self) -> bool {
        let (reply, left) = mpsc::channel();
        if self.request(Request::Leave(reply)) {
            let _ = left.recv();
        }
        self.shared.up.store(false, Ordering::Release);
        *lock(&self.shared.link) = None;
        self.shared.entered.load(Ordering::Acquire)
    }

    /// Take the terminal, if it is not held yet. Returns whether it is held.
    fn enter(&self) -> bool {
        let shared = &self.shared;
        let mut link = lock(&shared.link);
        if link.is_some() {
            return !shared.gone.load(Ordering::Acquire);
        }
        if shared.gone.load(Ordering::Acquire) {
            return false;
        }
        let (requests, received) = mpsc::channel();
        let (ready, entered) = mpsc::channel();
        let screen = Arc::clone(shared);
        let spawned = std::thread::Builder::new()
            .name("staged-screen".into())
            .spawn(move || Screen::run(&screen, &received, &ready));
        *link = Some(requests);
        let held = spawned.is_ok() && entered.recv().unwrap_or(false);
        if !held {
            shared.gone.store(true, Ordering::Release);
        }
        held
    }

    /// Ask the screen's thread for something. Returns whether it was asked.
    fn request(&self, request: Request) -> bool {
        if self.shared.gone.load(Ordering::Acquire) {
            return false;
        }
        lock(&self.shared.link)
            .as_ref()
            .is_some_and(|link| link.send(request).is_ok())
    }
}

/// The descriptors standard output and standard error had before they were
/// turned into the pipe, for the panic hook to put back.
static TURNED: Mutex<Option<(OwnedFd, OwnedFd)>> = Mutex::new(None);

/// Standard output and standard error turned into lines for the model, for
/// as long as the screen is up.
struct Capture {
    drained: Receiver<()>,
}

impl Capture {
    /// Turn the two descriptors into a pipe whose lines go to the model, and
    /// return a descriptor of the terminal to draw on.
    fn start<M: Shown>(shared: &Arc<Shared<M>>) -> std::io::Result<(Self, std::fs::File)> {
        let _ = std::io::stdout().flush();
        let out = std::io::stdout().as_fd().try_clone_to_owned()?;
        let err = std::io::stderr().as_fd().try_clone_to_owned()?;
        let terminal = std::fs::File::from(out.try_clone()?);
        let (reader, writer) = std::io::pipe()?;
        let (done, drained) = mpsc::channel();
        let shared = Arc::clone(shared);
        std::thread::Builder::new()
            .name("staged-output".into())
            .spawn(move || {
                let mut reader = std::io::BufReader::new(reader);
                let mut line = Vec::new();
                loop {
                    line.clear();
                    match reader.read_until(b'\n', &mut line) {
                        Ok(0) | Err(_) => break,
                        Ok(_) => {}
                    }
                    let text = String::from_utf8_lossy(&line);
                    let text = crate::terminal::clean(&crate::terminal::plain(
                        text.trim_end_matches(['\n', '\r']),
                    ));
                    if text.trim().is_empty() {
                        continue;
                    }
                    lock(&shared.model).output(&text);
                    shared.dirty.store(true, Ordering::Release);
                }
                let _ = done.send(());
            })?;
        *lock(&TURNED) = Some((out, err));
        static HOOK: std::sync::Once = std::sync::Once::new();
        HOOK.call_once(|| {
            // A panic is reported on the terminal, not into the pipe: the
            // descriptors are put back before the hooks that restore the
            // screen and print the message run.
            let previous = std::panic::take_hook();
            std::panic::set_hook(Box::new(move |info| {
                if let Ok(mut turned) = TURNED.try_lock() {
                    restore(turned.take());
                }
                previous(info);
            }));
        });
        rustix::stdio::dup2_stdout(&writer)?;
        rustix::stdio::dup2_stderr(&writer)?;
        Ok((Self { drained }, terminal))
    }

    /// Give the two descriptors back, and wait a moment for what was already
    /// written to reach the model.
    fn stop(self) {
        let _ = std::io::stdout().flush();
        restore(lock(&TURNED).take());
        // A child that outlives the screen keeps the pipe open; what it
        // writes later still reaches the model.
        let _ = self.drained.recv_timeout(Duration::from_millis(150));
    }
}

fn restore(turned: Option<(OwnedFd, OwnedFd)>) {
    if let Some((out, err)) = turned {
        let _ = rustix::stdio::dup2_stdout(&out);
        let _ = rustix::stdio::dup2_stderr(&err);
    }
}

/// How long the screen waits for a key before it looks at the model again.
const TICK: Duration = Duration::from_millis(40);
/// The least time between two frames: the live region's.
const CADENCE: Duration = Duration::from_millis(80);

/// The terminal, held by the screen's thread.
struct Screen<M> {
    shared: Arc<Shared<M>>,
    surface: Surface,
    capture: Option<Capture>,
    /// A descriptor of the terminal, to ask its size of.
    terminal: OwnedFd,
    drawn: Instant,
    /// The clock a frame last showed, so that a second passing draws one.
    second: u64,
    started: Instant,
}

impl<M: Shown> Screen<M> {
    fn take(shared: &Arc<Shared<M>>) -> std::io::Result<Self> {
        let mut surface = Surface::enter()?;
        let terminal = std::io::stdout().as_fd().try_clone_to_owned()?;
        let capture = match Capture::start(shared) {
            Ok((capture, drawn_on)) => {
                surface.write_to(drawn_on);
                Some(capture)
            }
            Err(_) => {
                restore(lock(&TURNED).take());
                None
            }
        };
        shared.up.store(true, Ordering::Release);
        shared.entered.store(true, Ordering::Release);
        shared.dirty.store(true, Ordering::Release);
        Ok(Self {
            shared: Arc::clone(shared),
            surface,
            capture,
            terminal,
            drawn: Instant::now() - CADENCE,
            second: u64::MAX,
            started: Instant::now(),
        })
    }

    /// Give the terminal back: the descriptors first, so that what follows
    /// is written to the terminal, then the screen.
    fn give_back(self) {
        self.shared.up.store(false, Ordering::Release);
        if let Some(capture) = self.capture {
            capture.stop();
        }
        drop(self.surface);
    }

    /// The thread that holds the terminal, from the first thing to show
    /// until the command leaves.
    fn run(shared: &Arc<Shared<M>>, requests: &Receiver<Request>, ready: &Sender<bool>) {
        let mut screen = match Self::take(shared) {
            Ok(screen) => Some(screen),
            Err(_) => {
                shared.gone.store(true, Ordering::Release);
                let _ = ready.send(false);
                return;
            }
        };
        let _ = ready.send(true);
        let mut asked: Option<Sender<bool>> = None;
        // An answer given before this thread took the request for it. The
        // request is sent before the question can be on the screen, so an
        // answer is always to a question whose request is on its way.
        let mut given: Option<bool> = None;
        let mut leaving: Option<Sender<()>> = None;
        loop {
            if screen.is_none() {
                // The terminal is with whatever it was lent to. Nothing is
                // read from it here, and a signal does what it does without
                // the screen.
                match requests.recv() {
                    Ok(Request::Retake(reply)) => {
                        screen = Self::take(shared).ok();
                        if screen.is_none() {
                            shared.gone.store(true, Ordering::Release);
                        }
                        let _ = reply.send(());
                        if screen.is_none() {
                            return;
                        }
                    }
                    Ok(Request::Ask(reply) | Request::Yield(reply)) => {
                        let _ = reply.send(false);
                    }
                    Ok(Request::Leave(reply)) => {
                        let _ = reply.send(());
                    }
                    Err(_) => return,
                }
                continue;
            }
            loop {
                match requests.try_recv() {
                    Ok(Request::Ask(reply)) => match given.take() {
                        Some(accepted) => {
                            let _ = reply.send(accepted);
                        }
                        None => asked = Some(reply),
                    },
                    Ok(Request::Yield(reply)) => {
                        if let Some(held) = screen.take() {
                            held.give_back();
                        }
                        let _ = reply.send(true);
                        break;
                    }
                    Ok(Request::Retake(reply)) => {
                        let _ = reply.send(());
                    }
                    Ok(Request::Leave(reply)) => {
                        if lock(&shared.model).waits_at_the_end() {
                            leaving = Some(reply);
                            shared.dirty.store(true, Ordering::Release);
                        } else {
                            if let Some(held) = screen.take() {
                                held.give_back();
                            }
                            let _ = reply.send(());
                            return;
                        }
                    }
                    Err(mpsc::TryRecvError::Empty) => break,
                    // The command's thread is gone without leaving.
                    Err(mpsc::TryRecvError::Disconnected) => {
                        if let Some(held) = screen.take() {
                            held.give_back();
                        }
                        return;
                    }
                }
            }
            let Some(held) = screen.as_mut() else {
                continue;
            };
            if held.draw().is_err() {
                // A terminal that cannot be written to cannot ask anything.
                shared.gone.store(true, Ordering::Release);
                if let Some(held) = screen.take() {
                    held.give_back();
                }
                if let Some(reply) = asked.take() {
                    let _ = reply.send(false);
                }
                if let Some(reply) = leaving.take() {
                    let _ = reply.send(());
                }
                return;
            }
            match held.surface.wait(TICK) {
                Ok(Input::Key(key)) => {
                    if interrupts(key) {
                        if let Some(held) = screen.take() {
                            held.give_back();
                        }
                        interrupt();
                    }
                    let Some(pressed) = pressed(key) else {
                        continue;
                    };
                    let action = {
                        let mut model = lock(&shared.model);
                        let action = model.key(pressed, Instant::now());
                        // Raised before the model is let go of, which the
                        // command's last safe point also holds while it
                        // looks.
                        if action == Action::Cancel {
                            shared.stop.store(true, Ordering::Release);
                        }
                        action
                    };
                    shared.dirty.store(true, Ordering::Release);
                    match action {
                        Action::None | Action::Cancel => {}
                        Action::Answer(accepted) => match asked.take() {
                            Some(reply) => {
                                let _ = reply.send(accepted);
                            }
                            None => given = Some(accepted),
                        },
                        Action::Copy(text) => {
                            let _ = held.surface.send(clipboard(&text).as_bytes());
                        }
                        Action::Leave => {
                            if let Some(reply) = leaving.take() {
                                if let Some(held) = screen.take() {
                                    held.give_back();
                                }
                                let _ = reply.send(());
                                return;
                            }
                        }
                    }
                }
                Ok(Input::Resized) => shared.dirty.store(true, Ordering::Release),
                Ok(Input::Quiet) => {}
                // The process was told to end. Dropping the screen restores
                // the terminal and lets the signal take its course.
                Ok(Input::Interrupted) | Err(_) => {
                    if let Some(held) = screen.take() {
                        held.give_back();
                    }
                    // Reached only if the signal's own action did not end
                    // the process.
                    shared.gone.store(true, Ordering::Release);
                    if let Some(reply) = asked.take() {
                        let _ = reply.send(false);
                    }
                    if let Some(reply) = leaving.take() {
                        let _ = reply.send(());
                    }
                    return;
                }
            }
        }
    }

    /// Draw the model if it changed or its clock moved, no more often than
    /// the live region does.
    fn draw(&mut self) -> std::io::Result<()> {
        let now = Instant::now();
        let mut model = lock(&self.shared.model);
        let second = now.duration_since(self.started).as_secs();
        let mut due = self.shared.dirty.load(Ordering::Acquire) || model.settle(now);
        // Figures that count time move a few times a second.
        if model.ticking() && now.duration_since(self.drawn) >= Duration::from_millis(250) {
            due = true;
        }
        if second != self.second && model.ticking() {
            due = true;
        }
        if !due || now.duration_since(self.drawn) < CADENCE {
            return Ok(());
        }
        self.shared.dirty.store(false, Ordering::Release);
        let layout = size_of(&self.terminal)
            .map_or_else(Layout::detect, |(columns, rows)| Layout::of(columns, rows));
        let buffer = model.draw(layout, self.surface.sizing(), now);
        drop(model);
        self.drawn = now;
        self.second = second;
        self.surface.show(&buffer)
    }
}

/// Whether a key is the one that ends the process: in raw mode the terminal
/// sends Ctrl-C as a key and raises no signal.
fn interrupts(key: KeyEvent) -> bool {
    key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c')
}

/// End the process as Ctrl-C ends it without the screen: the signal goes to
/// the whole foreground group, as the terminal would have sent it, so that
/// workers and builds end with the command.
fn interrupt() -> ! {
    let _ = rustix::process::kill_current_process_group(rustix::process::Signal::INT);
    // The signal's own action ends the process; if something keeps it from
    // that, the status is the one it would have had.
    std::thread::sleep(Duration::from_millis(200));
    std::process::exit(130);
}

fn pressed(key: KeyEvent) -> Option<Pressed> {
    if key
        .modifiers
        .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT)
    {
        return None;
    }
    Some(match key.code {
        KeyCode::Enter => Pressed::Enter,
        KeyCode::Esc => Pressed::Escape,
        KeyCode::Up => Pressed::Up,
        KeyCode::Down => Pressed::Down,
        KeyCode::Left => Pressed::Left,
        KeyCode::Right => Pressed::Right,
        KeyCode::PageUp => Pressed::PageUp,
        KeyCode::PageDown => Pressed::PageDown,
        KeyCode::Home => Pressed::Home,
        KeyCode::End => Pressed::End,
        KeyCode::Char(character) => Pressed::Char(character),
        _ => return None,
    })
}

/// The most text put on the clipboard: terminals refuse a longer sequence.
const CLIPBOARD_LIMIT: usize = 48 * 1024;

/// The sequence that puts `text` on the clipboard through the terminal
/// (OSC 52). A text too long for it keeps its end, which is where a log
/// says what happened last.
fn clipboard(text: &str) -> String {
    let mut start = text.len().saturating_sub(CLIPBOARD_LIMIT);
    while !text.is_char_boundary(start) {
        start += 1;
    }
    format!("\x1b]52;c;{}\x1b\\", base64(&text.as_bytes()[start..]))
}

/// `bytes` in the base64 alphabet of RFC 4648, padded.
fn base64(bytes: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut text = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let group = u32::from(chunk[0]) << 16
            | u32::from(chunk.get(1).copied().unwrap_or(0)) << 8
            | u32::from(chunk.get(2).copied().unwrap_or(0));
        for place in 0..4 {
            if place <= chunk.len() {
                text.push(ALPHABET[(group >> (18 - 6 * place) & 63) as usize] as char);
            } else {
                text.push('=');
            }
        }
    }
    text
}

#[cfg(all(test, unix))]
#[path = "session_tests.rs"]
mod session_tests;

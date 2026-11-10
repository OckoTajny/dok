//! Terminal helpers for the `dok` CLI: colours, tables, spinners and upload
//! progress. Everything degrades to plain, script-friendly text when stdout is
//! not a TTY, when `NO_COLOR`/`DOK_NO_COLOR` is set, or when `--json` is used.

use std::io::{IsTerminal, Read, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

static JSON: AtomicBool = AtomicBool::new(false);

/// Switch machine-readable output on/off for the current process.
pub fn set_json(on: bool) {
    JSON.store(on, Ordering::Relaxed);
}

pub fn json() -> bool {
    JSON.load(Ordering::Relaxed)
}

pub fn stdout_tty() -> bool {
    std::io::stdout().is_terminal()
}

pub fn stderr_tty() -> bool {
    std::io::stderr().is_terminal()
}

fn colour_disabled() -> bool {
    std::env::var_os("NO_COLOR").is_some()
        || std::env::var_os("DOK_NO_COLOR").is_some()
        || std::env::var("CLICOLOR").map(|v| v == "0").unwrap_or(false)
        || std::env::var("TERM").map(|t| t == "dumb").unwrap_or(false)
        || !stdout_tty()
}

/// Colour is opt-out: a TTY without NO_COLOR/TERM=dumb/CLICOLOR=0.
pub fn colour() -> bool {
    static ENABLED: OnceLock<bool> = OnceLock::new();
    *ENABLED.get_or_init(|| !colour_disabled())
}

pub fn paint(code: &str, text: &str) -> String {
    if colour() {
        format!("\x1b[{code}m{text}\x1b[0m")
    } else {
        text.to_string()
    }
}

pub fn bold(s: &str) -> String {
    paint("1", s)
}
pub fn dim(s: &str) -> String {
    paint("2", s)
}
pub fn red(s: &str) -> String {
    paint("31", s)
}
pub fn green(s: &str) -> String {
    paint("32", s)
}
pub fn yellow(s: &str) -> String {
    paint("33", s)
}
pub fn cyan(s: &str) -> String {
    paint("36", s)
}
pub fn magenta(s: &str) -> String {
    paint("35", s)
}

pub fn arrow() -> String {
    cyan("→")
}

pub fn info(msg: &str) {
    println!("{} {msg}", arrow());
}

pub fn success(msg: &str) {
    println!("{} {msg}", green("✓"));
}

pub fn error(msg: &str) {
    eprintln!("{} {msg}", red("✗"));
}

pub fn human_bytes(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut value = bytes as f64;
    let mut unit = 0usize;
    while value >= 1024.0 && unit + 1 < UNITS.len() {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes} B")
    } else {
        format!("{value:.1} {}", UNITS[unit])
    }
}

pub fn human_age(ts: i64) -> String {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(ts);
    let delta = now.saturating_sub(ts);
    match delta {
        0..=9 => "just now".to_string(),
        10..=59 => format!("{delta}s ago"),
        60..=3599 => format!("{}m ago", delta / 60),
        3600..=86_399 => format!("{}h ago", delta / 3600),
        _ => format!("{}d ago", delta / 86_400),
    }
}

pub fn human_window(secs: i64) -> String {
    if secs >= 86_400 {
        format!("{}d", secs / 86_400)
    } else if secs >= 3600 {
        format!("{}h", secs / 3600)
    } else if secs >= 60 {
        format!("{}m", secs / 60)
    } else {
        format!("{secs}s")
    }
}

/// Width of a string on screen, ignoring ANSI SGR escape sequences.
pub fn visible_width(s: &str) -> usize {
    let mut width = 0usize;
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\x1b' {
            if chars.peek() == Some(&'[') {
                chars.next();
                for next in chars.by_ref() {
                    if next.is_ascii_alphabetic() {
                        break;
                    }
                }
            }
            continue;
        }
        width += 1;
    }
    width
}

fn pad(s: &str, width: usize) -> String {
    let visible = visible_width(s);
    format!("{s}{}", " ".repeat(width.saturating_sub(visible)))
}

/// Print an aligned table. Cells may contain ANSI colours.
pub fn print_table(headers: &[&str], rows: &[Vec<String>]) {
    if rows.is_empty() {
        println!("  {}", dim("(none)"));
        return;
    }
    let mut widths: Vec<usize> = headers.iter().map(|h| visible_width(h)).collect();
    for row in rows {
        for (i, cell) in row.iter().enumerate() {
            if i < widths.len() {
                widths[i] = widths[i].max(visible_width(cell));
            }
        }
    }
    let head: Vec<String> = headers
        .iter()
        .enumerate()
        .map(|(i, h)| pad(&bold(h), widths[i]))
        .collect();
    println!("{}", head.join("  ").trim_end());
    for row in rows {
        let cells: Vec<String> = row
            .iter()
            .enumerate()
            .map(|(i, cell)| pad(cell, widths.get(i).copied().unwrap_or(0)))
            .collect();
        println!("{}", cells.join("  ").trim_end());
    }
}

struct SpinState {
    message: Mutex<String>,
    stop: AtomicBool,
    started: Instant,
}

/// A small animated status line. On a non-TTY (or with `--json`) it prints a
/// single `→ message` line instead, so logs stay readable.
pub struct Spinner {
    state: Option<Arc<SpinState>>,
    handle: Option<std::thread::JoinHandle<()>>,
}

impl Spinner {
    pub fn start(message: impl Into<String>) -> Self {
        let message = message.into();
        if json() || !stdout_tty() {
            if !json() {
                info(&message);
            }
            return Self {
                state: None,
                handle: None,
            };
        }
        let state = Arc::new(SpinState {
            message: Mutex::new(message),
            stop: AtomicBool::new(false),
            started: Instant::now(),
        });
        let thread_state = state.clone();
        let handle = std::thread::spawn(move || {
            const FRAMES: [&str; 10] = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];
            let mut frame = 0usize;
            let mut out = std::io::stdout();
            while !thread_state.stop.load(Ordering::Relaxed) {
                let message = thread_state
                    .message
                    .lock()
                    .map(|m| m.clone())
                    .unwrap_or_default();
                let secs = thread_state.started.elapsed().as_secs();
                let _ = write!(
                    out,
                    "\r\x1b[2K  {} {message} {}",
                    cyan(FRAMES[frame % FRAMES.len()]),
                    dim(&format!("· {secs}s"))
                );
                let _ = out.flush();
                frame += 1;
                std::thread::sleep(Duration::from_millis(90));
            }
            let _ = write!(out, "\r\x1b[2K");
            let _ = out.flush();
        });
        Self {
            state: Some(state),
            handle: Some(handle),
        }
    }

    fn stop(&mut self) {
        if let Some(state) = &self.state {
            state.stop.store(true, Ordering::Relaxed);
        }
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }

    pub fn finish(mut self) {
        self.stop();
    }
}

impl Drop for Spinner {
    fn drop(&mut self) {
        self.stop();
    }
}

/// Wraps an upload body and paints a progress bar on a TTY. The shared flag
/// flips once the whole body has been read, so the caller can switch from
/// "uploading" to a deploy spinner while the server is still building.
pub struct ProgressReader<R> {
    inner: R,
    total: u64,
    sent: u64,
    draw: bool,
    started: Instant,
    last_draw: Instant,
    finished: bool,
    done: Arc<AtomicBool>,
}

impl<R: Read> ProgressReader<R> {
    pub fn new(inner: R, total: u64) -> (Self, Arc<AtomicBool>) {
        let done = Arc::new(AtomicBool::new(false));
        let draw = stdout_tty() && !json();
        (
            Self {
                inner,
                total,
                sent: 0,
                draw,
                started: Instant::now(),
                last_draw: Instant::now(),
                finished: false,
                done: done.clone(),
            },
            done,
        )
    }

    fn render(&mut self, complete: bool) {
        if !self.draw {
            return;
        }
        let (width, elapsed) = (26usize, self.started.elapsed().as_secs_f64());
        let fraction = if self.total == 0 {
            1.0
        } else {
            (self.sent as f64 / self.total as f64).clamp(0.0, 1.0)
        };
        let filled = (fraction * width as f64).round() as usize;
        let bar = format!(
            "{}{}",
            cyan(&"█".repeat(filled)),
            dim(&"░".repeat(width - filled))
        );
        let mut out = std::io::stdout();
        let _ = write!(
            out,
            "\r\x1b[2K  {} [{}] {:>3}% · {}/{} · {:.1}s",
            dim("uploading"),
            bar,
            (fraction * 100.0).round() as usize,
            human_bytes(self.sent),
            human_bytes(self.total),
            elapsed
        );
        if complete {
            let _ = writeln!(out);
        }
        let _ = out.flush();
    }
}

impl<R: Read> Read for ProgressReader<R> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let read = self.inner.read(buf)?;
        if read == 0 {
            if !self.finished {
                self.finished = true;
                self.render(true);
                self.done.store(true, Ordering::Relaxed);
            }
            return Ok(0);
        }
        self.sent = self.sent.saturating_add(read as u64);
        let complete = self.sent >= self.total;
        if complete {
            self.done.store(true, Ordering::Relaxed);
        }
        if self.draw && (complete || self.last_draw.elapsed() >= Duration::from_millis(80)) {
            self.last_draw = Instant::now();
            self.render(complete && self.finished);
            if complete {
                self.finished = true;
            }
        }
        Ok(read)
    }
}

impl<R> Drop for ProgressReader<R> {
    fn drop(&mut self) {
        if self.draw && !self.finished {
            let mut out = std::io::stdout();
            let _ = write!(out, "\r\x1b[2K");
            let _ = out.flush();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn visible_width_ignores_ansi() {
        assert_eq!(visible_width("\x1b[32mok\x1b[0m"), 2);
        assert_eq!(visible_width("plain"), 5);
    }

    #[test]
    fn bytes_and_ages_are_readable() {
        assert_eq!(human_bytes(512), "512 B");
        assert_eq!(human_bytes(1536), "1.5 KB");
        assert!(human_age(0).ends_with("ago") || human_age(0) == "just now");
    }
}

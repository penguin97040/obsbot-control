//! Thread-safe in-memory activity log, mirrored to a local log file.

use std::collections::VecDeque;
use std::fs::File;
use std::io::Write;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;

const MAX_ENTRIES: usize = 2000;
/// Stop mirroring to disk past this size.
const MAX_FILE_BYTES: u64 = 10 * 1024 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Level {
    Info,
    Warn,
    Error,
}

impl Level {
    fn tag(self) -> &'static str {
        match self {
            Level::Info => "INFO ",
            Level::Warn => "WARN ",
            Level::Error => "ERROR",
        }
    }
}

#[derive(Clone, Debug)]
pub struct Entry {
    /// HH:MM:SS, 24-hour local time.
    pub time: String,
    pub level: Level,
    pub text: String,
}

struct LogFile {
    file: File,
    written: u64,
    full: bool,
}

struct State {
    entries: VecDeque<Entry>,
    file: Option<LogFile>,
}

static STATE: Mutex<State> = Mutex::new(State { entries: VecDeque::new(), file: None });
static GENERATION: AtomicU64 = AtomicU64::new(0);

fn lock() -> std::sync::MutexGuard<'static, State> {
    STATE.lock().unwrap_or_else(|e| e.into_inner())
}

/// Mirrors every later entry to this UTF-8 file, which is truncated first.
pub fn open_file(path: &Path) {
    if let Ok(file) = File::create(path) {
        lock().file = Some(LogFile { file, written: 0, full: false });
    }
}

pub fn write(level: Level, text: String) {
    let time = chrono::Local::now().format("%H:%M:%S").to_string();
    let mut state = lock();
    if let Some(f) = state.file.as_mut() {
        if !f.full {
            let line = format!("{time} {} {text}\n", level.tag());
            if f.file.write_all(line.as_bytes()).is_ok() {
                f.written += line.len() as u64;
            }
            if f.written > MAX_FILE_BYTES {
                let _ = writeln!(
                    f.file,
                    "{time} {} Log file size limit reached; later entries are only shown in the app.",
                    Level::Warn.tag()
                );
                f.full = true;
            }
            let _ = f.file.flush();
        }
    }
    state.entries.push_back(Entry { time, level, text });
    if state.entries.len() > MAX_ENTRIES {
        state.entries.pop_front();
    }
    drop(state);
    GENERATION.fetch_add(1, Ordering::Relaxed);
}

/// Changes whenever an entry is added or the log is cleared.
pub fn generation() -> u64 {
    GENERATION.load(Ordering::Relaxed)
}

pub fn snapshot() -> Vec<Entry> {
    lock().entries.iter().cloned().collect()
}

pub fn all_as_text() -> String {
    let mut out = String::new();
    for e in lock().entries.iter() {
        out.push_str(&format!("{} {} {}\n", e.time, e.level.tag(), e.text));
    }
    out
}

pub fn clear() {
    lock().entries.clear();
    GENERATION.fetch_add(1, Ordering::Relaxed);
}

#[macro_export]
macro_rules! log_info {
    ($($arg:tt)*) => { $crate::logx::write($crate::logx::Level::Info, format!($($arg)*)) };
}
#[macro_export]
macro_rules! log_warn {
    ($($arg:tt)*) => { $crate::logx::write($crate::logx::Level::Warn, format!($($arg)*)) };
}
#[macro_export]
macro_rules! log_error {
    ($($arg:tt)*) => { $crate::logx::write($crate::logx::Level::Error, format!($($arg)*)) };
}

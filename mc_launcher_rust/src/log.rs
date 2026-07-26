//! Logging utilities with ANSI color support.
#![allow(dead_code)]

#[cfg(not(windows))]
use std::io::{self, IsTerminal};
use std::sync::atomic::{AtomicBool, Ordering};

static QUIET: AtomicBool = AtomicBool::new(false);
static VERBOSE: AtomicBool = AtomicBool::new(false);

pub fn set_quiet(q: bool) {
    QUIET.store(q, Ordering::Relaxed);
}
pub fn set_verbose(v: bool) {
    VERBOSE.store(v, Ordering::Relaxed);
}

// --- ANSI colour support ---

static COLOR_OK: AtomicBool = AtomicBool::new(false);
static COLOR_CHECKED: AtomicBool = AtomicBool::new(false);

const RST: &str = "\x1b[0m";
const CLR_RED: &str = "\x1b[91m";
const CLR_GREEN: &str = "\x1b[92m";
const CLR_YELLOW: &str = "\x1b[93m";
const CLR_CYAN: &str = "\x1b[96m";
const CLR_GRAY: &str = "\x1b[90m";
const CLR_BOLD: &str = "\x1b[1m";

fn colors_enabled() -> bool {
    if COLOR_CHECKED.load(Ordering::Relaxed) {
        return COLOR_OK.load(Ordering::Relaxed);
    }
    let ok = if std::env::var("NO_COLOR").is_ok() {
        false
    } else {
        enable_vt()
    };
    COLOR_OK.store(ok, Ordering::Relaxed);
    COLOR_CHECKED.store(true, Ordering::Relaxed);
    ok
}

#[cfg(windows)]
fn enable_vt() -> bool {
    extern "system" {
        fn GetStdHandle(nStdHandle: u32) -> isize;
        fn GetConsoleMode(hConsoleHandle: isize, lpMode: *mut u32) -> i32;
        fn SetConsoleMode(hConsoleHandle: isize, dwMode: u32) -> i32;
    }
    const ENABLE_VIRTUAL_TERMINAL_PROCESSING: u32 = 0x0004;
    unsafe {
        let handle = GetStdHandle(0xFFFFFFF5u32);
        if handle == -1 {
            return false;
        }
        let mut mode: u32 = 0;
        if GetConsoleMode(handle, &mut mode) == 0 {
            return false;
        }
        mode |= ENABLE_VIRTUAL_TERMINAL_PROCESSING;
        SetConsoleMode(handle, mode) != 0
    }
}

#[cfg(not(windows))]
fn enable_vt() -> bool {
    io::stdout().is_terminal()
}

pub fn clr(c: &str, text: &str) -> String {
    if colors_enabled() {
        format!("{}{}{}", c, text, RST)
    } else {
        text.to_string()
    }
}

// --- logging macros and functions ---

#[macro_export]
macro_rules! info {
    ($($t:tt)*) => {{
        if !$crate::log::is_quiet() {
            println!("  {}", format!($($t)*));
        }
    }};
}

#[macro_export]
macro_rules! success {
    ($($t:tt)*) => {{
        if !$crate::log::is_quiet() {
            println!("  {} {}", $crate::log::clr("\x1b[92m", "[OK]"), format!($($t)*));
        }
    }};
}

#[macro_export]
macro_rules! warn_msg {
    ($($t:tt)*) => {
        println!("  {} {}", $crate::log::clr("\x1b[93m", "[WARN]"), format!($($t)*));
    };
}

#[macro_export]
macro_rules! error_msg {
    ($($t:tt)*) => {
        eprintln!("  {} {}", $crate::log::clr("\x1b[91m", "[ERROR]"), format!($($t)*));
    };
}

#[macro_export]
macro_rules! debug_msg {
    ($($t:tt)*) => {{
        if $crate::log::is_verbose() {
            println!("  {} {}", $crate::log::clr("\x1b[90m", "[DBG]"), format!($($t)*));
        }
    }};
}

#[macro_export]
macro_rules! die {
    ($msg:expr) => {{
        $crate::error_msg!("{}", $msg);
        std::process::exit(1);
    }};
    ($msg:expr, $hint:expr) => {{
        $crate::error_msg!("{}", $msg);
        eprintln!("         {}", $crate::log::clr("\x1b[90m", $hint));
        std::process::exit(1);
    }};
}

pub fn header(title: &str) {
    if !QUIET.load(Ordering::Relaxed) {
        println!(
            "\n  {}\n",
            clr(CLR_BOLD, &format!("=== {} ===", title))
        );
    }
}

pub fn step(n: usize, total: usize, msg: &str) {
    if !QUIET.load(Ordering::Relaxed) {
        println!(
            "{} {}",
            clr(CLR_CYAN, &format!("[{}/{}]", n, total)),
            msg
        );
    }
}

pub fn is_quiet() -> bool {
    QUIET.load(Ordering::Relaxed)
}
pub fn is_verbose() -> bool {
    VERBOSE.load(Ordering::Relaxed)
}

//! The verbosity of a run, as Composer's `ConsoleIO` applies it: every line
//! vivacity writes has the level of its Composer counterpart, and is shown
//! when the run's verbosity reaches it.
//!
//! - `QUIET` lines are always shown (`IOInterface::QUIET`: the unsolvable
//!   headline, the stale-lock warning, an exception's message, and vivacity's
//!   own reasons for stopping with exit 3);
//! - `NORMAL` lines (`writeError` without a level, the default) disappear
//!   under `-q`;
//! - `VERBOSE` lines (`IOInterface::VERBOSE`) need `-v`.
//!
//! `-vv` and `-vvv` are never set here: those runs are handed to Composer
//! (docs/plans/v0.21-verbosity.md), whose debug output vivacity cannot
//! reproduce.
//!
//! The level is process-wide, like the `ARGV` vivacity keeps for the
//! fallback: an embedder that runs two commands concurrently in one process
//! shares it. A thread-local would miss lines printed from a worker thread
//! (the lock verification of `install` runs on one). [`Scope`] sets it for one run
//! and puts it back to `NORMAL` when dropped, panics included.
//!
//! Lines can be *held* ([`hold`]): what Composer writes before anything the
//! command itself prints — while it creates the command's `Composer`
//! instance (`RootPackageLoader::load`'s root-version warning, `Factory`'s
//! ignored-lock warning), in `BaseCommand::initialize` (the
//! `COMPOSER_IGNORE_PLATFORM_REQ(S)` warnings) and at the top of `execute`
//! (`--no-suggest`'s deprecation). Every macro below writes the held lines
//! first, in order ([`release_held`]), whatever the command's first line of
//! its own turns out to be; a command handed to Composer before vivacity
//! wrote anything drops them ([`drop_held`]), since the Composer it runs
//! writes its own. The traces of `VIVACITY_TRACE` (diagnostics, not
//! Composer's output) bypass them.
//!
//! A run can also be *deferred* ([`defer`]): every line, on either stream,
//! is kept in order instead of written, until the command knows the run is
//! its own ([`release_deferred`]) or hands it to Composer before Composer
//! would have printed anything different ([`drop_deferred`]). `update`,
//! `require` and `remove` under `-v` use it up to the solved lock, where a
//! plugin the install would activate can still send the run to Composer
//! (docs/plans/verbose-plugins.md): the lines already printed would
//! otherwise come twice.

use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::sync::Mutex;

/// A run's verbosity, or the level of a line.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Verbosity {
    Quiet = 0,
    Normal = 1,
    Verbose = 2,
}

static LEVEL: AtomicU8 = AtomicU8::new(Verbosity::Normal as u8);

/// The current run's verbosity.
pub fn level() -> Verbosity {
    match LEVEL.load(Ordering::Relaxed) {
        0 => Verbosity::Quiet,
        2 => Verbosity::Verbose,
        _ => Verbosity::Normal,
    }
}

/// Whether a line of level `line` is shown in the current run.
pub fn shows(line: Verbosity) -> bool {
    level() >= line
}

/// Sets the verbosity for one run; back to `NORMAL` when dropped.
#[must_use = "the verbosity is reset when the scope is dropped"]
pub struct Scope(());

impl Scope {
    pub fn new(level: Verbosity) -> Scope {
        LEVEL.store(level as u8, Ordering::Relaxed);
        Scope(())
    }
}

impl Drop for Scope {
    fn drop(&mut self) {
        // Whatever a run still defers is its own output: written, not lost.
        release_deferred();
        drop_held();
        LEVEL.store(Verbosity::Normal as u8, Ordering::Relaxed);
    }
}

/// The deferred lines (`true`: stdout), in order, and whether the run
/// defers (the common case — not deferring — takes no lock).
static DEFERRED: Mutex<Vec<(bool, String)>> = Mutex::new(Vec::new());
static DEFERRING: AtomicBool = AtomicBool::new(false);

fn deferred() -> std::sync::MutexGuard<'static, Vec<(bool, String)>> {
    DEFERRED
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// From now on, every line is kept instead of written ([`release_deferred`],
/// [`drop_deferred`]).
pub fn defer() {
    DEFERRING.store(true, Ordering::Release);
}

/// Writes the deferred lines, in order, each on its stream, and stops
/// deferring.
pub fn release_deferred() {
    if !DEFERRING.load(Ordering::Acquire) {
        return;
    }
    let mut slot = deferred();
    DEFERRING.store(false, Ordering::Release);
    for (stdout, line) in slot.drain(..) {
        if stdout {
            println!("{line}");
        } else {
            eprintln!("{line}");
        }
    }
}

/// Forgets the deferred lines and stops deferring: the command goes to
/// Composer, which writes its own.
pub fn drop_deferred() {
    if !DEFERRING.load(Ordering::Acquire) {
        return;
    }
    let mut slot = deferred();
    DEFERRING.store(false, Ordering::Release);
    slot.clear();
}

/// Writes one line on stderr (`stdout` false) or stdout, after the held
/// lines — or keeps it while the run defers. The writing macros' body.
pub fn emit(stdout: bool, line: std::fmt::Arguments<'_>) {
    release_held();
    if DEFERRING.load(Ordering::Acquire) {
        let mut slot = deferred();
        // Checked again under the lock: a release may have just ended.
        if DEFERRING.load(Ordering::Acquire) {
            slot.push((stdout, line.to_string()));
            return;
        }
    }
    if stdout {
        println!("{line}");
    } else {
        eprintln!("{line}");
    }
}

/// The held stderr lines, in the order they were held, and whether there
/// are any (the common case — nothing held — takes no lock).
static HELD: Mutex<Vec<String>> = Mutex::new(Vec::new());
static HOLDING: AtomicBool = AtomicBool::new(false);

/// The held lines; a panic while they were locked left nothing to repair.
fn held() -> std::sync::MutexGuard<'static, Vec<String>> {
    HELD.lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// Holds a stderr line of level `level`, after those already held, until
/// vivacity writes its next line ([`release_held`]); nothing when the run's
/// verbosity hides it.
pub fn hold(level: Verbosity, line: String) {
    if !shows(level) {
        return;
    }
    held().push(line);
    HOLDING.store(true, Ordering::Release);
}

/// The held lines, in order, left held — for output Composer repeats (a
/// `Factory` run that fails is run again).
pub fn held_lines() -> Vec<String> {
    if !HOLDING.load(Ordering::Acquire) {
        return Vec::new();
    }
    held().clone()
}

/// Writes the held lines on stderr, once, in order. Called by every writing
/// macro before its own line, and before vivacity starts a process whose
/// output the user sees. The lines are written under the lock: a line
/// another thread writes at the same moment comes after them.
pub fn release_held() {
    if !HOLDING.load(Ordering::Acquire) {
        return;
    }
    let mut slot = held();
    let lines: Vec<String> = slot.drain(..).collect();
    HOLDING.store(false, Ordering::Release);
    drop(slot);
    if DEFERRING.load(Ordering::Acquire) {
        let mut d = deferred();
        if DEFERRING.load(Ordering::Acquire) {
            d.extend(lines.into_iter().map(|l| (false, l)));
            return;
        }
        drop(d);
        for line in lines {
            eprintln!("{line}");
        }
        return;
    }
    for line in lines {
        eprintln!("{line}");
    }
}

/// Forgets the held lines: the command goes to Composer, which writes its
/// own, or the run ends. Process-wide, like the level: an embedder running
/// two commands at once in one process shares it.
pub fn drop_held() {
    if !HOLDING.load(Ordering::Acquire) {
        return;
    }
    held().clear();
    HOLDING.store(false, Ordering::Release);
}

/// `GithubActionError::emit` (src/Composer/Console/GithubActionError.php):
/// under GitHub Actions (`GITHUB_ACTIONS` true as PHP reads an environment
/// string, `COMPOSER_TESTS_ARE_RUNNING` not), an error annotation on STDOUT
/// at the default level — `::error ::` and the message with `%`, `\r` and
/// `\n` escaped (`escapeData`). Composer emits one for an unsolvable set
/// (`Installer::doUpdate`, `extractDevPackages`, `doInstall`) and for the
/// message of any exception (`Application::doRun`).
pub fn github_action_error(message: &str) {
    let truthy = |name: &str| std::env::var(name).is_ok_and(|v| !v.is_empty() && v != "0");
    if !truthy("GITHUB_ACTIONS") || truthy("COMPOSER_TESTS_ARE_RUNNING") {
        return;
    }
    crate::out!("::error ::{}", escape_data(message));
}

/// `GithubActionError::escapeData`: `%` first, so that the escapes it
/// writes are not escaped again.
fn escape_data(data: &str) -> String {
    data.replace('%', "%25")
        .replace('\r', "%0D")
        .replace('\n', "%0A")
}

/// `Application::configureIO`'s verbosity, from the arguments (the command
/// line after the program name) and `SHELL_VERBOSITY`, as Symfony computes
/// it: the variable first (PHP's `(int)` cast, then exactly `-1` quiet, `1`
/// verbose, `2` very verbose, `3` debug — anything else is normal), then the
/// options, each of which *replaces* it: `-q`/`--quiet` wins over any `-v`;
/// else `-vvv`/`--verbose=3`, `-vv`/`--verbose=2`, `-v`/`--verbose[=…]`.
/// Tokens match as `ArgvInput::hasParameterOption` matches them, up to `--`:
/// a short option by prefix (`-vvvv` is `-vvv`, `-v -v` is `-v`), a long one
/// exactly or followed by `=`.
/// Returns 0 quiet, 1 normal, 2 verbose, 3 very verbose, 4 debug.
pub fn requested_verbosity<S: AsRef<str>>(args: &[S], shell_verbosity: Option<&str>) -> u8 {
    let has = |value: &str| {
        let leading = if value.starts_with("--") {
            format!("{value}=")
        } else {
            value.to_owned()
        };
        args.iter()
            .map(AsRef::as_ref)
            .take_while(|t| *t != "--")
            .any(|t| t == value || t.starts_with(&leading))
    };
    let base = match shell_verbosity.map(php_int).unwrap_or(0) {
        -1 => 0,
        1 => 2,
        2 => 3,
        3 => 4,
        _ => 1,
    };
    if has("--quiet") || has("-q") {
        0
    } else if has("-vvv") || has("--verbose=3") {
        4
    } else if has("-vv") || has("--verbose=2") {
        3
    } else if has("-v") || has("--verbose=1") || has("--verbose") {
        2
    } else {
        base
    }
}

/// PHP's `(int)` of a string: leading whitespace, then the longest numeric
/// prefix — an integer, or a float with a fraction or an exponent, truncated
/// toward zero (`"1e1"` is 10, `"1.9"` is 1, `"2e0x"` is 2); 0 when there is
/// none.
fn php_int(s: &str) -> i64 {
    let t = s.trim_start_matches([' ', '\t', '\n', '\r', '\x0b', '\x0c']);
    let b = t.as_bytes();
    let mut i = 0;
    if matches!(b.first(), Some(b'+' | b'-')) {
        i += 1;
    }
    let digits = |from: usize| b[from..].iter().take_while(|c| c.is_ascii_digit()).count();
    let int_len = digits(i);
    i += int_len;
    let mut float = false;
    if b.get(i) == Some(&b'.') {
        let frac = digits(i + 1);
        if int_len + frac > 0 {
            float = true;
            i += 1 + frac;
        }
    }
    if int_len == 0 && !float {
        return 0;
    }
    if matches!(b.get(i), Some(b'e' | b'E')) {
        let mut j = i + 1;
        if matches!(b.get(j), Some(b'+' | b'-')) {
            j += 1;
        }
        let exp = digits(j);
        if exp > 0 {
            float = true;
            i = j + exp;
        }
    }
    let text = &t[..i];
    if float {
        text.parse::<f64>().map(|f| f.trunc() as i64).unwrap_or(0)
    } else {
        text.parse::<i64>().unwrap_or(0)
    }
}

/// A `NORMAL` line on stderr (`$io->writeError($line)`).
#[macro_export]
macro_rules! say {
    ($($arg:tt)*) => {
        $crate::say_at!($crate::io::Verbosity::Normal, $($arg)*)
    };
}

/// A `QUIET` line on stderr: shown even under `-q`.
#[macro_export]
macro_rules! say_always {
    ($($arg:tt)*) => {
        $crate::say_at!($crate::io::Verbosity::Quiet, $($arg)*)
    };
}

/// A `VERBOSE` line on stderr: shown with `-v`.
#[macro_export]
macro_rules! say_verbose {
    ($($arg:tt)*) => {
        $crate::say_at!($crate::io::Verbosity::Verbose, $($arg)*)
    };
}

/// A line on stderr at a level known at run time (a held event, a problem's
/// part), after the held lines.
#[macro_export]
macro_rules! say_at {
    ($level:expr $(,)?) => {
        if $crate::io::shows($level) {
            $crate::io::emit(false, format_args!(""));
        }
    };
    ($level:expr, $($arg:tt)+) => {
        if $crate::io::shows($level) {
            $crate::io::emit(false, format_args!($($arg)*));
        }
    };
}

/// A `NORMAL` line on stdout (`$io->write($line)`), after the held lines.
#[macro_export]
macro_rules! out {
    () => {
        if $crate::io::shows($crate::io::Verbosity::Normal) {
            $crate::io::emit(true, format_args!(""));
        }
    };
    ($($arg:tt)+) => {
        if $crate::io::shows($crate::io::Verbosity::Normal) {
            $crate::io::emit(true, format_args!($($arg)*));
        }
    };
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn github_annotation_escapes_like_escape_data() {
        assert_eq!(
            escape_data("100% sure\r\n  Problem 1\n"),
            "100%25 sure%0D%0A  Problem 1%0A"
        );
        assert_eq!(escape_data("%0A"), "%250A");
    }

    // One test: the level and the deferral are process-wide and cargo runs
    // tests in threads.
    #[test]
    fn levels_and_symfony_parsing() {
        // Deferred lines are kept in order, released once, or dropped.
        defer();
        hold(Verbosity::Normal, "held".to_owned());
        emit(false, format_args!("err {}", 1));
        emit(true, format_args!("out"));
        assert_eq!(
            deferred().clone(),
            [
                (false, "held".to_owned()),
                (false, "err 1".to_owned()),
                (true, "out".to_owned())
            ]
        );
        release_deferred();
        assert!(deferred().is_empty());
        defer();
        emit(false, format_args!("gone"));
        drop_deferred();
        assert!(deferred().is_empty());
        assert!(!DEFERRING.load(Ordering::Acquire));
        assert_eq!(level(), Verbosity::Normal);
        {
            let _s = Scope::new(Verbosity::Quiet);
            assert!(!shows(Verbosity::Normal));
            assert!(shows(Verbosity::Quiet));
        }
        assert_eq!(level(), Verbosity::Normal);
        {
            let _s = Scope::new(Verbosity::Verbose);
            assert!(shows(Verbosity::Verbose));
        }
        assert_eq!(level(), Verbosity::Normal);
        let r = std::panic::catch_unwind(|| {
            let _s = Scope::new(Verbosity::Quiet);
            panic!("boom");
        });
        assert!(r.is_err());
        assert_eq!(level(), Verbosity::Normal);

        let v = |a: &[&str], e: Option<&str>| requested_verbosity(a, e);
        assert_eq!(v(&["install"], None), 1);
        assert_eq!(v(&["install", "-q"], None), 0);
        assert_eq!(v(&["install", "--quiet"], None), 0);
        assert_eq!(v(&["install", "-v"], None), 2);
        assert_eq!(v(&["install", "--verbose"], None), 2);
        assert_eq!(v(&["install", "--verbose=0"], None), 2);
        assert_eq!(v(&["install", "-v", "-v"], None), 2);
        assert_eq!(v(&["install", "-vv"], None), 3);
        assert_eq!(v(&["install", "-vvv"], None), 4);
        assert_eq!(v(&["install", "-vvvv"], None), 4);
        assert_eq!(v(&["install", "--verbose=3"], None), 4);
        assert_eq!(v(&["install", "-q", "-vvv"], None), 0);
        assert_eq!(v(&["install", "--quietly"], None), 1);
        // SHELL_VERBOSITY, then the options replace it.
        assert_eq!(v(&["install"], Some("-1")), 0);
        assert_eq!(v(&["install"], Some("1")), 2);
        assert_eq!(v(&["install"], Some(" 1abc")), 2);
        // PHP's cast reads a numeric prefix, exponent included.
        assert_eq!(php_int("1e1"), 10);
        assert_eq!(php_int("1.9"), 1);
        assert_eq!(php_int("-1.5"), -1);
        assert_eq!(php_int(" 2e0x"), 2);
        assert_eq!(php_int(".5"), 0);
        assert_eq!(php_int("1e"), 1);
        assert_eq!(php_int("x"), 0);
        assert_eq!(v(&["install"], Some("1e1")), 1);
        assert_eq!(v(&["install"], Some("-2")), 1);
        assert_eq!(v(&["install"], Some("x")), 1);
        assert_eq!(v(&["install", "-v"], Some("3")), 2);
        assert_eq!(v(&["install"], Some("3")), 4);
        assert_eq!(v(&["require", "--", "-q"], None), 1);
    }
}

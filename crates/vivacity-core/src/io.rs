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
//! (the policy check of `install` runs on one). [`Scope`] sets it for one run
//! and puts it back to `NORMAL` when dropped, panics included.

use std::sync::atomic::{AtomicU8, Ordering};

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
        LEVEL.store(Verbosity::Normal as u8, Ordering::Relaxed);
    }
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
        if $crate::io::shows($crate::io::Verbosity::Normal) {
            eprintln!($($arg)*);
        }
    };
}

/// A `QUIET` line on stderr: shown even under `-q`.
#[macro_export]
macro_rules! say_always {
    ($($arg:tt)*) => {
        eprintln!($($arg)*);
    };
}

/// A `VERBOSE` line on stderr: shown with `-v`.
#[macro_export]
macro_rules! say_verbose {
    ($($arg:tt)*) => {
        if $crate::io::shows($crate::io::Verbosity::Verbose) {
            eprintln!($($arg)*);
        }
    };
}

/// A `NORMAL` line on stdout (`$io->write($line)`).
#[macro_export]
macro_rules! out {
    ($($arg:tt)*) => {
        if $crate::io::shows($crate::io::Verbosity::Normal) {
            println!($($arg)*);
        }
    };
}

#[cfg(test)]
mod tests {
    use super::*;

    // One test: the level is process-wide and cargo runs tests in threads.
    #[test]
    fn levels_and_symfony_parsing() {
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

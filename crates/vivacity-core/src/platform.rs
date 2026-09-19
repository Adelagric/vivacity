//! What a platform check reports (`PlatformFailure`, the shared
//! `--ignore-platform-req` matching) and the per-platform switches
//! (`parallel_io`, `cache_dir`). The probe of the local PHP and the check
//! of a lock against it live in `vivacity_resolver::platform`
//! (`probe`, `platform_packages`, `check_install`), the port of
//! `PlatformRepository` that `install` and `update` both use.

use std::path::PathBuf;

#[derive(Debug, PartialEq, Eq)]
pub struct PlatformFailure {
    /// "php", "ext-mbstring", ...
    pub requirement: String,
    pub constraint: String,
    /// Requesting package (None = the lock's platform section).
    pub required_by: Option<String>,
    pub reason: FailureReason,
}

#[derive(Debug, PartialEq, Eq)]
pub enum FailureReason {
    Missing,
    Mismatch { installed: String },
    Unsupported,
}

/// Whether parallel file I/O pays on this machine: yes where I/O latency
/// dominates (Linux: ext4/WSL2 measured 2x on a wiped vendor/ and on a
/// cold classmap scan), no where the page cache is the bottleneck (APFS:
/// parallel reads 1.3-4x slower, DECISIONS.md M5). `VIVACITY_PARALLEL_IO`
/// (`0`/`1`) overrides the default.
pub fn parallel_io() -> bool {
    match std::env::var("VIVACITY_PARALLEL_IO") {
        Ok(v) => v != "0" && !v.is_empty(),
        Err(_) => cfg!(target_os = "linux"),
    }
}

pub fn cache_dir() -> PathBuf {
    if let Ok(d) = std::env::var("VIVACITY_CACHE_DIR") {
        return PathBuf::from(d);
    }
    #[cfg(windows)]
    {
        if let Ok(l) = std::env::var("LOCALAPPDATA") {
            if !l.is_empty() {
                return PathBuf::from(l).join("vivacity");
            }
        }
    }
    if let Ok(xdg) = std::env::var("XDG_CACHE_HOME") {
        return PathBuf::from(xdg).join("vivacity");
    }
    let home = std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .unwrap_or_else(|_| ".".to_owned());
    if cfg!(target_os = "macos") {
        PathBuf::from(home).join("Library/Caches/vivacity")
    } else {
        PathBuf::from(home).join(".cache/vivacity")
    }
}

/// `--ignore-platform-req` patterns (`*`, a name, `ext-*`, a trailing `+`).
pub fn is_ignored(requirement: &str, ignored: &[String]) -> bool {
    ignored.iter().any(|pat| {
        let pat = pat.strip_suffix('+').unwrap_or(pat);
        pat == "*"
            || pat == requirement
            || pat
                .strip_suffix('*')
                .is_some_and(|prefix| requirement.starts_with(prefix))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ignore_patterns() {
        let pats = |v: &[&str]| v.iter().map(|s| (*s).to_owned()).collect::<Vec<_>>();
        assert!(is_ignored("php", &pats(&["*"])));
        assert!(is_ignored("ext-gd", &pats(&["ext-*"])));
        assert!(!is_ignored("php", &pats(&["ext-*"])));
        assert!(is_ignored("ext-gd", &pats(&["ext-gd+"])));
        assert!(!is_ignored("ext-intl", &pats(&["ext-gd"])));
    }
}

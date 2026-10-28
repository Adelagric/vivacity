//! Subset of Composer versioning needed by the platform check: numeric
//! versions `X[.Y[.Z[.W]]]` with an optional stability suffix (`-dev`,
//! `-alpha.N`, `-beta.N`, `-RC.N`, `-patch.N`), compared like
//! `composer/semver` (4-component normalisation, dev < alpha < beta < RC <
//! stable < patch). Branches (`dev-master`, `1.x-dev`) are outside this
//! subset: `parse` returns an error and the caller treats the package as
//! out of scope rather than guessing.
//!
//! Parity is held by the differential tests against
//! `Composer\Semver\Semver::satisfies` (tests/oracle_semver.rs).

use std::cmp::Ordering;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Stability {
    Dev,
    Alpha,
    Beta,
    Rc,
    Stable,
    Patch,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Version {
    pub parts: [u64; 4],
    pub stability: Stability,
    /// Pre-release number (`-beta2` -> 2), 0 if absent.
    pub pre_number: u64,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
#[error("version outside the supported subset: {0:?}")]
pub struct UnsupportedVersion(pub String);

impl Version {
    pub fn parse(input: &str) -> Result<Self, UnsupportedVersion> {
        let s = input.trim();
        let s = s
            .strip_prefix('v')
            .or_else(|| s.strip_prefix('V'))
            .unwrap_or(s);
        if s.is_empty() {
            return Err(UnsupportedVersion(input.to_owned()));
        }

        // Split off the stability suffix: `1.2.3-beta2`, `1.2.3beta2`, `1.2.3-dev`.
        let (num, suffix) = split_stability(s);
        let (stability, pre_number) = parse_stability(suffix, input)?;

        let mut parts = [0u64; 4];
        let mut n = 0usize;
        for piece in num.split('.') {
            if n >= 4 || piece.is_empty() || !piece.bytes().all(|b| b.is_ascii_digit()) {
                return Err(UnsupportedVersion(input.to_owned()));
            }
            parts[n] = piece
                .parse()
                .map_err(|_| UnsupportedVersion(input.to_owned()))?;
            n += 1;
        }
        if n == 0 {
            return Err(UnsupportedVersion(input.to_owned()));
        }
        Ok(Version {
            parts,
            stability,
            pre_number,
        })
    }
}

/// Splits `1.2.3-beta2` / `1.2.3beta2` / `1.2.3_RC1` into (numeric, suffix).
/// The numeric part and the stability suffix: `VersionParser::normalize`
/// allows one `.`, `_` or `-` between them (`1.2.3.stable`, `2.0.0.beta1`,
/// `1.0-b3` all count).
fn split_stability(s: &str) -> (&str, &str) {
    match s.find(|c: char| !(c.is_ascii_digit() || c == '.')) {
        Some(i) => {
            let (num, suffix) = (&s[..i], &s[i..]);
            // Exactly one separator: `[._-]?` in the regex.
            if let Some(n) = num.strip_suffix('.') {
                (n, suffix)
            } else {
                (num, suffix.strip_prefix(['-', '_', '.']).unwrap_or(suffix))
            }
        }
        // `1.2.3.` is `1.2.3.0`: the optional separator with no modifier.
        None => (s.strip_suffix('.').unwrap_or(s), ""),
    }
}

fn parse_stability(suffix: &str, original: &str) -> Result<(Stability, u64), UnsupportedVersion> {
    if suffix.is_empty() {
        return Ok((Stability::Stable, 0));
    }
    let lower = suffix.to_ascii_lowercase();
    let (word, digits) = match lower.find(|c: char| c.is_ascii_digit()) {
        Some(i) => (&lower[..i], &lower[i..]),
        None => (lower.as_str(), ""),
    };
    let word = word.trim_end_matches(['-', '_', '.']);
    let stability = match word {
        "dev" => Stability::Dev,
        "alpha" | "a" => Stability::Alpha,
        "beta" | "b" => Stability::Beta,
        "rc" => Stability::Rc,
        "patch" | "pl" | "p" => Stability::Patch,
        "stable" | "" => Stability::Stable,
        _ => return Err(UnsupportedVersion(original.to_owned())),
    };
    let pre_number = if digits.is_empty() {
        0
    } else {
        digits
            .parse()
            .map_err(|_| UnsupportedVersion(original.to_owned()))?
    };
    Ok((stability, pre_number))
}

impl PartialOrd for Version {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Version {
    fn cmp(&self, other: &Self) -> Ordering {
        self.parts
            .cmp(&other.parts)
            .then(self.stability.cmp(&other.stability))
            .then(self.pre_number.cmp(&other.pre_number))
    }
}

/// Composer's "pretty -> normalized" normalisation (VersionParser::normalize),
/// for the subset met in locks: numeric versions (-> 4 components + canonical
/// suffix), `dev-*` branches (unchanged) and numeric branches `N.x-dev`
/// (x -> 9999999, padded to 4 components).
/// Parity held by tests/oracle_normalize.rs.
pub fn normalize_pretty(input: &str) -> Result<String, UnsupportedVersion> {
    let s = input.trim();
    // composer/semver 3: a `dev-` branch stays as written (`dev-master` is
    // `dev-master`; only `normalizeDefaultBranch`, never called here, maps
    // it to `9999999-dev`), the prefix itself lowercased (`DEV-MASTER` ->
    // `dev-MASTER`).
    if s.len() >= 4 && s[..4].eq_ignore_ascii_case("dev-") {
        let rest = &s[4..];
        if rest.is_empty() {
            return Err(UnsupportedVersion(input.to_owned()));
        }
        return Ok(format!("dev-{rest}"));
    }
    let stripped = s
        .strip_prefix('v')
        .or_else(|| s.strip_prefix('V'))
        .unwrap_or(s);

    // Numeric branch `1.2.x-dev` / `1.x-dev`.
    if let Some(stem) = stripped
        .strip_suffix(".x-dev")
        .or_else(|| stripped.strip_suffix(".X-dev"))
    {
        // The digit runs are kept as written (`2026.04.x-dev` stays
        // `2026.04.9999999.9999999-dev`): VersionParser concatenates the
        // matched strings, it never re-prints numbers.
        let mut out: Vec<String> = Vec::new();
        for piece in stem.split('.') {
            if piece.is_empty() || !piece.bytes().all(|b| b.is_ascii_digit()) || out.len() >= 3 {
                return Err(UnsupportedVersion(input.to_owned()));
            }
            out.push(piece.to_owned());
        }
        while out.len() < 4 {
            out.push("9999999".to_owned());
        }
        return Ok(format!("{}-dev", out.join(".")));
    }

    let (num, suffix) = split_stability(stripped);
    // The classical form wants a major of at most five digits, four parts
    // at most and a stability word before any modifier number; otherwise
    // the date form (`v2021-04` is `2021.04`, `20210401` stays).
    let classical = num.split('.').next().is_some_and(|m| m.len() <= 5)
        && num.split('.').count() <= 4
        && !suffix.starts_with(|c: char| c.is_ascii_digit());
    if !classical {
        return date_form(stripped).ok_or_else(|| UnsupportedVersion(input.to_owned()));
    }
    let (stability, _) = parse_stability(suffix, input)?;
    // Same rule: `1.02` is `1.02.0.0`, `v01.2.3` is `01.2.3.0`.
    let mut parts: Vec<&str> = Vec::new();
    for piece in num.split('.') {
        if parts.len() >= 4 || piece.is_empty() || !piece.bytes().all(|b| b.is_ascii_digit()) {
            return Err(UnsupportedVersion(input.to_owned()));
        }
        parts.push(piece);
    }
    if parts.is_empty() {
        return Err(UnsupportedVersion(input.to_owned()));
    }
    while parts.len() < 4 {
        parts.push("0");
    }
    let base = parts.join(".");
    let word = match stability {
        Stability::Stable => return Ok(base),
        Stability::Dev => "dev",
        Stability::Alpha => "alpha",
        Stability::Beta => "beta",
        Stability::Rc => "RC",
        Stability::Patch => "patch",
    };
    // Suffixes without a number stay bare (`-alpha`), else the number is
    // appended as written (`RC01` keeps its zero).
    let digits: String = suffix.chars().filter(char::is_ascii_digit).collect();
    if digits.is_empty() {
        Ok(format!("{base}-{word}"))
    } else {
        Ok(format!("{base}-{word}{digits}"))
    }
}

/// `VersionParser::normalize`'s date form without a modifier:
/// `^\d{4}(?:[.:-]?\d{2}){1,6}(?:[.:-]?\d{1,3}){0,2}$`, every non-digit
/// then turned into `.`.
fn date_form(s: &str) -> Option<String> {
    fn digits(b: &[u8], i: usize, n: usize) -> bool {
        b.len() >= i + n && b[i..i + n].iter().all(u8::is_ascii_digit)
    }
    // Groups of `[.:-]?` and `lo..=hi` digits, `min..=max` of them, from `i`
    // (backtracking as PCRE does), then the rest.
    fn groups(b: &[u8], i: usize, count: usize, spec: &[(usize, usize, usize, usize)]) -> bool {
        let Some(&(lo, hi, min, max)) = spec.first() else {
            return i == b.len();
        };
        if count < max {
            let seps: &[usize] = if b.get(i).is_some_and(|c| b".:-".contains(c)) {
                &[1, 0]
            } else {
                &[0]
            };
            for &sep in seps {
                for n in (lo..=hi).rev() {
                    if digits(b, i + sep, n) && groups(b, i + sep + n, count + 1, spec) {
                        return true;
                    }
                }
            }
        }
        count >= min && groups(b, i, 0, &spec[1..])
    }
    let b = s.as_bytes();
    (digits(b, 0, 4) && groups(b, 4, 0, &[(2, 2, 1, 6), (1, 3, 0, 2)])).then(|| {
        s.chars()
            .map(|c| if c.is_ascii_digit() { c } else { '.' })
            .collect()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(s: &str) -> Version {
        Version::parse(s).expect(s)
    }

    #[test]
    fn parses_and_orders() {
        assert_eq!(v("8.5.10").parts, [8, 5, 10, 0]);
        assert_eq!(v("v1.2").parts, [1, 2, 0, 0]);
        assert!(v("8.1") < v("8.1.1"));
        assert!(v("7.4.33") < v("8.0.0"));
        assert!(v("1.0.0-dev") < v("1.0.0-alpha1"));
        assert!(v("1.0.0-alpha2") < v("1.0.0-beta1"));
        assert!(v("1.0.0-RC1") < v("1.0.0"));
        assert!(v("1.0.0") < v("1.0.0-patch1"));
        assert!(v("1.0.0-beta1") < v("1.0.0-beta2"));
        assert_eq!(v("1.0"), v("1.0.0.0"));
    }

    #[test]
    fn rejects_out_of_subset() {
        for s in ["dev-master", "1.x-dev", "abc", "", "1.2.3.4.5", "1.2-foo"] {
            assert!(Version::parse(s).is_err(), "{s} should have been rejected");
        }
    }
}

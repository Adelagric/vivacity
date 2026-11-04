//! Port of `composer/class-map-generator` (docs/reference/cmg-*.php):
//! - `find_classes`: pre-cleaning (PhpFileCleaner: strings, comments and
//!   heredocs replaced) then THE PCRE pattern of PhpFileParser, run by pcre2
//!   (possessive quantifiers, lookbehind, `\x7f-\xff` bytes: out of reach of
//!   the `regex` crate, plan decision r1/F4);
//! - `Scanner`: Symfony Finder-style walk (php/inc/hh extensions, dot-files
//!   and VCS directories ignored, symlinks followed), PSR-0/PSR-4 filter,
//!   regex exclusion, realpath deduplication, "first one wins" ambiguities.
//!
//! Class names are raw BYTES, as in PHP: symfony/cache declares a class named
//! with a single non-UTF-8 byte, which Composer writes as is into the
//! classmap.
//!
//! Composer first goes through `php_strip_whitespace()` (PHP tokenizer): we do
//! not have it, so the cleaner additionally handles `#` comments (except `#[`
//! attributes), the only observable difference for class detection. Parity is
//! held by tests/oracle_classmap.rs (the phar's findClasses on every fixture
//! file).

use crate::pathutil::normalize_path;
use rayon::prelude::*;
use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
pub use vivacity_core::file_identity::{Clock, FileIdentity, PlacedPackage};

#[derive(Debug, thiserror::Error)]
pub enum ClassMapError {
    #[error("invalid internal regex: {0}")]
    Regex(String),
    #[error("cannot read {0}")]
    Read(PathBuf),
    #[error(
        "Could not scan for classes inside \"{0}\" which does not appear to be a file nor a folder"
    )]
    MissingPath(String),
}

const TYPE_WORDS: [&str; 4] = ["class", "interface", "trait", "enum"];

/// Minimal cleaning: replaces strings/heredocs with `null`, strips comments,
/// stops early when a single type is expected (maxMatches == 1).
fn clean(contents: &[u8], max_matches: usize, type_pattern: &pcre2::bytes::Regex) -> Vec<u8> {
    let len = contents.len();
    let mut out: Vec<u8> = Vec::with_capacity(len);
    let mut i = 0usize;
    let peek = |i: usize, c: u8| i + 1 < len && contents[i + 1] == c;

    while i < len {
        // skipToPhp
        while i < len {
            if contents[i] == b'<' && peek(i, b'?') {
                i += 2;
                break;
            }
            i += 1;
        }
        out.extend_from_slice(b"<?");
        'php: while i < len {
            let c = contents[i];
            if c == b'?' && peek(i, b'>') {
                out.extend_from_slice(b"?>");
                i += 2;
                break 'php;
            }
            if c == b'"' || c == b'\'' {
                i = skip_string(contents, i, c);
                out.extend_from_slice(b"null");
                continue;
            }
            if c == b'<' && peek(i, b'<') {
                if let Some((delim, end)) = heredoc_start(contents, i) {
                    i = skip_heredoc(contents, end, &delim);
                    out.extend_from_slice(b"null");
                    continue;
                }
            }
            if c == b'/' {
                if peek(i, b'/') {
                    i = skip_to_newline(contents, i);
                    continue;
                }
                if peek(i, b'*') {
                    i = skip_comment(contents, i);
                    continue;
                }
            }
            // `#`: line comment, except `#[` attributes (PHP 8); this is the
            // role of php_strip_whitespace in Composer.
            if c == b'#' && !peek(i, b'[') {
                i = skip_to_newline(contents, i);
                continue;
            }
            if max_matches == 1 && matches!(c, b'c' | b'i' | b't' | b'e') {
                for word in TYPE_WORDS {
                    if contents[i..].starts_with(word.as_bytes()) {
                        // pattern anchored at index-1: `.\b(?<![\$:>])type\s++name`
                        let start = i.saturating_sub(1);
                        if let Ok(Some(m)) = type_pattern.find_at(contents, start) {
                            if m.start() == start {
                                out.extend_from_slice(&contents[m.start()..m.end()]);
                                return out;
                            }
                        }
                    }
                }
            }
            i += 1;
            // strcspn over the reject characters
            let mut skip = 0;
            while i + skip < len
                && !matches!(
                    contents[i + skip],
                    b'?' | b'"' | b'\'' | b'<' | b'/' | b'#' | b'c' | b'i' | b't' | b'e'
                )
            {
                skip += 1;
            }
            out.push(c);
            if skip > 0 {
                out.extend_from_slice(&contents[i..i + skip]);
                i += skip;
            }
        }
    }
    out
}

fn skip_string(contents: &[u8], mut i: usize, delim: u8) -> usize {
    let len = contents.len();
    i += 1;
    while i < len {
        let c = contents[i];
        if c == b'\\' && i + 1 < len && (contents[i + 1] == b'\\' || contents[i + 1] == delim) {
            i += 2;
            continue;
        }
        if c == delim {
            return i + 1;
        }
        i += 1;
    }
    i
}

fn skip_comment(contents: &[u8], mut i: usize) -> usize {
    let len = contents.len();
    i += 2;
    while i < len {
        if contents[i] == b'*' && i + 1 < len && contents[i + 1] == b'/' {
            return i + 2;
        }
        i += 1;
    }
    i
}

fn skip_to_newline(contents: &[u8], mut i: usize) -> usize {
    while i < contents.len() && contents[i] != b'\n' && contents[i] != b'\r' {
        i += 1;
    }
    i
}

/// `<<<[ \t]*(['"]?)(ident)\1(\r\n|\n|\r)` anchored at i: (delimiter, index after).
fn heredoc_start(contents: &[u8], i: usize) -> Option<(Vec<u8>, usize)> {
    if !contents[i..].starts_with(b"<<<") {
        return None;
    }
    let mut j = i + 3;
    while j < contents.len() && (contents[j] == b' ' || contents[j] == b'\t') {
        j += 1;
    }
    let quote = match contents.get(j) {
        Some(b'\'') | Some(b'"') => {
            let q = contents[j];
            j += 1;
            Some(q)
        }
        _ => None,
    };
    let start = j;
    let is_ident_start = |c: u8| c.is_ascii_alphabetic() || c == b'_' || c >= 0x80;
    let is_ident = |c: u8| c.is_ascii_alphanumeric() || c == b'_' || c >= 0x80;
    if !contents.get(j).copied().is_some_and(is_ident_start) {
        return None;
    }
    while contents.get(j).copied().is_some_and(is_ident) {
        j += 1;
    }
    let delim = contents[start..j].to_vec();
    if let Some(q) = quote {
        if contents.get(j) != Some(&q) {
            return None;
        }
        j += 1;
    }
    match contents.get(j) {
        Some(b'\n') => Some((delim, j + 1)),
        Some(b'\r') => {
            if contents.get(j + 1) == Some(&b'\n') {
                Some((delim, j + 2))
            } else {
                Some((delim, j + 1))
            }
        }
        _ => None,
    }
}

fn skip_heredoc(contents: &[u8], mut i: usize, delim: &[u8]) -> usize {
    let len = contents.len();
    let is_ident = |c: u8| c.is_ascii_alphanumeric() || c == b'_' || c >= 0x80;
    while i < len {
        match contents[i] {
            b'\t' | b' ' => {
                i += 1;
                continue;
            }
            c if c == delim[0]
                && contents[i..].starts_with(delim)
                && !contents.get(i + delim.len()).copied().is_some_and(is_ident) =>
            {
                return i + delim.len();
            }
            _ => {}
        }
        i = skip_to_newline(contents, i);
        while i < len && (contents[i] == b'\n' || contents[i] == b'\r') {
            i += 1;
        }
    }
    i
}

pub struct ClassFinder {
    quick: pcre2::bytes::Regex,
    type_anchor: pcre2::bytes::Regex,
    main: pcre2::bytes::Regex,
}

impl ClassFinder {
    pub fn new() -> Result<ClassFinder, ClassMapError> {
        let build = |p: &str, extended: bool| {
            pcre2::bytes::RegexBuilder::new()
                .caseless(true)
                .extended(extended)
                .build(p)
                .map_err(|e| ClassMapError::Regex(e.to_string()))
        };
        Ok(ClassFinder {
            quick: build(r"\b(?:class|interface|trait|enum)\s", false)?,
            type_anchor: pcre2::bytes::RegexBuilder::new()
                .caseless(true)
                .dotall(true)
                .build(r".\b(?<![\$:>])(?:class|interface|trait|enum)\s++[a-zA-Z_\x7f-\xff:][a-zA-Z0-9_\x7f-\xff:\-]*+")
                .map_err(|e| ClassMapError::Regex(e.to_string()))?,
            main: build(
                r"(?:
                 \b(?<![\\$:>])(?P<type>class|interface|trait|enum) \s++ (?P<name>[a-zA-Z_\x7f-\xff:][a-zA-Z0-9_\x7f-\xff:\-]*+)
               | \b(?<![\\$:>])(?P<ns>namespace) (?P<nsname>\s++[a-zA-Z_\x7f-\xff][a-zA-Z0-9_\x7f-\xff]*+(?:\s*+\\\s*+[a-zA-Z_\x7f-\xff][a-zA-Z0-9_\x7f-\xff]*+)*+)? \s*+ [\{;]
            )",
                true,
            )?,
        })
    }

    /// `PhpFileParser::findClasses` on already-read contents (names as bytes).
    pub fn find_classes(&self, contents: &[u8]) -> Result<Vec<Vec<u8>>, ClassMapError> {
        if contents.iter().all(u8::is_ascii_whitespace) {
            return Ok(Vec::new());
        }
        let quick_count = self
            .quick
            .find_iter(contents)
            .filter_map(Result::ok)
            .count();
        if quick_count == 0 {
            return Ok(Vec::new());
        }
        let cleaned = clean(contents, quick_count, &self.type_anchor);

        let mut classes: Vec<Vec<u8>> = Vec::new();
        let mut namespace: Vec<u8> = Vec::new();
        for caps in self.main.captures_iter(&cleaned) {
            let caps = caps.map_err(|e| ClassMapError::Regex(e.to_string()))?;
            if let Some(ns) = caps.name("ns") {
                if !ns.as_bytes().is_empty() {
                    let nsname = caps.name("nsname").map(|m| m.as_bytes()).unwrap_or(b"");
                    namespace = nsname
                        .iter()
                        .copied()
                        .filter(|b| !matches!(b, b' ' | b'\t' | b'\r' | b'\n'))
                        .collect();
                    namespace.push(b'\\');
                    continue;
                }
            }
            let Some(name_m) = caps.name("name") else {
                continue;
            };
            let name = name_m.as_bytes();
            if name == b"extends" || name == b"implements" {
                continue;
            }
            let is_enum = caps
                .name("type")
                .is_some_and(|m| m.as_bytes().eq_ignore_ascii_case(b"enum"));
            let name: Vec<u8> = if name.first() == Some(&b':') {
                let mut out = b"xhp".to_vec();
                for &b in &name[1..] {
                    match b {
                        b'-' => out.push(b'_'),
                        b':' => out.extend_from_slice(b"__"),
                        b => out.push(b),
                    }
                }
                out
            } else if is_enum {
                match name.iter().rposition(|b| *b == b':') {
                    Some(pos) => name[..pos].to_vec(),
                    None => name.to_vec(),
                }
            } else {
                name.to_vec()
            };
            let mut class_name = namespace.clone();
            class_name.extend_from_slice(&name);
            let start = class_name
                .iter()
                .position(|b| *b != b'\\')
                .unwrap_or(class_name.len());
            classes.push(class_name[start..].to_vec());
        }
        Ok(classes)
    }
}

thread_local! {
    /// One `ClassFinder` per thread (compiled pcre2 regexes cannot be
    /// shared, and recompiling them per file is expensive) — reused from one
    /// scan to the next on the same rayon worker.
    static FINDER: std::cell::RefCell<Option<ClassFinder>> =
        const { std::cell::RefCell::new(None) };
}

/// `findClasses` through the current thread's finder (compiled on first
/// use).
fn find_one(contents: &[u8]) -> Result<Vec<Vec<u8>>, ClassMapError> {
    FINDER.with(|cell| {
        let mut slot = cell.borrow_mut();
        if slot.is_none() {
            *slot = Some(ClassFinder::new()?);
        }
        slot.as_ref().unwrap().find_classes(contents)
    })
}

/// Parallel detection on the rayon pool (one finder per thread), results in
/// input order to preserve "first one wins".
fn find_all(todo: &[(PathBuf, PathBuf, Vec<u8>)]) -> Result<Vec<Vec<Vec<u8>>>, ClassMapError> {
    if todo.is_empty() {
        return Ok(Vec::new());
    }
    todo.par_iter()
        .map(|(_, _, contents)| find_one(contents))
        .collect()
}

/// Version of the scan format/algorithm: bump it whenever detection (or the
/// cache encoding) changes, to invalidate existing caches.
/// v2: length-prefixed binary encoding (replaces the JSON+base64).
/// v3: entries in readdir order (was sorted by name).
/// v4: the sha1 of each file's content; a store entry's record is scanned
/// from the entry's own directory only, never from a copy under vendor/.
const CACHE_FORMAT: &str = "v4";

/// sha1 of a file's content: what lets a record's classes be reused for a
/// file whose identity changed but whose bytes did not. A class depends on
/// the content only (`find_classes` takes no path; the path filters run in
/// `merge_scanned`), so equal digests give equal classes.
type Digest = [u8; 20];

fn digest(bytes: &[u8]) -> Digest {
    use sha1::{Digest as _, Sha1};
    Sha1::digest(bytes).into()
}

/// Relative path of `file` under `base` (the scanned directory, or the
/// parent of a scanned file), as the records key it. `None` for a path that
/// is not UTF-8: a lossy key could name two files, so such a file is never
/// recorded nor served from a record (it is read and scanned every time).
fn rel_key(base: &Path, file: &Path) -> Option<String> {
    file.strip_prefix(base)
        .unwrap_or(file)
        .to_str()
        .map(str::to_owned)
}

/// The directory relative paths are taken from: the path itself, or the
/// parent of a single file (a `classmap` entry naming a file).
fn base_of(path: &Path) -> PathBuf {
    if path.is_file() {
        path.parent().map(Path::to_path_buf).unwrap_or_default()
    } else {
        path.to_path_buf()
    }
}

/// One file of a store entry's record: (content digest, raw classes).
type StoreRecord = HashMap<String, (Digest, Vec<Vec<u8>>)>;

/// The record of a directory of a store entry, shared by every project that
/// uses the entry: key = entry (name/version/ref) + the identity of the
/// entry's root directory (an entry removed and extracted again under the
/// same name, from another archive, is another directory) + relative
/// subdirectory + format version. It is only ever written from a scan of the entry's own
/// directory (`build`), never from a copy under vendor/ — a copy can have
/// been edited, cleaned or added to — and a project's scan only takes from
/// it the classes of a file whose bytes are the entry's: placed by this run
/// and untouched since (same identity), or with the same digest.
pub struct CacheSlot {
    file: PathBuf,
    /// The store entry's own copy of the scanned directory (or file).
    entry_dir: PathBuf,
}

impl CacheSlot {
    pub fn new(cache_root: &Path, store_entry: &Path, rel_subdir: &Path) -> CacheSlot {
        use sha1::{Digest as _, Sha1};
        let mut h = Sha1::new();
        h.update(CACHE_FORMAT.as_bytes());
        h.update(b"\0");
        h.update(store_entry.to_string_lossy().as_bytes());
        h.update(b"\0");
        if let Some(root) = FileIdentity::of(store_entry) {
            h.update(root.dev.to_le_bytes());
            h.update(root.ino.to_le_bytes());
            h.update(root.ctime.0.to_le_bytes());
            h.update(root.ctime.1.to_le_bytes());
        }
        h.update(b"\0");
        h.update(rel_subdir.to_string_lossy().as_bytes());
        let key: String = h.finalize().iter().map(|b| format!("{b:02x}")).collect();
        CacheSlot {
            file: cache_root.join("classmap").join(format!("{key}.bin")),
            entry_dir: store_entry.join(rel_subdir),
        }
    }

    /// Records by relative path.
    ///
    /// Length-prefixed binary format (little-endian):
    ///   u32 file_count
    ///   [ u32 rel_len, rel(utf-8 bytes), sha1 (20 bytes),
    ///     u32 class_count, [ u32 class_len, class(raw bytes) ]* ]*
    /// Class names are raw bytes (see the module header): no base64, no
    /// lossy round-trip. The relative paths come from `to_string_lossy`, the
    /// same function that keys the lookups.
    fn load(&self) -> Option<StoreRecord> {
        let bytes = std::fs::read(&self.file).ok()?;
        let mut r = ByteReader::new(&bytes);
        let n = r.u32()? as usize;
        // The counts come from the (possibly corrupt) file: clamp the
        // pre-allocations to what the remaining bytes could actually encode
        // (a file entry is ≥ 28 bytes, a class entry ≥ 4), so a garbage
        // count is caught by the truncation checks below instead of asking
        // the allocator for gigabytes. Exact for every well-formed file.
        let mut out = HashMap::with_capacity(n.min(r.remaining() / 28));
        for _ in 0..n {
            let rel = String::from_utf8(r.slice()?.to_vec()).ok()?;
            let sha: Digest = r.fixed(20)?.try_into().ok()?;
            let nc = r.u32()? as usize;
            let mut classes = Vec::with_capacity(nc.min(r.remaining() / 4));
            for _ in 0..nc {
                classes.push(r.slice()?.to_vec());
            }
            out.insert(rel, (sha, classes));
        }
        if !r.is_empty() {
            return None; // trailing bytes: corrupt file, rescan
        }
        Some(out)
    }

    /// The record, scanned from the entry's own directory when there is none
    /// yet (and written). `None` — and nothing written — when the entry
    /// cannot be scanned whole (gone, unreadable) or holds a symbolic link
    /// (its target may differ between the store and a copy).
    fn load_or_build(&self, stats: &CacheStats) -> Option<StoreRecord> {
        if let Some(r) = self.load() {
            return Some(r);
        }
        let (files, saw_symlink) = Scanner::collect_files(&self.entry_dir).ok()?;
        if saw_symlink {
            return None;
        }
        let base = base_of(&self.entry_dir);
        let mut todo: Vec<(PathBuf, PathBuf, Vec<u8>)> = Vec::with_capacity(files.len());
        for (file, real) in files {
            let contents = std::fs::read(&file).ok()?;
            todo.push((file, real, contents));
        }
        let found = find_all(&todo).ok()?;
        stats.store_scanned.fetch_add(todo.len(), Ordering::Relaxed);
        let record: StoreRecord = todo
            .iter()
            .zip(found)
            .filter_map(|((file, _, contents), classes)| {
                Some((rel_key(&base, file)?, (digest(contents), classes)))
            })
            .collect();
        self.store(&record);
        Some(record)
    }

    fn store(&self, record: &StoreRecord) {
        let sorted: BTreeMap<&String, &(Digest, Vec<Vec<u8>>)> = record.iter().collect();
        let mut buf: Vec<u8> = Vec::new();
        buf.extend_from_slice(&(sorted.len() as u32).to_le_bytes());
        for (rel, (sha, classes)) in sorted {
            push_slice(&mut buf, rel.as_bytes());
            buf.extend_from_slice(sha);
            buf.extend_from_slice(&(classes.len() as u32).to_le_bytes());
            for c in classes {
                push_slice(&mut buf, c);
            }
        }
        write_atomically(&self.file, &buf);
    }
}

/// Writes `bytes` to `file` through a temporary file of its own, created
/// exclusively under a random name next to it: the same slot can be written
/// by two directories scanned in parallel, by two processes, or by two
/// containers sharing the cache (whose pids can be equal); each writer
/// renames its own complete file over the slot. A failure leaves nothing.
fn write_atomically(file: &Path, bytes: &[u8]) {
    use std::io::Write as _;
    let Some(parent) = file.parent() else {
        return;
    };
    if std::fs::create_dir_all(parent).is_err() {
        return;
    }
    let Ok(mut tmp) = tempfile::Builder::new().suffix(".tmp").tempfile_in(parent) else {
        return;
    };
    if tmp.write_all(bytes).is_ok() {
        let _ = tmp.persist(file);
    }
}

/// What one dump's scans share: the reference time of the file systems they
/// read (set once, before the scans), and counters for `VIVACITY_TRACE`.
#[derive(Default)]
pub struct CacheContext {
    clock: std::sync::OnceLock<Clock>,
    stats: CacheStats,
}

#[derive(Default)]
struct CacheStats {
    /// Served on the identity recorded for this project, nothing read.
    served: AtomicUsize,
    /// Placed by this run and untouched since: the store record's classes,
    /// nothing read under vendor/.
    from_store: AtomicUsize,
    /// Read and hashed; the classes of a record with the same digest.
    read: AtomicUsize,
    /// Read, and the class detection run.
    scanned: AtomicUsize,
    /// Files of store entries scanned to build their records.
    store_scanned: AtomicUsize,
}

impl CacheContext {
    pub fn new() -> CacheContext {
        CacheContext::default()
    }

    /// The reference time, probed before any scan (later calls are ignored).
    pub fn set_clock(&self, clock: Clock) {
        let _ = self.clock.set(clock);
    }

    /// Whether `id` must not be trusted alone by a later run. Without a
    /// clock, every identity is.
    fn is_racy(&self, id: &FileIdentity) -> bool {
        self.clock.get().is_none_or(|c| c.is_racy(id))
    }

    /// One line for `VIVACITY_TRACE`.
    pub fn summary(&self) -> String {
        let s = &self.stats;
        let n = |a: &AtomicUsize| a.load(Ordering::Relaxed);
        format!(
            "{} served, {} from store, {} read, {} scanned; {} store files scanned",
            n(&s.served),
            n(&s.from_store),
            n(&s.read),
            n(&s.scanned),
            n(&s.store_scanned)
        )
    }
}

/// Where a project's directory may take classes from a store entry: the
/// entry's record for that directory, and — when this run laid the package
/// out — the identities of its files right after the placement.
pub struct StoreSeed {
    pub record: CacheSlot,
    pub placed: Option<Arc<PlacedPackage>>,
    /// Install directories of other packages placed inside this one's
    /// (canonical, OS bytes, with a trailing separator): their files are not
    /// this placement's to vouch for, even if its walk saw them.
    pub nested: Vec<Vec<u8>>,
}

/// Version of the per-file cache encoding; bump with `CACHE_FORMAT`.
/// r2: identity (dev, inode, size, mtime, ctime), racy flag and content
/// digest per file (was mtime and size).
const FILE_CACHE_FORMAT: &str = "r2";

/// Cache location for the per-file scan of a directory (or file) of ONE
/// project — its own sources, a `path` package, vendor/: key = canonical
/// path + format version. The directory is listed on every scan (a file
/// added or removed is seen); a file is served without being read only
/// when its identity ([`FileIdentity`]) is the one recorded and was not
/// racy then. A changed identity means a read and a digest; the class
/// detection runs only for content no record knows.
pub struct FileCacheSlot {
    file: PathBuf,
    ctx: Arc<CacheContext>,
    seed: Option<StoreSeed>,
}

/// One cached file of a project.
struct FileRecord {
    identity: Option<FileIdentity>,
    /// Taken within the racy window: re-read on the next scan.
    racy: bool,
    sha: Digest,
    classes: Vec<Vec<u8>>,
}

impl FileCacheSlot {
    pub fn new(
        cache_root: &Path,
        path: &Path,
        ctx: Arc<CacheContext>,
        seed: Option<StoreSeed>,
    ) -> FileCacheSlot {
        use sha1::{Digest as _, Sha1};
        let canonical =
            vivacity_core::pathutil::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
        let mut h = Sha1::new();
        h.update(CACHE_FORMAT.as_bytes());
        h.update(b"\0");
        h.update(FILE_CACHE_FORMAT.as_bytes());
        h.update(b"\0");
        h.update(canonical.to_string_lossy().as_bytes());
        let key: String = h.finalize().iter().map(|b| format!("{b:02x}")).collect();
        FileCacheSlot {
            file: cache_root.join("classmap-files").join(format!("{key}.bin")),
            ctx,
            seed,
        }
    }

    /// Records by relative path.
    ///
    ///   u32 file_count
    ///   [ u32 rel_len, rel, u8 flags (1: identity follows, 2: racy),
    ///     identity (56 bytes, when flagged), sha1 (20 bytes),
    ///     u32 class_count, [ u32 class_len, class ]* ]*
    fn load(&self) -> Option<HashMap<String, FileRecord>> {
        let bytes = std::fs::read(&self.file).ok()?;
        let mut r = ByteReader::new(&bytes);
        let n = r.u32()? as usize;
        let mut out = HashMap::with_capacity(n.min(r.remaining() / 29));
        for _ in 0..n {
            let rel = String::from_utf8(r.slice()?.to_vec()).ok()?;
            let flags = r.fixed(1)?[0];
            if flags & !3 != 0 {
                return None;
            }
            let identity = if flags & 1 != 0 {
                Some(FileIdentity::decode(r.fixed(FileIdentity::ENCODED_LEN)?)?)
            } else {
                None
            };
            let sha: Digest = r.fixed(20)?.try_into().ok()?;
            let nc = r.u32()? as usize;
            let mut classes = Vec::with_capacity(nc.min(r.remaining() / 4));
            for _ in 0..nc {
                classes.push(r.slice()?.to_vec());
            }
            out.insert(
                rel,
                FileRecord {
                    identity,
                    racy: flags & 2 != 0,
                    sha,
                    classes,
                },
            );
        }
        if !r.is_empty() {
            return None;
        }
        Some(out)
    }

    fn store(&self, records: &BTreeMap<String, FileRecord>) {
        let mut buf: Vec<u8> = Vec::new();
        buf.extend_from_slice(&(records.len() as u32).to_le_bytes());
        for (rel, rec) in records {
            push_slice(&mut buf, rel.as_bytes());
            let flags = u8::from(rec.identity.is_some()) | (u8::from(rec.racy) << 1);
            buf.push(flags);
            if let Some(id) = &rec.identity {
                id.encode(&mut buf);
            }
            buf.extend_from_slice(&rec.sha);
            buf.extend_from_slice(&(rec.classes.len() as u32).to_le_bytes());
            for c in &rec.classes {
                push_slice(&mut buf, c);
            }
        }
        write_atomically(&self.file, &buf);
    }
}

/// Appends `u32 len` then the bytes.
fn push_slice(buf: &mut Vec<u8>, s: &[u8]) {
    buf.extend_from_slice(&(s.len() as u32).to_le_bytes());
    buf.extend_from_slice(s);
}

/// Byte reader that fails cleanly (None) on any truncation.
struct ByteReader<'a> {
    bytes: &'a [u8],
    pos: usize,
}

impl<'a> ByteReader<'a> {
    fn new(bytes: &'a [u8]) -> ByteReader<'a> {
        ByteReader { bytes, pos: 0 }
    }
    fn u32(&mut self) -> Option<u32> {
        let end = self.pos.checked_add(4)?;
        let raw = self.bytes.get(self.pos..end)?;
        self.pos = end;
        Some(u32::from_le_bytes(raw.try_into().ok()?))
    }
    /// The next `n` bytes as they are.
    fn fixed(&mut self, n: usize) -> Option<&'a [u8]> {
        let end = self.pos.checked_add(n)?;
        let out = self.bytes.get(self.pos..end)?;
        self.pos = end;
        Some(out)
    }

    fn slice(&mut self) -> Option<&'a [u8]> {
        let len = self.u32()? as usize;
        let end = self.pos.checked_add(len)?;
        let out = self.bytes.get(self.pos..end)?;
        self.pos = end;
        Some(out)
    }
    fn is_empty(&self) -> bool {
        self.pos == self.bytes.len()
    }
    fn remaining(&self) -> usize {
        self.bytes.len() - self.pos
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AutoloadType {
    ClassMap,
    Psr0,
    Psr4,
}

#[derive(Default)]
pub struct ClassMap {
    /// class (raw bytes) -> normalized path; BTreeMap = ksort (bytes).
    pub map: BTreeMap<Vec<u8>, String>,
    pub ambiguous: BTreeMap<Vec<u8>, Vec<String>>,
    /// Classes of `ambiguous` in the order they became ambiguous (PHP's
    /// array keeps insertion order; the warnings come out in it).
    pub ambiguous_order: Vec<Vec<u8>>,
    /// (message, class, path)
    pub psr_violations: Vec<(String, Vec<u8>, String)>,
}

/// `ClassMap::getAmbiguousClasses`' default `$duplicatesFilter`,
/// `{/(test|fixture|example|stub)s?/}i` on the path with `\` as `/`: a
/// duplicate under a tests/fixtures/examples/stubs directory is not
/// reported.
pub fn is_duplicate_filtered(path: &str) -> bool {
    let p = path.replace('\\', "/").to_ascii_lowercase();
    for word in ["test", "fixture", "example", "stub"] {
        let mut from = 0;
        while let Some(i) = p[from..].find(&format!("/{word}")) {
            let after = &p[from + i + 1 + word.len()..];
            if after.starts_with('/') || after.starts_with("s/") {
                return true;
            }
            from += i + 1;
        }
    }
    false
}

pub struct Scanner {
    pub class_map: ClassMap,
    /// Real paths already taken (`$this->scannedFiles`), by their OS bytes:
    /// a `PathBuf` set compares component by component on every lookup,
    /// which was a quarter of a dump's time on 7 000 files.
    scanned: std::collections::HashSet<Vec<u8>>,
}

/// `(path, real path, raw classes)` of one directory, in walk order — the
/// output of the pure `scan_only` phase.
pub type ScannedFiles = Vec<(PathBuf, PathBuf, Vec<Vec<u8>>)>;

const VCS_DIRS: [&str; 9] = [
    ".svn",
    "_svn",
    "CVS",
    "_darcs",
    ".arch-params",
    ".monotone",
    ".bzr",
    ".git",
    ".hg",
];

impl Scanner {
    pub fn new() -> Result<Scanner, ClassMapError> {
        ClassFinder::new()?; // validates the regexes early
        Ok(Scanner {
            class_map: ClassMap::default(),
            scanned: std::collections::HashSet::new(),
        })
    }

    pub fn add_class(&mut self, class: &[u8], path: &str) {
        self.class_map.map.insert(class.to_vec(), path.to_owned());
    }

    /// `scanPaths($path, $excluded, $autoloadType, $namespace)`; `path` is
    /// absolute (file or directory). A missing path is an error for a classmap
    /// rule (as in Composer); missing PSR directories are filtered out upstream
    /// by the caller. Files are visited in readdir order, like Composer's
    /// Finder: the winner of an ambiguity is the same on the same directory.
    pub fn scan_path(
        &mut self,
        path: &Path,
        excluded: Option<&pcre2::bytes::Regex>,
        autoload_type: AutoloadType,
        namespace: &str,
    ) -> Result<(), ClassMapError> {
        self.scan_path_cached(path, excluded, autoload_type, namespace, None)
    }

    /// `scan_path` through a project's per-file cache: reading and detection
    /// are skipped for what the cache can vouch for (`scan_only`), everything
    /// else (exclusions, deduplication, PSR filter, ambiguities) is replayed
    /// identically.
    pub fn scan_path_cached(
        &mut self,
        path: &Path,
        excluded: Option<&pcre2::bytes::Regex>,
        autoload_type: AutoloadType,
        namespace: &str,
        cache: Option<&FileCacheSlot>,
    ) -> Result<(), ClassMapError> {
        let scanned_files = Scanner::scan_only(path, cache)?;
        self.merge_scanned(scanned_files, path, excluded, autoload_type, namespace)
    }

    /// The pure, parallelizable phase of a scan: produces `(path, real path,
    /// raw classes)` in walk order, served from the cache when present. Uses
    /// no `Scanner` state (deduplication/ambiguities/exclusions are applied
    /// by `merge_scanned`), so several directories can be scanned in
    /// parallel and then merged sequentially in order.
    pub fn scan_only(
        path: &Path,
        cache: Option<&FileCacheSlot>,
    ) -> Result<ScannedFiles, ClassMapError> {
        if let Some(slot) = cache {
            return Scanner::scan_only_files_cached(path, slot);
        }
        let (files, _) = Scanner::collect_files(path)?;
        // Sequential reads (parallel reading is slower on APFS), parallel
        // detection on the CPU.
        let mut todo: Vec<(PathBuf, PathBuf, Vec<u8>)> = Vec::new();
        for (file, real) in files {
            let contents = std::fs::read(&file).map_err(|_| ClassMapError::Read(file.clone()))?;
            todo.push((file, real, contents));
        }
        let found = find_all(&todo)?;
        Ok(todo
            .into_iter()
            .zip(found)
            .map(|((file, real, _), classes)| (file, real, classes))
            .collect())
    }

    /// `scan_only` through a project's per-file cache. The directory is
    /// listed (walk order, as always) and each file's identity taken; per
    /// file, in this order:
    /// 1. its identity is the one this project's record holds, and was not
    ///    racy then: the record's classes, nothing read;
    /// 2. this run laid its package out from a store entry and its identity
    ///    is still the one taken right after the placement: the entry
    ///    record's classes, nothing read under vendor/ (the record is
    ///    scanned from the entry's own directory when it has none yet);
    /// 3. otherwise it is read and hashed: the classes of this project's
    ///    record or of the store entry's for the same digest, else the class
    ///    detection.
    ///
    /// The record is rewritten when anything differed; a case-2 entry is
    /// written racy (the next scan reads it once: its bytes were never read
    /// here). A tree with a symbolic link inside is not recorded, and takes
    /// nothing from a placement (a link's target may lie elsewhere).
    fn scan_only_files_cached(
        path: &Path,
        slot: &FileCacheSlot,
    ) -> Result<ScannedFiles, ClassMapError> {
        enum How {
            Served,
            Placed,
            Read,
        }
        let (files, saw_symlink) = Scanner::collect_files(path)?;
        let cached = slot.load().unwrap_or_default();
        let base = base_of(path);
        let placed = slot
            .seed
            .as_ref()
            .and_then(|s| s.placed.as_ref().map(|p| (p, &s.nested)))
            .filter(|_| !saw_symlink);
        let stats = &slot.ctx.stats;

        let mut rels: Vec<Option<String>> = Vec::with_capacity(files.len());
        let mut ids: Vec<Option<FileIdentity>> = Vec::with_capacity(files.len());
        let mut how: Vec<How> = Vec::with_capacity(files.len());
        for (file, real) in &files {
            let rel = rel_key(&base, file);
            let id = FileIdentity::of(real);
            let real_bytes = real.as_os_str().as_encoded_bytes();
            let h = match (id, rel.as_ref().and_then(|r| cached.get(r))) {
                (Some(id), Some(rec)) if rec.identity == Some(id) && !rec.racy => How::Served,
                (Some(id), _)
                    if rel.is_some()
                        && placed.is_some_and(|(p, nested)| {
                            p.files.get(real_bytes) == Some(&id)
                                && !nested.iter().any(|n| real_bytes.starts_with(n))
                        }) =>
                {
                    How::Placed
                }
                _ => How::Read,
            };
            rels.push(rel);
            ids.push(id);
            how.push(h);
        }

        // The store entry's record: built (scanned from the entry) only for
        // a placement this run made; otherwise read when it exists.
        let any_placed = how.iter().any(|h| matches!(h, How::Placed));
        let any_read = how.iter().any(|h| matches!(h, How::Read));
        let store: Option<StoreRecord> = match &slot.seed {
            Some(seed) if any_placed => seed.record.load_or_build(stats),
            Some(seed) if any_read => seed.record.load(),
            _ => None,
        };
        // A placed file the entry's record does not know is read.
        for (h, rel) in how.iter_mut().zip(&rels) {
            let known = rel
                .as_ref()
                .is_some_and(|r| store.as_ref().is_some_and(|s| s.contains_key(r)));
            if matches!(h, How::Placed) && !known {
                *h = How::Read;
            }
        }

        // Sequential reads (parallel reading is slower on APFS); digests and
        // detection in parallel on the CPU.
        let mut todo: Vec<(usize, Vec<u8>)> = Vec::new();
        for (i, ((file, _), h)) in files.iter().zip(&how).enumerate() {
            if matches!(h, How::Read) {
                let contents =
                    std::fs::read(file).map_err(|_| ClassMapError::Read(file.clone()))?;
                todo.push((i, contents));
            }
        }
        let reused = |rel: Option<&String>, sha: &Digest| -> Option<Vec<Vec<u8>>> {
            let rel = rel?;
            if let Some(rec) = cached.get(rel).filter(|r| &r.sha == sha) {
                return Some(rec.classes.clone());
            }
            store
                .as_ref()?
                .get(rel)
                .filter(|(s, _)| s == sha)
                .map(|(_, c)| c.clone())
        };
        let read: Vec<(usize, Digest, Vec<Vec<u8>>)> = todo
            .par_iter()
            .map(|(i, contents)| {
                let sha = digest(contents);
                match reused(rels[*i].as_ref(), &sha) {
                    Some(classes) => {
                        stats.read.fetch_add(1, Ordering::Relaxed);
                        Ok((*i, sha, classes))
                    }
                    None => {
                        stats.scanned.fetch_add(1, Ordering::Relaxed);
                        find_one(contents).map(|c| (*i, sha, c))
                    }
                }
            })
            .collect::<Result<_, _>>()?;
        let mut read = read.into_iter();

        let changed = files.len() != cached.len() || how.iter().any(|h| !matches!(h, How::Served));
        let mut scanned_files: ScannedFiles = Vec::with_capacity(files.len());
        let mut records: BTreeMap<String, FileRecord> = BTreeMap::new();
        for (i, (((file, real), h), (rel, id))) in files
            .into_iter()
            .zip(how)
            .zip(rels.into_iter().zip(ids))
            .enumerate()
        {
            let (sha, classes, racy) = match h {
                // Both only with a key (`rel`) and its record entry.
                How::Served => {
                    stats.served.fetch_add(1, Ordering::Relaxed);
                    let (sha, classes) = rel
                        .as_ref()
                        .and_then(|r| cached.get(r))
                        .map(|rec| (rec.sha, rec.classes.clone()))
                        .unwrap_or_default();
                    (sha, classes, false)
                }
                How::Placed => {
                    stats.from_store.fetch_add(1, Ordering::Relaxed);
                    let (sha, classes) = rel
                        .as_ref()
                        .and_then(|r| store.as_ref()?.get(r))
                        .cloned()
                        .unwrap_or_default();
                    (sha, classes, true)
                }
                How::Read => {
                    let (j, sha, classes) = read.next().unwrap_or_default();
                    debug_assert_eq!(i, j, "reads come back in walk order");
                    let racy = id.as_ref().is_none_or(|id| slot.ctx.is_racy(id));
                    (sha, classes, racy)
                }
            };
            if let (true, Some(rel)) = (changed, rel) {
                records.insert(
                    rel,
                    FileRecord {
                        identity: id,
                        racy,
                        sha,
                        classes: classes.clone(),
                    },
                );
            }
            scanned_files.push((file, real, classes));
        }
        if changed && !saw_symlink {
            slot.store(&records);
        }
        Ok(scanned_files)
    }

    /// Merge phase (sequential, ordered): applies exclusions, realpath
    /// deduplication, the PSR filter and "first one wins" on ambiguities —
    /// the shared state that forces the order across directories.
    pub fn merge_scanned(
        &mut self,
        scanned_files: ScannedFiles,
        path: &Path,
        excluded: Option<&pcre2::bytes::Regex>,
        autoload_type: AutoloadType,
        namespace: &str,
    ) -> Result<(), ClassMapError> {
        let base_path = normalize_path(&path.to_string_lossy());
        for (file, real, classes) in scanned_files {
            let file_lossy = file.to_string_lossy();
            let file_path = vivacity_core::pathutil::normalize_path_cow(&file_lossy);
            let real_key = real.as_os_str().as_encoded_bytes().to_vec();
            if self.scanned.contains(&real_key) {
                continue;
            }
            if let Some(re) = excluded {
                let real_s = real.to_string_lossy();
                if re.is_match(real_s.as_bytes()).unwrap_or(false)
                    || re.is_match(file_path.as_bytes()).unwrap_or(false)
                {
                    continue;
                }
            }
            let mut classes = classes;
            if autoload_type != AutoloadType::ClassMap {
                classes = self.filter_by_namespace(
                    classes,
                    &file_path,
                    namespace,
                    autoload_type,
                    &base_path,
                );
                if !classes.is_empty() {
                    self.scanned.insert(real_key.clone());
                }
            } else {
                self.scanned.insert(real_key.clone());
            }
            for class in classes {
                if let Some(existing) = self.class_map.map.get(&class) {
                    if existing.as_str() != file_path.as_ref() {
                        let entry = self.class_map.ambiguous.entry(class.clone()).or_default();
                        if entry.is_empty() {
                            self.class_map.ambiguous_order.push(class);
                        }
                        entry.push(file_path.to_string());
                    }
                } else {
                    // `ClassMap::addClass` forgets the violations an earlier
                    // scan recorded for this file (another namespace's rule
                    // rejected it before this one took it).
                    self.class_map
                        .psr_violations
                        .retain(|(_, _, p)| p.as_str() != file_path.as_ref());
                    self.class_map.map.insert(class, file_path.to_string());
                }
            }
        }
        Ok(())
    }

    /// Finder-style walk: (path, real path) of php/inc/hh files, and whether
    /// a symlink was traversed (the cache is then disabled).
    fn collect_files(path: &Path) -> Result<(Vec<(PathBuf, PathBuf)>, bool), ClassMapError> {
        let mut files: Vec<(PathBuf, PathBuf)> = Vec::new();
        let mut saw_symlink = false;
        if path.is_file() {
            let real =
                vivacity_core::pathutil::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
            files.push((path.to_path_buf(), real));
        } else if path.is_dir() {
            let base_real =
                vivacity_core::pathutil::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
            let mut symlinked_dirs: Vec<PathBuf> = Vec::new();
            // Composer's ClassMapGenerator walks with Symfony Finder, no
            // sort: `RecursiveIteratorIterator::SELF_FIRST` over
            // `RecursiveDirectoryIterator` — raw readdir order, depth first,
            // a subdirectory descended where readdir lists it. walkdir
            // without a sorter is the same sequence on the same directory,
            // so the winner of an ambiguous class and the order of the
            // warnings are Composer's (2026-09-19; sorted by name before,
            // which differed on case-insensitive-sorted APFS/NTFS listings).
            for entry in walkdir::WalkDir::new(path)
                .follow_links(true)
                .into_iter()
                .filter_entry(|e| {
                    if e.depth() == 0 {
                        return true;
                    }
                    let name = e.file_name().to_string_lossy();
                    !(name.starts_with('.') || VCS_DIRS.contains(&name.as_ref()))
                })
                .filter_map(Result::ok)
            {
                if entry.path_is_symlink() {
                    saw_symlink = true;
                }
                if entry.file_type().is_dir() {
                    if entry.path_is_symlink() {
                        symlinked_dirs.push(entry.path().to_path_buf());
                    }
                    continue;
                }
                if !entry.file_type().is_file() {
                    continue;
                }
                let p = entry.into_path();
                let ext = p
                    .extension()
                    .map(|e| e.to_string_lossy().into_owned())
                    .unwrap_or_default();
                if !matches!(ext.as_str(), "php" | "inc" | "hh") {
                    continue;
                }
                let under_symlink = symlinked_dirs.iter().any(|d| p.starts_with(d));
                let real = if under_symlink {
                    vivacity_core::pathutil::canonicalize(&p).unwrap_or_else(|_| p.clone())
                } else {
                    match p.strip_prefix(path) {
                        Ok(rel) => base_real.join(rel),
                        Err(_) => {
                            vivacity_core::pathutil::canonicalize(&p).unwrap_or_else(|_| p.clone())
                        }
                    }
                };
                files.push((p, real));
            }
        } else {
            return Err(ClassMapError::MissingPath(
                path.to_string_lossy().into_owned(),
            ));
        }
        Ok((files, saw_symlink))
    }

    fn filter_by_namespace(
        &mut self,
        classes: Vec<Vec<u8>>,
        file_path: &str,
        base_namespace: &str,
        autoload_type: AutoloadType,
        base_path: &str,
    ) -> Vec<Vec<u8>> {
        let mut valid = Vec::new();
        let mut rejected = Vec::new();
        let real_sub = file_path.get(base_path.len() + 1..).unwrap_or("");
        let real_sub = match real_sub.rfind('.') {
            Some(p) => &real_sub[..p],
            None => real_sub,
        };
        let base_ns = base_namespace.as_bytes();
        let map_sep = |bytes: &[u8], from: u8| -> Vec<u8> {
            bytes
                .iter()
                .map(|b| if *b == from { b'/' } else { *b })
                .collect()
        };
        for class in classes {
            let sub_path: Vec<u8> = match autoload_type {
                AutoloadType::Psr0 => {
                    if !base_ns.is_empty() && !class.starts_with(base_ns) {
                        rejected.push(class);
                        continue;
                    }
                    match class.iter().rposition(|b| *b == b'\\') {
                        Some(pos) => {
                            let mut v = map_sep(&class[..=pos], b'\\');
                            v.extend(map_sep(&class[pos + 1..], b'_'));
                            v
                        }
                        None => map_sep(&class, b'_'),
                    }
                }
                AutoloadType::Psr4 => {
                    let sub_ns = if base_ns.is_empty() {
                        &class[..]
                    } else {
                        class.get(base_ns.len()..).unwrap_or(b"")
                    };
                    map_sep(sub_ns, b'\\')
                }
                AutoloadType::ClassMap => unreachable!("filtered upstream"),
            };
            if sub_path == real_sub.as_bytes() {
                valid.push(class);
            } else {
                rejected.push(class);
            }
        }
        if valid.is_empty() {
            for class in rejected {
                self.class_map.psr_violations.push((
                    format!(
                        "Class {} located in {file_path} does not comply with {} autoloading standard (rule: {base_namespace} => {base_path}). Skipping.",
                        String::from_utf8_lossy(&class),
                        match autoload_type {
                            AutoloadType::Psr0 => "psr-0",
                            _ => "psr-4",
                        }
                    ),
                    class.clone(),
                    file_path.to_owned(),
                ));
            }
            return Vec::new();
        }
        valid
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn duplicates_filter_matches_composer_regex() {
        assert!(is_duplicate_filtered("/v/vendor/a/b/tests/Foo.php"));
        assert!(is_duplicate_filtered("/v/vendor/a/b/Test/Foo.php"));
        assert!(is_duplicate_filtered("C:\\v\\Fixtures\\Foo.php"));
        assert!(is_duplicate_filtered("/v/examples/x/Foo.php"));
        assert!(is_duplicate_filtered("/v/stub/Foo.php"));
        assert!(!is_duplicate_filtered("/v/testing/Foo.php"));
        assert!(!is_duplicate_filtered("/v/contest/Foo.php"));
        assert!(!is_duplicate_filtered("/v/src/Tests.php"));
    }

    fn classes(src: &str) -> Vec<String> {
        ClassFinder::new()
            .expect("regex")
            .find_classes(src.as_bytes())
            .expect("find")
            .into_iter()
            .map(|c| String::from_utf8_lossy(&c).into_owned())
            .collect()
    }

    #[test]
    fn finds_namespaced_types_and_skips_noise() {
        let src = r#"<?php
namespace Foo\Bar;
// class NotMe
/* class NorMe */
# class NotEither
$s = "class InString"; $t = 'class InSingle';
$h = <<<EOT
class InHeredoc
EOT;
abstract class Baz extends \Other {}
interface Qux {}
trait T1 {}
enum Suit: string { case A = 'a'; }
final class Last implements Qux {}
"#;
        assert_eq!(
            classes(src),
            vec![
                "Foo\\Bar\\Baz",
                "Foo\\Bar\\Qux",
                "Foo\\Bar\\T1",
                "Foo\\Bar\\Suit",
                "Foo\\Bar\\Last"
            ]
        );
    }

    #[test]
    fn attribute_is_not_a_comment_and_braced_namespace_works() {
        let src = "<?php\nnamespace A { #[Attr]\nclass X {} }\nnamespace B;\nclass Y {}\n";
        assert_eq!(classes(src), vec!["A\\X", "B\\Y"]);
    }

    #[test]
    fn variable_and_static_uses_are_not_declarations() {
        let src = "<?php\n$x = new class {};\nFoo::class;\n$this->class = 1;\nclass Real {}\n";
        assert_eq!(classes(src), vec!["Real"]);
    }

    #[test]
    fn raw_bytes_are_preserved() {
        let src = b"<?php\nclass \x7f {}\nclass \xc3\xa9t\xc3\xa9 {}\n";
        let found = ClassFinder::new()
            .expect("regex")
            .find_classes(src)
            .expect("find");
        assert_eq!(found, vec![vec![0x7f], b"\xc3\xa9t\xc3\xa9".to_vec()]);
    }

    #[test]
    fn no_php_tag_means_nothing() {
        assert!(classes("class Foo {}").is_empty());
        assert!(classes("   \n").is_empty());
    }
}

#[cfg(all(test, unix))]
mod cache_tests {
    use super::*;

    fn write(p: &Path, content: &[u8]) {
        std::fs::create_dir_all(p.parent().expect("parent")).expect("mkdir");
        std::fs::write(p, content).expect("write");
    }

    /// A context whose clock puts every file written so far well outside
    /// the racy window (their identities are trusted on the next scan).
    fn settled(dir: &Path) -> Arc<CacheContext> {
        let ctx = CacheContext::new();
        let id = FileIdentity::of(dir).expect("id");
        ctx.set_clock(Clock::at(id.dev, (id.ctime.0 + 60, 0)));
        Arc::new(ctx)
    }

    fn counts(ctx: &CacheContext) -> [usize; 5] {
        let s = &ctx.stats;
        [
            &s.served,
            &s.from_store,
            &s.read,
            &s.scanned,
            &s.store_scanned,
        ]
        .map(|a| a.swap(0, Ordering::Relaxed))
    }

    type Classes = (BTreeMap<Vec<u8>, String>, BTreeMap<Vec<u8>, Vec<String>>);

    fn scan(dir: &Path, slot: Option<&FileCacheSlot>) -> Classes {
        let mut s = Scanner::new().expect("scanner");
        s.scan_path_cached(dir, None, AutoloadType::ClassMap, "", slot)
            .expect("scan");
        (s.class_map.map, s.class_map.ambiguous)
    }

    fn names(c: &Classes) -> Vec<String> {
        c.0.keys()
            .map(|k| String::from_utf8_lossy(k).into_owned())
            .collect()
    }

    /// Same bytes, mtime put back: only the ctime moves (`touch -r`).
    fn rewrite_keeping_mtime(p: &Path, content: &[u8]) {
        let mtime = std::fs::metadata(p)
            .expect("meta")
            .modified()
            .expect("mtime");
        std::thread::sleep(std::time::Duration::from_millis(15));
        std::fs::write(p, content).expect("write");
        std::fs::File::options()
            .write(true)
            .open(p)
            .and_then(|h| h.set_modified(mtime))
            .expect("touch -r");
    }

    /// A scan served from the cache must produce exactly the same classmap
    /// (classes, paths, ambiguities) as a direct scan, including for non-UTF-8
    /// names and files without any class.
    #[test]
    fn cached_scan_equals_direct_scan() {
        let tmp = tempfile::tempdir().expect("tmp");
        let src = tmp.path().join("p/src");
        write(&src.join("A.php"), b"<?php\nnamespace Acme;\nclass A {}\n");
        write(
            &src.join("Sub/B.php"),
            b"<?php\nnamespace Acme\\Sub;\nclass B {}\ninterface I {}\n",
        );
        write(&src.join("raw.php"), b"<?php\nclass \x7f {}\n");
        write(&src.join("nothing.php"), b"<?php\n// rien\n");
        write(
            &src.join("dup.php"),
            b"<?php\nnamespace Acme;\nclass A {}\n",
        );
        let ctx = settled(&src);
        let slot = FileCacheSlot::new(&tmp.path().join("cache"), &src, ctx.clone(), None);
        let direct = scan(&src, None);
        let first = scan(&src, Some(&slot));
        assert!(slot.file.is_file(), "cache not written");
        assert_eq!(counts(&ctx), [0, 0, 0, 5, 0]);
        let cached = scan(&src, Some(&slot));
        assert_eq!(counts(&ctx), [5, 0, 0, 0, 0], "served, nothing read");
        assert_eq!(direct, first);
        assert_eq!(direct, cached);
        assert_eq!(direct.0.len(), 4);
        assert!(direct.0.contains_key(&vec![0x7fu8]));
        assert_eq!(direct.1.len(), 1, "the Acme\\A ambiguity must be replayed");
    }

    #[test]
    fn a_changed_identity_is_read_and_a_changed_content_rescanned() {
        let tmp = tempfile::tempdir().expect("tmp");
        let src = tmp.path().join("src");
        let a = src.join("A.php");
        write(&a, b"<?php\nclass Str {}\n");
        write(&src.join("B.php"), b"<?php\nclass B {}\n");
        let ctx = settled(&src);
        let slot = FileCacheSlot::new(tmp.path(), &src, ctx.clone(), None);
        scan(&src, Some(&slot));
        counts(&ctx);
        // ctime alone: read and hashed, the record's classes, no detection.
        rewrite_keeping_mtime(&a, b"<?php\nclass Str {}\n");
        let ctx = settled(&src);
        let slot = FileCacheSlot::new(tmp.path(), &src, ctx.clone(), None);
        assert_eq!(names(&scan(&src, Some(&slot))), ["B", "Str"]);
        assert_eq!(counts(&ctx), [1, 0, 1, 0, 0]);
        // Same size, mtime restored, another class: detected.
        rewrite_keeping_mtime(&a, b"<?php\nclass Stx {}\n");
        let ctx = settled(&src);
        let slot = FileCacheSlot::new(tmp.path(), &src, ctx.clone(), None);
        assert_eq!(names(&scan(&src, Some(&slot))), ["B", "Stx"]);
        assert_eq!(counts(&ctx), [1, 0, 0, 1, 0]);
        assert_eq!(scan(&src, Some(&slot)), scan(&src, None));
        // Back to the original content: the digest no record holds now.
        rewrite_keeping_mtime(&a, b"<?php\nclass Str {}\n");
        assert_eq!(names(&scan(&src, Some(&slot))), ["B", "Str"]);
    }

    #[test]
    fn a_racy_entry_is_not_served() {
        let tmp = tempfile::tempdir().expect("tmp");
        let src = tmp.path().join("src");
        write(&src.join("A.php"), b"<?php\nclass A {}\n");
        // The reference time is the probe's, taken now: A was written within
        // the window.
        let ctx = Arc::new(CacheContext::new());
        ctx.set_clock(Clock::probe(&[&src], &[tmp.path()]));
        let slot = FileCacheSlot::new(tmp.path(), &src, ctx.clone(), None);
        scan(&src, Some(&slot));
        scan(&src, Some(&slot));
        assert_eq!(counts(&ctx), [0, 0, 1, 1, 0], "re-read, never served");
        // Without a clock, nothing is trusted either.
        let ctx = Arc::new(CacheContext::new());
        let slot = FileCacheSlot::new(tmp.path(), &src, ctx.clone(), None);
        scan(&src, Some(&slot));
        scan(&src, Some(&slot));
        assert_eq!(counts(&ctx), [0, 0, 2, 0, 0]);
    }

    #[test]
    fn added_and_removed_files_are_seen() {
        let tmp = tempfile::tempdir().expect("tmp");
        let src = tmp.path().join("src");
        write(&src.join("A.php"), b"<?php\nclass A {}\n");
        write(&src.join("tests/B.php"), b"<?php\nclass B {}\n");
        let ctx = settled(&src);
        let slot = FileCacheSlot::new(tmp.path(), &src, ctx, None);
        scan(&src, Some(&slot));
        write(&src.join("C.php"), b"<?php\nclass C {}\n");
        std::fs::remove_file(src.join("tests/B.php")).expect("rm");
        assert_eq!(names(&scan(&src, Some(&slot))), ["A", "C"]);
        assert_eq!(scan(&src, Some(&slot)), scan(&src, None));
    }

    /// The store entry's record comes from the entry's own directory; a
    /// copy placed by this run and untouched takes it without being read;
    /// an edited copy is read, rescanned, and changes neither the record nor
    /// what another project gets from it.
    #[test]
    fn the_store_record_comes_from_the_entry_and_serves_only_its_bytes() {
        let tmp = tempfile::tempdir().expect("tmp");
        let entry = tmp.path().join("store/acme/lib/1.0.0-abc");
        write(&entry.join("src/Str.php"), b"<?php\nclass Str {}\n");
        write(&entry.join("src/Other.php"), b"<?php\nclass Other {}\n");
        let copy = |to: &str| {
            let dest = tmp.path().join(to).join("vendor/acme/lib");
            for f in ["src/Str.php", "src/Other.php"] {
                write(&dest.join(f), &std::fs::read(entry.join(f)).expect("r"));
            }
            dest
        };
        let a = copy("A");
        let placed = Arc::new(
            PlacedPackage::record("acme/lib", &a, &entry, &[String::new()]).expect("placed"),
        );
        let seed = |placed: Option<Arc<PlacedPackage>>| StoreSeed {
            record: CacheSlot::new(tmp.path(), &entry, Path::new("src")),
            placed,
            nested: Vec::new(),
        };
        // A, placed by this run: the record is scanned from the entry, A's
        // copy is not read.
        let ctx = settled(&a);
        let slot_a = FileCacheSlot::new(
            tmp.path(),
            &a.join("src"),
            ctx.clone(),
            Some(seed(Some(placed.clone()))),
        );
        assert_eq!(
            names(&scan(&a.join("src"), Some(&slot_a))),
            ["Other", "Str"]
        );
        assert_eq!(counts(&ctx), [0, 2, 0, 0, 2]);
        // Placement entries are written racy: the next run (no placement in
        // hand) reads them once, then serves them.
        let slot_a = FileCacheSlot::new(tmp.path(), &a.join("src"), ctx.clone(), Some(seed(None)));
        assert_eq!(
            names(&scan(&a.join("src"), Some(&slot_a))),
            ["Other", "Str"]
        );
        assert_eq!(counts(&ctx), [0, 0, 2, 0, 0]);
        scan(&a.join("src"), Some(&slot_a));
        assert_eq!(counts(&ctx), [2, 0, 0, 0, 0]);
        // A's copy edited in place, same size and mtime: rescanned, even
        // with the placement still in hand.
        rewrite_keeping_mtime(&a.join("src/Str.php"), b"<?php\nclass Stx {}\n");
        let ctx = settled(&a);
        let slot_a = FileCacheSlot::new(
            tmp.path(),
            &a.join("src"),
            ctx.clone(),
            Some(seed(Some(placed.clone()))),
        );
        assert_eq!(
            names(&scan(&a.join("src"), Some(&slot_a))),
            ["Other", "Stx"]
        );
        assert_eq!(counts(&ctx), [1, 0, 0, 1, 0]);
        let record = seed(None).record.load().expect("record");
        assert_eq!(
            record["Str.php"].1,
            vec![b"Str".to_vec()],
            "the record is the entry's"
        );
        // B, a copy no placement vouches for: read, the entry's classes by
        // digest; its classmap is its own.
        let b = copy("B");
        let ctx = settled(&b);
        let slot_b = FileCacheSlot::new(tmp.path(), &b.join("src"), ctx.clone(), Some(seed(None)));
        assert_eq!(
            names(&scan(&b.join("src"), Some(&slot_b))),
            ["Other", "Str"]
        );
        assert_eq!(counts(&ctx), [0, 0, 2, 0, 0]);
        // A placement of ANOTHER package (nested install dirs) vouches for
        // nothing here.
        let c = copy("C");
        let elsewhere =
            Arc::new(PlacedPackage::record("x/y", &a, &entry, &[String::new()]).expect("placed"));
        let ctx = settled(&c);
        let slot_c = FileCacheSlot::new(
            tmp.path(),
            &c.join("src"),
            ctx.clone(),
            Some(seed(Some(elsewhere))),
        );
        scan(&c.join("src"), Some(&slot_c));
        assert_eq!(counts(&ctx), [0, 0, 2, 0, 0]);
    }

    /// A package placed inside another's install directory: the outer
    /// placement's walk may have seen the inner package's files, which are
    /// not the outer entry's bytes.
    #[test]
    fn a_nested_package_is_not_vouched_for_by_the_outer_placement() {
        let tmp = tempfile::tempdir().expect("tmp");
        let outer_entry = tmp.path().join("store/outer");
        write(
            &outer_entry.join("sub/In.php"),
            b"<?php\nclass FromOuter {}\n",
        );
        let dest = tmp.path().join("p/outer");
        write(&dest.join("sub/In.php"), b"<?php\nclass FromInner {}\n");
        let placed = Arc::new(
            PlacedPackage::record("o/o", &dest, &outer_entry, &[String::new()]).expect("placed"),
        );
        let mut inner = vivacity_core::pathutil::canonicalize(dest.join("sub"))
            .expect("canon")
            .into_os_string()
            .into_encoded_bytes();
        inner.push(b'/');
        let ctx = settled(&dest);
        let slot = FileCacheSlot::new(
            tmp.path(),
            &dest,
            ctx.clone(),
            Some(StoreSeed {
                record: CacheSlot::new(tmp.path(), &outer_entry, Path::new("")),
                placed: Some(placed),
                nested: vec![inner],
            }),
        );
        assert_eq!(names(&scan(&dest, Some(&slot))), ["FromInner"]);
        assert_eq!(counts(&ctx), [0, 0, 0, 1, 0]);
    }

    /// An entry removed and extracted again under the same name (another
    /// archive at the same URL) is another directory: its record is not the
    /// old one's.
    #[test]
    fn a_re_extracted_entry_does_not_reuse_the_old_record() {
        let tmp = tempfile::tempdir().expect("tmp");
        let entry = tmp.path().join("store/acme/a/1.0.0-noref");
        write(&entry.join("src/A.php"), b"<?php\nclass Old {}\n");
        let old = CacheSlot::new(tmp.path(), &entry, Path::new("src"));
        let ctx = CacheContext::new();
        assert!(old.load_or_build(&ctx.stats).is_some());
        std::fs::remove_dir_all(&entry).expect("rm");
        write(&entry.join("src/A.php"), b"<?php\nclass New {}\n");
        let new = CacheSlot::new(tmp.path(), &entry, Path::new("src"));
        assert_ne!(old.file, new.file);
        let record = new.load_or_build(&ctx.stats).expect("record");
        assert_eq!(record["A.php"].1, vec![b"New".to_vec()]);
    }

    /// A project's copy of an entry, placed from it by this run, and its slot.
    #[cfg(unix)]
    fn placed_from(
        cache: &Path,
        entry: &Path,
        project: &Path,
        content: &[u8],
        ctx: Arc<CacheContext>,
    ) -> (PathBuf, FileCacheSlot) {
        let src = project.join("src");
        write(&src.join("A.php"), content);
        let placed =
            PlacedPackage::record("acme/a", project, entry, &[String::new()]).expect("placed");
        let slot = FileCacheSlot::new(
            cache,
            &src,
            ctx,
            Some(StoreSeed {
                record: CacheSlot::new(cache, entry, Path::new("src")),
                placed: Some(Arc::new(placed)),
                nested: Vec::new(),
            }),
        );
        (src, slot)
    }

    /// The worst case of the entry's identity: an entry extracted again within one
    /// timestamp tick keeps its inode and its ctime, so its slot, here with a file of the
    /// same size written in place. A package placed from it gets its classes from the
    /// record, read from no byte of its own: they must be its own.
    #[cfg(unix)]
    #[test]
    fn the_same_slot_on_a_re_extracted_entry_serves_no_stale_classes() {
        let tmp = tempfile::tempdir().expect("tmp");
        let entry = tmp.path().join("store/acme/a/1.0.0-noref");
        write(&entry.join("src/A.php"), b"<?php\nclass Old {}\n");
        let slot_file = CacheSlot::new(tmp.path(), &entry, Path::new("src")).file;

        // The entry just extracted: within the racy window of the clock.
        let racy = CacheContext::new();
        racy.set_clock(Clock::probe(&[entry.as_path()], &[tmp.path()]));
        let (src, slot) = placed_from(
            tmp.path(),
            &entry,
            &tmp.path().join("one"),
            b"<?php\nclass Old {}\n",
            Arc::new(racy),
        );
        assert_eq!(names(&scan(&src, Some(&slot))), ["Old"]);
        let kept_while_racy = slot_file.exists();

        // Extracted again in the same tick: same slot, same size, other content.
        write(&entry.join("src/A.php"), b"<?php\nclass New {}\n");
        assert_eq!(
            CacheSlot::new(tmp.path(), &entry, Path::new("src")).file,
            slot_file,
            "the case needs the same slot"
        );
        let project = tmp.path().join("two");
        write(&project.join("src/A.php"), b"<?php\nclass New {}\n");
        let ctx = settled(&project.join("src"));
        let (src, slot) = placed_from(tmp.path(), &entry, &project, b"<?php\nclass New {}\n", ctx);
        assert_eq!(names(&scan(&src, Some(&slot))), ["New"]);
        assert!(!kept_while_racy, "a racy entry's record is not kept");
        assert!(slot_file.exists(), "a settled entry's record is kept");
    }

    /// Records written by 0.26.2 (format v4, kept whatever the entry's age) can hold another
    /// content's classes under the identity they name: they stay unread.
    #[cfg(unix)]
    #[test]
    fn a_record_of_the_previous_format_is_not_read() {
        use sha1::{Digest as _, Sha1};
        let tmp = tempfile::tempdir().expect("tmp");
        let entry = tmp.path().join("store/acme/a/1.0.0-noref");
        write(&entry.join("src/A.php"), b"<?php\nclass Real {}\n");

        // The slot as 0.26.2 names it, spelled out rather than taken from the code.
        let id = FileIdentity::of(&entry).expect("identity");
        let mut h = Sha1::new();
        h.update(b"v4\0");
        h.update(entry.to_string_lossy().as_bytes());
        h.update(b"\0");
        h.update(id.dev.to_le_bytes());
        h.update(id.ino.to_le_bytes());
        h.update(id.ctime.0.to_le_bytes());
        h.update(id.ctime.1.to_le_bytes());
        h.update(b"\0src");
        let key: String = h.finalize().iter().map(|b| format!("{b:02x}")).collect();

        // Its encoding: one file, a digest, one class.
        let mut record = Vec::new();
        record.extend_from_slice(&1u32.to_le_bytes());
        push_slice(&mut record, b"A.php");
        record.extend_from_slice(&[0u8; 20]);
        record.extend_from_slice(&1u32.to_le_bytes());
        push_slice(&mut record, b"Stale");
        write(
            &tmp.path().join("classmap").join(format!("{key}.bin")),
            &record,
        );

        let project = tmp.path().join("project");
        write(&project.join("src/A.php"), b"<?php\nclass Real {}\n");
        let ctx = settled(&project.join("src"));
        let (src, slot) = placed_from(tmp.path(), &entry, &project, b"<?php\nclass Real {}\n", ctx);
        assert_eq!(names(&scan(&src, Some(&slot))), ["Real"]);
    }

    #[test]
    fn an_entry_that_cannot_be_scanned_falls_back_to_the_copy() {
        let tmp = tempfile::tempdir().expect("tmp");
        let entry = tmp.path().join("store/gone");
        let a = tmp.path().join("A/src");
        write(&a.join("A.php"), b"<?php\nclass A {}\n");
        let placed =
            PlacedPackage::record("acme/gone", &tmp.path().join("A"), &entry, &[String::new()]);
        let ctx = settled(&a);
        let slot = FileCacheSlot::new(
            tmp.path(),
            &a,
            ctx.clone(),
            Some(StoreSeed {
                record: CacheSlot::new(tmp.path(), &entry, Path::new("src")),
                placed: placed.map(Arc::new),
                nested: Vec::new(),
            }),
        );
        assert_eq!(names(&scan(&a, Some(&slot))), ["A"]);
        assert_eq!(counts(&ctx), [0, 0, 0, 1, 0]);
        assert!(!tmp.path().join("classmap").exists(), "no store record");
    }

    #[test]
    fn a_class_found_valid_later_clears_its_violation() {
        // owncloud-core's `Tests\\ => tests/` then `Test\\ => tests/lib/`
        // (krsort): the first rejects tests/lib/TestCase.php, the second
        // takes it, and Composer prints no warning.
        let tmp = tempfile::tempdir().expect("tmp");
        let tests = tmp.path().join("tests");
        write(
            &tests.join("lib/TestCase.php"),
            b"<?php\nnamespace Test;\nclass TestCase {}\n",
        );
        let mut s = Scanner::new().expect("scanner");
        s.scan_path_cached(&tests, None, AutoloadType::Psr4, "Tests\\", None)
            .expect("scan");
        assert_eq!(s.class_map.psr_violations.len(), 1);
        s.scan_path_cached(&tests.join("lib"), None, AutoloadType::Psr4, "Test\\", None)
            .expect("scan");
        assert!(s.class_map.psr_violations.is_empty());
        assert_eq!(s.class_map.map.len(), 1);
    }

    #[test]
    fn symlinked_tree_is_not_recorded_nor_taken_from_a_placement() {
        let tmp = tempfile::tempdir().expect("tmp");
        let entry = tmp.path().join("store/e");
        write(&entry.join("real/X.php"), b"<?php\nclass X {}\n");
        let pkg = tmp.path().join("pkg");
        write(&pkg.join("real/X.php"), b"<?php\nclass X {}\n");
        std::os::unix::fs::symlink(pkg.join("real"), pkg.join("link")).expect("ln");
        let placed = PlacedPackage::record("a/b", &pkg, &entry, &[String::new()]).map(Arc::new);
        let ctx = settled(&pkg);
        let slot = FileCacheSlot::new(
            tmp.path(),
            &pkg,
            ctx.clone(),
            Some(StoreSeed {
                record: CacheSlot::new(tmp.path(), &entry, Path::new("")),
                placed,
                nested: Vec::new(),
            }),
        );
        assert_eq!(names(&scan(&pkg, Some(&slot))), ["X"]);
        assert!(
            !slot.file.exists(),
            "a tree with a symlink must not be recorded"
        );
        assert_eq!(counts(&ctx)[1], 0, "nothing taken from the placement");
    }

    /// A corrupt cache file — truncated, trailing garbage, or a garbage
    /// count field claiming billions of entries — must be ignored (rescan),
    /// never panic, never abort in the allocator, never return a partial
    /// classmap.
    #[test]
    fn corrupt_cache_is_rescanned() {
        let tmp = tempfile::tempdir().expect("tmp");
        let entry = tmp.path().join("store/acme/lib/1.0.0-abc");
        write(
            &entry.join("src/A.php"),
            b"<?php\nnamespace Acme;\nclass A {}\n",
        );
        let src = tmp.path().join("p/src");
        write(&src.join("A.php"), b"<?php\nnamespace Acme;\nclass A {}\n");
        let placed =
            PlacedPackage::record("acme/lib", &tmp.path().join("p"), &entry, &[String::new()])
                .map(Arc::new);
        let ctx = settled(&src);
        let slot = FileCacheSlot::new(
            &tmp.path().join("cache"),
            &src,
            ctx,
            Some(StoreSeed {
                record: CacheSlot::new(&tmp.path().join("cache"), &entry, Path::new("src")),
                placed,
                nested: Vec::new(),
            }),
        );
        let direct = scan(&src, None);
        scan(&src, Some(&slot)); // fills both records
        let store_file = slot.seed.as_ref().expect("seed").record.file.clone();
        for file in [slot.file.clone(), store_file] {
            let valid = std::fs::read(&file).expect("cache bytes");
            // count=1, rel="a", then an absurd class count (and no class bytes).
            let mut huge_class_count = Vec::new();
            huge_class_count.extend_from_slice(&1u32.to_le_bytes());
            huge_class_count.extend_from_slice(&1u32.to_le_bytes());
            huge_class_count.push(b'a');
            huge_class_count.extend_from_slice(&u32::MAX.to_le_bytes());
            let corruptions: Vec<Vec<u8>> = vec![
                valid[..valid.len() - 1].to_vec(),        // truncated
                [valid.clone(), vec![0u8]].concat(),      // trailing bytes
                vec![0xFF, 0xFF, 0xFF, 0xFF, 0x00, 0x01], // absurd file count
                huge_class_count,
                Vec::new(), // empty file
            ];
            for (i, bad) in corruptions.iter().enumerate() {
                write(&file, bad);
                assert!(
                    slot.load().is_none() || file != slot.file,
                    "corruption #{i}"
                );
                assert_eq!(
                    scan(&src, Some(&slot)),
                    direct,
                    "corruption #{i} must rescan"
                );
            }
            write(&file, &valid);
        }
    }
}

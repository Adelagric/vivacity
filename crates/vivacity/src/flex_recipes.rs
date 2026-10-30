//! symfony/flex's recipes, applied at `POST_UPDATE_CMD` (`Flex::install`
//! with recipes, symfony/flex 2.11.0): the configurators vivacity
//! reproduces, run over a virtual view of the project BEFORE anything is
//! written, so that the decision to stay native or to hand the command
//! over is taken while nothing has been written yet. The outcome is a
//! [`FlexPlan`]: the stderr lines and every file Flex would write, with the
//! state each was in when the plan was made; [`FlexPlan::apply`] checks that
//! state and writes, at the point where Composer dispatches the event.
//!
//! Any input whose effect is not reproduced byte for byte is a refusal (a
//! `String` reason, for the hand-over line), never an approximation.

use serde_json::{Map, Value};
use std::path::{Path, PathBuf};

/// A reason to hand the command to Composer.
pub(crate) type Refusal = String;
type R<T> = Result<T, Refusal>;

// ---------------------------------------------------------------- PHP helpers

/// `is_numeric($s)` for a string, PHP 8: optional leading and trailing
/// whitespace, a sign, digits with at most one dot (at least one digit),
/// an optional exponent.
pub(crate) fn php_is_numeric(s: &str) -> bool {
    let ws = |c: u8| matches!(c, b' ' | b'\t' | b'\n' | b'\r' | 0x0b | 0x0c);
    let b = s.as_bytes();
    let (mut i, mut end) = (0, b.len());
    while i < end && ws(b[i]) {
        i += 1;
    }
    while end > i && ws(b[end - 1]) {
        end -= 1;
    }
    if i < end && matches!(b[i], b'+' | b'-') {
        i += 1;
    }
    let mut digits = 0;
    let mut dot = false;
    while i < end && (b[i].is_ascii_digit() || (b[i] == b'.' && !dot)) {
        if b[i] == b'.' {
            dot = true;
        } else {
            digits += 1;
        }
        i += 1;
    }
    if digits == 0 {
        return false;
    }
    if i < end && matches!(b[i], b'e' | b'E') {
        let mut j = i + 1;
        if j < end && matches!(b[j], b'+' | b'-') {
            j += 1;
        }
        let start = j;
        while j < end && b[j].is_ascii_digit() {
            j += 1;
        }
        if j == start {
            return false;
        }
        i = j;
    }
    i == end
}

/// `filter_var($v, FILTER_VALIDATE_BOOLEAN)` without `FILTER_NULL_ON_FAILURE`:
/// true for `true`, `1`, and the strings `1`, `true`, `on`, `yes` (trimmed
/// of ` \t\n\r\v` — not NUL, measured — any case); false for everything
/// else.
pub(crate) fn php_filter_bool(v: &Value) -> bool {
    match v {
        Value::Bool(b) => *b,
        Value::Number(n) => n.as_f64() == Some(1.0),
        Value::String(s) => matches!(
            s.trim_matches(|c: char| matches!(c, ' ' | '\t' | '\n' | '\r' | '\x0b'))
                .to_ascii_lowercase()
                .as_str(),
            "1" | "true" | "on" | "yes"
        ),
        _ => false,
    }
}

/// A canonical PHP integer key (`"0"`, `"-3"`, `"12"`): PHP turns such a
/// string key into an int, which `array_merge` renumbers.
fn php_int_key(k: &str) -> bool {
    let digits = k.strip_prefix('-').unwrap_or(k);
    !digits.is_empty()
        && digits.bytes().all(|b| b.is_ascii_digit())
        && (digits == "0" || !digits.starts_with('0'))
        && k != "-0"
}

/// Symfony Console's tags in a message, as the non-decorated formatter
/// drops them: only the ones Flex's own lines carry. A message holding any
/// other `<` is refused by the caller before it gets here.
fn strip_tags(s: &str) -> String {
    let mut out = s.to_owned();
    for tag in [
        "<info>",
        "</info>",
        "<comment>",
        "</comment>",
        "<fg=green>",
        "</>",
    ] {
        out = out.replace(tag, "");
    }
    out
}

// ---------------------------------------------------------------- Options

/// `Flex::initOptions` then `Options`: the defaults, overridden and extended
/// by the whole of `extra` (`array_merge`).
pub(crate) struct Options {
    entries: Vec<(String, Value)>,
}

impl Options {
    pub(crate) fn new(manifest: &Value) -> R<Options> {
        let extra = manifest.get("extra");
        let root_dir = extra
            .and_then(|e| e.get("symfony"))
            .and_then(|s| s.get("root-dir"))
            .cloned()
            .unwrap_or_else(|| Value::String(".".to_owned()));
        let runtime = extra
            .and_then(|e| e.get("runtime"))
            .cloned()
            .unwrap_or_else(|| Value::Array(Vec::new()));
        let mut entries: Vec<(String, Value)> = vec![
            ("bin-dir".to_owned(), Value::String("bin".to_owned())),
            ("conf-dir".to_owned(), Value::String("conf".to_owned())),
            ("config-dir".to_owned(), Value::String("config".to_owned())),
            ("src-dir".to_owned(), Value::String("src".to_owned())),
            ("var-dir".to_owned(), Value::String("var".to_owned())),
            ("public-dir".to_owned(), Value::String("public".to_owned())),
            ("root-dir".to_owned(), root_dir),
            ("runtime".to_owned(), runtime),
        ];
        match extra {
            None | Some(Value::Null) => {}
            Some(Value::Object(map)) => {
                for (k, v) in map {
                    // An integer key is renumbered by `array_merge`: no
                    // option lookup could then be trusted.
                    if php_int_key(k) {
                        return Err(format!("extra has the integer key {k:?}"));
                    }
                    match entries.iter_mut().find(|(n, _)| n == k) {
                        Some(slot) => slot.1 = v.clone(),
                        None => entries.push((k.clone(), v.clone())),
                    }
                }
            }
            Some(Value::Array(a)) if a.is_empty() => {}
            Some(_) => return Err("extra is not an object".to_owned()),
        }
        let options = Options { entries };
        // Every path Flex builds starts from `root-dir`; `.` (the default)
        // is the only one reproduced.
        if options.get("root-dir") != Some(&Value::String(".".to_owned())) {
            return Err("extra.symfony.root-dir / extra.root-dir is set".to_owned());
        }
        Ok(options)
    }

    fn get(&self, name: &str) -> Option<&Value> {
        self.entries.iter().find(|(n, _)| n == name).map(|(_, v)| v)
    }

    /// `$this->options->get('runtime')[$key] ?? $default`, as a string.
    fn runtime_str(&self, key: &str, default: &str) -> R<String> {
        match self.get("runtime") {
            None | Some(Value::Null) => Ok(default.to_owned()),
            Some(Value::Object(m)) => match m.get(key) {
                None | Some(Value::Null) => Ok(default.to_owned()),
                Some(Value::String(s)) => Ok(s.clone()),
                Some(other) => Err(format!("extra.runtime.{key} is {other}, not a string")),
            },
            Some(Value::Array(a)) if a.is_empty() => Ok(default.to_owned()),
            Some(_) => Err("extra.runtime is not an object".to_owned()),
        }
    }

    pub(crate) fn dotenv_path(&self) -> R<String> {
        self.runtime_str("dotenv_path", ".env")
    }

    /// `Options::expandTargetDir`: every `%NAME%` whose option is set
    /// (`str_replace('_', '-', strtolower(NAME))`, `isset` so not null) is
    /// replaced by its value with trailing slashes removed; then the
    /// `phpunit.xml.dist` / `phpunit.dist.xml` swap when the name asked for
    /// is absent and the other one present.
    pub(crate) fn expand(&self, target: &str, files: &Vfs) -> R<String> {
        let b = target.as_bytes();
        let mut out = String::with_capacity(target.len());
        let mut i = 0;
        let mut copied = 0;
        while i < b.len() {
            if b[i] != b'%' {
                i += 1;
                continue;
            }
            // `{%(.+?)%}`: at least one byte, none of them a newline (no /s).
            let mut close = None;
            let mut j = i + 2;
            while j < b.len() {
                if b[j - 1] == b'\n' {
                    break;
                }
                if b[j] == b'%' {
                    close = Some(j);
                    break;
                }
                j += 1;
            }
            let Some(j) = close else {
                i += 1;
                continue;
            };
            let raw = &target[i + 1..j];
            let name = raw.to_ascii_lowercase().replace('_', "-");
            out.push_str(&target[copied..i]);
            match self.get(&name) {
                None | Some(Value::Null) => out.push_str(&target[i..=j]),
                Some(Value::String(s)) => out.push_str(s.trim_end_matches('/')),
                Some(other) => {
                    return Err(format!(
                        "%{raw}% expands extra.{name}, which is {other} and not a string"
                    ))
                }
            }
            i = j + 1;
            copied = i;
        }
        out.push_str(&target[copied..]);
        const PHPUNIT: [&str; 2] = ["phpunit.xml.dist", "phpunit.dist.xml"];
        if let Some(pos) = PHPUNIT.iter().position(|p| *p == out) {
            if !files.exists(&out)? {
                let other = PHPUNIT[1 - pos];
                if files.exists(other)? {
                    return Ok(other.to_owned());
                }
            }
        }
        Ok(out)
    }
}

// ---------------------------------------------------------------- the virtual project

/// A file the plan writes: its state when the plan was made (`None`:
/// absent) and the bytes it ends with.
#[derive(Debug, Clone)]
struct Touched {
    rel: String,
    before: Option<Vec<u8>>,
    after: Vec<u8>,
    /// `chmod(fileperms | 0111)` after the write (copy-from-recipe's
    /// `executable`).
    exec: bool,
    /// The plan only asserts this file's state (composer.lock, which the
    /// install writes before the event): checked, not written.
    assert_only: bool,
}

/// The project as Flex sees it during `install`: the disk, overlaid with
/// what the plan has written so far.
#[derive(Debug)]
pub(crate) struct Vfs {
    root: PathBuf,
    touched: Vec<Touched>,
    /// Every path whose state the simulation looked at (read, tested,
    /// written), with its state on disk at that moment: the decision
    /// refuses when one of them is also written by the install itself
    /// (`observed`), and the application checks that the ones it does not
    /// write are still as seen.
    seen: std::sync::Mutex<std::collections::BTreeMap<String, DiskState>>,
}

/// A path's state on disk, as far as Flex's checks can tell them apart.
#[derive(Debug, Clone, PartialEq, Eq)]
enum DiskState {
    Absent,
    Dir,
    File(Vec<u8>),
    Other,
}

fn disk_state(path: &Path) -> DiskState {
    match std::fs::metadata(path) {
        Err(_) => DiskState::Absent,
        Ok(m) if m.is_dir() => DiskState::Dir,
        Ok(m) if m.is_file() => std::fs::read(path)
            .map(DiskState::File)
            .unwrap_or(DiskState::Other),
        Ok(_) => DiskState::Other,
    }
}

/// A path Flex builds (`./config/x.yaml`, `.gitignore`, `phpunit.dist.xml`)
/// as the project-relative path it names. Anything that could leave the
/// project, or that PHP and Rust might resolve differently, is refused.
fn normalize(path: &str) -> R<String> {
    if path.starts_with('/') || path.contains('\\') || path.contains('\0') {
        return Err(format!("the path {path:?} is not a plain project path"));
    }
    let mut parts: Vec<&str> = Vec::new();
    for part in path.split('/') {
        match part {
            "" | "." => {}
            ".." => return Err(format!("the path {path:?} leaves the project")),
            p => parts.push(p),
        }
    }
    if parts.is_empty() {
        return Err(format!("the path {path:?} names the project itself"));
    }
    Ok(parts.join("/"))
}

impl Vfs {
    pub(crate) fn new(root: &Path) -> Vfs {
        Vfs {
            root: root.to_path_buf(),
            touched: Vec::new(),
            seen: Default::default(),
        }
    }

    /// `normalize`, and the path noted as looked at.
    fn note(&self, path: &str) -> R<String> {
        let rel = normalize(path)?;
        if let Ok(mut seen) = self.seen.lock() {
            if !seen.contains_key(&rel) {
                let state = disk_state(&self.root.join(&rel));
                seen.insert(rel.clone(), state);
            }
        }
        Ok(rel)
    }

    /// The paths the simulation looked at, project-relative.
    pub(crate) fn observed(&self) -> Vec<String> {
        self.seen
            .lock()
            .map(|s| s.keys().cloned().collect())
            .unwrap_or_default()
    }

    fn entry(&self, rel: &str) -> Option<&Touched> {
        self.touched.iter().find(|t| t.rel == rel)
    }

    fn disk(&self, rel: &str) -> PathBuf {
        self.root.join(rel)
    }

    /// `file_get_contents`, or `None` when there is no file.
    pub(crate) fn read(&self, path: &str) -> R<Option<Vec<u8>>> {
        let rel = self.note(path)?;
        if let Some(t) = self.entry(&rel) {
            return Ok(Some(t.after.clone()));
        }
        let disk = self.disk(&rel);
        match std::fs::metadata(&disk) {
            Ok(m) if m.is_file() => std::fs::read(&disk)
                .map(Some)
                .map_err(|e| format!("cannot read {}: {e}", disk.display())),
            Ok(_) => Ok(None),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(format!("cannot stat {}: {e}", disk.display())),
        }
    }

    /// A directory holding a file the plan writes counts as existing.
    fn virtual_dir(&self, rel: &str) -> bool {
        let prefix = format!("{rel}/");
        self.touched
            .iter()
            .any(|t| !t.assert_only && t.rel.starts_with(&prefix))
    }

    /// `file_exists`.
    pub(crate) fn exists(&self, path: &str) -> R<bool> {
        let rel = self.note(path)?;
        Ok(self.entry(&rel).is_some() || self.disk(&rel).exists() || self.virtual_dir(&rel))
    }

    /// `is_file`.
    pub(crate) fn is_file(&self, path: &str) -> R<bool> {
        let rel = self.note(path)?;
        Ok(self.entry(&rel).is_some() || self.disk(&rel).is_file())
    }

    /// The state the plan relies on for a file the event does not write but
    /// that something before it does (composer.lock, written by the update).
    pub(crate) fn assume(&mut self, path: &str, bytes: Vec<u8>) -> R<()> {
        let rel = normalize(path)?;
        if let Some(t) = self.touched.iter_mut().find(|t| t.rel == rel) {
            t.before = Some(bytes.clone());
            t.after = bytes;
            return Ok(());
        }
        self.touched.push(Touched {
            rel,
            before: Some(bytes.clone()),
            after: bytes,
            exec: false,
            assert_only: true,
        });
        Ok(())
    }

    /// `file_put_contents` (and `mkdir(dirname, 0777, true)` before it).
    pub(crate) fn write(&mut self, path: &str, bytes: Vec<u8>) -> R<()> {
        let rel = self.note(path)?;
        if let Some(t) = self.touched.iter_mut().find(|t| t.rel == rel) {
            t.after = bytes;
            t.assert_only = false;
            return Ok(());
        }
        let disk = self.disk(&rel);
        // A link would be followed by both, but where it leads is not the
        // project any more; a directory cannot be written at all.
        match std::fs::symlink_metadata(&disk) {
            Ok(m) if m.file_type().is_symlink() => {
                return Err(format!("{rel} is a symbolic link"));
            }
            Ok(m) if !m.is_file() => return Err(format!("{rel} is not a regular file")),
            _ => {}
        }
        // `mkdir` of the parent fails on a file standing in its way.
        let mut ancestor = PathBuf::new();
        let parts: Vec<&str> = rel.split('/').collect();
        for part in &parts[..parts.len() - 1] {
            ancestor.push(part);
            let a = self.root.join(&ancestor);
            match std::fs::symlink_metadata(&a) {
                Ok(m) if m.file_type().is_symlink() => {
                    return Err(format!("{} is a symbolic link", ancestor.display()))
                }
                Ok(m) if !m.is_dir() => {
                    return Err(format!("{} is not a directory", ancestor.display()))
                }
                _ => {}
            }
        }
        let before = self.read(&rel)?;
        self.touched.push(Touched {
            rel,
            before,
            after: bytes,
            exec: false,
            assert_only: false,
        });
        Ok(())
    }

    /// `file_put_contents($file, $data, FILE_APPEND)`.
    pub(crate) fn append(&mut self, path: &str, data: &[u8]) -> R<()> {
        let mut bytes = self.read(path)?.unwrap_or_default();
        bytes.extend_from_slice(data);
        self.write(path, bytes)
    }

    fn set_exec(&mut self, path: &str) -> R<()> {
        let rel = normalize(path)?;
        if let Some(t) = self.touched.iter_mut().find(|t| t.rel == rel) {
            t.exec = true;
        }
        Ok(())
    }

    /// Checks every file is still as the plan found it, then writes. A file
    /// that changed between the decision and the event is an error, never a
    /// write over something the plan did not see.
    fn apply(&self) -> anyhow::Result<()> {
        // What the plan only looked at (a copy skipped because the target
        // existed, a `.env.test` that turned PHPUnit off…) must not have
        // changed either: the decision rests on it.
        let seen = self
            .seen
            .lock()
            .map(|s| s.clone())
            .map_err(|_| anyhow::anyhow!("internal: the observed paths are poisoned"))?;
        for (rel, state) in &seen {
            if self.entry(rel).is_some() {
                continue;
            }
            if disk_state(&self.root.join(rel)) != *state {
                anyhow::bail!(
                    "{} changed between the decision to apply the Symfony recipes and their application; nothing of the recipes was written",
                    self.root.join(rel).display()
                );
            }
        }
        for t in &self.touched {
            let disk = self.root.join(&t.rel);
            let now = match std::fs::read(&disk) {
                Ok(b) => Some(b),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
                Err(e) => anyhow::bail!("cannot read {}: {e}", disk.display()),
            };
            if now != t.before {
                anyhow::bail!(
                    "{} changed between the decision to apply the Symfony recipes and their application; nothing of the recipes was written",
                    disk.display()
                );
            }
        }
        for t in self.touched.iter().filter(|t| !t.assert_only) {
            let disk = self.root.join(&t.rel);
            if let Some(parent) = disk.parent() {
                std::fs::create_dir_all(parent)
                    .map_err(|e| anyhow::anyhow!("cannot create {}: {e}", parent.display()))?;
            }
            // In place, like `file_put_contents`: an existing file keeps its
            // inode and its mode.
            if t.before.as_ref() != Some(&t.after) || t.before.is_none() {
                // composer.json and the lock: through `require`'s gate (a
                // signal's revert waits for the write, none follows it).
                let _gate = (t.rel == "composer.json" || t.rel == "composer.lock")
                    .then(vivacity_core::signals::write_gate);
                std::fs::write(&disk, &t.after)
                    .map_err(|e| anyhow::anyhow!("cannot write {}: {e}", disk.display()))?;
            }
            if t.exec {
                add_exec_bits(&disk)?;
            }
        }
        Ok(())
    }
}

#[cfg(unix)]
fn add_exec_bits(path: &Path) -> anyhow::Result<()> {
    use std::os::unix::fs::PermissionsExt as _;
    let mode = std::fs::metadata(path)
        .map_err(|e| anyhow::anyhow!("cannot stat {}: {e}", path.display()))?
        .permissions()
        .mode();
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode | 0o111))
        .map_err(|e| anyhow::anyhow!("cannot chmod {}: {e}", path.display()))
}

/// `chmod` has no executable bit to add on Windows (PHP ignores it too).
#[cfg(not(unix))]
fn add_exec_bits(_path: &Path) -> anyhow::Result<()> {
    Ok(())
}

// ---------------------------------------------------------------- symfony.lock

/// Flex's `Lock`: `symfony.lock` as read at activation, the entries the run
/// sets, and its write (`ksort`, `JsonFile::write` with the file's own
/// indentation, only when something changed).
pub(crate) struct SymfonyLock {
    path: String,
    entries: Map<String, Value>,
    indent: String,
    changed: bool,
}

impl SymfonyLock {
    pub(crate) fn load(files: &Vfs, path: &str) -> R<SymfonyLock> {
        let mut lock = SymfonyLock {
            path: path.to_owned(),
            entries: Map::new(),
            indent: "    ".to_owned(),
            changed: false,
        };
        let Some(bytes) = files.read(path)? else {
            return Ok(lock);
        };
        let text = String::from_utf8(bytes).map_err(|_| format!("{path} is not UTF-8"))?;
        lock.entries = match serde_json::from_str::<Value>(&text) {
            Ok(Value::Object(m)) => m,
            Ok(Value::Array(a)) if a.is_empty() => Map::new(),
            _ => return Err(format!("{path} is not a JSON object")),
        };
        lock.indent = vivacity_resolver::json_manipulator::detect_indenting(&text)
            .map_err(|e| format!("{path}: {e}"))?;
        Ok(lock)
    }

    /// `Lock::set`.
    pub(crate) fn set(&mut self, name: &str, data: Value) {
        if self.entries.get(name) != Some(&data) {
            self.entries.insert(name.to_owned(), data);
            self.changed = true;
        }
    }

    /// `Lock::add`: `array_merge` of the entry with `$data`.
    fn add(&mut self, name: &str, data: Map<String, Value>) {
        let mut current = match self.entries.get(name) {
            Some(Value::Object(m)) => m.clone(),
            _ => Map::new(),
        };
        for (k, v) in data {
            current.insert(k, v);
        }
        self.entries.insert(name.to_owned(), Value::Object(current));
        self.changed = true;
    }

    /// `Lock::write`, into the virtual project.
    fn write(&self, files: &mut Vfs) -> R<()> {
        if !self.changed {
            return Ok(());
        }
        if self.entries.is_empty() {
            return Err("symfony.lock would be emptied".to_owned());
        }
        let mut sorted: Vec<(&String, &Value)> = self.entries.iter().collect();
        sorted.sort_by(|a, b| a.0.as_bytes().cmp(b.0.as_bytes()));
        let mut map = Map::new();
        for (k, v) in sorted {
            map.insert(k.clone(), v.clone());
        }
        let mut text = vivacity_core::phpjson::php_json_encode_with(
            &Value::Object(map),
            vivacity_core::phpjson::FLAGS_JSONFILE,
        )
        .map_err(|e| format!("symfony.lock: {e}"))?;
        if self.indent != "    " {
            text = vivacity_resolver::config_source::reindent(&text, &self.indent);
        }
        text.push('\n');
        // `filePutContentsIfModified`.
        if files.read(&self.path)?.as_deref() != Some(text.as_bytes()) {
            files.write(&self.path, text.into_bytes())?;
        }
        Ok(())
    }
}

// ---------------------------------------------------------------- recipes

/// A recipe file entry (`Downloader::getRecipes` decodes `contents`).
#[derive(Debug, Clone)]
pub(crate) struct RecipeFile {
    pub(crate) contents: Vec<u8>,
    /// `None`: absent or not a boolean, which `copyFile(…, bool
    /// $executable)` would not take.
    pub(crate) executable: Option<bool>,
}

/// `Recipe`: what `fetchRecipes` hands to the configurators.
#[derive(Debug, Clone)]
pub(crate) struct Recipe {
    pub(crate) name: String,
    /// `getOrigin()`, unformatted.
    pub(crate) origin: String,
    pub(crate) manifest: Map<String, Value>,
    pub(crate) files: Vec<(String, RecipeFile)>,
    pub(crate) is_contrib: bool,
    pub(crate) package_type: String,
}

impl Recipe {
    /// `Recipe::getFormattedOrigin`, with the console tags dropped.
    pub(crate) fn formatted_origin(&self) -> String {
        // `/^([^:]++):([^@]++)@(.+)$/`: a name without `:`, a version up to
        // the first `@` (it may hold a `:`), the rest without a newline —
        // `$` also matches before a final one.
        let o = &self.origin;
        let subject = o.strip_suffix('\n').unwrap_or(o);
        let Some(colon) = subject.find(':') else {
            return o.clone();
        };
        let rest = &subject[colon + 1..];
        let Some(at) = rest.find('@') else {
            return o.clone();
        };
        let (name, version, from) = (&subject[..colon], &rest[..at], &rest[at + 1..]);
        if name.is_empty() || version.is_empty() || from.is_empty() || from.contains('\n') {
            return o.clone();
        }
        format!("{name} (>={version}): From {from}")
    }
}

/// What the plan prints and writes at `POST_UPDATE_CMD`, after the thanks
/// reminder and the blank line.
#[derive(Debug)]
pub(crate) struct FlexPlan {
    pub(crate) lines: Vec<(vivacity_core::io::Verbosity, String)>,
    files: Vfs,
}

impl FlexPlan {
    pub(crate) fn print_and_apply(&self) -> anyhow::Result<()> {
        for (level, line) in &self.lines {
            vivacity_core::say_at!(*level, "{line}");
        }
        self.files.apply()
    }
}

/// The run's shared state: what `Flex::install` and its configurators read
/// and write.
pub(crate) struct Ctx<'a> {
    pub(crate) files: Vfs,
    pub(crate) options: Options,
    pub(crate) lock: SymfonyLock,
    lines: Vec<(vivacity_core::io::Verbosity, String)>,
    /// `Options::$writtenFiles`.
    written_files: std::collections::HashSet<String>,
    /// `getLockedRepository(true)` / the local repository after the install
    /// (`isPackageInstalled`): name and normalized version of each package,
    /// aliases included.
    pub(crate) installed: &'a [(String, String)],
    /// The composer.lock bytes the update writes (`None` with
    /// `config.lock: false`).
    pub(crate) composer_lock: Option<&'a str>,
}

impl<'a> Ctx<'a> {
    pub(crate) fn new(
        project: &Path,
        manifest: &Value,
        symfony_lock_path: &str,
        installed: &'a [(String, String)],
        composer_lock: Option<&'a str>,
    ) -> R<Ctx<'a>> {
        let mut files = Vfs::new(project);
        if let Some(text) = composer_lock {
            files.assume("composer.lock", text.as_bytes().to_vec())?;
        }
        let lock = SymfonyLock::load(&files, symfony_lock_path)?;
        Ok(Ctx {
            files,
            options: Options::new(manifest)?,
            lock,
            lines: Vec::new(),
            written_files: Default::default(),
            installed,
            composer_lock,
        })
    }

    fn say(&mut self, line: String) {
        self.lines
            .push((vivacity_core::io::Verbosity::Normal, line));
    }

    /// `AbstractConfigurator::write`: four spaces, VERBOSE unless asked.
    fn write_msg(&mut self, level: vivacity_core::io::Verbosity, messages: &[String]) {
        for m in messages {
            self.lines.push((level, format!("    {}", strip_tags(m))));
        }
    }

    fn verbose(&mut self, message: &str) {
        self.write_msg(vivacity_core::io::Verbosity::Verbose, &[message.to_owned()]);
    }

    /// `ArrayRepository::findPackage($name, $constraint)` over the given
    /// packages: by name only (no provider), the constraint against each
    /// version.
    fn find_package(&self, name: &str, constraint: &str) -> R<bool> {
        let parsed = vivacity_resolver::constraint::parse_constraints(constraint)
            .map_err(|e| format!("cannot parse the constraint {constraint:?}: {e}"))?;
        let name = name.to_lowercase();
        Ok(self
            .installed
            .iter()
            .any(|(n, v)| *n == name && parsed.constraint.matches_version(v)))
    }
}

/// `Flex::install` from `Symfony operations` on, for recipes already
/// fetched and ordered (`fetchRecipes`' return), their `symfony.lock`
/// entries already set by the caller. Then `finish` minus the
/// synchronisation (decided by the caller on the resulting files).
pub(crate) fn run_recipes(
    ctx: &mut Ctx<'_>,
    recipes: &[Recipe],
    manifest: &Value,
    manifest_text: &str,
) -> R<()> {
    let session = vivacity_core::random_bytes(16)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect::<String>();
    ctx.say(format!(
        "Symfony operations: {} recipe{} ({session})",
        recipes.len(),
        if recipes.len() > 1 { "s" } else { "" }
    ));
    let install_contribs = match std::env::var("SYMFONY_ALLOW_CONTRIB") {
        Ok(v) => php_filter_bool(&Value::String(v)),
        Err(_) => match manifest
            .get("extra")
            .and_then(|e| e.get("symfony"))
            .and_then(|s| s.get("allow-contrib"))
        {
            None | Some(Value::Null) => false,
            Some(Value::Bool(b)) => *b,
            Some(other) => php_filter_bool(other),
        },
    };
    let original_composer_json = manifest_text.as_bytes().to_vec();
    ctx.files
        .assume("composer.json", original_composer_json.clone())?;
    for recipe in recipes {
        if recipe.is_contrib && !install_contribs {
            return Err(format!(
                "asks whether to run the contrib recipe of {} (allow-contrib is off)",
                recipe.name
            ));
        }
        ctx.say(format!("  - Configuring {}", recipe.formatted_origin()));
        configure(ctx, recipe)?;
    }
    for recipe in recipes {
        if let Some(config) = recipe.manifest.get("add-lines") {
            add_lines(ctx, recipe, config)?;
        }
    }
    // `finish`: `Lock::write`, then `updateComposerLock` when composer.json
    // changed (its md5 compared, i.e. its bytes).
    ctx.lock.write(&mut ctx.files)?;
    let now = ctx.files.read("composer.json")?.unwrap_or_default();
    if now != original_composer_json {
        update_composer_lock(ctx, &now)?;
    }
    Ok(())
}

/// Ends the plan.
pub(crate) fn into_plan(ctx: Ctx<'_>) -> FlexPlan {
    FlexPlan {
        lines: ctx.lines,
        files: ctx.files,
    }
}

/// The configurator keys `Configurator::install` walks, in its order, and
/// what vivacity does with each.
fn configure(ctx: &mut Ctx<'_>, recipe: &Recipe) -> R<()> {
    for key in [
        "bundles",
        "copy-from-recipe",
        "copy-from-package",
        "env",
        "dotenv",
        "container",
        "makefile",
        "composer-scripts",
        "composer-commands",
        "gitignore",
        "dockerfile",
        "docker-compose",
    ] {
        // `isset`: a null value is not configured.
        let Some(config) = recipe.manifest.get(key).filter(|v| !v.is_null()) else {
            continue;
        };
        match key {
            "bundles" => bundles(ctx, recipe, config)?,
            "copy-from-recipe" => copy_from_recipe(ctx, recipe, config)?,
            "env" => env(ctx, recipe, config, "")?,
            "dotenv" => {
                let Value::Object(by_suffix) = config else {
                    return Err(format!("{}'s dotenv is not an object", recipe.name));
                };
                for (suffix, vars) in by_suffix {
                    if php_int_key(suffix) {
                        return Err(format!("{}'s dotenv has an integer key", recipe.name));
                    }
                    env(ctx, recipe, vars, suffix)?;
                }
            }
            "composer-scripts" => composer_scripts(ctx, recipe, config)?,
            "gitignore" => gitignore(ctx, recipe, config)?,
            "dockerfile" | "docker-compose" => {
                if docker_enabled(ctx, recipe)? {
                    return Err(format!(
                        "would run the {key} configurator of {} (not emulated)",
                        recipe.name
                    ));
                }
            }
            other => {
                return Err(format!(
                    "would run the {other} configurator of {} (not emulated)",
                    recipe.name
                ))
            }
        }
    }
    Ok(())
}

/// `DockerComposeConfigurator::shouldConfigureDockerRecipe`: only a set
/// `extra.symfony.docker` is reproduced (false: both docker configurators
/// return at once). Unset, Flex asks — or, without a terminal, configures.
fn docker_enabled(ctx: &Ctx<'_>, recipe: &Recipe) -> R<bool> {
    match ctx
        .options
        .get("symfony")
        .and_then(|s| s.get("docker"))
        .filter(|v| !v.is_null())
    {
        Some(v) => Ok(php_filter_bool(v)),
        None => Err(format!(
            "asks whether to apply the Docker configuration of {} (extra.symfony.docker is not set)",
            recipe.name
        )),
    }
}

/// `AbstractConfigurator::markData`.
fn mark_data(name: &str, data: &str) -> String {
    format!(
        "\n###> {name} ###\n{}\n###< {name} ###\n",
        data.trim_end_matches(['\r', '\n'])
    )
}

/// `AbstractConfigurator::isFileMarked`.
fn is_file_marked(ctx: &Ctx<'_>, name: &str, file: &str) -> R<bool> {
    Ok(ctx.files.read(file)?.is_some_and(|c| {
        let needle = format!("###> {name} ###");
        contains(&c, needle.as_bytes())
    }))
}

fn contains(hay: &[u8], needle: &[u8]) -> bool {
    needle.is_empty() || hay.windows(needle.len()).any(|w| w == needle)
}

/// A manifest value used as a PHP string.
fn string_of<'v>(v: &'v Value, what: &str, recipe: &Recipe) -> R<&'v str> {
    v.as_str()
        .ok_or_else(|| format!("{}'s {what} is {v}, not a string", recipe.name))
}

// ---------------------------------------------------------------- bundles

/// `$registered`: each bundle class with its environments, in order.
pub(crate) type Bundles = Vec<(String, Vec<(String, bool)>)>;

/// `config/bundles.php` exactly as `BundlesConfigurator::buildContents`
/// writes it: the entries it holds, or `None` when the file has any other
/// shape — PHP would `require` it, vivacity only recognises it.
pub(crate) fn parse_bundles(bytes: &[u8]) -> Option<Bundles> {
    let text = std::str::from_utf8(bytes).ok()?;
    let body = text
        .strip_prefix("<?php\n\nreturn [\n")?
        .strip_suffix("];\n")?;
    let mut out: Bundles = Vec::new();
    for line in body.split_terminator('\n') {
        let line = line.strip_prefix("    ")?.strip_suffix("],")?;
        let (class, envs) = line.split_once("::class => [")?;
        if class.is_empty()
            || class.starts_with('\\')
            || !class
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '\\')
            || out.iter().any(|(c, _)| c == class)
        {
            return None;
        }
        let mut list = Vec::new();
        for env in envs.split(", ") {
            let (key, value) = env.split_once("' => ")?;
            let key = key.strip_prefix('\'')?;
            if key.is_empty()
                || !key
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
                || list.iter().any(|(k, _): &(String, bool)| k == key)
            {
                return None;
            }
            let value = match value {
                "true" => true,
                "false" => false,
                _ => return None,
            };
            list.push((key.to_owned(), value));
        }
        out.push((class.to_owned(), list));
    }
    // The proof: the same bytes come back.
    (render_bundles(&out).as_bytes() == bytes).then_some(out)
}

/// `BundlesConfigurator::buildContents`.
pub(crate) fn render_bundles(bundles: &Bundles) -> String {
    let mut out = String::from("<?php\n\nreturn [\n");
    for (class, envs) in bundles {
        let pairs: Vec<String> = envs.iter().map(|(e, v)| format!("'{e}' => {v}")).collect();
        out.push_str(&format!("    {class}::class => [{}],\n", pairs.join(", ")));
    }
    out.push_str("];\n");
    out
}

/// `BundlesConfigurator::configure`.
fn bundles(ctx: &mut Ctx<'_>, recipe: &Recipe, config: &Value) -> R<()> {
    ctx.verbose("Enabling the package as a Symfony bundle");
    let Value::Object(classes) = config else {
        return Err(format!("{}'s bundles is not an object", recipe.name));
    };
    let file = format!(
        "./{}",
        ctx.options.expand("%CONFIG_DIR%/bundles.php", &ctx.files)?
    );
    let mut registered = match ctx.files.read(&file)? {
        None => Vec::new(),
        Some(bytes) => parse_bundles(&bytes).ok_or_else(|| {
            format!("{file} is not in the form Flex writes (it would be evaluated by PHP)")
        })?,
    };
    const FWB: &str = "Symfony\\Bundle\\FrameworkBundle\\FrameworkBundle";
    let envs_of = |class: &str, v: &Value| -> R<Vec<String>> {
        let Value::Array(a) = v else {
            return Err(format!("{}'s envs for {class} are not a list", recipe.name));
        };
        a.iter()
            .map(|e| {
                e.as_str()
                    .filter(|s| {
                        !s.is_empty()
                            && s.chars()
                                .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
                    })
                    .map(str::to_owned)
                    .ok_or_else(|| format!("{}'s env {e} for {class} is not plain", recipe.name))
            })
            .collect()
    };
    let set_env = |registered: &mut Bundles, class: &str, env: &str| {
        let slot = match registered.iter().position(|(c, _)| c == class) {
            Some(i) => i,
            None => {
                registered.push((class.to_owned(), Vec::new()));
                registered.len() - 1
            }
        };
        let envs = &mut registered[slot].1;
        match envs.iter_mut().find(|(e, _)| e == env) {
            Some(e) => e.1 = true,
            None => envs.push((env.to_owned(), true)),
        }
    };
    // `prepareBundles` adds an `ltrim`med key next to a `\`-prefixed one.
    if classes.keys().any(|c| c.starts_with('\\')) {
        return Err(format!(
            "{} registers a bundle class with a leading backslash",
            recipe.name
        ));
    }
    for (class, envs) in classes {
        if !class
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '\\')
            || class.is_empty()
        {
            return Err(format!(
                "{} registers the odd bundle class {class:?}",
                recipe.name
            ));
        }
        let envs = envs_of(class, envs)?;
        if class == FWB {
            for env in &envs {
                set_env(&mut registered, class, env);
            }
        }
    }
    for (class, envs) in classes {
        if class == FWB {
            continue;
        }
        let envs = envs_of(class, envs)?;
        // "do not override existing configured envs for a bundle"
        if registered.iter().any(|(c, _)| c == class) {
            continue;
        }
        for env in &envs {
            set_env(&mut registered, class, env);
        }
    }
    ctx.files
        .write(&file, render_bundles(&registered).into_bytes())
}

// ---------------------------------------------------------------- copy-from-recipe

/// `Path::concatenate`.
fn concatenate(parts: &[&str]) -> String {
    let mut out = parts.first().map(|s| (*s).to_owned()).unwrap_or_default();
    for next in parts.iter().skip(1) {
        out = format!(
            "{}/{}",
            out.trim_end_matches('/'),
            next.trim_start_matches('/')
        );
    }
    out
}

/// `CopyFromRecipeConfigurator::getLocalFilePath($basePath = '.', …)`:
/// `str_replace('.'.DIRECTORY_SEPARATOR, '', $destination)` — every
/// occurrence, as PHP does.
fn local_file_path(destination: &str) -> String {
    let separator = if cfg!(windows) { ".\\" } else { "./" };
    destination.replace(separator, "")
}

/// `CopyFromRecipeConfigurator::configure`.
fn copy_from_recipe(ctx: &mut Ctx<'_>, recipe: &Recipe, config: &Value) -> R<()> {
    ctx.verbose("Copying files from recipe");
    let Value::Object(map) = config else {
        return Err(format!(
            "{}'s copy-from-recipe is not an object",
            recipe.name
        ));
    };
    let mut copied: Vec<Value> = Vec::new();
    for (source, target) in map {
        let target = ctx.options.expand(
            string_of(target, "copy-from-recipe target", recipe)?,
            &ctx.files,
        )?;
        let target_path = concatenate(&[".", &target]);
        if source.ends_with('/') {
            // `copyDir`: the recipe's files under `$source`, in its order.
            let files: Vec<(String, RecipeFile)> = recipe
                .files
                .iter()
                .filter(|(f, _)| f.starts_with(source.as_str()))
                .cloned()
                .collect();
            for (file, data) in files {
                let to = concatenate(&[&target_path, &file[source.len()..]]);
                copied.push(Value::String(copy_file(ctx, &to, &data)?));
            }
        } else {
            let Some((_, data)) = recipe.files.iter().find(|(f, _)| f == source) else {
                return Err(format!(
                    "{}'s recipe has no file {source:?} to copy",
                    recipe.name
                ));
            };
            let data = data.clone();
            copied.push(Value::String(copy_file(ctx, &target_path, &data)?));
        }
    }
    let mut files = Map::new();
    files.insert("files".to_owned(), Value::Array(copied));
    ctx.lock.add(&recipe.name, files);
    Ok(())
}

/// `CopyFromRecipeConfigurator::copyFile`, with `Options::shouldWriteFile`
/// for an update (no `force`): an existing file is left as it is.
fn copy_file(ctx: &mut Ctx<'_>, to: &str, data: &RecipeFile) -> R<String> {
    // `copyFile(string $to, string $contents, bool $executable, …)`: the
    // arguments are checked before `shouldWriteFile` is asked anything.
    let Some(executable) = data.executable else {
        return Err(format!("the recipe file {to} has no boolean `executable`"));
    };
    let copied = local_file_path(to);
    if !ctx.written_files.insert(to.to_owned()) || ctx.files.exists(to)? {
        return Ok(copied);
    }
    let contents = String::from_utf8(data.contents.clone())
        .map_err(|_| format!("the recipe file {to} is not UTF-8"))?;
    let expanded = ctx.options.expand(&contents, &ctx.files)?;
    ctx.files.write(to, expanded.into_bytes())?;
    if executable {
        ctx.files.set_exec(to)?;
    }
    // `Path::relativize` with the working directory `.`: `str_replace('.',
    // '.', …)`, so the path as built, and a slash after a directory.
    ctx.verbose(&format!("  Created <fg=green>\"{to}\"</>"));
    Ok(copied)
}

// ---------------------------------------------------------------- env / dotenv

/// `EnvConfigurator::evaluateValue`: `%generate(secret)%` is random.
fn evaluate_value(recipe: &Recipe, value: &str) -> R<String> {
    // `$` of `~^%generate\(secret,\s*([0-9]+)\)%$~` also matches before a
    // final newline.
    let anchored = value.strip_suffix('\n').unwrap_or(value);
    let generated = value == "%generate(secret)%"
        || anchored
            .strip_prefix("%generate(secret,")
            .and_then(|r| r.strip_suffix(")%"))
            .is_some_and(|n| {
                let n = n.trim_start_matches([' ', '\t', '\n', '\r', '\x0b', '\x0c']);
                !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit())
            });
    if generated {
        return Err(format!(
            "would write a random secret for {} (%generate(secret)%)",
            recipe.name
        ));
    }
    Ok(value.to_owned())
}

/// The `$vars` of an env configurator, keys checked: PHP would turn an
/// integer key into an int, and `$key[0]` of an empty one warns.
fn env_vars<'v>(recipe: &Recipe, vars: &'v Value) -> R<Vec<(&'v str, &'v str)>> {
    let Value::Object(map) = vars else {
        if matches!(vars, Value::Array(a) if a.is_empty()) {
            return Ok(Vec::new());
        }
        return Err(format!("{}'s env is not an object", recipe.name));
    };
    map.iter()
        .map(|(k, v)| {
            if k.is_empty() || php_int_key(k) {
                return Err(format!("{}'s env has the key {k:?}", recipe.name));
            }
            Ok((k.as_str(), string_of(v, "env value", recipe)?))
        })
        .collect()
}

/// `EnvConfigurator::configure` (and, with a suffix, the
/// `DotenvConfigurator` that builds it).
fn env(ctx: &mut Ctx<'_>, recipe: &Recipe, vars: &Value, suffix: &str) -> R<()> {
    ctx.verbose(&format!(
        "Adding environment variable defaults{}",
        if suffix.is_empty() {
            String::new()
        } else {
            format!(" ({suffix})")
        }
    ));
    let vars = env_vars(recipe, vars)?;
    let dotenv = ctx.options.dotenv_path()?;
    let targets: Vec<String> = if suffix.is_empty() {
        vec![format!("{dotenv}.dist"), dotenv.clone()]
    } else {
        vec![format!("{dotenv}.{suffix}")]
    };
    for file in targets {
        let path = format!("./{file}");
        if !ctx.files.is_file(&path)? || is_file_marked(ctx, &recipe.name, &path)? {
            continue;
        }
        let mut data = String::new();
        for (key, value) in &vars {
            let value = evaluate_value(recipe, value)?;
            if key.starts_with('#') && php_is_numeric(&key[1..]) {
                if value.is_empty() {
                    data.push_str("#\n");
                } else {
                    data.push_str(&format!("# {value}\n"));
                }
                continue;
            }
            let mut value = ctx.options.expand(&value, &ctx.files)?;
            if value.contains([' ', '\t', '\n', '&', '!', '"']) {
                value = format!(
                    "\"{}\"",
                    value
                        .replace('\\', "\\\\")
                        .replace('"', "\\\"")
                        .replace('\t', "\\t")
                        .replace('\n', "\\n")
                );
            }
            data.push_str(&format!("{key}={value}\n"));
        }
        // `updateData` cannot match (the start mark is the one
        // `isFileMarked` just looked for): appended.
        let data = mark_data(&recipe.name, &data);
        ctx.files.append(&path, data.as_bytes())?;
    }
    if suffix.is_empty() && !ctx.files.exists(&format!("./{dotenv}.test"))? {
        php_unit(ctx, recipe, &vars)?;
    }
    Ok(())
}

/// `DOMDocument::saveXML` of an attribute value: libxml's escaping, with a
/// character reference for anything outside ASCII.
fn xml_attribute(value: &str) -> R<String> {
    let mut out = String::new();
    for c in value.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\t' => out.push_str("&#9;"),
            '\n' => out.push_str("&#10;"),
            '\r' => out.push_str("&#13;"),
            c if (c as u32) < 0x20 || c == '\u{7f}' => {
                return Err(format!("the value {value:?} holds a control character"))
            }
            c if c.is_ascii() => out.push(c),
            c => out.push_str(&format!("&#x{:X};", c as u32)),
        }
    }
    Ok(out)
}

/// `EnvConfigurator::configurePhpUnit` (no update): the env lines added
/// before `</php>` in each PHPUnit file that is not already marked.
fn php_unit(ctx: &mut Ctx<'_>, recipe: &Recipe, vars: &[(&str, &str)]) -> R<()> {
    for file in ["phpunit.xml.dist", "phpunit.dist.xml", "phpunit.xml"] {
        let path = format!("./{file}");
        if !ctx.files.is_file(&path)? {
            continue;
        }
        let contents = ctx.files.read(&path)?.unwrap_or_default();
        if contains(&contents, format!("###+ {} ###", recipe.name).as_bytes()) {
            continue;
        }
        let mut data = String::new();
        for (key, value) in vars {
            let value = evaluate_value(recipe, value)?;
            if let Some(rest) = key.strip_prefix('#') {
                if php_is_numeric(rest) {
                    // A comment node, saved as is.
                    if value.contains("--") || value.contains(['\r', '\0']) {
                        return Err(format!("{}'s comment {value:?} is not plain", recipe.name));
                    }
                    data.push_str(&format!("        <!-- {value} -->\n"));
                } else {
                    let value = ctx.options.expand(&value, &ctx.files)?;
                    data.push_str(&format!(
                        "        <!-- env name=\"{}\" value=\"{}\" -->\n",
                        xml_attribute(rest)?,
                        xml_attribute(&value)?
                    ));
                }
            } else {
                let value = ctx.options.expand(&value, &ctx.files)?;
                data.push_str(&format!(
                    "        <env name=\"{}\" value=\"{}\"/>\n",
                    xml_attribute(key)?,
                    xml_attribute(&value)?
                ));
            }
        }
        let name = &recipe.name;
        let data = format!(
            "\n        <!-- ###+ {name} ### -->\n{}\n        <!-- ###- {name} ### -->\n",
            data.trim_end_matches(['\r', '\n'])
        );
        // `preg_replace('{^(\s+</php>)}m', $data.'$1', …)`: in the
        // replacement, `\` and `$` are not literal.
        if data.contains(['\\', '$']) {
            return Err(format!(
                "{}'s PHPUnit env lines hold a character preg_replace would interpret",
                recipe.name
            ));
        }
        ctx.files
            .write(&path, insert_before_php_close(&contents, data.as_bytes()))?;
    }
    Ok(())
}

/// `preg_replace('{^(\s+</php>)}m', $data.'$1', $contents)`: `$data` before
/// every run of whitespace that starts a line and ends at `</php>`.
pub(crate) fn insert_before_php_close(contents: &[u8], data: &[u8]) -> Vec<u8> {
    let space = |c: u8| matches!(c, b' ' | b'\t' | b'\n' | b'\r' | 0x0b | 0x0c);
    let mut out = Vec::with_capacity(contents.len() + data.len());
    let mut copied = 0;
    let mut p = 0;
    while p < contents.len() {
        if p == 0 || contents[p - 1] == b'\n' {
            let mut e = p;
            while e < contents.len() && space(contents[e]) {
                e += 1;
            }
            if e > p && contents[e..].starts_with(b"</php>") {
                out.extend_from_slice(&contents[copied..p]);
                out.extend_from_slice(data);
                copied = p;
                p = e + b"</php>".len();
                continue;
            }
        }
        p += 1;
    }
    out.extend_from_slice(&contents[copied..]);
    out
}

// ---------------------------------------------------------------- gitignore

/// `GitignoreConfigurator::configure`.
fn gitignore(ctx: &mut Ctx<'_>, recipe: &Recipe, config: &Value) -> R<()> {
    ctx.verbose("Adding entries to .gitignore");
    let file = "./.gitignore";
    if is_file_marked(ctx, &recipe.name, file)? {
        return Ok(());
    }
    let Value::Array(values) = config else {
        return Err(format!("{}'s gitignore is not a list", recipe.name));
    };
    let mut data = String::new();
    for v in values {
        let v = ctx
            .options
            .expand(string_of(v, "gitignore entry", recipe)?, &ctx.files)?;
        data.push_str(&format!("{v}\n"));
    }
    let data = format!(
        "\n{}",
        mark_data(&recipe.name, &data).trim_start_matches(['\r', '\n'])
    );
    ctx.files.append(file, data.as_bytes())
}

// ---------------------------------------------------------------- composer-scripts

/// `ComposerScriptsConfigurator::configure`: `scripts.auto-scripts` merged
/// with the recipe's (`array_merge`), written back through
/// `JsonManipulator::addSubNode` — unconditionally.
fn composer_scripts(ctx: &mut Ctx<'_>, recipe: &Recipe, config: &Value) -> R<()> {
    let Value::Object(scripts) = config else {
        return Err(format!(
            "{}'s composer-scripts is not an object",
            recipe.name
        ));
    };
    let bytes = ctx.files.read("composer.json")?.unwrap_or_default();
    let text = String::from_utf8(bytes).map_err(|_| "composer.json is not UTF-8".to_owned())?;
    let json: Value = serde_json::from_str(&text).map_err(|e| format!("composer.json: {e}"))?;
    let mut merged = match json.get("scripts").and_then(|s| s.get("auto-scripts")) {
        None | Some(Value::Null) => Map::new(),
        Some(Value::Object(m)) => m.clone(),
        Some(Value::Array(a)) if a.is_empty() => Map::new(),
        Some(_) => return Err("scripts.auto-scripts is not an object".to_owned()),
    };
    if json
        .get("scripts")
        .is_some_and(|s| !s.is_object() && !s.is_null())
    {
        return Err("scripts is not an object".to_owned());
    }
    for (k, v) in scripts {
        merged.insert(k.clone(), v.clone());
    }
    if merged.keys().any(|k| php_int_key(k)) {
        return Err("scripts.auto-scripts has an integer key".to_owned());
    }
    let mut manipulator = vivacity_resolver::json_manipulator::JsonManipulator::new(&text)
        .map_err(|e| format!("composer.json: {e}"))?;
    manipulator
        .add_sub_node("scripts", "auto-scripts", &Value::Object(merged), true)
        .map_err(|e| format!("composer.json: {e}"))?;
    ctx.files
        .write("composer.json", manipulator.contents().into_bytes())
}

/// `Flex::updateComposerLock`: the lock as `Locker::getLockData` reads it
/// (decoded to PHP arrays), its `content-hash` recomputed from the new
/// composer.json, written back by `JsonFile::write`.
fn update_composer_lock(ctx: &mut Ctx<'_>, composer_json: &[u8]) -> R<()> {
    let Some(lock_text) = ctx.composer_lock else {
        return Err(
            "would update composer.lock's content-hash, but no composer.lock is written (config.lock: false)"
                .to_owned(),
        );
    };
    let json_text =
        std::str::from_utf8(composer_json).map_err(|_| "composer.json is not UTF-8".to_owned())?;
    let mut data: Value =
        serde_json::from_str(lock_text).map_err(|e| format!("composer.lock: {e}"))?;
    let hash = vivacity_core::content_hash::content_hash(json_text)
        .map_err(|e| format!("composer.json: {e}"))?;
    match data.as_object_mut() {
        Some(m) => {
            m.insert("content-hash".to_owned(), Value::String(hash));
        }
        None => return Err("composer.lock is not an object".to_owned()),
    }
    let mut text =
        vivacity_core::phpjson::php_json_encode_with(&data, vivacity_core::phpjson::FLAGS_JSONFILE)
            .map_err(|e| format!("composer.lock: {e}"))?;
    let indent = vivacity_resolver::json_manipulator::detect_indenting(lock_text)
        .map_err(|e| format!("composer.lock: {e}"))?;
    if indent != "    " {
        text = vivacity_resolver::config_source::reindent(&text, &indent);
    }
    text.push('\n');
    if text != lock_text {
        ctx.files.write("composer.lock", text.into_bytes())?;
    }
    Ok(())
}

// ---------------------------------------------------------------- add-lines

/// `AddLinesConfigurator::configure` (`postInstall`, after every recipe's
/// `install`): its file cache is per recipe, its writes go to disk at the
/// end of each recipe.
fn add_lines(ctx: &mut Ctx<'_>, recipe: &Recipe, config: &Value) -> R<()> {
    use vivacity_core::io::Verbosity;
    let Value::Array(patches) = config else {
        return Err(format!("{}'s add-lines is not a list", recipe.name));
    };
    // `$this->fileContents`, in insertion order (`readFile` inserts too).
    let mut contents: Vec<(String, Vec<u8>)> = Vec::new();
    let isset = |p: &Value, k: &str| p.get(k).is_some_and(|v| !v.is_null());
    for patch in patches {
        if !patch.is_object() {
            return Err(format!("{}'s add-lines holds a non-object", recipe.name));
        }
        if !isset(patch, "file") {
            ctx.verbose(&format!(
                "The \"file\" key is required for the \"add-lines\" configurator for recipe \"{}\". Skipping",
                recipe.name
            ));
            continue;
        }
        if isset(patch, "requires") && !is_package_installed(ctx, recipe, &patch["requires"])? {
            continue;
        }
        if !isset(patch, "content") {
            ctx.verbose(&format!(
                "The \"content\" key is required for the \"add-lines\" configurator for recipe \"{}\". Skipping",
                recipe.name
            ));
            continue;
        }
        let content = string_of(&patch["content"], "add-lines content", recipe)?.to_owned();
        let patch_file = string_of(&patch["file"], "add-lines file", recipe)?.to_owned();
        let file = concatenate(&[".", &ctx.options.expand(&patch_file, &ctx.files)?]);
        let warn = patch
            .get("warn_if_missing")
            .is_some_and(crate::manifest::php_truthy);
        let level = if warn {
            Verbosity::Normal
        } else {
            Verbosity::Verbose
        };
        if !ctx.files.is_file(&file)? {
            missing_lines_message(
                ctx,
                level,
                &format!(
                    "Could not add lines to file <info>{patch_file}</info> as it does not exist. Missing lines:"
                ),
                &content,
            )?;
            continue;
        }
        if !isset(patch, "position") {
            ctx.verbose(&format!(
                "The \"position\" key is required for the \"add-lines\" configurator for recipe \"{}\". Skipping",
                recipe.name
            ));
            continue;
        }
        let position = patch["position"].as_str().unwrap_or_default().to_owned();
        if !matches!(position.as_str(), "top" | "bottom" | "after_target") {
            ctx.verbose(&format!(
                "The \"position\" key must be one of \"top\", \"bottom\", \"after_target\" for the \"add-lines\" configurator for recipe \"{}\". Skipping",
                recipe.name
            ));
            continue;
        }
        if position == "after_target" && !isset(patch, "target") {
            ctx.verbose(&format!(
                "The \"target\" key is required when \"position\" is \"after_target\" for the \"add-lines\" configurator for recipe \"{}\". Skipping",
                recipe.name
            ));
            continue;
        }
        let target = match patch.get("target").filter(|v| !v.is_null()) {
            Some(t) => Some(string_of(t, "add-lines target", recipe)?.to_owned()),
            None => None,
        };
        // `readFile`: the cache, else the file (inserted in the cache).
        let current = match contents.iter().find(|(f, _)| *f == file) {
            Some((_, c)) => c.clone(),
            None => {
                let c = ctx.files.read(&file)?.unwrap_or_default();
                contents.push((file.clone(), c.clone()));
                c
            }
        };
        let patched = patched_contents(
            ctx,
            &file,
            &current,
            &content,
            &position,
            target.as_deref(),
            level,
        )?;
        match contents.iter_mut().find(|(f, _)| *f == file) {
            Some(slot) => slot.1 = patched,
            None => contents.push((file.clone(), patched)),
        }
    }
    for (file, bytes) in contents {
        // `relativize`: the root-dir prefix (`.`) and its separator off.
        let shown = file
            .strip_prefix('.')
            .map(|r| r.get(1..).unwrap_or_default())
            .unwrap_or(&file)
            .trim_start_matches(['/', '\\'])
            .to_owned();
        ctx.verbose(&format!("[add-lines] Patching file \"{shown}\""));
        ctx.files.write(&file, bytes)?;
    }
    Ok(())
}

/// The block `AddLinesConfigurator` writes when it cannot place the lines.
/// At the NORMAL level (`warn_if_missing`) it reaches the user, so its text
/// must survive the console formatter untouched: no other tag than the
/// ones it carries itself.
fn missing_lines_message(
    ctx: &mut Ctx<'_>,
    level: vivacity_core::io::Verbosity,
    head: &str,
    content: &str,
) -> R<()> {
    if level == vivacity_core::io::Verbosity::Normal
        && (strip_tags(head).contains(['<', '\\']) || content.contains(['<', '\\']))
    {
        return Err("would print lines the console formatter could alter".to_owned());
    }
    ctx.write_msg(
        level,
        &[
            head.to_owned(),
            "<comment>\"\"\"</comment>".to_owned(),
            content.to_owned(),
            "<comment>\"\"\"</comment>".to_owned(),
            String::new(),
        ],
    );
    Ok(())
}

/// `AddLinesConfigurator::getPatchedContents`.
#[allow(clippy::too_many_arguments)]
fn patched_contents(
    ctx: &mut Ctx<'_>,
    file: &str,
    current: &[u8],
    value: &str,
    position: &str,
    target: Option<&str>,
    level: vivacity_core::io::Verbosity,
) -> R<Vec<u8>> {
    if contains(current, value.as_bytes()) {
        return Ok(current.to_vec());
    }
    let mut out = current.to_vec();
    match position {
        "bottom" => {
            out.push(b'\n');
            out.extend_from_slice(value.as_bytes());
        }
        "top" => {
            let mut o = value.as_bytes().to_vec();
            o.push(b'\n');
            o.extend_from_slice(current);
            out = o;
        }
        _ => {
            let target = target.unwrap_or_default();
            let lines: Vec<&[u8]> = current.split(|b| *b == b'\n').collect();
            match lines.iter().position(|l| contains(l, target.as_bytes())) {
                Some(i) => {
                    let mut o: Vec<u8> = Vec::with_capacity(current.len() + value.len() + 1);
                    for (k, l) in lines.iter().enumerate() {
                        if k > 0 {
                            o.push(b'\n');
                        }
                        o.extend_from_slice(l);
                        if k == i {
                            o.push(b'\n');
                            o.extend_from_slice(value.as_bytes());
                        }
                    }
                    out = o;
                }
                None => {
                    missing_lines_message(
                        ctx,
                        level,
                        &format!(
                            "Could not add lines after \"{target}\" as no such string was found in \"{file}\". Missing lines:"
                        ),
                        value,
                    )?;
                }
            }
        }
    }
    Ok(out)
}

/// `AddLinesConfigurator::isPackageInstalled`: `name[:constraint]`, each
/// found in the local repository.
fn is_package_installed(ctx: &Ctx<'_>, recipe: &Recipe, requires: &Value) -> R<bool> {
    let list: Vec<&str> = match requires {
        Value::String(s) => vec![s.as_str()],
        Value::Array(a) => a
            .iter()
            .map(|v| string_of(v, "add-lines requires", recipe))
            .collect::<R<_>>()?,
        other => return Err(format!("{}'s add-lines requires is {other}", recipe.name)),
    };
    for entry in list {
        let (name, constraint) = entry.split_once(':').unwrap_or((entry, "*"));
        if !ctx.find_package(name, constraint)? {
            return Ok(false);
        }
    }
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn is_numeric_follows_php() {
        // Measured with PHP 8.5's is_numeric.
        for (s, want) in [
            ("1", true),
            (" 1", true),
            ("1 ", true),
            ("1e3", true),
            ("0x1A", false),
            (".5", true),
            ("5.", true),
            ("-1", true),
            ("+1", true),
            ("", false),
            ("abc", false),
            ("1_000", false),
            (" ", false),
            ("1.2.3", false),
        ] {
            assert_eq!(php_is_numeric(s), want, "{s:?}");
        }
    }

    #[test]
    fn filter_bool_follows_php() {
        use serde_json::json;
        for (v, want) in [
            (json!(true), true),
            (json!(false), false),
            (json!(0), false),
            (json!(1), true),
            (json!(2), false),
            (json!("1"), true),
            (json!("0"), false),
            (json!("true"), true),
            (json!("yes"), true),
            (json!("on"), true),
            (json!("off"), false),
            (json!(""), false),
            (json!(" true "), true),
            (json!(" yes\u{0}"), false),
            (json!("TRUE"), true),
            (json!("x"), false),
            (json!([]), false),
        ] {
            assert_eq!(php_filter_bool(&v), want, "{v}");
        }
    }

    fn options(extra: Value) -> Options {
        Options::new(&serde_json::json!({ "extra": extra })).expect("options")
    }

    #[test]
    fn expand_target_dir_follows_options() {
        let d = tempfile::tempdir().expect("tmp");
        let files = Vfs::new(d.path());
        let o = options(serde_json::json!({"public-dir": "web/", "my_dir": null}));
        let e = |s: &str| o.expand(s, &files).expect("expand");
        assert_eq!(e("%CONFIG_DIR%/packages/"), "config/packages/");
        assert_eq!(e("%PUBLIC_DIR%/bundles"), "web/bundles");
        assert_eq!(e("%kernel.project_dir%"), "%kernel.project_dir%");
        assert_eq!(e("%env(DATABASE_URL)%"), "%env(DATABASE_URL)%");
        // `.+?` takes at least one byte, and the next `%` closes it.
        assert_eq!(e("%%BIN_DIR%"), "%%BIN_DIR%");
        // Measured with preg_replace_callback: `%% %` is the first match,
        // so `BIN_DIR%` has lost its opening `%`.
        assert_eq!(e("100%% %BIN_DIR%"), "100%% %BIN_DIR%");
        assert_eq!(e("100% %BIN_DIR%"), "100% %BIN_DIR%");
        assert_eq!(e("a %BIN_DIR% b %SRC_DIR%"), "a bin b src");
        // No newline inside a name.
        assert_eq!(e("%BIN\n_DIR%"), "%BIN\n_DIR%");
        // A null option is not set (`isset`).
        assert_eq!(e("%MY_DIR%"), "%MY_DIR%");
        // A non-string option: PHP would coerce or throw — refused.
        let arr = options(serde_json::json!({"weird": ["a"]}));
        assert!(arr.expand("%WEIRD%", &files).is_err());
        assert!(arr.expand("%RUNTIME%", &files).is_err());
    }

    #[test]
    fn the_phpunit_names_swap_as_in_options() {
        let d = tempfile::tempdir().expect("tmp");
        let files = Vfs::new(d.path());
        let o = options(serde_json::json!({}));
        assert_eq!(
            o.expand("phpunit.xml.dist", &files).expect("e"),
            "phpunit.xml.dist"
        );
        std::fs::write(d.path().join("phpunit.dist.xml"), "x").expect("w");
        assert_eq!(
            o.expand("phpunit.xml.dist", &files).expect("e"),
            "phpunit.dist.xml"
        );
        std::fs::write(d.path().join("phpunit.xml.dist"), "x").expect("w");
        assert_eq!(
            o.expand("phpunit.xml.dist", &files).expect("e"),
            "phpunit.xml.dist"
        );
    }

    #[test]
    fn root_dir_other_than_dot_is_refused() {
        assert!(
            Options::new(&serde_json::json!({"extra": {"symfony": {"root-dir": "app"}}})).is_err()
        );
        assert!(Options::new(&serde_json::json!({"extra": {"root-dir": "app"}})).is_err());
        assert!(
            Options::new(&serde_json::json!({"extra": {"symfony": {"root-dir": "."}}})).is_ok()
        );
    }

    #[test]
    fn bundles_round_trip_or_refuse() {
        let canonical = "<?php\n\nreturn [\n    A\\B\\CBundle::class => ['all' => true],\n    D\\EBundle::class => ['dev' => true, 'test' => false],\n];\n";
        let parsed = parse_bundles(canonical.as_bytes()).expect("canonical");
        assert_eq!(parsed.len(), 2);
        assert_eq!(render_bundles(&parsed), canonical);
        assert!(parse_bundles(b"<?php\n\nreturn [\n];\n").is_some_and(|b| b.is_empty()));
        // Anything else is evaluated by PHP, not recognised here.
        for other in [
            "<?php\nreturn [\n    A::class => ['all' => true],\n];\n",
            "<?php\n\nreturn [\n    A::class => ['all' => true], // x\n];\n",
            "<?php\n\nreturn [\n    \\A::class => ['all' => true],\n];\n",
            "<?php\n\nreturn [\n    A::class => ['all' => true],\n];\n\n",
            "<?php\r\n\r\nreturn [\r\n];\r\n",
            "<?php\n\nreturn [\n    A::class => ['all' => 1],\n];\n",
            "<?php\n\nreturn [\n    A::class => ['all' => true],\n    A::class => ['dev' => true],\n];\n",
        ] {
            assert!(parse_bundles(other.as_bytes()).is_none(), "{other:?}");
        }
    }

    #[test]
    fn generated_secrets_are_refused_with_a_final_newline_too() {
        let r = Recipe {
            name: "a/b".into(),
            origin: String::new(),
            manifest: Map::new(),
            files: Vec::new(),
            is_contrib: false,
            package_type: "library".into(),
        };
        for v in [
            "%generate(secret)%",
            "%generate(secret,32)%",
            "%generate(secret, 32)%",
            "%generate(secret,32)%\n",
        ] {
            assert!(evaluate_value(&r, v).is_err(), "{v:?}");
        }
        for v in [
            "%generate(secret)%\n",
            "x%generate(secret)%",
            "%generate(secret,)%",
        ] {
            assert!(evaluate_value(&r, v).is_ok(), "{v:?}");
        }
    }

    #[test]
    fn xml_attributes_escape_like_libxml() {
        // Measured: DOMDocument::saveXML of <env value="…"/> (PHP 8.5).
        assert_eq!(
            xml_attribute("a&b<c>d\"e'f\tg\nh\ri \u{e9} \u{20ac}").expect("x"),
            "a&amp;b&lt;c&gt;d&quot;e'f&#9;g&#10;h&#13;i &#xE9; &#x20AC;"
        );
        assert!(xml_attribute("a\u{1}").is_err());
    }

    #[test]
    fn php_close_insertion_follows_preg_replace() {
        let f =
            |c: &str| String::from_utf8(insert_before_php_close(c.as_bytes(), b"X")).expect("u");
        assert_eq!(f("<php>\n    </php>\n"), "<php>\nX    </php>\n");
        // A blank line before: the run of whitespace starts at the earlier
        // line start, and so does the insertion.
        assert_eq!(f("<php>\n\n    </php>"), "<php>\nX\n    </php>");
        // `</php>` at the very start of a line: `\s+` needs one byte.
        assert_eq!(f("<php>\n</php>"), "<php>\n</php>");
        assert_eq!(
            f("a\n  </php>\nb\n  </php>"),
            "a\nX  </php>\nbX\n  </php>".replace("bX\n", "b\nX")
        );
    }

    #[test]
    fn origins_format_like_recipe() {
        let r = |o: &str| Recipe {
            name: "a/b".into(),
            origin: o.into(),
            manifest: Map::new(),
            files: Vec::new(),
            is_contrib: false,
            package_type: "library".into(),
        };
        assert_eq!(
            r("symfony/console:5.3@github.com/symfony/recipes:main").formatted_origin(),
            "symfony/console (>=5.3): From github.com/symfony/recipes:main"
        );
        assert_eq!(
            r("twig/extra-bundle:v3.21.0@auto-generated recipe").formatted_origin(),
            "twig/extra-bundle (>=v3.21.0): From auto-generated recipe"
        );
        assert_eq!(r("plain").formatted_origin(), "plain");
        // Measured with preg_match: the version may hold a `:`, and `$`
        // matches before a final newline.
        assert_eq!(
            r("a/b:1.0:x@foo").formatted_origin(),
            "a/b (>=1.0:x): From foo"
        );
        assert_eq!(
            r("a/b:1.0@foo\n").formatted_origin(),
            "a/b (>=1.0): From foo"
        );
        assert_eq!(r("a/b:1.0@f\noo").formatted_origin(), "a/b:1.0@f\noo");
        assert_eq!(r("").formatted_origin(), "");
    }

    #[test]
    fn concatenate_and_local_paths() {
        assert_eq!(concatenate(&[".", "config/"]), "./config/");
        assert_eq!(
            concatenate(&["./config/", "/packages/x.yaml"]),
            "./config/packages/x.yaml"
        );
        assert_eq!(concatenate(&[".", "/.env.test"]), "./.env.test");
        if !cfg!(windows) {
            assert_eq!(
                local_file_path("./config/packages/x.yaml"),
                "config/packages/x.yaml"
            );
            // Every occurrence, as str_replace does.
            assert_eq!(local_file_path("./a./b"), "ab");
        }
    }

    #[test]
    fn add_lines_after_target_splices_one_element() {
        let d = tempfile::tempdir().expect("tmp");
        let installed: Vec<(String, String)> = Vec::new();
        let mut ctx = Ctx::new(
            d.path(),
            &serde_json::json!({}),
            "symfony.lock",
            &installed,
            None,
        )
        .expect("ctx");
        let out = patched_contents(
            &mut ctx,
            "./f",
            b"a\n<x>\nb",
            "L1\nL2",
            "after_target",
            Some("<x>"),
            vivacity_core::io::Verbosity::Verbose,
        )
        .expect("patch");
        assert_eq!(out, b"a\n<x>\nL1\nL2\nb");
        let same = patched_contents(
            &mut ctx,
            "./f",
            b"a\nL1\nL2",
            "L1\nL2",
            "bottom",
            None,
            vivacity_core::io::Verbosity::Verbose,
        )
        .expect("patch");
        assert_eq!(same, b"a\nL1\nL2");
    }
}

//! The forms of a package's `bin`, against the 2.10.3 phar rather than against
//! written expectations: each case runs Composer, then vivacity under the same
//! absolute path, and compares what they leave and what they print.
//!
//! A form that is not ordinary goes to Composer before any write; under
//! `--no-fallback` it stops with exit 3 and nothing is written. A form known
//! only after the extraction (a link leading out, a directory) stays native:
//! neither a proxy nor a `chmod`, and the phar's skip line.
#![cfg(unix)]

use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::OnceLock;

/// The Composer these tests compare with, when it is 2.10.3: any other version has
/// other lines and another `bin` code, and drift.yml runs the suite under newer ones,
/// so those skip with a word on stderr. No Composer at all fails, as the other oracle
/// tests do: a green run must have compared something.
fn composer() -> Option<&'static Path> {
    static FOUND: OnceLock<Option<PathBuf>> = OnceLock::new();
    FOUND
        .get_or_init(|| {
            let path = std::env::var_os("PATH").expect("PATH");
            let composer = std::env::split_paths(&path)
                .map(|d| d.join("composer"))
                .find(|c| c.is_file())
                .expect("composer on the PATH");
            let out = clean(Command::new(&composer))
                .arg("--version")
                .output()
                .expect("composer --version");
            let text = String::from_utf8_lossy(&out.stdout).into_owned();
            if text.contains("Composer version 2.10.3 ") {
                Some(composer)
            } else {
                eprintln!("bin_forms compares with Composer 2.10.3, skipped under: {text}");
                None
            }
        })
        .as_deref()
}

/// Nothing inherited that could change a run.
fn clean(mut cmd: Command) -> Command {
    for (key, _) in std::env::vars_os() {
        let k = key.to_string_lossy().to_ascii_uppercase();
        if k.starts_with("COMPOSER")
            || k.starts_with("VIVACITY_")
            || k.starts_with("SYMFONY_")
            || k == "SHELL_VERBOSITY"
            || k.ends_with("_PROXY")
        {
            cmd.env_remove(&key);
        }
    }
    cmd
}

/// A server for one case: the repository's `packages.json` and the dists. The
/// listener is bound first, because the repository names its port.
fn serve(listener: std::net::TcpListener, files: BTreeMap<String, Vec<u8>>) {
    std::thread::spawn(move || {
        for mut stream in listener.incoming().flatten() {
            let mut request = Vec::new();
            let mut buf = [0u8; 4096];
            while !request.windows(4).any(|w| w == b"\r\n\r\n") {
                match stream.read(&mut buf) {
                    Ok(0) | Err(_) => break,
                    Ok(n) => request.extend_from_slice(&buf[..n]),
                }
            }
            let text = String::from_utf8_lossy(&request);
            let path = text.split_whitespace().nth(1).unwrap_or("").to_owned();
            let head = |status: &str, len: usize| {
                format!("HTTP/1.1 {status}\r\nContent-Length: {len}\r\nConnection: close\r\n\r\n")
            };
            match files.get(&path) {
                Some(body) => {
                    let _ = stream.write_all(head("200 OK", body.len()).as_bytes());
                    let _ = stream.write_all(body);
                }
                None => {
                    let _ = stream.write_all(head("404 Not Found", 0).as_bytes());
                }
            }
        }
    });
}

/// One file of a dist.
enum Entry {
    File(&'static str, &'static str),
    Dir(&'static str),
}

/// What every dist holds: files at the paths the forms name, not executable, so
/// that a `chmod` shows.
const PACKAGE: &[Entry] = &[
    Entry::File("composer.json", "{\"name\": \"acme/tool\"}\n"),
    Entry::File("bin/x", "#!/usr/bin/env php\n<?php echo 'x';\n"),
    Entry::File("other/x", "#!/usr/bin/env php\n<?php echo 'other';\n"),
    Entry::File("x", "#!/usr/bin/env php\n<?php echo 'root';\n"),
    Entry::File("a/b", "#!/usr/bin/env php\n<?php echo 'b';\n"),
    Entry::Dir("dir/"),
];

fn zip_of(entries: &[Entry]) -> Vec<u8> {
    use zip::write::SimpleFileOptions;
    let mut w = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    let options = SimpleFileOptions::default().unix_permissions(0o644);
    let at = |p: &str| format!("acme-tool-abc/{p}");
    for entry in entries {
        match entry {
            Entry::File(p, content) => {
                w.start_file(at(p), options).expect("file");
                w.write_all(content.as_bytes()).expect("write");
            }
            Entry::Dir(p) => w
                .add_directory(at(p), options.unix_permissions(0o755))
                .expect("dir"),
        }
    }
    w.finish().expect("zip").into_inner()
}

/// A package version as a repository or a lock holds it.
fn package(port: u16, version: &str, bin: Option<&Value>) -> Value {
    let mut p = json!({
        "name": "acme/tool",
        "version": version,
        "dist": {
            "type": "zip",
            "url": format!("http://127.0.0.1:{port}/tool.zip"),
            "reference": "abc",
            "shasum": ""
        },
        "type": "library"
    });
    if let Some(bin) = bin {
        p["bin"] = bin.clone();
    }
    p
}

fn manifest_text(port: u16, constraint: &str) -> String {
    serde_json::to_string_pretty(&json!({
        "name": "acme/root",
        "version": "1.0.0",
        "repositories": [
            {"type": "composer", "url": format!("http://127.0.0.1:{port}")},
            {"packagist.org": false}
        ],
        "require": {"acme/tool": constraint},
        "config": {"secure-http": false}
    }))
    .expect("manifest")
}

/// A repository's `packages.json` holding these versions of acme/tool.
fn repository(versions: Vec<Value>) -> Value {
    let by_version: serde_json::Map<String, Value> = versions
        .into_iter()
        .map(|v| (v["version"].as_str().expect("version").to_owned(), v))
        .collect();
    json!({"packages": {"acme/tool": by_version}})
}

fn lock_text(manifest: &str, packages: Vec<Value>, dev: Vec<Value>) -> String {
    let hash = vivacity_core::content_hash::content_hash(manifest).expect("content-hash");
    serde_json::to_string_pretty(&json!({
        "_readme": [],
        "content-hash": hash,
        "packages": packages,
        "packages-dev": dev,
        "aliases": [],
        "minimum-stability": "stable",
        "stability-flags": {},
        "prefer-stable": false,
        "prefer-lowest": false,
        "platform": {},
        "platform-dev": {},
        "plugin-api-version": "2.6.0"
    }))
    .expect("lock")
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Node {
    Dir(u32),
    File(u32, Vec<u8>),
    Link(PathBuf),
}

/// Every path under `dir`, with its mode, its bytes or its target.
fn inventory(dir: &Path) -> BTreeMap<String, Node> {
    fn walk(base: &Path, dir: &Path, out: &mut BTreeMap<String, Node>) {
        for entry in std::fs::read_dir(dir).expect("read_dir") {
            let path = entry.expect("entry").path();
            let rel = path
                .strip_prefix(base)
                .expect("under base")
                .to_string_lossy()
                .into_owned();
            let meta = std::fs::symlink_metadata(&path).expect("meta");
            let mode = meta.mode() & 0o777;
            if meta.file_type().is_symlink() {
                out.insert(rel, Node::Link(std::fs::read_link(&path).expect("link")));
            } else if meta.is_dir() {
                out.insert(rel, Node::Dir(mode));
                walk(base, &path, out);
            } else {
                out.insert(rel, Node::File(mode, std::fs::read(&path).expect("read")));
            }
        }
    }
    let mut out = BTreeMap::new();
    walk(dir, dir, &mut out);
    out
}

/// The paths two inventories disagree on.
fn differences(a: &BTreeMap<String, Node>, b: &BTreeMap<String, Node>) -> Vec<String> {
    let describe = |n: Option<&Node>| match n {
        None => "absent".to_owned(),
        Some(Node::Dir(m)) => format!("dir {m:o}"),
        Some(Node::Link(t)) => format!("link to {}", t.display()),
        Some(Node::File(m, bytes)) => format!(
            "file {m:o}, {} bytes: {:?}",
            bytes.len(),
            String::from_utf8_lossy(&bytes[..bytes.len().min(300)])
        ),
    };
    a.keys()
        .chain(b.keys())
        .collect::<std::collections::BTreeSet<_>>()
        .into_iter()
        .filter(|k| a.get(*k) != b.get(*k))
        .map(|k| {
            format!(
                "{k}\n    phar:     {}\n    vivacity: {}",
                describe(a.get(k)),
                describe(b.get(k))
            )
        })
        .collect()
}

#[derive(Debug)]
struct Output {
    code: i32,
    stdout: String,
    stderr: String,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Run {
    /// A lock is there, the command installs it.
    Install,
    /// No lock: the command resolves, writes it and installs.
    Update,
}

/// What one case starts from, its port known.
struct Setup {
    manifest: String,
    repository: Value,
    /// composer.lock's packages and packages-dev: no file when `None`.
    lock: Option<(Vec<Value>, Vec<Value>)>,
    /// vendor/composer/installed.json's packages, with their directories.
    installed: Vec<Value>,
    /// A `path` repository's package beside the project.
    path_package: bool,
}

/// One case's place: the same absolute paths for every run, emptied in between,
/// so that nothing has to be normalized.
struct Case {
    _root: tempfile::TempDir,
    work: PathBuf,
    env: PathBuf,
    composer: &'static Path,
    setup: Setup,
}

/// A `bin` value, which may name the case's work directory.
type Form = fn(&Path) -> Value;

impl Case {
    /// `None` when there is no Composer 2.10.3 to compare with.
    fn new(setup: impl Fn(u16, &Path) -> Setup) -> Option<Case> {
        let composer = composer()?;
        let root = tempfile::tempdir().expect("tmp");
        // `realpath` and the proxies see the path the runs see.
        let base = std::fs::canonicalize(root.path()).expect("canonical");
        let work = base.join("work");
        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind");
        let port = listener.local_addr().expect("addr").port();
        let setup = setup(port, &work);
        serve(
            listener,
            BTreeMap::from([
                (
                    "/packages.json".to_owned(),
                    serde_json::to_vec(&setup.repository).expect("repository"),
                ),
                ("/tool.zip".to_owned(), zip_of(PACKAGE)),
            ]),
        );
        Some(Case {
            env: base.join("env"),
            work,
            _root: root,
            composer,
            setup,
        })
    }

    /// The form in a zip dist: in the lock for `Install`, in the repository for
    /// `Update`.
    fn of_form(run: Run, form: Form) -> Option<Case> {
        Case::new(|port, work| {
            let bin = form(work);
            Setup {
                manifest: manifest_text(port, "1.0.0"),
                repository: repository(vec![package(port, "1.0.0", Some(&bin))]),
                lock: (run == Run::Install)
                    .then(|| (vec![package(port, "1.0.0", Some(&bin))], vec![])),
                installed: vec![],
                path_package: false,
            }
        })
    }

    /// The form in a `path` package, whose tree can hold a link leading out.
    fn of_path_package(run: Run, form: Form) -> Option<Case> {
        Case::new(|_, work| {
            let mut locked = json!({
                "name": "acme/tool",
                "version": "1.0.0",
                "dist": {"type": "path", "url": "../pkg", "reference": "abc"},
                "type": "library",
                "transport-options": {"relative": true},
                "bin": form(work)
            });
            if run == Run::Update {
                locked = Value::Null;
            }
            Setup {
                manifest: serde_json::to_string_pretty(&json!({
                    "name": "acme/root",
                    "version": "1.0.0",
                    "repositories": [
                        {"type": "path", "url": "../pkg"},
                        {"packagist.org": false}
                    ],
                    "require": {"acme/tool": "1.0.0"}
                }))
                .expect("manifest"),
                repository: json!({"packages": {}}),
                lock: (run == Run::Install).then(|| (vec![locked], vec![])),
                installed: vec![],
                path_package: true,
            }
        })
    }

    fn project(&self) -> PathBuf {
        self.work.join("project")
    }

    fn witness_mode(&self) -> u32 {
        std::fs::metadata(self.work.join("witness"))
            .expect("witness")
            .mode()
            & 0o777
    }

    /// The work directory as a run starts from it, the caches empty.
    fn reset(&self, form: Option<Form>) {
        let _ = std::fs::remove_dir_all(&self.work);
        let _ = std::fs::remove_dir_all(&self.env);
        let project = self.project();
        std::fs::create_dir_all(&project).expect("project");
        std::fs::create_dir_all(&self.env).expect("env");
        let witness = self.work.join("witness");
        std::fs::write(&witness, "#!/usr/bin/env php\n<?php echo 'outside';\n").expect("witness");
        std::fs::set_permissions(&witness, std::fs::Permissions::from_mode(0o600))
            .expect("chmod witness");
        std::fs::write(project.join("composer.json"), &self.setup.manifest).expect("composer.json");
        if let Some((packages, dev)) = &self.setup.lock {
            std::fs::write(
                project.join("composer.lock"),
                lock_text(&self.setup.manifest, packages.clone(), dev.clone()),
            )
            .expect("composer.lock");
        }
        if !self.setup.installed.is_empty() {
            let composer_dir = project.join("vendor/composer");
            std::fs::create_dir_all(&composer_dir).expect("vendor/composer");
            for p in &self.setup.installed {
                let name = p["name"].as_str().expect("name");
                let dir = project.join("vendor").join(name);
                std::fs::create_dir_all(&dir).expect("package dir");
                std::fs::write(dir.join("composer.json"), "{}\n").expect("package file");
            }
            let installed = json!({
                "packages": self.setup.installed,
                "dev": true,
                "dev-package-names": []
            });
            std::fs::write(
                composer_dir.join("installed.json"),
                serde_json::to_string_pretty(&installed).expect("installed.json"),
            )
            .expect("installed.json");
        }
        if self.setup.path_package {
            let pkg = self.work.join("pkg");
            std::fs::create_dir_all(pkg.join("bin")).expect("pkg");
            std::fs::create_dir_all(pkg.join("dir")).expect("pkg/dir");
            let mut composer_json = json!({"name": "acme/tool", "version": "1.0.0"});
            if let Some(form) = form {
                composer_json["bin"] = form(&self.work);
            }
            std::fs::write(
                pkg.join("composer.json"),
                serde_json::to_string_pretty(&composer_json).expect("package manifest"),
            )
            .expect("pkg/composer.json");
            let bin = pkg.join("bin/x");
            std::fs::write(&bin, "#!/usr/bin/env php\n<?php echo 'x';\n").expect("pkg/bin/x");
            std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o644)).expect("chmod");
            std::os::unix::fs::symlink("../../witness", pkg.join("bin/out")).expect("link");
        }
    }

    fn run(&self, program: &Path, args: &[&str]) -> Output {
        let mut cmd = clean(Command::new(program));
        let out = cmd
            .current_dir(self.project())
            .env("COMPOSER_HOME", self.env.join("home"))
            .env("COMPOSER_CACHE_DIR", self.env.join("cache"))
            .env("VIVACITY_CACHE_DIR", self.env.join("store"))
            .args(args)
            .output()
            .expect("run");
        Output {
            code: out.status.code().unwrap_or(-1),
            stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
        }
    }

    fn vivacity(&self, args: &[&str]) -> Output {
        self.run(Path::new(env!("CARGO_BIN_EXE_vivacity")), args)
    }
}

/// The line of a run: what the fallback forwards as it is, on both sides.
/// `--no-progress` because the phar's bar depends on time, not on the input: with two
/// operations, it starts from the jobs active at that instant and redraws every 0.1 s
/// (`Loop::wait`), and 30 runs of the phar alone on one input gave ` 0/2` in 23 and
/// ` 0/1` in 7 under load. With the option, the same runs give one output.
fn line(run: Run) -> Vec<&'static str> {
    match run {
        Run::Install => vec![
            "install",
            "--no-interaction",
            "--no-scripts",
            "--no-progress",
        ],
        Run::Update => vec![
            "update",
            "--no-interaction",
            "--no-scripts",
            "--no-audit",
            "--no-progress",
        ],
    }
}

fn dump(what: &str, out: &Output) -> String {
    format!(
        "--- {what}: exit {}\nstdout:\n{}stderr:\n{}",
        out.code, out.stdout, out.stderr
    )
}

/// Composer's run on the case, what it left and what it printed.
fn phar_run(case: &Case, args: &[&str], form: Option<Form>) -> (Output, BTreeMap<String, Node>) {
    case.reset(form);
    let phar = case.run(case.composer, args);
    assert_eq!(case.witness_mode(), 0o600, "the phar moved the witness");
    (phar, inventory(&case.work))
}

/// A form that goes to Composer before any write: stopped under `--no-fallback`,
/// otherwise the phar's own run. What vivacity prints in front of it is its
/// narrative, and on a late hand-over the `late` lines, which Composer prints
/// again (or, on a form its loader throws on, never reaches: `Updating
/// dependencies` then has no counterpart).
fn assert_handed_over(case: &Case, args: &[&str], form: Option<Form>, late: &[&str]) {
    let (phar, phar_tree) = phar_run(case, args, form);

    case.reset(form);
    let before = inventory(&case.work);
    let mut stopping = args.to_vec();
    stopping.push("--no-fallback");
    let stopped = case.vivacity(&stopping);
    let context = format!("{}\n{}", dump("phar", &phar), dump("vivacity", &stopped));
    assert_eq!(
        format!("{:o}", case.witness_mode()),
        "600",
        "the witness moved under --no-fallback\n{context}"
    );
    assert_eq!(stopped.code, 3, "--no-fallback must stop\n{context}");
    let written = differences(&before, &inventory(&case.work));
    assert!(
        written.is_empty(),
        "--no-fallback wrote:\n{}",
        written.join("\n")
    );

    case.reset(form);
    let handed = case.vivacity(args);
    let context = format!("{}\n{}", dump("phar", &phar), dump("vivacity", &handed));
    let at = handed
        .stderr
        .find("vivacity: delegating to `composer ")
        .unwrap_or_else(|| panic!("Composer was not reached\n{context}"));
    let (printed_before, rest) = handed.stderr.split_at(at);
    let composer_stderr = rest.split_once('\n').map_or("", |(_, r)| r);
    assert_eq!(handed.code, phar.code, "exit code\n{context}");
    assert_eq!(handed.stdout, phar.stdout, "stdout\n{context}");
    assert_eq!(composer_stderr, phar.stderr, "Composer's stderr\n{context}");
    let printed: Vec<&str> = printed_before
        .lines()
        .filter(|l| !l.starts_with("vivacity: ") && !l.starts_with("  - "))
        .collect();
    assert_eq!(
        printed, late,
        "what vivacity printed before the hand-over\n{context}"
    );
    assert_eq!(format!("{:o}", case.witness_mode()), "600", "{context}");
    let tree = differences(&phar_tree, &inventory(&case.work));
    assert!(tree.is_empty(), "tree\n{}\n{context}", tree.join("\n"));
}

/// What `update` has printed when the solver's choice reveals a form: the late
/// hand-over of HANDOVER.md.
const LATE: &[&str] = &[
    "Loading composer repositories with package information",
    "Warning: Accessing 127.0.0.1 over http which is an insecure protocol.",
    "Updating dependencies",
];

/// The same from a `path` repository, read without a request.
const LATE_PATH: &[&str] = &[
    "Loading composer repositories with package information",
    "Updating dependencies",
];

fn handed_over(run: Run, form: Form) {
    let Some(case) = Case::of_form(run, form) else {
        return;
    };
    let late = if run == Run::Update { LATE } else { &[] };
    assert_handed_over(&case, &line(run), None, late);
}

/// What the phar prints that vivacity does not reproduce on a form it keeps: the
/// `ensureBinariesPresence` pass after the dump repeats a directory's or an
/// outside link's line, and a missing `bin` stays silent as in 0.26.2.
fn expected_native_stderr(phar: &str, missing_silent: bool) -> String {
    let mut after_dump = false;
    let mut out = String::new();
    for l in phar.lines() {
        if l == "Generating autoload files" {
            after_dump = true;
        }
        let skip = l.starts_with("    Skipped installation of bin ");
        let repeated = after_dump
            && skip
            && (l.ends_with(": found a directory at that path")
                || l.ends_with(": the bin resolves to a path outside of the package directory")
                || l.ends_with(": file not found in package"));
        let missing = missing_silent && skip && l.ends_with(": file not found in package");
        if !repeated && !missing {
            out.push_str(l);
            out.push('\n');
        }
    }
    out
}

/// A form vivacity keeps: stays native under `--no-fallback`, same exit, same
/// tree, the witness untouched, the phar's lines. Its closing line is its own.
fn assert_native(case: &Case, args: &[&str], form: Option<Form>, missing_silent: bool) {
    let (phar, phar_tree) = phar_run(case, args, form);
    case.reset(form);
    let mut native = args.to_vec();
    native.push("--no-fallback");
    let viv = case.vivacity(&native);
    let context = format!("{}\n{}", dump("phar", &phar), dump("vivacity", &viv));
    assert_eq!(
        format!("{:o}", case.witness_mode()),
        "600",
        "the witness moved\n{context}"
    );
    assert_eq!(viv.code, phar.code, "exit code\n{context}");
    assert_eq!(viv.stdout, phar.stdout, "stdout\n{context}");
    let own: String = viv
        .stderr
        .lines()
        .filter(|l| !l.starts_with("vivacity: "))
        .map(|l| format!("{l}\n"))
        .collect();
    assert_eq!(
        own,
        expected_native_stderr(&phar.stderr, missing_silent),
        "stderr\n{context}"
    );
    let tree = differences(&phar_tree, &inventory(&case.work));
    assert!(tree.is_empty(), "tree\n{}\n{context}", tree.join("\n"));
}

fn native(run: Run, form: Form) {
    let Some(case) = Case::of_form(run, form) else {
        return;
    };
    assert_native(&case, &line(run), None, false);
}

/// The witness by its absolute path, without its leading `/`.
fn witness_tail(work: &Path) -> String {
    format!("{}/witness", work.display())
        .trim_start_matches('/')
        .to_owned()
}

fn absolute(work: &Path) -> Value {
    json!([format!("/{}", witness_tail(work))])
}
fn dot_slash_slash(work: &Path) -> Value {
    json!([format!(".//{}", witness_tail(work))])
}
fn dot_slash_slash_inside(_: &Path) -> Value {
    json!([".//bin/x"])
}
fn double_slash(_: &Path) -> Value {
    json!(["a//b"])
}
fn dot(_: &Path) -> Value {
    json!(["."])
}
fn dot_slash(_: &Path) -> Value {
    json!(["./"])
}
fn trailing_dot(_: &Path) -> Value {
    json!(["a/."])
}
fn dot_dot(_: &Path) -> Value {
    json!([".."])
}
fn trailing_dot_dot(_: &Path) -> Value {
    json!(["a/.."])
}
fn backslash(_: &Path) -> Value {
    json!(["a\\b"])
}
fn empty(_: &Path) -> Value {
    json!([""])
}
fn string(_: &Path) -> Value {
    json!("bin/x")
}
fn object(_: &Path) -> Value {
    json!({"k": "bin/x"})
}
fn non_string(_: &Path) -> Value {
    json!([1])
}
fn directory(_: &Path) -> Value {
    json!(["dir"])
}
fn name_conflict(_: &Path) -> Value {
    json!(["bin/x", "other/x"])
}
fn ordinary(_: &Path) -> Value {
    json!(["bin/x"])
}
fn ordinary_dot_slash(_: &Path) -> Value {
    json!(["./bin/x"])
}
fn missing(_: &Path) -> Value {
    json!(["bin/nope"])
}
fn link_out(_: &Path) -> Value {
    json!(["bin/out"])
}

macro_rules! each_command {
    ($check:ident: $($name:ident => $form:ident,)*) => {
        $(mod $name {
            use super::*;
            #[test]
            fn install() {
                $check(Run::Install, $form);
            }
            #[test]
            fn update() {
                $check(Run::Update, $form);
            }
        })*
    };
}

each_command! { handed_over:
    absolute_form => absolute,
    dot_slash_slash_form => dot_slash_slash,
    dot_slash_slash_inside_form => dot_slash_slash_inside,
    double_slash_form => double_slash,
    dot_form => dot,
    dot_slash_form => dot_slash,
    trailing_dot_form => trailing_dot,
    dot_dot_form => dot_dot,
    trailing_dot_dot_form => trailing_dot_dot,
    backslash_form => backslash,
    empty_form => empty,
    string_form => string,
    object_form => object,
    non_string_form => non_string,
}

each_command! { native:
    directory_form => directory,
    name_conflict_form => name_conflict,
    ordinary_form => ordinary,
    ordinary_dot_slash_form => ordinary_dot_slash,
}

/// A directory on an install with nothing to do: the phar's pass after the dump
/// writes its line, vivacity keeps quiet (the difference HANDOVER names). The
/// tree stays the same and the second run moves nothing.
#[test]
fn a_directory_on_a_no_op_install() {
    let Some(case) = Case::of_form(Run::Install, directory) else {
        return;
    };
    let line_of_phar =
        "    Skipped installation of bin dir for package acme/tool: found a directory at that path";
    let args = line(Run::Install);
    case.reset(None);
    assert_eq!(case.run(case.composer, &args).code, 0);
    let again = case.run(case.composer, &args);
    assert_eq!(again.code, 0, "{}", dump("phar", &again));
    assert_eq!(
        again.stderr.lines().filter(|l| *l == line_of_phar).count(),
        1,
        "{}",
        dump("phar", &again)
    );
    let phar_tree = inventory(&case.work);

    case.reset(None);
    assert_eq!(case.vivacity(&args).code, 0);
    let again = case.vivacity(&args);
    assert_eq!(again.code, 0, "{}", dump("vivacity", &again));
    assert!(
        !again.stderr.contains("Skipped"),
        "{}",
        dump("vivacity", &again)
    );
    let tree = differences(&phar_tree, &inventory(&case.work));
    assert!(tree.is_empty(), "tree\n{}", tree.join("\n"));
}

/// A `bin` the dist does not hold: the phar's `file not found` line is the one
/// consigned difference, nothing else differs.
mod missing_form {
    use super::*;
    fn silent(run: Run) {
        let Some(case) = Case::of_form(run, missing) else {
            return;
        };
        assert_native(&case, &line(run), None, true);
    }
    #[test]
    fn install() {
        silent(Run::Install);
    }
    #[test]
    fn update() {
        silent(Run::Update);
    }
}

/// A link of the package leading out: known only once the package is laid out,
/// so native. A zip dist cannot bring one (its extraction refuses it): a `path`
/// repository does.
mod link_out_form {
    use super::*;
    fn out(run: Run) {
        let Some(case) = Case::of_path_package(run, link_out) else {
            return;
        };
        assert_native(&case, &line(run), Some(link_out), false);
    }
    #[test]
    fn install() {
        out(Run::Install);
    }
    #[test]
    fn update() {
        out(Run::Update);
    }
}

/// The same forms from a `path` package's composer.json, on `update`: they reach
/// the solver's selected packages through another loader.
mod path_package_metadata {
    use super::*;
    #[test]
    fn dot_slash_slash_form() {
        let Some(case) = Case::of_path_package(Run::Update, dot_slash_slash) else {
            return;
        };
        assert_handed_over(&case, &line(Run::Update), Some(dot_slash_slash), LATE_PATH);
    }
}

/// An entry of installed.json, as an earlier Composer run may have left it.
fn installed_entry(name: &str, bin: Value) -> Value {
    json!({
        "name": name,
        "version": "1.0.0",
        "version_normalized": "1.0.0.0",
        "dist": {"type": "zip", "url": "http://127.0.0.1:1/none.zip", "reference": "abc", "shasum": ""},
        "type": "library",
        "bin": bin,
        "installation-source": "dist",
        "install-path": format!("../{name}")
    })
}

/// A form only in installed.json: the package going away, and one staying as it
/// is, whichever command runs.
mod already_installed {
    use super::*;

    fn gone(run: Run, form: Form) {
        let Some(case) = Case::new(|port, work| Setup {
            manifest: manifest_text(port, "1.0.0"),
            repository: repository(vec![package(port, "1.0.0", Some(&ordinary(work)))]),
            lock: (run == Run::Install)
                .then(|| (vec![package(port, "1.0.0", Some(&ordinary(work)))], vec![])),
            installed: vec![installed_entry("acme/gone", form(work))],
            path_package: false,
        }) else {
            return;
        };
        assert_handed_over(&case, &line(run), None, &[]);
    }

    fn staying(run: Run, form: Form) {
        let Some(case) = Case::new(|port, work| Setup {
            manifest: manifest_text(port, "1.0.0"),
            repository: repository(vec![package(port, "1.0.0", Some(&ordinary(work)))]),
            lock: (run == Run::Install)
                .then(|| (vec![package(port, "1.0.0", Some(&ordinary(work)))], vec![])),
            installed: vec![installed_entry("acme/tool", form(work))],
            path_package: false,
        }) else {
            return;
        };
        assert_handed_over(&case, &line(run), None, &[]);
    }

    #[test]
    fn gone_dot_slash_slash_install() {
        gone(Run::Install, dot_slash_slash);
    }
    #[test]
    fn gone_dot_slash_slash_update() {
        gone(Run::Update, dot_slash_slash);
    }
    #[test]
    fn gone_dot_dot_install() {
        gone(Run::Install, dot_dot);
    }
    #[test]
    fn gone_dot_dot_update() {
        gone(Run::Update, dot_dot);
    }
    #[test]
    fn gone_non_string_install() {
        gone(Run::Install, non_string);
    }
    #[test]
    fn staying_double_slash_install() {
        staying(Run::Install, double_slash);
    }
    #[test]
    fn staying_double_slash_update() {
        staying(Run::Update, double_slash);
    }
}

/// What decides on the selected packages and not on the pool.
mod selection {
    use super::*;

    /// A candidate in the pool the solver does not keep hands nothing over.
    #[test]
    fn an_unselected_candidate_stays_native() {
        let Some(case) = Case::new(|port, work| Setup {
            manifest: manifest_text(port, "*"),
            repository: repository(vec![
                package(port, "1.0.0", Some(&dot_slash_slash(work))),
                package(port, "2.0.0", Some(&ordinary(work))),
            ]),
            lock: None,
            installed: vec![],
            path_package: false,
        }) else {
            return;
        };
        assert_native(&case, &line(Run::Update), None, false);
    }

    /// `validate_package` too decides on the kept packages only: a `..` left out
    /// in the pool stops neither side.
    #[test]
    fn an_unselected_dot_dot_stays_native() {
        let Some(case) = Case::new(|port, work| Setup {
            manifest: manifest_text(port, "*"),
            repository: repository(vec![
                package(port, "1.0.0", Some(&dot_dot(work))),
                package(port, "2.0.0", Some(&ordinary(work))),
            ]),
            lock: None,
            installed: vec![],
            path_package: false,
        }) else {
            return;
        };
        assert_native(&case, &line(Run::Update), None, false);
    }

    /// The one candidate that does count before the solve: the phar's loader throws
    /// on a non-string while it fills the pool, selected or not. vivacity reads a
    /// selected package's form and goes on here. Consigned in HANDOVER; this holds
    /// the two facts, so that a change on either side shows.
    #[test]
    fn a_non_string_left_out_stops_only_the_phar() {
        let Some(case) = Case::new(|port, work| Setup {
            manifest: manifest_text(port, "*"),
            repository: repository(vec![
                package(port, "1.0.0", Some(&non_string(work))),
                package(port, "2.0.0", Some(&ordinary(work))),
            ]),
            lock: None,
            installed: vec![],
            path_package: false,
        }) else {
            return;
        };
        let (phar, _) = phar_run(&case, &line(Run::Update), None);
        assert_eq!(phar.code, 1, "{}", dump("phar", &phar));
        case.reset(None);
        let mut args = line(Run::Update);
        args.push("--no-fallback");
        let viv = case.vivacity(&args);
        assert_eq!(viv.code, 0, "{}", dump("vivacity", &viv));
    }

    /// `install` with no lock resolves through the same session.
    #[test]
    fn an_install_without_a_lock_hands_over_a_selected_form() {
        let Some(case) = Case::of_form(Run::Update, dot_slash_slash) else {
            return;
        };
        assert_handed_over(
            &case,
            &line(Run::Install),
            None,
            &[
                "No composer.lock file present. Updating dependencies to latest instead of installing from lock file. See https://getcomposer.org/install for more information.",
                LATE[0],
                LATE[1],
                LATE[2],
            ],
        );
    }

    /// The same candidate, selected: hands over.
    #[test]
    fn the_same_candidate_selected_hands_over() {
        let Some(case) = Case::new(|port, work| Setup {
            manifest: manifest_text(port, "1.0.0"),
            repository: repository(vec![
                package(port, "1.0.0", Some(&dot_slash_slash(work))),
                package(port, "2.0.0", Some(&ordinary(work))),
            ]),
            lock: None,
            installed: vec![],
            path_package: false,
        }) else {
            return;
        };
        assert_handed_over(&case, &line(Run::Update), None, LATE);
    }

    /// A package nothing but another package requires is selected like any other.
    #[test]
    fn a_transitive_package_hands_over() {
        let Some(case) = Case::new(|port, work| {
            let mut dep = package(port, "1.0.0", None);
            dep["name"] = json!("acme/dep");
            dep["require"] = json!({"acme/tool": "1.0.0"});
            let manifest = manifest_text(port, "1.0.0").replace("acme/tool", "acme/dep");
            Setup {
                manifest,
                repository: json!({"packages": {
                    "acme/dep": {"1.0.0": dep},
                    "acme/tool": {"1.0.0": package(port, "1.0.0", Some(&dot_slash_slash(work)))}
                }}),
                lock: None,
                installed: vec![],
                path_package: false,
            }
        }) else {
            return;
        };
        assert_handed_over(&case, &line(Run::Update), None, LATE);
    }

    /// `update` with a form in the lock it reads, whatever the repository now
    /// holds: decided before anything is printed.
    #[test]
    fn a_form_in_the_lock_hands_an_update_over_at_once() {
        let Some(case) = Case::new(|port, work| Setup {
            manifest: manifest_text(port, "1.0.0"),
            repository: repository(vec![package(port, "1.0.0", Some(&ordinary(work)))]),
            lock: Some((
                vec![package(port, "1.0.0", Some(&dot_slash_slash(work)))],
                vec![],
            )),
            installed: vec![],
            path_package: false,
        }) else {
            return;
        };
        assert_handed_over(&case, &line(Run::Update), None, &[]);
    }

    /// `require` reaches the same point of the session, its edit put back or held.
    #[test]
    fn require_hands_over_a_selected_form() {
        let Some(case) = Case::new(|port, work| Setup {
            manifest: manifest_text(port, "1.0.0")
                .replace("\"require\": {\n    \"acme/tool\": \"1.0.0\"\n  },\n", ""),
            repository: repository(vec![package(port, "1.0.0", Some(&dot_slash_slash(work)))]),
            lock: None,
            installed: vec![],
            path_package: false,
        }) else {
            return;
        };
        assert_handed_over(
            &case,
            &[
                "require",
                "acme/tool:1.0.0",
                "--no-interaction",
                "--no-scripts",
                "--no-audit",
                "--no-progress",
            ],
            None,
            &[
                "./composer.json has been updated",
                "Running composer update acme/tool",
                LATE[0],
                LATE[1],
                LATE[2],
            ],
        );
    }

    /// `remove` starts from the same read of the lock and installed.json.
    #[test]
    fn remove_hands_over_a_form_in_the_lock() {
        let Some(case) = Case::new(|port, work| Setup {
            manifest: manifest_text(port, "1.0.0"),
            repository: repository(vec![package(port, "1.0.0", Some(&dot_slash_slash(work)))]),
            lock: Some((
                vec![package(port, "1.0.0", Some(&dot_slash_slash(work)))],
                vec![],
            )),
            installed: vec![],
            path_package: false,
        }) else {
            return;
        };
        assert_handed_over(
            &case,
            &[
                "remove",
                "acme/tool",
                "--no-interaction",
                "--no-scripts",
                "--no-audit",
                "--no-progress",
            ],
            None,
            &[],
        );
    }

    /// `--dry-run` writes nothing on either side, and goes the same way.
    #[test]
    fn a_dry_run_hands_over_too() {
        let Some(case) = Case::of_form(Run::Install, dot_slash_slash) else {
            return;
        };
        assert_handed_over(
            &case,
            &[
                "install",
                "--dry-run",
                "--no-interaction",
                "--no-scripts",
                "--no-progress",
            ],
            None,
            &[],
        );
    }

    /// `install --no-dev` does not read the dev section (`getLockedRepository`):
    /// a form there stays out of the run, for the phar and for vivacity.
    #[test]
    fn a_dev_package_is_not_read_under_no_dev() {
        let Some(case) = Case::new(|port, work| {
            let mut dep = package(port, "1.0.0", Some(&dot_slash_slash(work)));
            dep["name"] = json!("acme/devtool");
            Setup {
                manifest: manifest_text(port, "1.0.0"),
                repository: repository(vec![package(port, "1.0.0", Some(&ordinary(work)))]),
                lock: Some((
                    vec![package(port, "1.0.0", Some(&ordinary(work)))],
                    vec![dep],
                )),
                installed: vec![],
                path_package: false,
            }
        }) else {
            return;
        };
        let mut args = line(Run::Install);
        args.push("--no-dev");
        assert_native(&case, &args, None, false);
    }
}

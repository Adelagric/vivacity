//! The forms of a package's `bin`, against the 2.10.3 phar rather than against
//! written expectations: each case runs Composer, then vivacity under the same
//! absolute path, and compares what they leave and what they print.
//!
//! A form that is not ordinary goes to Composer before any write; under
//! `--no-fallback` it stops with exit 3 and nothing is written. A form known
//! only after the extraction (a link leading out, a directory) stays native:
//! neither a proxy nor a `chmod`, and the phar's skip line.
//!
//! Both sides run with `--no-progress`: these comparisons do not cover the phar's
//! progress bar. vivacity prints none, a difference the CHANGELOG records.
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
    Link(&'static str, &'static str),
}

/// What every dist holds: files at the paths the forms name, not executable, so
/// that a `chmod` shows.
const PACKAGE: &[Entry] = &[
    Entry::File("composer.json", "{\"name\": \"acme/tool\"}\n"),
    Entry::File("bin/x", "#!/usr/bin/env php\n<?php echo 'x';\n"),
    Entry::File("other/x", "#!/usr/bin/env php\n<?php echo 'other';\n"),
    Entry::File("x", "#!/usr/bin/env php\n<?php echo 'root';\n"),
    Entry::File("a/b", "#!/usr/bin/env php\n<?php echo 'b';\n"),
    Entry::File("bin/x.bat", "@echo off\n"),
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
            Entry::Link(p, target) => w.add_symlink(at(p), *target, options).expect("link"),
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
                (
                    "/link.zip".to_owned(),
                    zip_of(&[
                        Entry::File("composer.json", "{\"name\": \"acme/tool\"}\n"),
                        // From vendor/acme/tool/bin/, up to where the witness is.
                        Entry::Link("bin/out", "../../../../../witness"),
                    ]),
                ),
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
        self.run_with(program, args, &[])
    }

    fn run_with(&self, program: &Path, args: &[&str], vars: &[(&str, &str)]) -> Output {
        let mut cmd = clean(Command::new(program));
        let out = cmd
            .current_dir(self.project())
            .env("COMPOSER_HOME", self.env.join("home"))
            .env("COMPOSER_CACHE_DIR", self.env.join("cache"))
            .env("VIVACITY_CACHE_DIR", self.env.join("store"))
            .envs(vars.iter().copied())
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

/// vivacity's closing line, its own, out of its stderr.
fn without_own_line(stderr: &str) -> String {
    stderr
        .lines()
        .filter(|l| !l.starts_with("vivacity: "))
        .map(|l| format!("{l}\n"))
        .collect()
}

/// A form vivacity keeps: stays native under `--no-fallback`, same exit, same
/// tree, the witness untouched, the phar's lines. Its closing line is its own.
fn assert_native(case: &Case, args: &[&str], form: Option<Form>) {
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
    assert_eq!(
        without_own_line(&viv.stderr),
        phar.stderr,
        "stderr\n{context}"
    );
    let tree = differences(&phar_tree, &inventory(&case.work));
    assert!(tree.is_empty(), "tree\n{}\n{context}", tree.join("\n"));
}

fn native(run: Run, form: Form) {
    let Some(case) = Case::of_form(run, form) else {
        return;
    };
    assert_native(&case, &line(run), None);
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

/// A directory on an install with nothing to do: only the pass after the dump speaks,
/// on both sides.
#[test]
fn a_directory_on_a_no_op_install() {
    let Some(case) = Case::of_form(Run::Install, directory) else {
        return;
    };
    let args = line(Run::Install);
    case.reset(None);
    assert_eq!(case.run(case.composer, &args).code, 0);
    let phar = case.run(case.composer, &args);
    assert!(
        phar.stderr.contains(": found a directory at that path"),
        "{}",
        dump("phar", &phar)
    );
    let phar_tree = inventory(&case.work);

    case.reset(None);
    assert_eq!(case.vivacity(&args).code, 0);
    let viv = case.vivacity(&args);
    let context = format!("{}\n{}", dump("phar", &phar), dump("vivacity", &viv));
    assert_eq!(viv.code, phar.code, "{context}");
    assert_eq!(without_own_line(&viv.stderr), phar.stderr, "{context}");
    let tree = differences(&phar_tree, &inventory(&case.work));
    assert!(tree.is_empty(), "tree\n{}", tree.join("\n"));
}

/// OPEN, by the design of this fix: a `bin` the dist does not hold stays silent as in
/// 0.26.2. The phar writes the line at the operation and after the dump. Everything else
/// is the same.
mod missing_form {
    use super::*;
    fn silent(run: Run) {
        let Some(case) = Case::of_form(run, missing) else {
            return;
        };
        let (phar, phar_tree) = phar_run(&case, &line(run), None);
        let notice = "    Skipped installation of bin bin/nope for package acme/tool: file not found in package\n";
        assert_eq!(
            phar.stderr.matches(notice).count(),
            2,
            "{}",
            dump("phar", &phar)
        );
        case.reset(None);
        let mut args = line(run);
        args.push("--no-fallback");
        let viv = case.vivacity(&args);
        assert_eq!(viv.code, phar.code, "{}", dump("vivacity", &viv));
        assert_eq!(
            without_own_line(&viv.stderr),
            phar.stderr.replace(notice, "")
        );
        let tree = differences(&phar_tree, &inventory(&case.work));
        assert!(tree.is_empty(), "tree\n{}", tree.join("\n"));
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
        assert_native(&case, &line(run), Some(link_out));
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
        assert_native(&case, &line(Run::Update), None);
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
        assert_native(&case, &line(Run::Update), None);
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
        assert_native(&case, &args, None);
    }
}

/// The lines of a stderr, sorted within each run of skip lines: all that several zip dists
/// make vary.
fn blocks_sorted(stderr: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut block: Vec<String> = Vec::new();
    for l in stderr.lines() {
        if l.starts_with("    Skipped installation of bin ") {
            block.push(l.to_owned());
        } else {
            block.sort();
            out.append(&mut block);
            out.push(l.to_owned());
        }
    }
    block.sort();
    out.append(&mut block);
    out
}

/// Skip lines of several packages: the operations, then the pass after the dump.
mod several_packages {
    use super::*;

    const NAMES: [&str; 3] = ["ccc", "aaa", "bbb"];

    /// Three zip dists, each `bin` a directory.
    fn zip_case() -> Option<Case> {
        Case::new(|port, work| {
            let mut by_name = serde_json::Map::new();
            let mut require = serde_json::Map::new();
            for n in NAMES {
                let mut p = package(port, "1.0.0", Some(&directory(work)));
                p["name"] = json!(format!("acme/{n}"));
                by_name.insert(format!("acme/{n}"), json!({"1.0.0": p}));
                require.insert(format!("acme/{n}"), json!("1.0.0"));
            }
            let mut manifest: Value =
                serde_json::from_str(&manifest_text(port, "1.0.0")).expect("manifest");
            manifest["require"] = Value::Object(require);
            Setup {
                manifest: serde_json::to_string_pretty(&manifest).expect("manifest"),
                repository: json!({"packages": by_name}),
                lock: None,
                installed: vec![],
                path_package: false,
            }
        })
    }

    /// Nothing to do: the local repository's order, the file's. The phar does not vary.
    #[test]
    fn a_no_op_install_is_the_same_byte_for_byte() {
        let Some(case) = zip_case() else {
            return;
        };
        let update = line(Run::Update);
        let install = line(Run::Install);
        case.reset(None);
        assert_eq!(case.run(case.composer, &update).code, 0);
        let phar = case.run(case.composer, &install);
        case.reset(None);
        let mut stopping = update.clone();
        stopping.push("--no-fallback");
        assert_eq!(case.vivacity(&stopping).code, 0);
        let viv = case.vivacity(&install);
        assert_eq!(
            without_own_line(&viv.stderr),
            phar.stderr,
            "{}\n{}",
            dump("phar", &phar),
            dump("vivacity", &viv)
        );
    }

    /// OPEN, the phar varies on one input: several zip dists are extracted in the
    /// background, and both its skip lines and the local repository follow the order the
    /// extractions end in. 15 runs on one input gave aaa, bbb, ccc in 9 and bbb, ccc, aaa
    /// in 6. vivacity prints the operations' order, one of the phar's. So the lines are
    /// compared as a set for each of the two blocks.
    #[test]
    fn placed_zip_dists_are_the_same_up_to_the_order_the_phar_varies_on() {
        let Some(case) = zip_case() else {
            return;
        };
        let args = line(Run::Update);
        let (phar, phar_tree) = phar_run(&case, &args, None);
        case.reset(None);
        let viv = case.vivacity(&args);
        let context = format!("{}\n{}", dump("phar", &phar), dump("vivacity", &viv));
        assert_eq!(viv.code, phar.code, "{context}");
        assert_eq!(
            blocks_sorted(&without_own_line(&viv.stderr)),
            blocks_sorted(&phar.stderr),
            "{context}"
        );
        let tree = differences(&phar_tree, &inventory(&case.work));
        assert!(tree.is_empty(), "tree\n{}", tree.join("\n"));
    }

    /// `path` packages are placed in line: the phar does not vary (8 runs of 8), each
    /// line falls under its operation, and the second pass follows the operations.
    #[test]
    fn path_packages_are_the_same_byte_for_byte() {
        for symlink in [true, false] {
            let Some(case) = Case::new(|_, _| Setup {
                manifest: serde_json::to_string_pretty(&json!({
                    "name": "acme/root",
                    "version": "1.0.0",
                    "repositories": [
                        {"type": "path", "url": "../pk/*", "options": {"symlink": symlink}},
                        {"packagist.org": false}
                    ],
                    "require": {"acme/aaa": "1.0.0"}
                }))
                .expect("manifest"),
                repository: json!({"packages": {}}),
                lock: None,
                installed: vec![],
                path_package: false,
            }) else {
                return;
            };
            let packages = |case: &Case| {
                for (name, require) in [
                    ("zzz", json!({"acme/mmm": "1.0.0"})),
                    ("mmm", json!({})),
                    ("aaa", json!({"acme/zzz": "1.0.0"})),
                ] {
                    let dir = case.work.join("pk").join(name);
                    std::fs::create_dir_all(dir.join("dir")).expect("package");
                    std::fs::write(
                        dir.join("composer.json"),
                        json!({
                            "name": format!("acme/{name}"),
                            "version": "1.0.0",
                            "bin": ["dir"],
                            "require": require
                        })
                        .to_string(),
                    )
                    .expect("composer.json");
                }
            };
            let update = line(Run::Update);
            let install = line(Run::Install);
            case.reset(None);
            packages(&case);
            let phar = case.run(case.composer, &update);
            let phar_again = case.run(case.composer, &install);
            let phar_tree = inventory(&case.work);
            case.reset(None);
            packages(&case);
            let mut stopping = update.clone();
            stopping.push("--no-fallback");
            let viv = case.vivacity(&stopping);
            let context = format!("{}\n{}", dump("phar", &phar), dump("vivacity", &viv));
            assert_eq!(viv.code, phar.code, "{context}");
            assert_eq!(without_own_line(&viv.stderr), phar.stderr, "{context}");
            let viv_again = case.vivacity(&install);
            let context = format!(
                "{}\n{}",
                dump("phar", &phar_again),
                dump("vivacity", &viv_again)
            );
            assert_eq!(
                without_own_line(&viv_again.stderr),
                phar_again.stderr,
                "{context}"
            );
            let tree = differences(&phar_tree, &inventory(&case.work));
            assert!(tree.is_empty(), "tree\n{}", tree.join("\n"));
        }
    }

    /// `--no-autoloader`: the pass runs without a dump (`Installer::run`).
    #[test]
    fn without_a_dump_the_pass_still_speaks() {
        let Some(case) = Case::of_form(Run::Install, directory) else {
            return;
        };
        let mut args = line(Run::Install);
        args.push("--no-autoloader");
        assert_native(&case, &args, None);
    }
}

/// Differences that stay, each with its reason. Each test holds the phar's side and
/// vivacity's, so that a change on either shows.
mod open {
    use super::*;

    /// A branch alias is a second package of the phar's local repository, whose
    /// `getBinaries` is the aliased one's: the pass after the dump writes its line twice.
    /// vivacity has one entry per package there: telling aliases apart needs the alias
    /// rules (`extra.branch-alias`, the lock's `aliases`), more than this pass is worth.
    #[test]
    fn a_branch_alias_doubles_the_phars_line_after_the_dump() {
        let Some(case) = Case::new(|_, _| Setup {
            manifest: serde_json::to_string_pretty(&json!({
                "name": "acme/root",
                "version": "1.0.0",
                "minimum-stability": "dev",
                "repositories": [{"type": "path", "url": "../pkg"}, {"packagist.org": false}],
                "require": {"acme/tool": "*@dev"}
            }))
            .expect("manifest"),
            repository: json!({"packages": {}}),
            lock: None,
            installed: vec![],
            path_package: false,
        }) else {
            return;
        };
        let place = |case: &Case| {
            let dir = case.work.join("pkg");
            std::fs::create_dir_all(dir.join("dir")).expect("pkg");
            std::fs::write(
                dir.join("composer.json"),
                json!({
                    "name": "acme/tool",
                    "version": "dev-main",
                    "extra": {"branch-alias": {"dev-main": "1.x-dev"}},
                    "bin": ["dir"]
                })
                .to_string(),
            )
            .expect("composer.json");
        };
        let notice =
            "    Skipped installation of bin dir for package acme/tool: found a directory at that path\n";
        let args = line(Run::Update);
        case.reset(None);
        place(&case);
        let phar = case.run(case.composer, &args);
        assert_eq!(
            phar.stderr.matches(notice).count(),
            3,
            "{}",
            dump("phar", &phar)
        );
        case.reset(None);
        place(&case);
        let viv = case.vivacity(&args);
        assert_eq!(
            viv.stderr.matches(notice).count(),
            2,
            "{}",
            dump("vivacity", &viv)
        );
    }

    /// An inline alias, the same: the phar's local repository holds the alias too, so its
    /// pass after the dump writes the line twice, on a no-op install too.
    #[test]
    fn an_inline_alias_doubles_the_phars_line_after_the_dump() {
        let Some(case) = Case::new(|port, _| Setup {
            manifest: mixed_manifest(port),
            repository: json!({"packages": {}}),
            lock: None,
            installed: vec![],
            path_package: false,
        }) else {
            return;
        };
        let place = |case: &Case, _: usize| {
            write_path_package(
                case,
                "a",
                json!({"name": "acme/a", "version": "dev-main", "bin": ["dir"]}),
            );
        };
        let notice =
            "    Skipped installation of bin dir for package acme/a: found a directory at that path\n";
        let steps = [
            Step {
                require: Some(&["a:dev-main as 1.0.x-dev"]),
                args: line(Run::Update),
            },
            Step {
                require: None,
                args: line(Run::Install),
            },
        ];
        let phar = case.sequence(case.composer, false, &place, &steps);
        assert_eq!(
            phar[0].stderr.matches(notice).count(),
            3,
            "{}",
            phar[0].stderr
        );
        assert_eq!(
            phar[1].stderr.matches(notice).count(),
            2,
            "{}",
            phar[1].stderr
        );
        let viv = case.sequence(
            Path::new(env!("CARGO_BIN_EXE_vivacity")),
            true,
            &place,
            &steps,
        );
        assert_eq!(
            viv[0].stderr.matches(notice).count(),
            2,
            "{}",
            viv[0].stderr
        );
        assert_eq!(
            viv[1].stderr.matches(notice).count(),
            1,
            "{}",
            viv[1].stderr
        );
    }

    /// Two `path` packages with one proxy name, placed by one run: on the phar the first
    /// operation holds it (z, which a requires), here the first of the lock (a): another
    /// proxy target in `vendor/bin`. As in 0.26.2. `install` knows the lock's order, not
    /// the operations': passing them on is another change than this fix. With zip dists,
    /// the phar varies.
    #[test]
    fn two_packages_placed_with_one_proxy_name_follow_the_lock_here() {
        let Some(case) = Case::new(|port, _| Setup {
            manifest: mixed_manifest(port),
            repository: json!({"packages": {}}),
            lock: None,
            installed: vec![],
            path_package: false,
        }) else {
            return;
        };
        let place = |case: &Case, _: usize| {
            for (name, require) in [("a", json!({"acme/z": "*"})), ("z", json!({}))] {
                write_path_package(
                    case,
                    name,
                    json!({"name": format!("acme/{name}"), "version": "1.0.0",
                           "bin": ["bin/x"], "require": require}),
                );
                let bin = case.work.join("pk").join(name).join("bin");
                std::fs::create_dir_all(&bin).expect("bin");
                std::fs::write(bin.join("x"), "#!/usr/bin/env php\n<?php echo 1;\n").expect("x");
            }
        };
        let steps = [Step {
            require: Some(&["a"]),
            args: line(Run::Update),
        }];
        let proxy = |case: &Case| {
            std::fs::read_to_string(case.project().join("vendor/bin/x")).expect("proxy")
        };
        let phar = case.sequence(case.composer, false, &place, &steps);
        assert!(
            proxy(&case).contains("../acme/z/bin/x"),
            "{}",
            phar[0].stderr
        );
        let viv = case.sequence(
            Path::new(env!("CARGO_BIN_EXE_vivacity")),
            true,
            &place,
            &steps,
        );
        assert!(
            proxy(&case).contains("../acme/a/bin/x"),
            "{}",
            viv[0].stderr
        );
    }

    /// A zip dist with a link leading out: vivacity's extraction refuses the archive, the
    /// phar's keeps the link and its `bin` check skips it. Before this fix too, and it
    /// is the safe side: nothing is written.
    #[test]
    fn a_link_out_of_a_zip_is_refused_where_the_phar_skips_it() {
        let Some(case) = Case::new(|port, _| {
            let mut p = package(port, "1.0.0", Some(&json!(["bin/out"])));
            p["dist"]["url"] = json!(format!("http://127.0.0.1:{port}/link.zip"));
            Setup {
                manifest: manifest_text(port, "1.0.0"),
                repository: repository(vec![p.clone()]),
                lock: Some((vec![p], vec![])),
                installed: vec![],
                path_package: false,
            }
        }) else {
            return;
        };
        let args = line(Run::Install);
        let (phar, _) = phar_run(&case, &args, None);
        assert_eq!(phar.code, 0, "{}", dump("phar", &phar));
        assert!(phar
            .stderr
            .contains(": the bin resolves to a path outside of the package directory"));
        case.reset(None);
        let viv = case.vivacity(&args);
        assert_eq!(viv.code, 1, "{}", dump("vivacity", &viv));
        assert!(
            viv.stderr.contains("hostile archive refused"),
            "{}",
            dump("vivacity", &viv)
        );
        assert_eq!(case.witness_mode(), 0o600);
    }

    /// `COMPOSER_BIN_COMPAT=full`, a `.bat` already there for a second `bin`: the phar says
    /// so, vivacity keeps the file and says nothing. Same tree; before this fix too.
    #[test]
    fn a_bat_proxy_already_there_is_silent_here() {
        let Some(case) = Case::of_form(Run::Install, |_| json!(["bin/x.bat", "bin/x"])) else {
            return;
        };
        let args = line(Run::Install);
        let vars = [("COMPOSER_BIN_COMPAT", "full")];
        case.reset(None);
        let phar = case.run_with(case.composer, &args, &vars);
        let phar_tree = inventory(&case.work);
        let bat = "a .bat proxy was already installed";
        assert!(phar.stderr.contains(bat), "{}", dump("phar", &phar));
        case.reset(None);
        let viv = case.run_with(Path::new(env!("CARGO_BIN_EXE_vivacity")), &args, &vars);
        assert_eq!(viv.code, phar.code, "{}", dump("vivacity", &viv));
        assert!(!viv.stderr.contains(bat), "{}", dump("vivacity", &viv));
        let tree = differences(&phar_tree, &inventory(&case.work));
        assert!(tree.is_empty(), "tree\n{}", tree.join("\n"));
    }

    /// `update`, when only the selected metadata shows the form. The phar's lines come
    /// after what vivacity already wrote: this is the known late hand-over, which every
    /// hand-over after the resolution has unless `hold_trigger` holds the output, as the
    /// next test shows on a case older than this fix. Holding until the solver would take
    /// the streaming away from ordinary runs, whose `Loading…` comes before the network.
    #[test]
    fn a_selected_form_is_handed_over_late() {
        let Some(case) = Case::of_form(Run::Update, dot_slash_slash) else {
            return;
        };
        let args = line(Run::Update);
        case.reset(None);
        let phar = case.run(case.composer, &args);
        case.reset(None);
        let viv = case.vivacity(&args);
        for l in LATE {
            assert_eq!(phar.stderr.lines().filter(|x| x == l).count(), 1, "{l}");
            assert_eq!(
                viv.stderr.lines().filter(|x| x == l).count(),
                2,
                "{l}\n{}",
                viv.stderr
            );
        }
    }

    /// The same class, on a reason that is not `bin`: a plugin the lock would install and
    /// `allow-plugins` does not cover, decided after the resolution (`plugin_allow`).
    #[test]
    fn a_blocked_plugin_is_handed_over_as_late() {
        let Some(case) = Case::new(|port, _| {
            let mut p = package(port, "1.0.0", None);
            p["type"] = json!("composer-plugin");
            p["require"] = json!({"composer-plugin-api": "^2.0"});
            p["extra"] = json!({"class": "Acme\\Plugin"});
            Setup {
                manifest: manifest_text(port, "1.0.0"),
                repository: repository(vec![p]),
                lock: None,
                installed: vec![],
                path_package: false,
            }
        }) else {
            return;
        };
        assert_handed_over(&case, &line(Run::Update), None, LATE);
    }
}

/// One step of a sequence: what `require` becomes (`name` or `name:constraint`), if it
/// changes, then a command line.
struct Step<'a> {
    require: Option<&'a [&'a str]>,
    args: Vec<&'a str>,
}

impl Case {
    fn write_requires(&self, names: &[&str]) {
        let require: serde_json::Map<String, Value> = names
            .iter()
            .map(|n| {
                let (name, constraint) = n.split_once(':').unwrap_or((n, "*"));
                (format!("acme/{name}"), json!(constraint))
            })
            .collect();
        let path = self.project().join("composer.json");
        let mut manifest: Value =
            serde_json::from_str(&std::fs::read_to_string(&path).expect("read")).expect("json");
        manifest["require"] = Value::Object(require);
        std::fs::write(
            &path,
            serde_json::to_string_pretty(&manifest).expect("json"),
        )
        .expect("write");
    }

    /// Runs the steps on `program` from a reset; a step other than the last may fail on
    /// neither side. The last output, each output's lines included.
    fn sequence(
        &self,
        program: &Path,
        native: bool,
        prepare: &dyn Fn(&Case, usize),
        steps: &[Step<'_>],
    ) -> Vec<Output> {
        self.reset(None);
        let mut outputs = Vec::new();
        for (i, step) in steps.iter().enumerate() {
            prepare(self, i);
            if let Some(names) = step.require {
                self.write_requires(names);
            }
            let mut args = step.args.clone();
            if native {
                args.push("--no-fallback");
            }
            let out = self.run(program, &args);
            assert_eq!(out.code, 0, "step {i} of {}", dump("run", &out));
            outputs.push(out);
        }
        outputs
    }
}

/// A repository of zip dists: name, `bin`.
fn repository_of(port: u16, packages: &[(&str, Value)]) -> Value {
    let by_name: serde_json::Map<String, Value> = packages
        .iter()
        .map(|(name, bin)| {
            let mut p = package(port, "1.0.0", Some(bin));
            p["name"] = json!(format!("acme/{name}"));
            (format!("acme/{name}"), json!({"1.0.0": p}))
        })
        .collect();
    json!({"packages": by_name})
}

/// The manifest of a case whose packages come from the zip repository and from `../pk/*`.
fn mixed_manifest(port: u16) -> String {
    serde_json::to_string_pretty(&json!({
        "name": "acme/root",
        "version": "1.0.0",
        "minimum-stability": "dev",
        "repositories": [
            {"type": "path", "url": "../pk/*"},
            {"type": "composer", "url": format!("http://127.0.0.1:{port}")},
            {"packagist.org": false}
        ],
        "require": {},
        "config": {"secure-http": false}
    }))
    .expect("manifest")
}

/// A `path` package under ../pk/, its `bin` a directory, as `composer.json` says.
fn write_path_package(case: &Case, name: &str, manifest: Value) {
    let dir = case.work.join("pk").join(name);
    std::fs::create_dir_all(dir.join("dir")).expect("package");
    std::fs::write(dir.join("composer.json"), manifest.to_string()).expect("composer.json");
}

fn plain_path_package(case: &Case, name: &str, version: &str) {
    write_path_package(
        case,
        name,
        json!({"name": format!("acme/{name}"), "version": version, "bin": ["dir"]}),
    );
}

const BIN_X: fn() -> Value = || json!(["bin/x"]);

/// Every step's stderr and exit, and the tree at the end, against the phar's.
fn assert_same_sequence(case: &Case, prepare: &dyn Fn(&Case, usize), steps: &[Step<'_>]) {
    let phar = case.sequence(case.composer, false, prepare, steps);
    let phar_tree = inventory(&case.work);
    let viv = case.sequence(
        Path::new(env!("CARGO_BIN_EXE_vivacity")),
        true,
        prepare,
        steps,
    );
    for (i, (p, v)) in phar.iter().zip(&viv).enumerate() {
        let context = format!("step {i}\n{}\n{}", dump("phar", p), dump("vivacity", v));
        assert_eq!(without_own_line(&v.stderr), p.stderr, "{context}");
        assert_eq!(v.stdout, p.stdout, "{context}");
    }
    let tree = differences(&phar_tree, &inventory(&case.work));
    assert!(tree.is_empty(), "tree\n{}", tree.join("\n"));
}

/// Two packages with one proxy name, one going away. The phar removes the proxy with
/// the package that goes, then writes it again for the one that stays, in the pass after
/// the dump, or a package arriving takes it first. Removals come first in its operations.
mod shared_proxy_name {
    use super::*;

    const UPDATE: [&str; 5] = [
        "update",
        "--no-interaction",
        "--no-scripts",
        "--no-audit",
        "--no-progress",
    ];

    /// a alone first and b after it, so that a holds the name: with both at once, which
    /// of two extractions ends first decides, on the phar.
    fn case_of(a: Value) -> Option<Case> {
        let a = a.clone();
        Case::new(move |port, _| Setup {
            manifest: manifest_text(port, "*"),
            repository: repository_of(port, &[("a", a.clone()), ("b", BIN_X()), ("c", BIN_X())]),
            lock: None,
            installed: vec![],
            path_package: false,
        })
    }

    fn removing(a: Value) {
        let Some(case) = case_of(a) else {
            return;
        };
        assert_same_sequence(
            &case,
            &|_, _| {},
            &[
                Step {
                    require: Some(&["a"]),
                    args: UPDATE.to_vec(),
                },
                Step {
                    require: Some(&["a", "b"]),
                    args: UPDATE.to_vec(),
                },
                Step {
                    require: None,
                    args: vec![
                        "remove",
                        "acme/b",
                        "--no-interaction",
                        "--no-scripts",
                        "--no-audit",
                        "--no-progress",
                    ],
                },
            ],
        );
    }

    fn dropping(a: Value) {
        let Some(case) = case_of(a) else {
            return;
        };
        assert_same_sequence(
            &case,
            &|_, _| {},
            &[
                Step {
                    require: Some(&["a"]),
                    args: UPDATE.to_vec(),
                },
                Step {
                    require: Some(&["a", "b"]),
                    args: UPDATE.to_vec(),
                },
                Step {
                    require: Some(&["a"]),
                    args: UPDATE.to_vec(),
                },
            ],
        );
    }

    #[test]
    fn remove_keeps_the_proxy_for_the_package_that_stays() {
        removing(BIN_X());
    }

    #[test]
    fn remove_with_a_directory_too() {
        removing(json!(["bin/x", "dir"]));
    }

    #[test]
    fn update_keeps_the_proxy_for_the_package_that_stays() {
        dropping(BIN_X());
    }

    #[test]
    fn update_with_a_directory_too() {
        dropping(json!(["bin/x", "dir"]));
    }

    /// b goes and c arrives in one run: c takes the name, without a conflict line.
    #[test]
    fn a_package_arriving_takes_the_name_of_the_one_going() {
        let Some(case) = case_of(BIN_X()) else {
            return;
        };
        assert_same_sequence(
            &case,
            &|_, _| {},
            &[
                Step {
                    require: Some(&["a"]),
                    args: UPDATE.to_vec(),
                },
                Step {
                    require: Some(&["a", "b"]),
                    args: UPDATE.to_vec(),
                },
                Step {
                    require: Some(&["a", "c"]),
                    args: UPDATE.to_vec(),
                },
            ],
        );
    }

    /// vendor/bin wiped and c arriving: c is placed first, a's presence check finds the name
    /// taken, and the pass after the dump says nothing of a conflict.
    #[test]
    fn a_placed_package_takes_the_name_before_the_presence_check() {
        let Some(case) = case_of(BIN_X()) else {
            return;
        };
        assert_same_sequence(
            &case,
            &|case: &Case, step| {
                if step == 1 {
                    std::fs::remove_dir_all(case.project().join("vendor/bin")).expect("wipe");
                }
            },
            &[
                Step {
                    require: Some(&["a"]),
                    args: UPDATE.to_vec(),
                },
                Step {
                    require: Some(&["a", "c"]),
                    args: UPDATE.to_vec(),
                },
            ],
        );
    }
}

/// `path` packages and zip dists in one project. A package enters the phar's local
/// repository when its placement ends: a `path` package in line, an archive when its
/// extraction ends, so the first before the second, whatever the operations' order.
mod path_and_zip {
    use super::*;

    fn case_of(zips: &'static [&'static str]) -> Option<Case> {
        Case::new(move |port, _| {
            let packages: Vec<(&str, Value)> = zips.iter().map(|n| (*n, json!(["dir"]))).collect();
            Setup {
                manifest: mixed_manifest(port),
                repository: repository_of(port, &packages),
                lock: None,
                installed: vec![],
                path_package: false,
            }
        })
    }

    /// Zip a, path p: p first once placed, by name on a no-op install, p first from the
    /// lock. Does not vary on the phar (6 runs of 6, 8 of 8 on the sequences below).
    #[test]
    fn a_path_package_comes_before_a_zip_dist() {
        let Some(case) = case_of(&["a"]) else {
            return;
        };
        let prepare = |case: &Case, step: usize| {
            if step == 0 {
                plain_path_package(case, "p", "1.0.0");
            }
            if step == 2 {
                std::fs::remove_dir_all(case.project().join("vendor")).expect("vendor");
            }
        };
        assert_same_sequence(
            &case,
            &prepare,
            &[
                Step {
                    require: Some(&["a", "p"]),
                    args: line(Run::Update),
                },
                Step {
                    require: None,
                    args: line(Run::Install),
                },
                Step {
                    require: None,
                    args: line(Run::Install),
                },
            ],
        );
    }

    /// Packages arriving next to installed ones, a `path` package upgraded.
    #[test]
    fn arrivals_follow_the_installed_ones() {
        let Some(case) = case_of(&["a", "c"]) else {
            return;
        };
        let prepare = |case: &Case, step: usize| match step {
            0 => {
                plain_path_package(case, "p", "1.0.0");
                plain_path_package(case, "q", "1.0.0");
            }
            2 => plain_path_package(case, "p", "2.0.0"),
            _ => {}
        };
        assert_same_sequence(
            &case,
            &prepare,
            &[
                Step {
                    require: Some(&["a", "p"]),
                    args: line(Run::Update),
                },
                Step {
                    require: Some(&["a", "p", "c", "q"]),
                    args: line(Run::Update),
                },
                Step {
                    require: None,
                    args: line(Run::Update),
                },
            ],
        );
    }

    /// A zip dist upgraded while a `path` package arrives, then the reverse.
    #[test]
    fn upgrades_are_arrivals_too() {
        let Some(case) = Case::new(|port, _| {
            let mut c = package(port, "1.0.0", Some(&json!(["dir"])));
            c["name"] = json!("acme/c");
            let mut a1 = package(port, "1.0.0", Some(&json!(["dir"])));
            a1["name"] = json!("acme/a");
            let mut a2 = package(port, "2.0.0", Some(&json!(["dir"])));
            a2["name"] = json!("acme/a");
            Setup {
                manifest: mixed_manifest(port),
                repository: json!({"packages": {
                    "acme/a": {"1.0.0": a1, "2.0.0": a2},
                    "acme/c": {"1.0.0": c}
                }}),
                lock: None,
                installed: vec![],
                path_package: false,
            }
        }) else {
            return;
        };
        let prepare = |case: &Case, step: usize| match step {
            0 => {
                plain_path_package(case, "p", "1.0.0");
                plain_path_package(case, "q", "1.0.0");
            }
            2 => plain_path_package(case, "p", "2.0.0"),
            _ => {}
        };
        assert_same_sequence(
            &case,
            &prepare,
            &[
                Step {
                    require: Some(&["a:1.0.0", "p"]),
                    args: line(Run::Update),
                },
                Step {
                    require: Some(&["a:2.0.0", "p", "q"]),
                    args: line(Run::Update),
                },
                Step {
                    require: Some(&["a:2.0.0", "p", "q", "c"]),
                    args: line(Run::Update),
                },
            ],
        );
    }

    /// Two zip dists and a `path` package: p first, and among the zips the phar varies, so
    /// they are compared as a set (p, a, b in 5 and p, b, a in 3 of 8 runs).
    #[test]
    fn several_zip_dists_stay_a_set_after_the_path_package() {
        let Some(case) = case_of(&["a", "b"]) else {
            return;
        };
        let prepare = |case: &Case, step: usize| {
            if step == 0 {
                plain_path_package(case, "p", "1.0.0");
            }
        };
        let steps = [Step {
            require: Some(&["a", "b", "p"]),
            args: line(Run::Update),
        }];
        let phar = case.sequence(case.composer, false, &prepare, &steps);
        let viv = case.sequence(
            Path::new(env!("CARGO_BIN_EXE_vivacity")),
            true,
            &prepare,
            &steps,
        );
        let viv_err = without_own_line(&viv[0].stderr);
        for stderr in [&phar[0].stderr, &viv_err] {
            let after_dump = stderr
                .split_once("Generating autoload files\n")
                .map_or("", |(_, rest)| rest);
            assert!(
                after_dump.starts_with("    Skipped installation of bin dir for package acme/p: "),
                "{stderr}"
            );
        }
        assert_eq!(blocks_sorted(&viv_err), blocks_sorted(&phar[0].stderr));
    }
}

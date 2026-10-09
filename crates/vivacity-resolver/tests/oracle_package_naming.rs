//! `package_naming_error` against the oracle: the same names through
//! `ValidatingArrayLoader::hasPackageNamingError` inside the phar, for both
//! values of `$isLink`. The corpus is the five rules and their edges, plus
//! every link name of every fixture's root manifest — those must all come
//! back error-free, since a false positive would refuse a real project.
//!
//! Prerequisite: `composer` on the PATH (dev/CI). Missing it FAILS the test.
#[path = "../../../tools/oracle_phar.rs"]
mod oracle_phar;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use vivacity_resolver::lockfile::package_naming_error;

fn phar() -> PathBuf {
    oracle_phar::composer_phar()
}

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// The names the five rules turn on, and their edges.
fn crafted() -> Vec<String> {
    [
        // platform packages: exempt, uppercase included
        "php",
        "PHP",
        "php-64bit",
        "php-ipv6",
        "php-zts",
        "php-debug",
        "hhvm",
        "ext-json",
        "ext-Json",
        "EXT-json",
        "lib-curl",
        "lib-icu",
        "composer",
        "composer-plugin-api",
        "composer-runtime-api",
        "ext-",
        "lib-",
        "composer-nope",
        "phpx",
        // shape
        "acme/lib",
        "a/b",
        "a0/b0",
        "a.b/c.d",
        "a_b/c_d",
        "a-b/c-d",
        "acme/lib--x",
        "acme/lib---x",
        "a--b/c",
        "a/b/c",
        "a/",
        "/b",
        "",
        "a",
        "acme/lib ",
        " acme/lib",
        "acme/lib\n",
        "-acme/lib",
        "acme-/lib",
        "acme/-lib",
        "acme/lib-",
        "a/b.",
        "a/.b",
        "a/b_",
        "a/_b",
        // reserved, vendor or package, any case
        "con/lib",
        "CON/lib",
        "acme/nul",
        "acme/NUL",
        "com1/x",
        "x/lpt9",
        "aux/aux",
        "console/lib",
        "acme/nullable",
        // .json
        "acme/lib.json",
        "acme/lib.JSON",
        "acme/json",
        "acme/lib-json",
        // uppercase, both branches of the suggestion
        "Acme/Lib",
        "Foo/BarBaz",
        "a/bC",
        "ACME/Thing",
        "XML/HttpRequest",
        "acme/XMLHttp",
        "aA/bB",
        "A/B",
        "ABCd/e",
        "aAAb/c",
        "acme/libX",
    ]
    .iter()
    .map(|s| (*s).to_owned())
    .collect()
}

/// Every link name of every fixture root manifest: none may be refused.
fn fixture_link_names() -> Vec<String> {
    let mut out = Vec::new();
    let mut dirs: Vec<PathBuf> = Vec::new();
    for base in ["fixtures/projects", "fixtures/corpus"] {
        if let Ok(entries) = std::fs::read_dir(root().join(base)) {
            for e in entries.flatten() {
                dirs.push(e.path());
            }
        }
    }
    for dir in dirs {
        let Ok(text) = std::fs::read_to_string(dir.join("composer.json")) else {
            continue;
        };
        let Ok(json) = serde_json::from_str::<serde_json::Value>(&text) else {
            continue;
        };
        for key in ["require", "require-dev", "conflict", "provide", "replace"] {
            if let Some(map) = json.get(key).and_then(serde_json::Value::as_object) {
                out.extend(map.keys().cloned());
            }
        }
        if let Some(name) = json.get("name").and_then(serde_json::Value::as_str) {
            out.push(name.to_owned());
        }
    }
    out.sort();
    out.dedup();
    assert!(
        out.len() > 100,
        "expected the fixtures' names, found {}",
        out.len()
    );
    out
}

fn oracle(names: &[String]) -> Vec<[Option<String>; 2]> {
    let script = format!(
        r#"require "phar://{}/vendor/autoload.php";
           $out = [];
           foreach (json_decode(stream_get_contents(STDIN), true) as $n) {{
               $out[] = [
                   \Composer\Package\Loader\ValidatingArrayLoader::hasPackageNamingError($n, false),
                   \Composer\Package\Loader\ValidatingArrayLoader::hasPackageNamingError($n, true),
               ];
           }}
           echo json_encode($out);"#,
        phar().display()
    );
    let mut child = Command::new("php")
        .arg("-r")
        .arg(&script)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .expect("php runs");
    child
        .stdin
        .as_mut()
        .expect("stdin")
        .write_all(serde_json::to_string(names).expect("json").as_bytes())
        .expect("write");
    let out = child.wait_with_output().expect("php output");
    assert!(out.status.success(), "php failed");
    let raw: Vec<Vec<Option<String>>> = serde_json::from_slice(&out.stdout).expect("oracle json");
    raw.into_iter()
        .map(|mut pair| {
            let link = pair.pop().flatten();
            let plain = pair.pop().flatten();
            [plain, link]
        })
        .collect()
}

#[test]
fn naming_errors_match_composer() {
    let names = crafted();
    let expected = oracle(&names);
    for (name, [plain, link]) in names.iter().zip(expected) {
        assert_eq!(
            package_naming_error(name, false),
            plain,
            "{name:?} as a package name"
        );
        assert_eq!(
            package_naming_error(name, true),
            link,
            "{name:?} as a link name"
        );
    }
}

#[test]
fn no_real_name_is_refused() {
    let names = fixture_link_names();
    let expected = oracle(&names);
    for (name, [plain, link]) in names.iter().zip(expected) {
        assert_eq!(plain, None, "the oracle refuses the real name {name:?}");
        assert_eq!(package_naming_error(name, false), None, "{name:?}");
        assert_eq!(package_naming_error(name, true), link, "{name:?}");
    }
}

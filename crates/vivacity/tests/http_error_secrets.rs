//! The binary's own text for a failed download (`Error: HTTP failure for
//! <url>: <message>`, a diagnostic of vivacity's, not Composer's) never
//! shows the credentials the dist URL carries: the raw output of a real
//! `install` whose archive fails, read as the user sees it, without any
//! normalisation. What stays readable is what diagnoses the failure:
//! scheme, host, port, path and the other query parameters.

use std::io::{Read, Write};
use std::path::Path;
use std::process::Command;

/// A server that answers 404 to every request, for as long as the test runs.
fn not_found_server() -> u16 {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind");
    let port = listener.local_addr().expect("addr").port();
    std::thread::spawn(move || {
        for mut stream in listener.incoming().flatten() {
            let mut request = [0u8; 8192];
            let _ = stream.read(&mut request);
            let _ = stream.write_all(
                b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
            );
        }
    });
    port
}

/// A port nothing listens on: bound, then released.
fn closed_port() -> u16 {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind");
    listener.local_addr().expect("addr").port()
}

/// A project whose lock holds one package, its dist at `url`; no
/// repository to ask (Packagist off), `secure-http` off for `http://`.
fn project(dir: &Path, url: &str) {
    let manifest = r#"{
    "repositories": [{ "packagist.org": false }],
    "require": { "probe/missing": "1.0.0" },
    "config": { "secure-http": false }
}
"#;
    let hash = vivacity_core::content_hash::content_hash(manifest).expect("content-hash");
    let lock = serde_json::json!({
        "_readme": [],
        "content-hash": hash,
        "packages": [{
            "name": "probe/missing",
            "version": "1.0.0",
            "dist": { "type": "zip", "url": url, "shasum": "" },
            "type": "library"
        }],
        "packages-dev": [],
        "aliases": [],
        "minimum-stability": "stable",
        "stability-flags": {},
        "prefer-stable": false,
        "prefer-lowest": false,
        "platform": {},
        "platform-dev": {},
        "plugin-api-version": "2.6.0"
    });
    std::fs::write(dir.join("composer.json"), manifest).expect("composer.json");
    std::fs::write(
        dir.join("composer.lock"),
        serde_json::to_string_pretty(&lock).expect("lock"),
    )
    .expect("composer.lock");
}

/// `vivacity install` in a project of its own, with nothing inherited that
/// could change the run (another manifest, auth, offline mode, a proxy, a
/// cache): exit code and the raw stdout and stderr.
fn install(url: &str) -> (i32, String, String) {
    let tmp = tempfile::tempdir().expect("tmp");
    let dir = tmp.path().join("p");
    std::fs::create_dir_all(&dir).expect("mkdir");
    project(&dir, url);
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_vivacity"));
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
    let out = cmd
        .args([
            "install",
            "--no-fallback",
            "--no-scripts",
            "--no-interaction",
        ])
        .current_dir(&dir)
        .env("COMPOSER_HOME", tmp.path().join("home"))
        .env("COMPOSER_CACHE_DIR", tmp.path().join("cache"))
        .env("VIVACITY_CACHE_DIR", tmp.path().join("store"))
        .output()
        .expect("run vivacity");
    (
        out.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

/// Each form, its secrets, and the URL as the error must show it.
#[test]
fn a_failed_download_shows_no_credentials() {
    let port = not_found_server();
    let at = |rest: &str| format!("http://{rest}");
    let host = format!("127.0.0.1:{port}");
    let cases: Vec<(String, Vec<&str>, String)> = vec![
        (
            at(&format!("viv-user-7f3a:s3cr3t-pw-91c2@{host}/x.zip")),
            vec!["viv-user-7f3a", "s3cr3t-pw-91c2"],
            at(&format!("***@{host}/x.zip")),
        ),
        (
            at(&format!("tok-only-5d8e@{host}/x.zip")),
            vec!["tok-only-5d8e"],
            at(&format!("***@{host}/x.zip")),
        ),
        (
            at(&format!("{host}/x.zip?private_token=pt-secret-1a2b&a=1")),
            vec!["pt-secret-1a2b"],
            at(&format!("{host}/x.zip?private_token=***&a=1")),
        ),
        (
            at(&format!("{host}/x.zip?private-token=ph-secret-3c4d&a=1")),
            vec!["ph-secret-3c4d"],
            at(&format!("{host}/x.zip?private-token=***&a=1")),
        ),
        (
            at(&format!("{host}/x.zip?access_token=at-secret-5e6f&a=1")),
            vec!["at-secret-5e6f"],
            at(&format!("{host}/x.zip?access_token=***&a=1")),
        ),
    ];
    // Every form is run before the verdict, so a failure names them all.
    let mut failures = Vec::new();
    for (url, secrets, masked) in cases {
        let (code, stdout, stderr) = install(&url);
        let mut wrong = Vec::new();
        if code != 100 {
            wrong.push(format!("exit code {code}"));
        }
        for secret in &secrets {
            if stderr.contains(secret) || stdout.contains(secret) {
                wrong.push(format!("`{secret}` shown"));
            }
        }
        let line = format!("Error: HTTP failure for {masked}: ");
        if !stderr.contains(&line) {
            wrong.push(format!("no `{line}`"));
        }
        // reqwest's own text, which names the request again.
        if !stderr.contains("404 Not Found") {
            wrong.push("the cause is gone".to_owned());
        }
        if !wrong.is_empty() {
            failures.push(format!(
                "{url}: {}\nstdout:\n{stdout}\nstderr:\n{stderr}",
                wrong.join("; ")
            ));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

/// A refused connection: reqwest's message names the URL too (`error
/// sending request for url (…)`), after four attempts.
#[test]
fn a_refused_connection_shows_no_credentials() {
    let port = closed_port();
    let url = format!(
        "http://viv-user-8a1b:pw-closed-2c3d@127.0.0.1:{port}/x.zip?access_token=at-closed-4e5f&private_token=pt-closed-6a7b&a=1"
    );
    let (code, stdout, stderr) = install(&url);
    assert_eq!(code, 100, "exit code\nstdout:\n{stdout}\nstderr:\n{stderr}");
    for secret in [
        "viv-user-8a1b",
        "pw-closed-2c3d",
        "at-closed-4e5f",
        "pt-closed-6a7b",
    ] {
        assert!(
            !stderr.contains(secret) && !stdout.contains(secret),
            "`{secret}` shown\nstdout:\n{stdout}\nstderr:\n{stderr}"
        );
    }
    let line = format!(
        "Error: HTTP failure for http://***@127.0.0.1:{port}/x.zip?access_token=***&private_token=***&a=1: "
    );
    assert!(stderr.contains(&line), "no `{line}`\nstderr:\n{stderr}");
    assert!(
        stderr.contains("error sending request for url (http://127.0.0.1:"),
        "the cause is gone\nstderr:\n{stderr}"
    );
}

/// A password with a raw `?`: the URL does not parse as meant, nothing
/// is fetched, and the error still names it — without the credentials.
/// (`secure-http`'s warning names what it reads as the host, the user
/// name here: a line of Composer's, out of this test.)
#[test]
fn an_unparsable_url_shows_no_credentials() {
    let port = not_found_server();
    let url = format!("http://viv-user-0f1e:pw?raw-secret-9d0e@127.0.0.1:{port}/x.zip");
    let (code, stdout, stderr) = install(&url);
    assert_eq!(code, 100, "exit code\nstdout:\n{stdout}\nstderr:\n{stderr}");
    assert!(
        !stderr.contains("raw-secret-9d0e") && !stdout.contains("raw-secret-9d0e"),
        "the password shown\nstdout:\n{stdout}\nstderr:\n{stderr}"
    );
    let error = stderr
        .lines()
        .find(|l| l.starts_with("Error: "))
        .unwrap_or_else(|| panic!("no `Error:` line\nstderr:\n{stderr}"));
    assert!(
        error.starts_with(&format!(
            "Error: HTTP failure for http://***@127.0.0.1:{port}/x.zip: "
        )) && !error.contains("viv-user-0f1e"),
        "{error}"
    );
}

/// A server that counts the connections it is offered (and answers 404),
/// for as long as the test runs.
fn counting_server() -> (u16, std::sync::Arc<std::sync::atomic::AtomicUsize>) {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind");
    let port = listener.local_addr().expect("addr").port();
    let seen = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let count = seen.clone();
    std::thread::spawn(move || {
        for mut stream in listener.incoming().flatten() {
            count.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            let mut request = [0u8; 8192];
            let _ = stream.read(&mut request);
            let _ = stream.write_all(
                b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
            );
        }
    });
    (port, seen)
}

/// `vivacity install` of a lock whose dist URL takes its scheme from the
/// reference (`%reference%://…`, reference `http`), `secure-http` at its
/// default: the check made before the command writes anything reads the URL
/// as written and finds no insecure scheme; the download expands it to
/// `http://…`, which `secure-http` refuses where the request leaves
/// (`fetch::refuse_insecure`), a hand-over. `stub` puts a `composer` that
/// exits 42 first on the PATH and lets the hand-over run; without it,
/// `--no-fallback`. Exit code, raw stdout and stderr, and whether vendor/
/// exists.
fn install_unforeseen(url_after_scheme: &str, stub: bool) -> (i32, String, String, bool) {
    let tmp = tempfile::tempdir().expect("tmp");
    let dir = tmp.path().join("p");
    std::fs::create_dir_all(&dir).expect("mkdir");
    let manifest = r#"{
    "repositories": [{ "packagist.org": false }],
    "require": { "probe/missing": "1.0.0" }
}
"#;
    let hash = vivacity_core::content_hash::content_hash(manifest).expect("content-hash");
    let lock = serde_json::json!({
        "_readme": [],
        "content-hash": hash,
        "packages": [{
            "name": "probe/missing",
            "version": "1.0.0",
            "dist": {
                "type": "zip",
                "url": format!("%reference%://{url_after_scheme}"),
                "reference": "http",
                "shasum": ""
            },
            "type": "library"
        }],
        "packages-dev": [],
        "aliases": [],
        "minimum-stability": "stable",
        "stability-flags": {},
        "prefer-stable": false,
        "prefer-lowest": false,
        "platform": {},
        "platform-dev": {},
        "plugin-api-version": "2.6.0"
    });
    std::fs::write(dir.join("composer.json"), manifest).expect("composer.json");
    std::fs::write(
        dir.join("composer.lock"),
        serde_json::to_string_pretty(&lock).expect("lock"),
    )
    .expect("composer.lock");
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_vivacity"));
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
    cmd.args(["install", "--no-scripts", "--no-interaction"]);
    if stub {
        let bin = tmp.path().join("bin");
        std::fs::create_dir_all(&bin).expect("mkdir bin");
        let composer = bin.join("composer");
        std::fs::write(&composer, "#!/bin/sh\nexit 42\n").expect("stub");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&composer, std::fs::Permissions::from_mode(0o755))
                .expect("chmod");
        }
        let path = std::env::var_os("PATH").unwrap_or_default();
        let paths = std::iter::once(bin).chain(std::env::split_paths(&path));
        cmd.env("PATH", std::env::join_paths(paths).expect("PATH"));
    } else {
        cmd.arg("--no-fallback");
    }
    let out = cmd
        .current_dir(&dir)
        .env("COMPOSER_HOME", tmp.path().join("home"))
        .env("COMPOSER_CACHE_DIR", tmp.path().join("cache"))
        .env("VIVACITY_CACHE_DIR", tmp.path().join("store"))
        .output()
        .expect("run vivacity");
    (
        out.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
        dir.join("vendor").exists(),
    )
}

/// The reason of that hand-over names the URL — without the credentials,
/// a name and a password, a name alone, a password with a raw `?` or `#`:
/// scheme, host, port and path stay; the query is cut, as before. The
/// hand-over itself is what it was: the reason before anything is
/// requested or written, exit 3 under `--no-fallback`, and (unix) the
/// stub's own exit code once handed over.
#[test]
fn an_unforeseen_insecure_request_hands_over_without_credentials() {
    let (port, seen) = counting_server();
    let host = format!("127.0.0.1:{port}");
    let cases: Vec<(String, Vec<&str>)> = vec![
        (
            format!("viv-user-2b4d:pw-unforeseen-6f8a@{host}/x.zip?access_token=at-unf-1c3e&a=1"),
            vec!["viv-user-2b4d", "pw-unforeseen-6f8a", "at-unf-1c3e"],
        ),
        (
            format!("tok-unforeseen-9e7c@{host}/x.zip"),
            vec!["tok-unforeseen-9e7c"],
        ),
        (
            format!("viv-user-5a6b:pw?raw-unforeseen-3d2c@{host}/x.zip"),
            vec!["viv-user-5a6b", "raw-unforeseen-3d2c"],
        ),
        (
            format!("viv-user-8c9d:pw#raw-unforeseen-7b1a@{host}/x.zip"),
            vec!["viv-user-8c9d", "raw-unforeseen-7b1a"],
        ),
    ];
    let reason = format!(
        "  - http://***@{host}/x.zip would be fetched over an insecure protocol, which secure-http refuses"
    );
    let modes: &[bool] = if cfg!(unix) { &[false, true] } else { &[false] };
    let mut failures = Vec::new();
    for (url, secrets) in &cases {
        for &stub in modes {
            let (code, stdout, stderr, vendor) = install_unforeseen(url, stub);
            let mut wrong = Vec::new();
            let want = if stub { 42 } else { 3 };
            if code != want {
                wrong.push(format!("exit code {code}, not {want}"));
            }
            for secret in secrets {
                if stderr.contains(secret) || stdout.contains(secret) {
                    wrong.push(format!("`{secret}` shown"));
                }
            }
            let lines: Vec<&str> = stderr.lines().collect();
            let at = lines.iter().position(|l| *l == reason);
            if at.is_none() {
                wrong.push(format!("no `{reason}`"));
            }
            let after = at.and_then(|i| lines.get(i + 1)).copied().unwrap_or("");
            let next = if stub {
                "vivacity: delegating to `composer install --no-scripts --no-interaction`…"
            } else {
                "--no-fallback given: stopping here (no partial vendor/ was written)."
            };
            if after != next {
                wrong.push(format!("`{after}` after the reason, not `{next}`"));
            }
            if vendor {
                wrong.push("vendor/ written".to_owned());
            }
            if !wrong.is_empty() {
                failures.push(format!(
                    "{url} ({}): {}\nstdout:\n{stdout}\nstderr:\n{stderr}",
                    if stub { "handed over" } else { "--no-fallback" },
                    wrong.join("; ")
                ));
            }
        }
    }
    let requests = seen.load(std::sync::atomic::Ordering::SeqCst);
    if requests != 0 {
        failures.push(format!(
            "{requests} connection(s) made before the hand-over"
        ));
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

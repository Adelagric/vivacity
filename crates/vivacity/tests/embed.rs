//! The library entry point: a host program calls `vivacity::run` with a
//! command line and gets the binary's exit code.

#[test]
fn run_returns_the_binary_exit_codes() {
    assert_eq!(vivacity::run(["vivacity", "--version"]), 0);
    assert_eq!(vivacity::run(["vivacity", "--help"]), 0);
    assert_eq!(vivacity::run(["vivacity", "bogus"]), 2);
    assert_eq!(vivacity::run(["vivacity", "install", "--no-such-flag"]), 2);
    let missing = tempfile::tempdir().expect("tmp");
    let dir = missing.path().join("absent");
    assert_eq!(
        vivacity::run([
            "vivacity",
            "install",
            "--no-fallback",
            "--working-dir",
            dir.to_str().expect("utf8")
        ]),
        1
    );
}

/// Composer's global options on a CI command line (`composer install
/// --no-dev --no-interaction --no-ansi -q`) parse on every command: the run
/// gets past argument parsing (exit 1 on the absent project, never clap's 2).
#[test]
fn composer_global_options_parse_on_every_command() {
    let missing = tempfile::tempdir().expect("tmp");
    let dir = missing.path().join("absent");
    let dir = dir.to_str().expect("utf8");
    for command in [
        &["install", "--no-fallback", "--prefer-dist", "--no-progress"][..],
        &["update", "--no-fallback"],
        &["dump-autoload", "--no-fallback", "--no-scripts"],
        &["require", "--no-fallback", "psr/log"],
        &["remove", "--no-fallback", "psr/log"],
    ] {
        for globals in [
            &["-n"][..],
            &["--no-interaction"],
            &["-q"],
            &["--quiet"],
            &["-v"],
            &["-v", "-v"],
            &["--no-ansi"],
            &["-n", "-q", "--no-ansi"],
            // Quiet wins over any -v, as in Symfony: no delegation.
            &["-q", "-vvv"],
        ] {
            let argv: Vec<&str> = std::iter::once("vivacity")
                .chain(command.iter().copied())
                .chain(globals.iter().copied())
                .chain(["--working-dir", dir])
                .collect();
            assert_eq!(vivacity::run(argv.clone()), 1, "{argv:?}");
        }
    }
    // `-vv`/`-vvv` go to Composer whole (its debug output is not
    // reproduced); `--no-fallback` stops with exit 3 before anything is read.
    for command in [
        &["install", "--no-fallback"][..],
        &["update", "--no-fallback"],
        &["dump-autoload", "--no-fallback"],
        &["require", "--no-fallback", "psr/log"],
        &["remove", "--no-fallback", "psr/log"],
    ] {
        for verbose in ["-vv", "-vvv"] {
            let argv: Vec<&str> = std::iter::once("vivacity")
                .chain(command.iter().copied())
                .chain([verbose, "--working-dir", dir])
                .collect();
            assert_eq!(vivacity::run(argv.clone()), 3, "{argv:?}");
        }
    }
    // `--no-scripts` is dump-autoload's default; asking for both is a usage error.
    assert_eq!(
        vivacity::run(["vivacity", "dump-autoload", "--no-scripts", "--run-scripts"]),
        2
    );
}

/// `config --global cache-dir` is answered (the cache directory, on
/// stdout, exit 0, as `composer config --global cache-dir`); every other
/// form of `config` stays a usage error.
#[test]
fn config_answers_the_global_cache_dir_only() {
    assert_eq!(
        vivacity::run(["vivacity", "config", "--global", "cache-dir"]),
        0
    );
    assert_eq!(vivacity::run(["vivacity", "config", "-g", "cache-dir"]), 0);
    assert_eq!(
        vivacity::run(["vivacity", "config", "cache-dir", "--global"]),
        0
    );
    for other in [
        &["config"][..],
        &["config", "cache-dir"],
        &["config", "--global"],
        &["config", "--global", "vendor-dir"],
        &["config", "--list"],
        &["config", "--global", "cache-dir", "extra"],
        &["config", "--global", "cache-dir", "/new/value"],
    ] {
        let argv: Vec<&str> = std::iter::once("vivacity")
            .chain(other.iter().copied())
            .collect();
        assert_eq!(vivacity::run(argv), 2, "{other:?}");
    }
}

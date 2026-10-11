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
        &["dump-autoload", "--no-scripts"],
        &["require", "--no-fallback", "psr/log"],
        &["remove", "--no-fallback", "psr/log"],
    ] {
        for globals in [
            &["-n"][..],
            &["--no-interaction"],
            &["-q"],
            &["--quiet"],
            &["-v"],
            &["-vvv"],
            &["--no-ansi"],
            &["-n", "-q", "--no-ansi"],
        ] {
            let argv: Vec<&str> = std::iter::once("vivacity")
                .chain(command.iter().copied())
                .chain(globals.iter().copied())
                .chain(["--working-dir", dir])
                .collect();
            assert_eq!(vivacity::run(argv.clone()), 1, "{argv:?}");
        }
    }
    // `--no-scripts` is dump-autoload's default; asking for both is a usage error.
    assert_eq!(
        vivacity::run(["vivacity", "dump-autoload", "--no-scripts", "--run-scripts"]),
        2
    );
}

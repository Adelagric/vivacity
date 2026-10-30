//! The binary: the command line handed to the library.

fn main() {
    // `require` reverts composer.json on SIGINT/SIGTERM/SIGHUP, as Composer
    // does; the library alone leaves an embedder's signals alone.
    vivacity_core::signals::enable();
    std::process::exit(vivacity::run(std::env::args_os()));
}

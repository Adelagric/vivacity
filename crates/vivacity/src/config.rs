//! `config --global cache-dir`, the one form of `config` vivacity answers:
//! the Composer cache directory it uses, printed as `composer config
//! --global cache-dir` prints it (`ConfigCommand`, `Config::RELATIVE_PATHS`:
//! a relative value stays relative). Where vivacity does not reproduce the
//! value, Composer answers. Every other form of `config` is a usage error
//! (exit 2). Plan: docs/plans/action-cache.md.

use super::*;

#[derive(clap::Args, Debug)]
pub(crate) struct ConfigArgs {
    /// Read the global configuration (the only scope vivacity answers).
    #[arg(short = 'g', long, required = true)]
    global: bool,
    /// The setting to print: `cache-dir` only.
    #[arg(value_name = "SETTING-KEY", value_parser = ["cache-dir"])]
    setting_key: String,
}

pub(crate) fn run_config(args: &ConfigArgs) -> anyhow::Result<i32> {
    anyhow::ensure!(
        args.global && args.setting_key == "cache-dir",
        "only `config --global cache-dir` is answered"
    );
    match vivacity_core::fetch::cache_dir_setting() {
        Ok(dir) => {
            // `$io->write($value, true, IOInterface::QUIET)`: stdout, always.
            vivacity_core::io::emit(true, format_args!("{dir}"));
            Ok(0)
        }
        Err(why) => {
            let Some(composer) = which_composer() else {
                vivacity_core::say_always!(
                    "composer not found for the fallback — install Composer or set COMPOSER_CACHE_DIR ({why})."
                );
                return Ok(3);
            };
            hand_over(&composer, &command_line(), false, Some(&why)).context("cannot run composer")
        }
    }
}

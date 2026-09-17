//! `--run-scripts`: the project's scripts, run by Composer itself
//! (`composer run-script <event>`), at the points where `Installer::run`
//! and `AutoloadGenerator::dump` dispatch them. vivacity never embeds PHP;
//! the static callables of a `scripts` entry receive Composer's
//! `Script\Event`, which only Composer can build. Only the events the
//! manifest declares are run (`run-script` refuses an event without a
//! listener); `--no-dev` sets `COMPOSER_DEV_MODE` on Composer's side.
//! What this cannot carry: the per-package events (`pre/post-package-*`,
//! which need the operation) and the `optimize` flag of the autoload
//! events (`run-script` passes no flags). The plugins' own listeners of
//! those events run again through `run-script`, as they would under
//! Composer.

use serde_json::Value;
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

pub const PRE_INSTALL_CMD: &str = "pre-install-cmd";
pub const POST_INSTALL_CMD: &str = "post-install-cmd";
pub const PRE_AUTOLOAD_DUMP: &str = "pre-autoload-dump";
pub const POST_AUTOLOAD_DUMP: &str = "post-autoload-dump";

/// The event names the manifest's `scripts` section declares.
pub fn declared_events(manifest: &Value) -> BTreeSet<String> {
    manifest
        .get("scripts")
        .and_then(Value::as_object)
        .map(|m| m.keys().cloned().collect())
        .unwrap_or_default()
}

/// The driver of a run: the `composer` binary and the declared events.
pub struct Runner {
    composer: PathBuf,
    project: PathBuf,
    events: BTreeSet<String>,
    with_dev: bool,
}

impl Runner {
    /// `None` when the manifest declares none of the events vivacity can
    /// drive; `Err` when it does and `composer` is missing.
    pub fn new(
        project: &Path,
        manifest: &Value,
        with_dev: bool,
        composer: Option<PathBuf>,
    ) -> Result<Option<Runner>, String> {
        let events = declared_events(manifest);
        let drivable = [
            PRE_INSTALL_CMD,
            POST_INSTALL_CMD,
            PRE_AUTOLOAD_DUMP,
            POST_AUTOLOAD_DUMP,
        ];
        if !drivable.iter().any(|e| events.contains(*e)) {
            return Ok(None);
        }
        let Some(composer) = composer else {
            return Err(
                "composer not found on PATH: --run-scripts hands the scripts to `composer run-script`"
                    .to_owned(),
            );
        };
        Ok(Some(Runner {
            composer,
            project: project.to_path_buf(),
            events,
            with_dev,
        }))
    }

    /// `composer run-script [--no-dev] <event>` when the manifest declares
    /// it: Composer's own dispatch, stdio inherited. Returns the exit code
    /// (0 when the event is not declared).
    pub fn run(&self, event: &str) -> anyhow::Result<i32> {
        if !self.events.contains(event) {
            return Ok(0);
        }
        let mut cmd = std::process::Command::new(&self.composer);
        cmd.arg("run-script")
            .arg("--no-interaction")
            .arg("-d")
            .arg(&self.project);
        if !self.with_dev {
            cmd.arg("--no-dev");
        }
        cmd.arg(event);
        let status = cmd
            .status()
            .map_err(|e| anyhow::anyhow!("cannot run {}: {e}", self.composer.display()))?;
        Ok(status.code().unwrap_or(1))
    }
}

//! Emulation of the `codeception/c3` plugin (docs/reference/plugins/
//! codeception-c3/, MIT), read at 2.9.0 under Composer 2: on
//! `POST_INSTALL_CMD` (`copyC3V2`) and `POST_UPDATE_CMD` (`askForUpdateV2`)
//! it copies `vendor/codeception/c3/c3.php` to `<cwd>/c3.php` — unless the
//! two are already identical (md5), or a different `c3.php` is there and
//! the confirmation to replace it is declined (the default, so always
//! under `--no-interaction`). `uninstall` of the plugin deletes the file.
//! Its messages go to stdout (`$io->write`).

use crate::error::{Error, Result};
use std::path::Path;

pub const PLUGIN_NAME: &str = "codeception/c3";

/// What the listener did, for the lines it prints.
#[derive(Debug, PartialEq, Eq)]
pub enum C3Action {
    /// `c3.php is already up-to-date` (printed on install, not on update).
    UpToDate,
    /// A different `c3.php` is in place: not replaced without a yes.
    Kept,
    Copied,
}

/// `copyC3V2` / `askForUpdateV2`: `vendor` is the vendor directory,
/// `root` the working directory (`getcwd()`).
pub fn copy_c3(vendor: &Path, root: &Path) -> Result<C3Action> {
    let source = vendor.join("codeception").join("c3").join("c3.php");
    let target = root.join("c3.php");
    let src_bytes = std::fs::read(&source).map_err(Error::io(&source))?;
    if let Ok(existing) = std::fs::read(&target) {
        if existing == src_bytes {
            return Ok(C3Action::UpToDate);
        }
        return Ok(C3Action::Kept);
    }
    std::fs::write(&target, &src_bytes).map_err(Error::io(&target))?;
    Ok(C3Action::Copied)
}

/// `Installer::uninstall` → `deleteFile`: `true` when a file was removed.
pub fn delete_c3(root: &Path) -> Result<bool> {
    let target = root.join("c3.php");
    if !target.exists() {
        return Ok(false);
    }
    std::fs::remove_file(&target).map_err(Error::io(&target))?;
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn copies_once_and_never_overwrites() {
        let d = tempfile::tempdir().expect("tmp");
        let vendor = d.path().join("vendor");
        std::fs::create_dir_all(vendor.join("codeception/c3")).expect("dirs");
        std::fs::write(vendor.join("codeception/c3/c3.php"), b"<?php // c3").expect("src");
        assert_eq!(copy_c3(&vendor, d.path()).expect("copy"), C3Action::Copied);
        assert_eq!(
            std::fs::read(d.path().join("c3.php")).expect("read"),
            b"<?php // c3"
        );
        assert_eq!(
            copy_c3(&vendor, d.path()).expect("again"),
            C3Action::UpToDate
        );
        std::fs::write(d.path().join("c3.php"), b"edited").expect("edit");
        assert_eq!(copy_c3(&vendor, d.path()).expect("kept"), C3Action::Kept);
        assert_eq!(
            std::fs::read(d.path().join("c3.php")).expect("read"),
            b"edited"
        );
        assert!(delete_c3(d.path()).expect("delete"));
        assert!(!delete_c3(d.path()).expect("nothing"));
    }
}

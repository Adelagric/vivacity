//! Clone of a store tree into vendor/, the hot path of the install.
//! macOS/APFS: `clonefile(2)` of the whole directory (one syscall, copy-on-write,
//! measured 8x faster than extraction in M0). Elsewhere, or if clonefile
//! fails (other FS, different volume): recursive walk, each file a reflink
//! (FICLONE, Linux btrfs/xfs) or else a real copy. Always towards an ABSENT
//! destination (the caller removes the previous version first), so no mixed
//! states.
//!
//! Never a hard link: Composer never shares a file between two projects, so
//! a file written in place in one project's vendor/ must change that project
//! only — not the store entry, not another project, not one installed later.
//! Up to 0.26.0 the walk linked (pnpm's model) and an in-place edit went
//! through the store into every project (docs/plans/vendor-isolation.md);
//! [`detach`] gives the files of a project laid out that way inodes of their
//! own.

use crate::error::{Error, Result};
use std::path::Path;

pub fn clone_tree(src: &Path, dst: &Path) -> Result<()> {
    if let Some(parent) = dst.parent() {
        std::fs::create_dir_all(parent).map_err(Error::io(parent))?;
    }

    #[cfg(target_os = "macos")]
    {
        use std::os::unix::ffi::OsStrExt as _;
        let c_src = std::ffi::CString::new(src.as_os_str().as_bytes());
        let c_dst = std::ffi::CString::new(dst.as_os_str().as_bytes());
        if let (Ok(c_src), Ok(c_dst)) = (c_src, c_dst) {
            // SAFETY: FFI call to clonefile through libc, two valid C strings,
            // no shared memory.
            let rc = unsafe { libc::clonefile(c_src.as_ptr(), c_dst.as_ptr(), 0) };
            if rc == 0 {
                return Ok(());
            }
            // Failure (non-APFS FS, different volumes...): fall through to the next strategies.
        }
    }

    copy_tree(src, dst)
}

fn copy_tree(src: &Path, dst: &Path) -> Result<()> {
    std::fs::create_dir_all(dst).map_err(Error::io(dst))?;
    // The mode a new directory gets (0777 & ~umask), read once per clone on
    // the destination's root, still untouched.
    #[cfg(unix)]
    let default = mode_of(dst)?;
    #[cfg(not(unix))]
    let default = 0;
    copy_dir(src, dst, default)
}

/// One directory of the clone, its tree, then its mode. A directory is
/// created with the default mode; the store's carries the archive's
/// (`unzip` restores it, and Composer moves the extracted tree into place
/// whole). Applied once the directory is filled — its mode may forbid
/// writing into it — and only when it differs: one `stat` of the source per
/// directory, a `chmod` for an unusual mode only.
fn copy_dir(src: &Path, dst: &Path, default: u32) -> Result<()> {
    copy_entries(src, dst, default)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        let want = mode_of(src)?;
        if want != default {
            std::fs::set_permissions(dst, std::fs::Permissions::from_mode(want))
                .map_err(Error::io(dst))?;
        }
    }
    #[cfg(not(unix))]
    let _ = default;
    Ok(())
}

/// The permission bits of a path.
#[cfg(unix)]
fn mode_of(path: &Path) -> Result<u32> {
    use std::os::unix::fs::PermissionsExt as _;
    Ok(std::fs::metadata(path)
        .map_err(Error::io(path))?
        .permissions()
        .mode()
        & 0o7777)
}

fn copy_entries(src: &Path, dst: &Path, default: u32) -> Result<()> {
    for entry in std::fs::read_dir(src).map_err(Error::io(src))? {
        let entry = entry.map_err(Error::io(src))?;
        let from = entry.path();
        let to = dst.join(entry.file_name());
        let ftype = entry.file_type().map_err(Error::io(&from))?;
        if ftype.is_dir() {
            std::fs::create_dir(&to).map_err(Error::io(&to))?;
            copy_dir(&from, &to, default)?;
        } else if ftype.is_symlink() {
            let target = std::fs::read_link(&from).map_err(Error::io(&from))?;
            #[cfg(unix)]
            symlink_like_unzip(&target, &to)?;
            #[cfg(windows)]
            clone_symlink_windows(&from, &target, &to)?;
            #[cfg(not(any(unix, windows)))]
            {
                let _ = target;
                std::fs::copy(&from, &to).map_err(Error::io(&to))?;
            }
        } else {
            // Linux: a reflink first (FICLONE, a CoW copy like clonefile on
            // APFS), a single syscall on btrfs/xfs. On ext4 (no reflink) the
            // ioctl fails cleanly and the file is copied. `std::fs::copy`
            // keeps the permission bits (the archive's, as `unzip` left them
            // for Composer).
            #[cfg(target_os = "linux")]
            let done = reflink(&from, &to);
            #[cfg(not(target_os = "linux"))]
            let done = false;
            if !done {
                std::fs::copy(&from, &to).map_err(Error::io(&to))?;
            }
        }
    }
    Ok(())
}

/// Reflink (copy-on-write) `from` → `to` via the FICLONE ioctl. Creates an
/// independent inode sharing the extents: modifying the clone does not touch
/// the store. The new file's mode is re-applied from the source (FICLONE
/// does not copy permissions), to preserve the executable bit of binaries.
/// Returns false (without leaving a partial file) if the FS does not support
/// reflink — the caller then copies.
#[cfg(target_os = "linux")]
fn reflink(from: &Path, to: &Path) -> bool {
    use std::os::unix::io::AsRawFd as _;
    // Once a filesystem has said it cannot reflink, stop asking: the probe
    // is an open/create/ioctl/unlink per file, and the answer does not
    // change within a run (the store and vendor/ stay where they are).
    static UNSUPPORTED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
    if UNSUPPORTED.load(std::sync::atomic::Ordering::Relaxed) {
        return false;
    }
    let Ok(src) = std::fs::File::open(from) else {
        return false;
    };
    let dst = match std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .open(to)
    {
        Ok(f) => f,
        Err(_) => return false,
    };
    // SAFETY: FICLONE ioctl on two valid, open descriptors; no shared
    // memory. The kernel reads src, writes dst.
    let rc = unsafe { libc::ioctl(dst.as_raw_fd(), libc::FICLONE, src.as_raw_fd()) };
    if rc != 0 {
        let err = std::io::Error::last_os_error();
        drop(dst);
        let _ = std::fs::remove_file(to); // empty file created by the open
                                          // ENOTTY / EOPNOTSUPP / EXDEV / EINVAL: this filesystem (or this
                                          // pair of filesystems) never will; other errors are per file.
        if matches!(
            err.raw_os_error(),
            Some(libc::ENOTTY) | Some(libc::EOPNOTSUPP) | Some(libc::EXDEV) | Some(libc::EINVAL)
        ) {
            UNSUPPORTED.store(true, std::sync::atomic::Ordering::Relaxed);
        }
        return false;
    }
    if let Ok(meta) = src.metadata() {
        let _ = std::fs::set_permissions(to, meta.permissions());
    }
    true
}

/// Windows: creating symlinks requires a privilege (Developer Mode or
/// SeCreateSymbolicLinkPrivilege). We try the real symlink, and failing that
/// copy the RESOLVED content — the resulting vendor/ is functional but no
/// longer a link (a parity divergence documented in docs/windows.md). A
/// dangling link fails loudly instead of vanishing silently.
#[cfg(windows)]
fn clone_symlink_windows(from: &Path, target: &Path, to: &Path) -> Result<()> {
    let is_dir = std::fs::metadata(from).map(|m| m.is_dir()).unwrap_or(false);
    let made = if is_dir {
        std::os::windows::fs::symlink_dir(target, to)
    } else {
        std::os::windows::fs::symlink_file(target, to)
    };
    if made.is_ok() {
        return Ok(());
    }
    if is_dir {
        copy_tree(from, to) // read_dir follows the link
    } else {
        std::fs::copy(from, to).map_err(Error::io(to))?;
        Ok(())
    }
}

/// Every regular file under `dir` that shares its inode (a hard link, as
/// vivacity up to 0.26.0 laid vendor/ out from its store) gets one of its
/// own: a copy of its CURRENT content, renamed over it. The content is never
/// reset to the archive's: an edit made through the link may be a deliberate
/// local patch, and which project made it cannot be told. A symbolic link is
/// not followed (a `path` package's source stays as it is). Before `patch`
/// writes into a package: a file that cannot be given its own inode is an
/// error, so the patch never reaches a store entry.
pub fn detach(dir: &Path) -> Result<()> {
    walk_detach(dir, true, &mut Vec::new()).map(|_| ())
}

/// [`detach`] for the one-time pass over a project an older version laid
/// out (`installer::detach_once`), where an install Composer would complete
/// must not fail: a file that cannot be given its own inode, or a directory
/// that cannot be read, is left as it is, silently. Returns whether the walk
/// went through without leaving anything behind. Every file found linked is
/// recorded in `seen`, with what it was before it was detached
/// ([`LinkedFile`]): once it is, which names shared its inode can no longer
/// be told.
pub fn detach_all(dir: &Path, seen: &mut Vec<LinkedFile>) -> bool {
    walk_detach(dir, false, seen).unwrap_or(false)
}

/// A file the one-time pass found sharing its inode, as it was then.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LinkedFile {
    pub path: std::path::PathBuf,
    /// Names the inode had (`None`: the system could not say; Windows only).
    pub links: Option<u64>,
    /// (device, inode) under Unix, (volume serial, file index) under
    /// Windows: equal for two names of one file.
    pub file_id: Option<(u64, u64)>,
    /// The sha1 of the content it had (and kept), hashed while it was
    /// copied; the reason when it could not be read.
    pub current_sha1: std::result::Result<String, String>,
    /// Whether it got an inode of its own, or why not.
    pub detached: std::result::Result<(), String>,
}

fn walk_detach(dir: &Path, strict: bool, seen: &mut Vec<LinkedFile>) -> Result<bool> {
    let entries = match std::fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(true),
        Err(_) => return Ok(false),
    };
    let mut complete = true;
    for entry in entries {
        let Ok(entry) = entry else {
            complete = false;
            continue;
        };
        let path = entry.path();
        let Ok(meta) = std::fs::symlink_metadata(&path) else {
            complete = false;
            continue;
        };
        if meta.is_dir() {
            complete &= walk_detach(&path, strict, seen)?;
            continue;
        }
        if !meta.is_file() {
            continue;
        }
        let (links, file_id) = link_info(&path, &meta);
        // A file the system cannot be asked about is taken as linked:
        // copying it once costs a copy, leaving a link shared with the
        // store costs the isolation.
        if links.unwrap_or(2) <= 1 {
            continue;
        }
        if strict {
            unshare(&path, &meta, None).map_err(Error::io(&path))?;
            continue;
        }
        let mut hasher = sha1::Sha1::default();
        let made = unshare(&path, &meta, Some(&mut hasher));
        let current_sha1 = match &made {
            Ok(()) => Ok(hex::encode(sha1::Digest::finalize(hasher))),
            // The copy stopped part way: the content is read on its own.
            Err(_) => std::fs::read(&path)
                .map(|bytes| hex::encode(<sha1::Sha1 as sha1::Digest>::digest(&bytes)))
                .map_err(|e| e.to_string()),
        };
        complete &= made.is_ok();
        seen.push(LinkedFile {
            path,
            links,
            file_id,
            current_sha1,
            detached: made.map_err(|e| e.to_string()),
        });
    }
    Ok(complete)
}

/// `path` replaced by a copy of itself: a temporary file beside it, then a
/// rename over it. Never a stray file left in the directory. The bytes
/// copied go through `hasher` when one is given.
fn unshare(
    path: &Path,
    meta: &std::fs::Metadata,
    hasher: Option<&mut sha1::Sha1>,
) -> std::io::Result<()> {
    let name = path.file_name().unwrap_or_default().to_string_lossy();
    let tmp = path.with_file_name(format!(".{name}.vivacity-detach"));
    let made = copy_keeping(path, &tmp, meta, hasher).and_then(|()| std::fs::rename(&tmp, path));
    if made.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    made
}

/// A copy of `from` at `to` that keeps what an install by Composer leaves
/// untouched on a file it does not reinstall: the bytes, the permission
/// bits, the modification time, and under Unix the owner (an install run
/// as root must not hand a user's vendor/ over to root).
fn copy_keeping(
    from: &Path,
    to: &Path,
    meta: &std::fs::Metadata,
    hasher: Option<&mut sha1::Sha1>,
) -> std::io::Result<()> {
    let mut src = std::fs::File::open(from)?;
    let mut dst = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(to)?;
    match hasher {
        None => std::io::copy(&mut src, &mut dst)?,
        Some(hasher) => std::io::copy(&mut Hashing { src, hasher }, &mut dst)?,
    };
    dst.set_modified(meta.modified()?)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt as _;
        let made = dst.metadata()?;
        if (made.uid(), made.gid()) != (meta.uid(), meta.gid()) {
            // Before the mode: a change of owner clears set-id bits.
            std::os::unix::fs::fchown(&dst, Some(meta.uid()), Some(meta.gid()))?;
        }
    }
    dst.set_permissions(meta.permissions())
}

/// A reader that passes what it reads through a sha1.
struct Hashing<'a, R> {
    src: R,
    hasher: &'a mut sha1::Sha1,
}

impl<R: std::io::Read> std::io::Read for Hashing<'_, R> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let n = self.src.read(buf)?;
        sha1::Digest::update(self.hasher, &buf[..n]);
        Ok(n)
    }
}

/// The number of names a regular file has, and its identity.
#[cfg(unix)]
fn link_info(_path: &Path, meta: &std::fs::Metadata) -> (Option<u64>, Option<(u64, u64)>) {
    use std::os::unix::fs::MetadataExt as _;
    (Some(meta.nlink()), Some((meta.dev(), meta.ino())))
}

/// The number of names a regular file has, and its identity:
/// `GetFileInformationByHandle` through winapi-util (std's
/// `number_of_links` is unstable).
#[cfg(windows)]
fn link_info(path: &Path, _meta: &std::fs::Metadata) -> (Option<u64>, Option<(u64, u64)>) {
    match std::fs::File::open(path).and_then(|f| winapi_util::file::information(&f)) {
        Ok(info) => (
            Some(info.number_of_links()),
            Some((info.volume_serial_number(), info.file_index())),
        ),
        Err(_) => (None, None),
    }
}

#[cfg(not(any(unix, windows)))]
fn link_info(_path: &Path, _meta: &std::fs::Metadata) -> (Option<u64>, Option<(u64, u64)>) {
    (Some(1), None)
}

/// A symbolic link the way `unzip` (Composer's extractor) leaves one: on
/// macOS the link's own mode is 0777 (`fchmodat` without following), where a plain `symlink()`
/// gets the umask applied; Linux ignores link modes.
#[cfg(unix)]
pub fn symlink_like_unzip(target: &std::path::Path, link: &std::path::Path) -> Result<()> {
    std::os::unix::fs::symlink(target, link).map_err(Error::io(link))?;
    #[cfg(target_os = "macos")]
    {
        use std::os::unix::ffi::OsStrExt;
        let c = std::ffi::CString::new(link.as_os_str().as_bytes()).map_err(|_| Error::Io {
            path: link.to_path_buf(),
            source: std::io::Error::new(std::io::ErrorKind::InvalidInput, "path contains NUL"),
        })?;
        // SAFETY: FFI call with a valid NUL-terminated path; no memory is
        // shared, the return value is checked.
        let rc =
            unsafe { libc::fchmodat(libc::AT_FDCWD, c.as_ptr(), 0o777, libc::AT_SYMLINK_NOFOLLOW) };
        if rc != 0 {
            return Err(Error::io(link)(std::io::Error::last_os_error()));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[cfg(unix)]
    fn directory_modes_follow_the_source() {
        use std::os::unix::fs::PermissionsExt as _;
        let tmp = tempfile::tempdir().expect("tmp");
        let src = tmp.path().join("src");
        std::fs::create_dir_all(src.join("locked/inner")).expect("mkdir");
        std::fs::write(src.join("locked/inner/f"), b"x").expect("write");
        let set = |p: &Path, m: u32| {
            std::fs::set_permissions(p, std::fs::Permissions::from_mode(m)).expect("chmod")
        };
        // Set leaf first: 0500 forbids writing into `locked` afterwards.
        set(&src.join("locked/inner"), 0o700);
        set(&src.join("locked"), 0o500);
        set(&src, 0o775);
        let dst = tmp.path().join("dst");
        copy_tree(&src, &dst).expect("clone");
        let mode = |p: &Path| std::fs::metadata(p).expect("stat").permissions().mode() & 0o7777;
        assert_eq!(mode(&dst), 0o775);
        assert_eq!(mode(&dst.join("locked")), 0o500);
        assert_eq!(mode(&dst.join("locked/inner")), 0o700);
        assert!(dst.join("locked/inner/f").exists());
        // So that the temporary directory can be removed.
        set(&src.join("locked"), 0o700);
        set(&dst.join("locked"), 0o700);
    }

    #[test]
    fn clones_files_dirs_symlinks_and_exec_bits() {
        let tmp = tempfile::tempdir().expect("tmp");
        let src = tmp.path().join("src");
        std::fs::create_dir_all(src.join("sub")).expect("mkdir");
        std::fs::write(src.join("a.txt"), b"hello").expect("write");
        std::fs::write(src.join("sub/tool"), b"#!/bin/sh\n").expect("write");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            std::fs::set_permissions(src.join("sub/tool"), std::fs::Permissions::from_mode(0o755))
                .expect("chmod");
            std::os::unix::fs::symlink("a.txt", src.join("link")).expect("ln");
        }
        // Windows: the fixture's link requires Developer Mode or
        // SeCreateSymbolicLinkPrivilege — without it we cannot build the
        // fixture, so that portion is announced and then skipped (the copy
        // fallback of clone_symlink_windows cannot be forced from here).
        #[cfg(windows)]
        let with_link = match std::os::windows::fs::symlink_file("a.txt", src.join("link")) {
            Ok(()) => true,
            Err(e) => {
                eprintln!("symlink refused on this host ({e}) — link portion not exercised");
                false
            }
        };

        let dst = tmp.path().join("dst/pkg");
        clone_tree(&src, &dst).expect("clone");
        assert_eq!(std::fs::read(dst.join("a.txt")).expect("read"), b"hello");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            let mode = std::fs::metadata(dst.join("sub/tool"))
                .expect("meta")
                .permissions()
                .mode();
            assert_eq!(mode & 0o111, 0o111, "executable bit lost on clone");
            assert!(dst
                .join("link")
                .symlink_metadata()
                .expect("meta")
                .file_type()
                .is_symlink());
        }
        #[cfg(windows)]
        if with_link {
            // Never a lost entry: either a real link (privilege present —
            // the same one that allowed the fixture), or a copy of the
            // resolved content; in both cases reading yields the target's
            // content.
            let meta = dst.join("link").symlink_metadata().expect("entry lost");
            assert!(
                meta.file_type().is_symlink() || meta.file_type().is_file(),
                "neither link nor file"
            );
            assert_eq!(std::fs::read(dst.join("link")).expect("read"), b"hello");
        }
        // A write in place into the clone (same file, truncated and
        // rewritten) does not reach the source, whatever the FS.
        write_in_place(&dst.join("a.txt"), b"changed");
        assert_eq!(std::fs::read(dst.join("a.txt")).expect("read"), b"changed");
        assert_eq!(std::fs::read(src.join("a.txt")).expect("read"), b"hello");
    }

    /// Overwrites `path` through its own inode, the way a shell redirection,
    /// `file_put_contents` or an editor writing in place does — never by
    /// renaming a new file over it.
    fn write_in_place(path: &Path, bytes: &[u8]) {
        use std::io::Write as _;
        let mut f = std::fs::OpenOptions::new()
            .write(true)
            .truncate(true)
            .open(path)
            .expect("open for writing");
        f.write_all(bytes).expect("write");
    }

    /// The walk that serves every FS without clonefile (ext4, overlayfs,
    /// HFS+, NTFS): two projects copied from one store tree, one edited in
    /// place; the other and the store keep their bytes, the edited one keeps
    /// its mode.
    #[test]
    fn the_walk_isolates_projects_from_the_store_and_each_other() {
        let tmp = tempfile::tempdir().expect("tmp");
        let store = tmp.path().join("store");
        std::fs::create_dir_all(store.join("src")).expect("mkdir");
        std::fs::write(store.join("src/Lib.php"), b"<?php class Lib {}\n").expect("write");
        let (a, b) = (tmp.path().join("a"), tmp.path().join("b"));
        copy_tree(&store, &a).expect("copy a");
        copy_tree(&store, &b).expect("copy b");
        write_in_place(&a.join("src/Lib.php"), b"<?php class Lix {}\n");
        assert_eq!(
            std::fs::read(a.join("src/Lib.php")).expect("read"),
            b"<?php class Lix {}\n"
        );
        for untouched in [&store, &b] {
            assert_eq!(
                std::fs::read(untouched.join("src/Lib.php")).expect("read"),
                b"<?php class Lib {}\n",
                "{} changed",
                untouched.display()
            );
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            let mode = |p: &Path| std::fs::metadata(p).expect("stat").permissions().mode() & 0o7777;
            assert_eq!(
                mode(&a.join("src/Lib.php")),
                mode(&store.join("src/Lib.php"))
            );
        }
    }

    /// A project laid out by hard links (vivacity up to 0.26.0): `detach`
    /// gives each file its own copy, with the content it has NOW — an edit
    /// already made through the link stays — and its mode; a later write
    /// in place reaches neither the old entry nor the other project.
    #[test]
    fn detach_keeps_the_current_content_and_unshares_the_file() {
        let tmp = tempfile::tempdir().expect("tmp");
        let entry = tmp.path().join("store/v3/acme/lib/1.0.0-abc");
        std::fs::create_dir_all(&entry).expect("mkdir");
        std::fs::write(entry.join("Lib.php"), b"original\n").expect("write");
        std::fs::write(entry.join("Other.php"), b"other\n").expect("write");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            std::fs::set_permissions(
                entry.join("Lib.php"),
                std::fs::Permissions::from_mode(0o640),
            )
            .expect("chmod");
        }
        let project = |n: &str| tmp.path().join(n).join("vendor/acme/lib");
        for p in ["a", "b"] {
            std::fs::create_dir_all(project(p)).expect("mkdir");
            for f in ["Lib.php", "Other.php"] {
                std::fs::hard_link(entry.join(f), project(p).join(f)).expect("ln");
            }
        }
        // An edit through the link before the upgrade: every name sees it.
        write_in_place(&entry.join("Other.php"), b"edited\n");
        let long_ago =
            std::time::SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(1_600_000_000);
        std::fs::File::options()
            .write(true)
            .open(entry.join("Lib.php"))
            .and_then(|f| f.set_modified(long_ago))
            .expect("set mtime");
        // A file of the project that was never linked is not recorded.
        std::fs::write(project("a").join("Own.php"), b"own\n").expect("write");
        let mut seen = Vec::new();
        assert!(
            detach_all(&tmp.path().join("a/vendor"), &mut seen),
            "nothing left linked"
        );
        // What each linked file was before it got its own inode: the names
        // it had, its identity (the store entry's), the content it kept.
        seen.sort_by(|a, b| a.path.cmp(&b.path));
        let sha = |b: &[u8]| hex::encode(<sha1::Sha1 as sha1::Digest>::digest(b));
        assert_eq!(seen.len(), 2, "{seen:?}");
        assert_eq!(seen[0].path, project("a").join("Lib.php"));
        assert_eq!(seen[0].current_sha1, Ok(sha(b"original\n")));
        assert_eq!(seen[1].current_sha1, Ok(sha(b"edited\n")));
        for f in &seen {
            assert_eq!(f.detached, Ok(()));
            #[cfg(unix)]
            {
                use std::os::unix::fs::MetadataExt as _;
                let name = f.path.file_name().unwrap();
                let old = std::fs::metadata(entry.join(name)).unwrap();
                assert_eq!(f.links, Some(3));
                assert_eq!(f.file_id, Some((old.dev(), old.ino())));
            }
        }
        std::fs::remove_file(project("a").join("Own.php")).expect("rm");
        assert_eq!(
            std::fs::read(project("a").join("Lib.php")).unwrap(),
            b"original\n"
        );
        assert_eq!(
            std::fs::read(project("a").join("Other.php")).unwrap(),
            b"edited\n"
        );
        let mtime = |p: &Path| std::fs::metadata(p).unwrap().modified().unwrap();
        assert_eq!(
            mtime(&project("a").join("Lib.php")),
            mtime(&entry.join("Lib.php"))
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::{MetadataExt as _, PermissionsExt as _};
            let meta = std::fs::metadata(project("a").join("Lib.php")).unwrap();
            assert_eq!(meta.nlink(), 1);
            assert_eq!(meta.permissions().mode() & 0o7777, 0o640);
        }
        write_in_place(&project("a").join("Lib.php"), b"patched\n");
        write_in_place(&project("a").join("Other.php"), b"patched\n");
        assert_eq!(std::fs::read(entry.join("Lib.php")).unwrap(), b"original\n");
        assert_eq!(
            std::fs::read(project("b").join("Lib.php")).unwrap(),
            b"original\n"
        );
        assert_eq!(
            std::fs::read(project("b").join("Other.php")).unwrap(),
            b"edited\n"
        );
        // The strict form, used before `patch`, does the same.
        detach(&tmp.path().join("b/vendor")).expect("detach");
        assert_eq!(
            std::fs::read(project("b").join("Other.php")).unwrap(),
            b"edited\n"
        );
        // Nothing of the detach is left behind.
        let names: Vec<_> = std::fs::read_dir(project("a"))
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .collect();
        assert_eq!(names.len(), 2, "{names:?}");
    }

    /// A linked file in a directory the walk cannot write into (an archive
    /// may ship one read-only): the one-time pass leaves it as it is and
    /// says it did not go through, instead of failing an install Composer
    /// would complete; the strict form fails.
    #[test]
    #[cfg(unix)]
    fn a_file_that_cannot_be_unshared_is_left_and_reported_by_the_lenient_walk() {
        use std::os::unix::fs::PermissionsExt as _;
        let tmp = tempfile::tempdir().expect("tmp");
        let entry = tmp.path().join("entry");
        std::fs::create_dir_all(&entry).expect("mkdir");
        std::fs::write(entry.join("f"), b"x").expect("write");
        let locked = tmp.path().join("vendor/acme/lib/locked");
        std::fs::create_dir_all(&locked).expect("mkdir");
        std::fs::hard_link(entry.join("f"), locked.join("f")).expect("ln");
        std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o555)).expect("chmod");
        // Root writes anyway: nothing to observe then.
        let writable = std::fs::write(locked.join(".probe"), b"").is_ok();
        let _ = std::fs::remove_file(locked.join(".probe"));
        if !writable {
            assert!(!detach_all(&tmp.path().join("vendor"), &mut Vec::new()));
            assert!(detach(&tmp.path().join("vendor")).is_err());
            assert_eq!(std::fs::read(locked.join("f")).expect("read"), b"x");
            let names: Vec<_> = std::fs::read_dir(&locked).unwrap().collect();
            assert_eq!(names.len(), 1, "no temporary file left");
        }
        std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o755)).expect("chmod");
    }
}

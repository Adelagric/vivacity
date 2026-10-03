//! Extraction of a zip dist into a directory, stripping the single root
//! directory of GitHub/Packagist zipballs (ArchiveDownloader's rule: strip iff
//! the archive has exactly one top-level entry and it is a directory;
//! otherwise everything is extracted as is), and a DISTRUSTFUL extraction:
//! - paths: `enclosed_name()` (rejects `..` and absolute paths);
//! - symlinks (unix mode S_IFLNK): relative target only, and the lexically
//!   resolved path must stay inside the package root;
//! - modes taken from the archive, as `unzip` poses them (see `unzip_mode`);
//! - refusal of absurd decompressed sizes (crude zip bomb).

use crate::error::{Error, Result};
use std::io::Read;
use std::path::{Component, Path, PathBuf};

/// Maximum decompressed size of a dist (512 MB); beyond that, something is
/// wrong (the largest real packages weigh a few tens of MB).
const MAX_UNCOMPRESSED: u64 = 512 * 1024 * 1024;

const S_IFMT: u32 = 0o170000;
const S_IFLNK: u32 = 0o120000;

/// Path of an entry, or an error if it is absolute or contains `..`:
/// enclosed_name accepts an inner `..` (`r/../x` stays inside the root) that
/// stripping the first component would turn into an escape, and no
/// legitimate dist contains one.
fn entry_path(entry: &zip::read::ZipFile<'_>, dest: &Path) -> Result<PathBuf> {
    // On unix the name is used as BYTES, which is what `unzip` does with it: it
    // transcodes nothing, so a name that is not valid UTF-8 reaches the
    // filesystem as it stands — and APFS then refuses it, exactly as it refuses
    // it for `unzip` (measured: exit 50, and vivacity used to succeed there by
    // laying out the crate's cp437 reading of the name instead). The same
    // reading was wrong for a perfectly ordinary name too: a UTF-8 name stored
    // without the UTF-8 flag, which happens with zips built on Windows, came out
    // as mojibake where `unzip` writes the bytes.
    //
    // Elsewhere the crate's decoded name is kept: a Windows filename has to be
    // convertible to UTF-16, so bytes are not an option, and what 7-Zip does
    // with such a name there is unmeasured (CONTRIBUTING).
    #[cfg(unix)]
    let candidate = {
        use std::os::unix::ffi::OsStrExt as _;
        PathBuf::from(std::ffi::OsStr::from_bytes(entry.name_raw()))
    };
    #[cfg(not(unix))]
    let candidate = PathBuf::from(entry.name());
    let ok = !candidate.as_os_str().is_empty()
        && !candidate.is_absolute()
        && candidate
            .components()
            .all(|c| matches!(c, Component::Normal(_) | Component::CurDir));
    // `enclosed_name` is still consulted: it refuses a Windows prefix and a
    // `..` the component walk could read differently, and it is the rule the
    // rest of this file was written against.
    if !ok || entry.enclosed_name().is_none() {
        return Err(Error::HostileArchive {
            dest: dest.to_path_buf(),
            reason: format!("invalid entry path: {:?}", entry.name()),
        });
    }
    Ok(candidate)
}

/// The single top-level directory whose content becomes the package, or `None`
/// when everything is taken as is. `ArchiveDownloader::install` lists the
/// extracted tree with a Finder that excludes `.DS_Store` **by name** — file or
/// directory, `notName('.DS_Store')` — and keeps the strip when exactly one
/// entry is left and `is_dir()` says it is a directory. It then does
/// `rename($extractedDir, $path)`: that directory MOVES onto the destination,
/// so a `.DS_Store` beside it stays in the temporary directory and is thrown
/// away with it — which is why the name is returned rather than a count of
/// components to drop.
/// Every entry's path, whether it is a directory entry, and whether it is a
/// symlink — read before the first write, because both the single-root rule and
/// the refusal below need the whole picture.
fn zip_listing(
    archive: &mut zip::ZipArchive<std::io::Cursor<&[u8]>>,
    attrs: &std::collections::HashMap<Vec<u8>, (u8, u32)>,
    dest: &Path,
) -> Result<Vec<(PathBuf, bool, bool)>> {
    let mut out = Vec::with_capacity(archive.len());
    for i in 0..archive.len() {
        let entry = archive.by_index(i).map_err(Error::zip(dest))?;
        let (host, ext) = attrs.get(entry.name_raw()).copied().unwrap_or_else(|| {
            (
                HOST_UNIX,
                entry.unix_mode().map_or(0, |m| (m & 0o177777) << 16),
            )
        });
        let is_symlink = host == HOST_UNIX && (ext >> 16) & S_IFMT == S_IFLNK;
        out.push((entry_path(&entry, dest)?, entry.is_dir(), is_symlink));
    }
    Ok(out)
}

/// The archive's whole top level is one symlink. `is_dir()` follows links, so
/// Composer keeps the single-root rule and `rename($extractedDir, $path)` moves
/// THE LINK onto `vendor/<name>`: measured, the installed package *is* a
/// symlink (`vendor/hostile/rootlink -> ..`). A store entry here is a directory
/// tree — there is nothing that could become a link — so such an archive is
/// refused rather than laid out as something else.
fn lone_top_level_symlink(listing: &[(PathBuf, bool, bool)]) -> Option<PathBuf> {
    let mut only: Option<&(PathBuf, bool, bool)> = None;
    for e in listing {
        let mut comps =
            e.0.components()
                .filter(|c| matches!(c, Component::Normal(_)));
        let Some(Component::Normal(first)) = comps.next() else {
            continue;
        };
        if first == ".DS_Store" {
            continue;
        }
        match only {
            None => only = Some(e),
            Some(seen) if std::ptr::eq(seen, e) => {}
            Some(_) => return None,
        }
    }
    let (path, _, is_symlink) = only?;
    // One entry in all, and it is the top-level name itself.
    let depth = path
        .components()
        .filter(|c| matches!(c, Component::Normal(_)))
        .count();
    (*is_symlink && depth == 1).then(|| path.clone())
}

/// The rule itself, over `(entry path, is a directory entry)`.
fn root_strip_of<'a>(
    entries: impl Iterator<Item = (&'a Path, bool)>,
) -> Option<std::ffi::OsString> {
    let mut top: std::collections::BTreeMap<std::ffi::OsString, bool> =
        std::collections::BTreeMap::new();
    for (raw, dir_entry) in entries {
        let mut comps = raw
            .components()
            .filter(|c| matches!(c, Component::Normal(_)));
        let Some(Component::Normal(first)) = comps.next() else {
            continue;
        };
        // Excluded by NAME, whatever it is: the Finder's `notName` does not
        // care whether `.DS_Store` is a file or a directory.
        if first == ".DS_Store" {
            continue;
        }
        let is_dir = dir_entry || comps.next().is_some();
        *top.entry(first.to_owned()).or_insert(false) |= is_dir;
    }
    let (name, is_dir) = top.iter().next()?;
    (top.len() == 1 && *is_dir).then(|| name.clone())
}

/// Host byte and external attributes of every entry, keyed by raw name, read
/// from the central directory: the crate keeps the host out of its public API
/// and `unix_mode()` SYNTHESISES a mode for a DOS host (`0664` / `0775`,
/// `types.rs:562-573`) where `unzip` poses something else entirely. Record
/// layout (APPNOTE 4.3.12): signature, `version made by` at +4 (its high byte
/// is the host), name length at +28, extra length at +30, comment length at
/// +32, external attributes at +38, name at +46.
fn central_attrs(zip_bytes: &[u8]) -> std::collections::HashMap<Vec<u8>, (u8, u32)> {
    let mut out = std::collections::HashMap::new();
    let mut i = 0usize;
    while let Some(pos) = zip_bytes
        .get(i..)
        .and_then(|s| s.windows(4).position(|w| w == b"PK\x01\x02"))
        .map(|p| i + p)
    {
        let field = |off: usize| -> Option<usize> {
            let b = zip_bytes.get(pos + off..pos + off + 2)?;
            Some(u16::from_le_bytes([b[0], b[1]]) as usize)
        };
        let (Some(name_len), Some(extra_len), Some(comment_len)) =
            (field(28), field(30), field(32))
        else {
            break;
        };
        let Some(attrs) = zip_bytes
            .get(pos + 38..pos + 42)
            .map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
        else {
            break;
        };
        let Some(host) = zip_bytes.get(pos + 5).copied() else {
            break;
        };
        if let Some(name) = zip_bytes.get(pos + 46..pos + 46 + name_len) {
            out.insert(name.to_vec(), (host, attrs));
        }
        i = pos + 46 + name_len + extra_len + comment_len;
    }
    out
}

const HOST_UNIX: u8 = 3;

/// What `unzip` poses for one entry, measured case by case
/// (docs/plans/v0.20-hostile-archives.md):
/// - a unix host answers with the stored mode, WITHOUT the umask and without
///   the special bits (`04755` lands as `0755`); a stored mode of zero lands
///   as `0000`, unreadable, and that is the reference;
/// - any other host answers with the stored mode when it carries one at all,
///   then, for the DOS read-only bit, with `0444` narrowed by the process
///   default (`0400` under umask 077, measured), then with that default
///   alone — `0666` for a file, `0777` for a directory, both masked by the
///   umask, which is exactly what `fs::write` and `create_dir_all` do.
// The two payloads are read by `apply_mode`, which only does anything on unix:
// the modes of a zip are a unix notion, and Windows has no `chmod` to match.
#[cfg_attr(not(unix), allow(dead_code))]
enum ModeRule {
    Exact(u32),
    NarrowDefault(u32),
    Default,
}

fn unzip_mode(host: u8, attrs: u32) -> ModeRule {
    let stored = (attrs >> 16) & 0o777;
    if host == HOST_UNIX || stored != 0 {
        return ModeRule::Exact(stored);
    }
    if attrs & 0x01 != 0 {
        return ModeRule::NarrowDefault(0o444);
    }
    ModeRule::Default
}

/// Applies the rule to something that exists: the default is what the
/// creation already produced, so it is read back rather than guessed (no
/// `umask(2)` call, hence no unsafe and no race).
#[cfg(unix)]
fn apply_mode(path: &Path, rule: &ModeRule) -> Result<()> {
    use std::os::unix::fs::PermissionsExt as _;
    let mode = match rule {
        ModeRule::Exact(m) => *m,
        ModeRule::NarrowDefault(m) => {
            let cur = std::fs::symlink_metadata(path).map_err(Error::io(path))?;
            cur.permissions().mode() & 0o777 & m
        }
        ModeRule::Default => return Ok(()),
    };
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode)).map_err(Error::io(path))
}

#[cfg(not(unix))]
fn apply_mode(_path: &Path, _rule: &ModeRule) -> Result<()> {
    Ok(())
}

/// The path an entry lands at, or `None` when the entry is thrown away: with a
/// single root, everything beside it goes (it stays in the temporary directory
/// Composer renames away), and the root's own entry becomes nothing.
fn strip_root(raw: &Path, root: Option<&std::ffi::OsStr>) -> Option<PathBuf> {
    let mut comps = raw
        .components()
        .filter(|c| matches!(c, Component::Normal(_)))
        .peekable();
    if let Some(root) = root {
        match comps.next() {
            Some(Component::Normal(first)) if first == root => {}
            _ => return None,
        }
    }
    let rest: PathBuf = comps.collect();
    (!rest.as_os_str().is_empty()).then_some(rest)
}

/// Memoized `create_dir_all`: avoids one `create_dir_all` (hence one stat +
/// N syscalls) per zip entry when hundreds of files share the same parent
/// directory. Inserts the path and its newly created ancestors.
fn ensure_dir(p: &Path, made: &mut std::collections::HashSet<PathBuf>) -> Result<()> {
    if made.contains(p) {
        return Ok(());
    }
    std::fs::create_dir_all(p).map_err(Error::io(p))?;
    let mut cur = Some(p);
    while let Some(c) = cur {
        if !made.insert(c.to_path_buf()) {
            break; // ancestor already known: so is the rest
        }
        cur = c.parent();
    }
    Ok(())
}

/// Whether Composer would find an external unzip tool on this Windows box.
/// `ZipDownloader::__construct` builds its command list with Symfony's
/// `ExecutableFinder`, and on Windows it looks for **`7z` first**, with
/// `C:\Program Files\7-Zip` added to the search path — outside `PATH`
/// entirely — then for `unzip`. Probing `PATH` alone therefore answered "no
/// tool" on a machine where Composer finds one, and turned a symlink entry
/// into a plain file where the reference makes a link.
///
/// What is NOT settled here, and is written down rather than guessed: whether
/// `7z x -y` recreates a symlink entry as a link at all (it may need `-snl`).
/// The answer decides this branch on a box that has 7-Zip but no `unzip`, and
/// it needs a measurement on Windows — see CONTRIBUTING.
#[cfg(windows)]
fn windows_unzip_tool_present() -> bool {
    let in_path = |exe: &str| {
        std::env::var_os("PATH")
            .is_some_and(|path| std::env::split_paths(&path).any(|dir| dir.join(exe).exists()))
    };
    in_path("7z.exe")
        || std::path::Path::new("C:\\Program Files\\7-Zip\\7z.exe").exists()
        || in_path("unzip.exe")
}

/// Reads one entry against the budget the archive has left, counting the bytes
/// **really** read. A declared size is the archive's word, and both formats
/// let it lie: a zip's two size fields are `u32`s an attacker edits (measured
/// — 300 Mo of zeros declared as 1 octet, extracted in full with a 607 Mo
/// peak), and a tar's pax `size` record overrides the ustar header the crate
/// hands back, which is why counting headers bounded nothing at all.
///
/// `take(allowance + 1)` is what makes the lie unprofitable, and the `+ 1`
/// carries its own reason: a read stopping exactly on a `Take`'s limit hands
/// `Crc32Reader` a short read instead of the end of the stream, and it only
/// verifies the checksum once the inner reader is drained — so the integrity
/// check would be skipped in silence.
fn read_within(
    reader: &mut impl Read,
    allowance: &mut u64,
    hint: u64,
) -> std::io::Result<Option<Vec<u8>>> {
    let mut buf = Vec::with_capacity(hint.min(*allowance) as usize);
    let read = reader.by_ref().take(*allowance + 1).read_to_end(&mut buf)? as u64;
    if read > *allowance {
        return Ok(None);
    }
    *allowance -= read;
    Ok(Some(buf))
}

pub fn extract_zip(zip_bytes: &[u8], dest: &Path) -> Result<()> {
    extract_zip_with_limit(zip_bytes, dest, MAX_UNCOMPRESSED)
}

/// `extract_zip` with the budget as an argument: the tests prove the
/// accounting on a handful of bytes instead of half a gigabyte.
fn extract_zip_with_limit(zip_bytes: &[u8], dest: &Path, limit: u64) -> Result<()> {
    let mut archive =
        zip::ZipArchive::new(std::io::Cursor::new(zip_bytes)).map_err(Error::zip(dest))?;
    let mut made: std::collections::HashSet<PathBuf> = std::collections::HashSet::new();
    ensure_dir(dest, &mut made)?;
    let attrs = central_attrs(zip_bytes);
    let listing = zip_listing(&mut archive, &attrs, dest)?;
    if let Some(link) = lone_top_level_symlink(&listing) {
        return Err(Error::HostileArchive {
            dest: dest.to_path_buf(),
            reason: format!(
                "the archive's whole content is the symlink {}, which Composer would install as a link in place of the package directory",
                link.display()
            ),
        });
    }
    let strip = root_strip_of(listing.iter().map(|(p, d, _)| (p.as_path(), *d)));
    // Directory modes are applied at the END: an entry can give its directory
    // a mode that forbids writing into it (`0500`), and the files that follow
    // still have to land. `unzip` restores them at the end for the same
    // reason.
    let mut dir_modes: Vec<(PathBuf, ModeRule)> = Vec::new();
    // Symlinks this archive has created, by stripped relative path. Nothing may
    // be written THROUGH one: `check_symlink_target` resolves a target
    // lexically, which assumes every component of the path is a real
    // directory — and an archive can break that assumption with two entries.
    // `a -> .` passes (it stays inside), then `a/b -> ..` passes too (its
    // lexical parent is `a`, so `a/..` is the root), yet on disk `a` is the
    // root, so `a/b` points OUTSIDE and a file written under it escapes.
    // `unzip`, which is what `ZipDownloader::extract` runs, refuses exactly
    // this: "checkdir error: … exists but is not directory".
    let mut symlinked: std::collections::HashSet<PathBuf> = std::collections::HashSet::new();
    // Files this archive has written, for the same reason: an archive can name
    // `a` as a file and then `a/b`, and `unzip` refuses the second with
    // "checkdir error: … exists but is not directory" (exit 2), which sends
    // Composer into its `ZipArchive` fallback — where the entry fails again and
    // the install stops. Caught here before the write, so the message names the
    // conflict instead of being an `EEXIST` from `create_dir_all`.
    let mut written: std::collections::HashSet<PathBuf> = std::collections::HashSet::new();

    let mut allowance = limit;
    let over = || Error::HostileArchive {
        dest: dest.to_path_buf(),
        reason: format!("uncompressed size > {limit} bytes"),
    };
    for i in 0..archive.len() {
        let mut entry = archive.by_index(i).map_err(Error::zip(dest))?;
        let raw = entry_path(&entry, dest)?;
        let Some(stripped) = strip_root(&raw, strip.as_deref()) else {
            continue;
        };
        // The two guards below look up the entry's OWN ancestors in the sets
        // rather than scanning them: scanning was O(entries²) and showed up as
        // 325 s on a 200 000-entry archive where Composer takes 24 s (measured,
        // DECISIONS 2026-10-03). `ancestors()` yields the path itself first,
        // hence the `skip(1)`.
        if let Some(link) = stripped
            .ancestors()
            .skip(1)
            .find(|a| !a.as_os_str().is_empty() && symlinked.contains(*a))
        {
            return Err(Error::HostileArchive {
                dest: dest.to_path_buf(),
                reason: format!(
                    "entry {} would be written through the symlink {}",
                    stripped.display(),
                    link.display()
                ),
            });
        }
        if let Some(file) = stripped
            .ancestors()
            .skip(1)
            .find(|a| !a.as_os_str().is_empty() && written.contains(*a))
        {
            return Err(Error::HostileArchive {
                dest: dest.to_path_buf(),
                reason: format!(
                    "entry {} would be written under {}, which the archive wrote as a file",
                    stripped.display(),
                    file.display()
                ),
            });
        }
        if made.contains(&dest.join(&stripped)) && !entry.is_dir() {
            return Err(Error::HostileArchive {
                dest: dest.to_path_buf(),
                reason: format!(
                    "entry {} is a file where the archive already made a directory",
                    stripped.display()
                ),
            });
        }
        let out = dest.join(&stripped);

        // Host and attributes from the central directory; an entry the scan
        // did not find falls back to the crate's own answer.
        let (host, ext) = attrs.get(entry.name_raw()).copied().unwrap_or_else(|| {
            (
                HOST_UNIX,
                entry.unix_mode().map_or(0, |m| (m & 0o177777) << 16),
            )
        });
        let rule = unzip_mode(host, ext);
        let is_symlink = host == HOST_UNIX && (ext >> 16) & S_IFMT == S_IFLNK;

        if entry.is_dir() {
            ensure_dir(&out, &mut made)?;
            dir_modes.push((out.clone(), rule));
        } else if is_symlink {
            let hint = entry.size();
            let bytes = read_within(&mut entry, &mut allowance, hint)
                .map_err(Error::io(&out))?
                .ok_or_else(over)?;
            let target = String::from_utf8(bytes).map_err(|_| Error::HostileArchive {
                dest: dest.to_path_buf(),
                reason: format!("symlink {} has a non-UTF-8 target", stripped.display()),
            })?;
            check_symlink_target(&stripped, &target, dest, &symlinked)?;
            if let Some(p) = out.parent() {
                ensure_dir(p, &mut made)?;
            }
            let _ = std::fs::remove_file(&out);
            symlinked.insert(stripped.clone());
            #[cfg(unix)]
            crate::clone::symlink_like_unzip(std::path::Path::new(&target), &out)?;
            // Windows: Composer extracts with an external tool when it finds
            // one (`ZipDownloader::$unzipCommands`), and such a tool recreates
            // the link when the process may create links (the GitHub runner);
            // with PHP's ZipArchive the entry becomes an ordinary file whose
            // content is the target. Same rule here — see
            // `windows_unzip_tool_present` for what "finds one" means, which is
            // not just the PATH.
            #[cfg(windows)]
            {
                let linked = windows_unzip_tool_present()
                    && std::os::windows::fs::symlink_file(&target, &out).is_ok();
                if !linked {
                    std::fs::write(&out, target.as_bytes()).map_err(Error::io(&out))?;
                }
            }
            #[cfg(not(any(unix, windows)))]
            std::fs::write(&out, target.as_bytes()).map_err(Error::io(&out))?;
        } else {
            if let Some(p) = out.parent() {
                ensure_dir(p, &mut made)?;
            }
            let hint = entry.size();
            let buf = read_within(&mut entry, &mut allowance, hint)
                .map_err(Error::io(&out))?
                .ok_or_else(over)?;
            std::fs::write(&out, &buf).map_err(Error::io(&out))?;
            apply_mode(&out, &rule)?;
            written.insert(stripped.clone());
        }
    }
    for (path, rule) in dir_modes.iter().rev() {
        apply_mode(path, rule)?;
    }
    Ok(())
}

/// Extraction of a `tar` dist (asset-packagist: npm's `.tgz`), as
/// `TarDownloader` does it through `PharData::extractTo` — measured on
/// PHP 8.5 (docs/plans/v0.17-tar-dist.md):
/// - a file gets the exact mode of its header (no umask), special bits dropped;
/// - a directory entry's mode is ignored (0777 & ~umask, like the
///   implicit parents — npm archives have no directory entries at all);
/// - a symlink or hard-link entry becomes an EMPTY regular file with the
///   header mode (PharData never creates links: nothing to check);
/// - any other entry type (device, fifo) is refused.
///
/// Then `ArchiveDownloader`'s single-root rule, shared with zip. Paths
/// with `..` or absolute are refused (PharData normalises them lexically;
/// no real archive has one).
pub fn extract_tar(tgz_bytes: &[u8], dest: &Path) -> Result<()> {
    extract_tar_with_limit(tgz_bytes, dest, MAX_UNCOMPRESSED)
}

/// `extract_tar` with the budget as an argument, for the same reason as
/// `extract_zip_with_limit`.
fn extract_tar_with_limit(tgz_bytes: &[u8], dest: &Path, limit: u64) -> Result<()> {
    let hostile = |reason: String| Error::HostileArchive {
        dest: dest.to_path_buf(),
        reason,
    };
    // A tar name is bytes, and `PharData` hands them to the filesystem as they
    // are — so on unix so do we, as for a zip entry: refusing a name that is not
    // UTF-8 meant refusing an archive the reference installs. On a filesystem
    // that will not have such a name (APFS) both sides fail, which is the
    // agreement that matters.
    let path_of = |bytes: &[u8]| -> Result<PathBuf> {
        #[cfg(unix)]
        let p = {
            use std::os::unix::ffi::OsStrExt as _;
            PathBuf::from(std::ffi::OsStr::from_bytes(bytes))
        };
        #[cfg(not(unix))]
        let p = PathBuf::from(
            std::str::from_utf8(bytes)
                .map_err(|_| hostile("entry path is not UTF-8".to_owned()))?,
        );
        if p.as_os_str().is_empty()
            || p.is_absolute()
            || !p
                .components()
                .all(|c| matches!(c, Component::Normal(_) | Component::CurDir))
        {
            return Err(hostile(format!("invalid entry path: {p:?}")));
        }
        Ok(p)
    };
    // Whole archive read once: the single-root rule needs every path
    // before the first write, and a tar stream is not seekable.
    struct Entry {
        path: PathBuf,
        kind: tar::EntryType,
        #[cfg_attr(not(unix), allow(dead_code))]
        mode: u32,
        data: Vec<u8>,
    }
    let mut entries: Vec<Entry> = Vec::new();
    let mut allowance = limit;
    let decoder = flate2::read::GzDecoder::new(tgz_bytes);
    let mut archive = tar::Archive::new(decoder);
    let iter = archive.entries().map_err(Error::io(dest))?;
    for entry in iter {
        let mut entry = entry.map_err(Error::io(dest))?;
        let kind = entry.header().entry_type();
        let path = {
            let raw = entry.path_bytes();
            path_of(&raw)?
        };
        // `& 0o777`, not `& 0o7777`: PharData drops setuid/setgid/sticky
        // (measured — 04755, 02755 and 01755 all land as 0755), and keeping
        // them wrote a setuid file where the reference writes none.
        let mode = entry.header().mode().map_err(Error::io(dest))? & 0o777;
        let data = match kind {
            tar::EntryType::Regular | tar::EntryType::Continuous => {
                // `entry.size()`, not `header().size()`: the crate bounds its
                // own reads with the former, which a pax `size` record
                // overrides — so counting the latter counted a number nobody
                // reads (measured: 261 Ko of `.tgz` read 256 Mo with the
                // total stuck at 0).
                let hint = entry.size();
                read_within(&mut entry, &mut allowance, hint)
                    .map_err(Error::io(dest))?
                    .ok_or_else(|| hostile(format!("uncompressed size > {limit} bytes")))?
            }
            tar::EntryType::Directory | tar::EntryType::Symlink | tar::EntryType::Link => {
                Vec::new()
            }
            // pax / GNU long-name headers are consumed by the crate; what
            // reaches here with another type is not a file tree.
            other => {
                return Err(hostile(format!(
                    "unsupported entry type {other:?} for {}",
                    path.display()
                )))
            }
        };
        entries.push(Entry {
            path,
            kind,
            mode,
            data,
        });
    }
    let strip = root_strip_of(
        entries
            .iter()
            .map(|e| (e.path.as_path(), e.kind == tar::EntryType::Directory)),
    );
    let mut made: std::collections::HashSet<PathBuf> = std::collections::HashSet::new();
    ensure_dir(dest, &mut made)?;
    for e in &entries {
        let Some(stripped) = strip_root(&e.path, strip.as_deref()) else {
            continue;
        };
        let out = dest.join(&stripped);
        if e.kind == tar::EntryType::Directory {
            ensure_dir(&out, &mut made)?;
            continue;
        }
        if let Some(p) = out.parent() {
            ensure_dir(p, &mut made)?;
        }
        std::fs::write(&out, &e.data).map_err(Error::io(&out))?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            std::fs::set_permissions(&out, std::fs::Permissions::from_mode(e.mode))
                .map_err(Error::io(&out))?;
        }
    }
    Ok(())
}

/// A symlink target must be relative and stay lexically inside the extracted
/// root (the classic attack: `link -> ../../../../etc/passwd`).
fn check_symlink_target(
    link_rel: &Path,
    target: &str,
    dest: &Path,
    symlinked: &std::collections::HashSet<PathBuf>,
) -> Result<()> {
    let hostile = |reason: String| Error::HostileArchive {
        dest: dest.to_path_buf(),
        reason,
    };
    let target_path = Path::new(target);
    if target_path.is_absolute() {
        return Err(hostile(format!(
            "absolute symlink: {link_rel:?} -> {target}"
        )));
    }
    // Resolved lexically from the link's own directory, keeping the components
    // and not just a depth: they are what says whether the target crosses a
    // symlink this archive created, whose real place on disk is elsewhere.
    let mut parts: Vec<std::ffi::OsString> = link_rel
        .components()
        .filter_map(|c| match c {
            Component::Normal(n) => Some(n.to_owned()),
            _ => None,
        })
        .collect();
    parts.pop(); // the link's own name: what remains is its directory
    let comps: Vec<Component<'_>> = target_path.components().collect();
    for (i, c) in comps.iter().enumerate() {
        match c {
            Component::ParentDir => {
                if parts.pop().is_none() {
                    return Err(hostile(format!(
                        "symlink escaping the archive: {link_rel:?} -> {target}"
                    )));
                }
            }
            Component::Normal(n) => {
                parts.push((*n).to_owned());
                // Crossing a symlink this archive created is what makes the
                // lexical resolution a lie; pointing AT one is harmless, so
                // only a component with something after it counts.
                if i + 1 < comps.len() {
                    let crossed: PathBuf = parts.iter().collect();
                    if symlinked.contains(&crossed) {
                        return Err(hostile(format!(
                            "symlink {link_rel:?} -> {target} passes through the symlink {}",
                            crossed.display()
                        )));
                    }
                }
            }
            Component::CurDir => {}
            _ => {
                return Err(hostile(format!(
                    "invalid symlink: {link_rel:?} -> {target}"
                )))
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tar_tests {
    use super::*;

    fn tgz(entries: &[(&str, tar::EntryType, u32, &[u8])]) -> Vec<u8> {
        let mut b = tar::Builder::new(Vec::new());
        for (path, kind, mode, data) in entries {
            let mut h = tar::Header::new_gnu();
            h.set_entry_type(*kind);
            h.set_mode(*mode);
            h.set_size(data.len() as u64);
            h.set_mtime(0);
            match kind {
                tar::EntryType::Symlink | tar::EntryType::Link => {
                    b.append_link(&mut h, path, "target").expect("link")
                }
                // The builder refuses `..` and absolute paths: raw header
                // for the hostile cases.
                _ if path.contains("..") || path.starts_with('/') => {
                    let gnu = h.as_gnu_mut().expect("gnu");
                    gnu.name[..path.len()].copy_from_slice(path.as_bytes());
                    h.set_cksum();
                    b.append(&h, *data).expect("raw")
                }
                _ => b.append_data(&mut h, path, *data).expect("data"),
            }
        }
        let raw = b.into_inner().expect("tar");
        let mut e = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
        std::io::Write::write_all(&mut e, &raw).expect("gz");
        e.finish().expect("gz")
    }

    #[test]
    fn strips_single_root_keeps_modes_and_flattens_links() {
        use tar::EntryType as T;
        let d = tempfile::tempdir().expect("tmp");
        let out = d.path().join("out");
        extract_tar(
            &tgz(&[
                ("package/a.txt", T::Regular, 0o664, b"a"),
                ("package/bin/x", T::Regular, 0o755, b"#!"),
                ("package/dir/", T::Directory, 0o700, b""),
                ("package/link", T::Symlink, 0o777, b""),
            ]),
            &out,
        )
        .expect("extract");
        assert_eq!(std::fs::read(out.join("a.txt")).expect("a"), b"a");
        assert!(out.join("dir").is_dir());
        let link = std::fs::symlink_metadata(out.join("link")).expect("link");
        assert!(
            link.is_file() && link.len() == 0,
            "a link becomes an empty file"
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            let mode = |p: &str| {
                std::fs::metadata(out.join(p))
                    .expect(p)
                    .permissions()
                    .mode()
                    & 0o7777
            };
            assert_eq!(mode("a.txt"), 0o664);
            assert_eq!(mode("bin/x"), 0o755);
            assert_eq!(mode("link"), 0o777);
            assert_ne!(mode("dir"), 0o700, "directory modes are not applied");
        }
    }

    #[test]
    fn refuses_escapes_and_devices() {
        use tar::EntryType as T;
        let d = tempfile::tempdir().expect("tmp");
        for (path, kind) in [
            ("package/../escape", T::Regular),
            ("/abs", T::Regular),
            ("package/dev", T::Char),
        ] {
            let r = extract_tar(&tgz(&[(path, kind, 0o644, b"x")]), &d.path().join("o"));
            assert!(
                matches!(r, Err(Error::HostileArchive { .. })),
                "{path} should be refused: {r:?}"
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write as _;
    use zip::write::SimpleFileOptions;

    fn build_zip(entries: &[(&str, &[u8], Option<u32>)]) -> Vec<u8> {
        let mut w = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
        for (name, content, mode) in entries {
            let mut opts = SimpleFileOptions::default();
            if let Some(m) = mode {
                opts = opts.unix_permissions(*m);
            }
            if name.ends_with('/') {
                w.add_directory(name.trim_end_matches('/'), opts)
                    .expect("dir");
            } else {
                w.start_file(*name, opts).expect("start");
                w.write_all(content).expect("write");
            }
        }
        w.finish().expect("finish").into_inner()
    }

    /// Rewrites the declared uncompressed size in the local header (+22) and
    /// in the central directory record (+24), which is all an attacker has to
    /// do to make a plafond that counts declared sizes let anything through.
    fn forge_declared_size(zip: &mut [u8], real: u32, forged: u32) {
        let mut patched = 0;
        for (sig, off) in [
            (b"PK\x03\x04".as_slice(), 22usize),
            (b"PK\x01\x02".as_slice(), 24),
        ] {
            let mut i = 0;
            while let Some(pos) = zip[i..].windows(4).position(|w| w == sig).map(|p| i + p) {
                let cur = u32::from_le_bytes(zip[pos + off..pos + off + 4].try_into().expect("4"));
                if cur == real {
                    zip[pos + off..pos + off + 4].copy_from_slice(&forged.to_le_bytes());
                    patched += 1;
                }
                i = pos + 4;
            }
        }
        assert_eq!(patched, 2, "the two size fields of the entry");
    }

    #[test]
    fn a_zip_that_lies_about_its_size_is_still_bounded() {
        let payload = vec![b'z'; 4096];
        let mut zip = build_zip(&[("pkg/big.bin", &payload, Some(0o644))]);
        forge_declared_size(&mut zip, 4096, 1);
        let d = tmpdir();
        // A budget of 1024 bytes: the declared size says 1, the entry holds
        // 4096, and what counts is what is read.
        let err = extract_zip_with_limit(&zip, d.path(), 1024).expect_err("refused");
        assert!(
            format!("{err}").contains("uncompressed size"),
            "unexpected error: {err}"
        );
        assert!(!d.path().join("big.bin").exists());
    }

    #[test]
    fn a_zip_of_many_small_entries_shares_one_budget() {
        // Each entry is honest and small; the budget is for the archive, not
        // for one entry, so the fourth one is refused.
        let payload = vec![b'x'; 400];
        let zip = build_zip(&[
            ("pkg/a", &payload, Some(0o644)),
            ("pkg/b", &payload, Some(0o644)),
            ("pkg/c", &payload, Some(0o644)),
            ("pkg/d", &payload, Some(0o644)),
        ]);
        let d = tmpdir();
        let err = extract_zip_with_limit(&zip, d.path(), 1024).expect_err("refused");
        assert!(format!("{err}").contains("uncompressed size"), "{err}");
    }

    #[test]
    fn an_honest_zip_just_under_the_budget_is_extracted() {
        let payload = vec![b'y'; 1000];
        let zip = build_zip(&[("pkg/ok.bin", &payload, Some(0o644))]);
        let d = tmpdir();
        extract_zip_with_limit(&zip, d.path(), 1024).expect("extracted");
        assert_eq!(
            std::fs::read(d.path().join("ok.bin")).expect("read").len(),
            1000
        );
    }

    /// The tar counterpart: a pax `size` record overrides the ustar header,
    /// so counting the header counted a number nobody reads. The archive is
    /// built by hand — no writer exposes a pax record — and the test asserts
    /// the lie itself before asserting the refusal.
    #[test]
    fn a_tar_that_lies_in_its_header_is_still_bounded() {
        const REAL: usize = 4096;
        let data = vec![b't'; REAL];
        let mut builder = tar::Builder::new(Vec::new());
        // The pax extended header that applies to the next entry: one record,
        // "<total length> size=4096\n", length included in the count.
        let body = format!("size={REAL}\n");
        let mut total = body.len() + 3;
        while total.to_string().len() + 1 + body.len() != total {
            total += 1;
        }
        let record = format!("{total} {body}");
        assert_eq!(record.len(), total, "a pax record counts its own length");
        let mut px = tar::Header::new_ustar();
        px.set_entry_type(tar::EntryType::XHeader);
        px.set_mode(0o644);
        px.set_mtime(0);
        px.set_size(record.len() as u64);
        px.set_path("PaxHeaders/big.bin").expect("pax path");
        px.set_cksum();
        builder.append(&px, record.as_bytes()).expect("pax header");
        // The entry itself: a ustar header announcing zero, followed by the
        // 4096 bytes the pax record promises.
        let mut h = tar::Header::new_ustar();
        h.set_entry_type(tar::EntryType::Regular);
        h.set_mode(0o644);
        h.set_mtime(0);
        h.set_size(0);
        h.set_path("pkg/big.bin").expect("path");
        h.set_cksum();
        builder.append(&h, data.as_slice()).expect("entry");
        let tar_bytes = builder.into_inner().expect("tar");
        let mut enc = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
        enc.write_all(&tar_bytes).expect("gzip");
        let tgz = enc.finish().expect("gzip");

        // The lie, as the crate sees it.
        {
            let mut archive = tar::Archive::new(flate2::read::GzDecoder::new(tgz.as_slice()));
            let entry = archive
                .entries()
                .expect("entries")
                .next()
                .expect("one entry")
                .expect("entry");
            assert_eq!(entry.header().size().expect("header size"), 0);
            assert_eq!(entry.size(), REAL as u64);
        }

        let d = tmpdir();
        let err = extract_tar_with_limit(&tgz, d.path(), 1024).expect_err("refused");
        assert!(format!("{err}").contains("uncompressed size"), "{err}");
    }

    #[test]
    fn an_archive_that_is_only_a_symlink_is_refused() {
        // Measured on Composer: `is_dir()` follows the link, so the strip
        // applies and the rename moves the LINK onto the package directory —
        // `vendor/hostile/rootlink -> ..`. A store entry is a tree, so this is
        // refused instead of laid out as a directory holding a link.
        let mut w = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
        w.add_symlink("pkg", "..", SimpleFileOptions::default())
            .expect("symlink");
        let zip = w.finish().expect("finish").into_inner();
        let d = tmpdir();
        let err = extract_zip(&zip, d.path()).expect_err("refused");
        assert!(
            format!("{err}").contains("whole content is the symlink"),
            "{err}"
        );
        // A .DS_Store beside it changes nothing: the Finder excludes it by name.
        let mut w = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
        w.start_file(".DS_Store", SimpleFileOptions::default())
            .expect("start");
        w.write_all(b"junk").expect("write");
        w.add_symlink("pkg", ".", SimpleFileOptions::default())
            .expect("symlink");
        let zip = w.finish().expect("finish").into_inner();
        let d = tmpdir();
        assert!(extract_zip(&zip, d.path()).is_err());
        // Two top-level entries: not that case at all, and the link stays a
        // link inside the package.
        let mut w = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
        w.start_file("pkg/real.txt", SimpleFileOptions::default())
            .expect("start");
        w.write_all(b"x").expect("write");
        w.add_symlink("pkg/alias.txt", "real.txt", SimpleFileOptions::default())
            .expect("symlink");
        let zip = w.finish().expect("finish").into_inner();
        let d = tmpdir();
        extract_zip(&zip, d.path()).expect("laid out");
        let alias = d.path().join("alias.txt");
        let meta = alias.symlink_metadata().expect("meta");
        #[cfg(unix)]
        assert!(meta.file_type().is_symlink());
        // Windows: a link when a tool is found AND the process may create one,
        // the target's bytes in a plain file otherwise — the rule the branch
        // above documents, so the test accepts exactly those two outcomes.
        #[cfg(windows)]
        assert!(
            meta.file_type().is_symlink() || std::fs::read(&alias).expect("read") == b"real.txt",
            "neither a link nor the target's bytes"
        );
    }

    #[test]
    fn a_file_and_a_path_under_it_are_refused_both_ways() {
        // `unzip` answers "checkdir error: … exists but is not directory" and
        // exits 2, which sends Composer into its ZipArchive fallback, where the
        // entry fails again and the install stops. Both sides fail; the point
        // of refusing here is to say WHICH entry conflicts with which, instead
        // of surfacing an `EEXIST` from `create_dir_all`.
        let zip = build_zip(&[
            ("pkg/a", b"i am a file", Some(0o600)),
            ("pkg/a/b", b"and i am under it", Some(0o644)),
        ]);
        let d = tmpdir();
        let err = extract_zip(&zip, d.path()).expect_err("refused");
        assert!(
            format!("{err}").contains("entry a/b would be written under a"),
            "{err}"
        );

        // The other order: the directory is made first, then an entry claims
        // its name as a file.
        let zip = build_zip(&[
            ("pkg/a/b", b"under first", Some(0o644)),
            ("pkg/a", b"then the file", Some(0o600)),
        ]);
        let d = tmpdir();
        let err = extract_zip(&zip, d.path()).expect_err("refused");
        assert!(
            format!("{err}")
                .contains("entry a is a file where the archive already made a directory"),
            "{err}"
        );

        // A directory entry named like one already made is not a conflict: an
        // archive may well carry `pkg/a/` after `pkg/a/b`.
        let zip = build_zip(&[("pkg/a/b", b"x", Some(0o644)), ("pkg/a/", b"", Some(0o755))]);
        let d = tmpdir();
        extract_zip(&zip, d.path()).expect("laid out");
        assert!(d.path().join("a").is_dir());
    }

    /// Windows, measured on the machine that runs the test rather than
    /// assumed: Composer extracts with `7z x -bb0 -y %file% -o%path%` when it
    /// finds 7-Zip (PATH **or** `C:\Program Files\7-Zip`, preferred over
    /// `unzip` there) and with `unzip -qq %file% -d %path%` otherwise
    /// (`ZipDownloader.php:47-51`). Whether that tool recreates a symlink entry
    /// as a link — 7-Zip may want `-snl` — decides what `extract_zip` must
    /// produce, and no amount of reading settles it. So the test runs the tool
    /// Composer would run, looks at what it made, and demands the same of us.
    ///
    /// Run with `-- --nocapture` in CI so the answer is in the log, not only in
    /// a failure.
    #[test]
    #[cfg(windows)]
    fn a_symlink_entry_follows_whatever_tool_composer_would_use() {
        use std::process::Command;
        let zip = {
            let mut w = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
            w.start_file("pkg/real.txt", SimpleFileOptions::default())
                .expect("start");
            w.write_all(b"real").expect("write");
            w.add_symlink(
                "pkg/link.txt",
                "real.txt",
                SimpleFileOptions::default().unix_permissions(0o777),
            )
            .expect("symlink");
            w.finish().expect("finish").into_inner()
        };

        let work = tmpdir();
        let file = work.path().join("dist.zip");
        std::fs::write(&file, &zip).expect("write zip");

        // The tool Composer would pick, in Composer's order.
        let in_path = |exe: &str| {
            std::env::var_os("PATH").and_then(|path| {
                std::env::split_paths(&path)
                    .map(|dir| dir.join(exe))
                    .find(|p| p.exists())
            })
        };
        let seven = in_path("7z.exe").or_else(|| {
            let p = std::path::PathBuf::from("C:\\Program Files\\7-Zip\\7z.exe");
            p.exists().then_some(p)
        });
        let tool = seven
            .map(|p| (p, true))
            .or_else(|| in_path("unzip.exe").map(|p| (p, false)));

        // Can this process create a link at all? Without Developer Mode or the
        // privilege, no — and then the file is the only possible answer.
        let may_link = {
            let probe = work.path().join("probe");
            std::os::windows::fs::symlink_file("target", &probe).is_ok()
        };

        let tool_made_link = match &tool {
            Some((exe, is_7z)) => {
                let out = work.path().join("tool");
                std::fs::create_dir_all(&out).expect("mkdir");
                let status = if *is_7z {
                    Command::new(exe)
                        .args(["x", "-bb0", "-y"])
                        .arg(&file)
                        .arg(format!("-o{}", out.display()))
                        .status()
                } else {
                    Command::new(exe)
                        .arg("-qq")
                        .arg(&file)
                        .arg("-d")
                        .arg(&out)
                        .status()
                };
                let made = out.join("pkg/link.txt");
                let meta = std::fs::symlink_metadata(&made);
                let answer = meta
                    .as_ref()
                    .map(|m| m.file_type().is_symlink())
                    .unwrap_or(false);
                eprintln!(
                    "tool {} (7z={}) exited {:?}: pkg/link.txt is {}",
                    exe.display(),
                    is_7z,
                    status.map(|s| s.code()),
                    match meta {
                        Ok(m) if m.file_type().is_symlink() => "a symlink".to_owned(),
                        Ok(_) => format!(
                            "a regular file holding {:?}",
                            std::fs::read_to_string(&made).unwrap_or_default()
                        ),
                        Err(e) => format!("absent ({e})"),
                    }
                );
                answer
            }
            None => {
                eprintln!("no 7z and no unzip on this machine: Composer would use ZipArchive, which never makes a link");
                false
            }
        };
        eprintln!("this process may create links: {may_link}");

        let ours = work.path().join("ours");
        extract_zip(&zip, &ours).expect("extract_zip");
        let link = ours.join("link.txt");
        let meta = std::fs::symlink_metadata(&link).expect("meta");
        if tool_made_link && may_link {
            assert!(
                meta.file_type().is_symlink(),
                "the tool made a link and links are allowed, so we must make one"
            );
            assert_eq!(
                std::fs::read_link(&link).expect("readlink"),
                std::path::Path::new("real.txt")
            );
        } else {
            assert!(
                !meta.file_type().is_symlink(),
                "no link was possible, so the entry must be a plain file"
            );
            assert_eq!(std::fs::read(&link).expect("read"), b"real.txt");
        }
    }

    fn tmpdir() -> tempfile::TempDir {
        tempfile::tempdir().expect("tmpdir")
    }

    /// Entries in order: a link when `link` is true (`add_symlink`, what a
    /// zipball carries), an ordinary file otherwise.
    fn build_mixed(entries: &[(&str, &str, bool)]) -> Vec<u8> {
        let mut w = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
        for (name, payload, link) in entries {
            if *link {
                w.add_symlink(*name, *payload, SimpleFileOptions::default())
                    .expect("symlink");
            } else {
                w.start_file(*name, SimpleFileOptions::default())
                    .expect("start");
                w.write_all(payload.as_bytes()).expect("write");
            }
        }
        w.finish().expect("finish").into_inner()
    }

    #[test]
    fn a_chain_of_symlinks_cannot_escape() {
        // Reported by a third-party review of v0.18.0, reproduced: `a -> .`
        // stays inside, and `a/b -> ..` resolves to the root LEXICALLY (its
        // parent being `a`), so both passed the target check — yet on disk `a`
        // IS the root, so `a/b` points outside and a file written under it
        // escaped the extraction directory. `unzip`, which
        // `ZipDownloader::extract` runs, refuses the same thing ("exists but is
        // not directory"), so nothing of Composer's behaviour is given up.
        let d = tmpdir();
        let dest = d.path().join("pkg");
        let zip = build_mixed(&[
            ("pkg/a", ".", true),
            ("pkg/a/b", "..", true),
            ("pkg/a/b/pwned.txt", "ESCAPED", false),
        ]);
        let err = extract_zip(&zip, &dest).expect_err("must be refused");
        assert!(
            format!("{err}").contains("through the symlink"),
            "unexpected error: {err}"
        );
        assert!(
            !d.path().join("pwned.txt").exists(),
            "a file was written outside the extraction directory"
        );
    }

    #[test]
    fn a_symlink_target_may_not_cross_a_symlink_of_the_archive() {
        // The same family, one step earlier: nothing is written through `a`
        // here, but `z`'s target crosses it, so the link left on disk would
        // point outside once followed.
        let d = tmpdir();
        let dest = d.path().join("pkg");
        let zip = build_mixed(&[("pkg/a", ".", true), ("pkg/z", "a/../evil", true)]);
        let err = extract_zip(&zip, &dest).expect_err("must be refused");
        assert!(
            format!("{err}").contains("passes through the symlink"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn a_plain_symlink_inside_the_package_still_works() {
        // The guard must not cost the ordinary case: a link next to its target.
        let d = tmpdir();
        let dest = d.path().join("pkg");
        let zip = build_mixed(&[
            ("pkg/src/Real.php", "<?php // real", false),
            ("pkg/src/Alias.php", "Real.php", true),
        ]);
        extract_zip(&zip, &dest).expect("a legitimate link is extracted");
        let alias = dest.join("src/Alias.php");
        let meta = std::fs::symlink_metadata(&alias).expect("alias");
        if meta.file_type().is_symlink() {
            assert_eq!(
                std::fs::read_to_string(&alias).expect("read through the link"),
                "<?php // real"
            );
        } else {
            // Windows without unzip/7z: an ordinary file holding the target.
            assert_eq!(
                std::fs::read_to_string(&alias).expect("read the file"),
                "Real.php"
            );
        }
    }

    #[test]
    fn strips_root_and_preserves_exec_bit() {
        let zip = build_zip(&[
            ("root-abc/", b"", None),
            ("root-abc/src/a.php", b"<?php", None),
            (
                "root-abc/bin/tool",
                b"#!/usr/bin/env php\n<?php",
                Some(0o100755),
            ),
        ]);
        let d = tmpdir();
        extract_zip(&zip, d.path()).expect("extract");
        assert!(d.path().join("src/a.php").is_file());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            let mode = std::fs::metadata(d.path().join("bin/tool"))
                .expect("meta")
                .permissions()
                .mode();
            assert_eq!(mode & 0o111, 0o111, "executable bit lost");
        }
    }

    #[test]
    fn strips_only_a_single_top_level_directory() {
        // Single directory without an explicit directory entry: strip.
        let single = build_zip(&[("pkg/a.txt", b"a", None), ("pkg/sub/b.txt", b"b", None)]);
        let d = tmpdir();
        extract_zip(&single, d.path()).expect("extract");
        assert!(d.path().join("a.txt").is_file() && d.path().join("sub/b.txt").is_file());

        // File at the root + directory: nothing is stripped, nothing is lost.
        let mixed = build_zip(&[("README", b"r", None), ("src/a.php", b"<?php", None)]);
        let d = tmpdir();
        extract_zip(&mixed, d.path()).expect("extract");
        assert!(d.path().join("README").is_file(), "root file lost");
        assert!(
            d.path().join("src/a.php").is_file(),
            "directory wrongly flattened"
        );

        // Two top-level directories: nothing is stripped.
        let two = build_zip(&[("a/x", b"x", None), ("b/y", b"y", None)]);
        let d = tmpdir();
        extract_zip(&two, d.path()).expect("extract");
        assert!(d.path().join("a/x").is_file() && d.path().join("b/y").is_file());

        // A single file at the root: not a directory, no strip.
        let file = build_zip(&[("only.txt", b"o", None)]);
        let d = tmpdir();
        extract_zip(&file, d.path()).expect("extract");
        assert!(d.path().join("only.txt").is_file(), "single file lost");

        // Top-level .DS_Store: ignored for the count (the single directory is
        // stripped, it disappears); without a single directory, extracted like
        // the rest.
        let ds = build_zip(&[(".DS_Store", b"junk", None), ("pkg/a.txt", b"a", None)]);
        let d = tmpdir();
        extract_zip(&ds, d.path()).expect("extract");
        assert!(d.path().join("a.txt").is_file());
        assert!(!d.path().join(".DS_Store").exists());
        let ds2 = build_zip(&[(".DS_Store", b"junk", None), ("a.txt", b"a", None)]);
        let d = tmpdir();
        extract_zip(&ds2, d.path()).expect("extract");
        assert!(d.path().join("a.txt").is_file() && d.path().join(".DS_Store").is_file());
    }

    fn build_zip_with_symlink(link_name: &str, target: &str) -> Vec<u8> {
        let mut w = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
        w.add_directory("r", SimpleFileOptions::default())
            .expect("dir");
        w.start_file("r/real.txt", SimpleFileOptions::default())
            .expect("start");
        w.write_all(b"x").expect("write");
        w.add_symlink(link_name, target, SimpleFileOptions::default())
            .expect("symlink");
        w.finish().expect("finish").into_inner()
    }

    #[test]
    fn valid_symlink_is_recreated_hostile_ones_rejected() {
        let ok = build_zip_with_symlink("r/sub/link", "../real.txt");
        let d = tmpdir();
        extract_zip(&ok, d.path()).expect("extract");
        let meta = d.path().join("sub/link").symlink_metadata().expect("meta");
        #[cfg(unix)]
        assert!(meta.file_type().is_symlink());
        #[cfg(not(unix))]
        {
            // A link when unzip/7z is on the PATH and links may be created
            // (the GitHub runner), else — like PHP ZipArchive — an ordinary
            // file containing the target.
            if meta.file_type().is_symlink() {
                assert_eq!(
                    std::fs::read_link(d.path().join("sub/link")).expect("target"),
                    std::path::PathBuf::from("../real.txt")
                );
            } else {
                assert!(meta.file_type().is_file());
                assert_eq!(
                    std::fs::read(d.path().join("sub/link")).expect("read"),
                    b"../real.txt"
                );
            }
        }

        for target in ["../../etc/passwd", "/etc/passwd", "../../../x"] {
            let bad = build_zip_with_symlink("r/link", target);
            let d = tmpdir();
            assert!(
                extract_zip(&bad, d.path()).is_err(),
                "hostile symlink accepted: {target}"
            );
        }
    }

    #[test]
    fn zip_slip_paths_are_rejected() {
        // The writer sanitises names: we forge the `..` by byte-patching
        // (same length), as a crafted archive would.
        let benign = build_zip(&[("r/", b"", None), ("r/AA/evil.txt", b"x", None)]);
        let patched: Vec<u8> = {
            let needle = b"r/AA/evil.txt";
            let replacement = b"r/../evil.txt";
            let mut bytes = benign.clone();
            let mut i = 0;
            while i + needle.len() <= bytes.len() {
                if &bytes[i..i + needle.len()] == needle {
                    bytes[i..i + needle.len()].copy_from_slice(replacement);
                }
                i += 1;
            }
            bytes
        };
        assert_ne!(benign, patched, "the patch replaced nothing");
        let d = tmpdir();
        assert!(
            extract_zip(&patched, d.path()).is_err(),
            "zip-slip accepted"
        );
    }

    #[test]
    fn bzip2_entry_decompresses_through_the_pure_rust_backend() {
        // The bzip2 backend is libbz2-rs-sys (workspace Cargo.toml) so an
        // embedder's link stays free of BZ2_* symbols; no fixture carries a
        // bzip2-compressed entry, so the decoder is exercised here.
        let mut w = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
        let opts = SimpleFileOptions::default().compression_method(zip::CompressionMethod::Bzip2);
        w.start_file("pkg/src/a.php", opts).expect("start");
        let body = "<?php // enough bytes for a real bzip2 block\n".repeat(200);
        w.write_all(body.as_bytes()).expect("write");
        let bytes = w.finish().expect("finish").into_inner();

        let d = tmpdir();
        extract_zip(&bytes, d.path()).expect("extract");
        let out = std::fs::read_to_string(d.path().join("src/a.php")).expect("read");
        assert_eq!(out, body);
    }
}

/// The bytes of one file of a zip dist, addressed as the extracted tree
/// addresses it: the same single-root strip, then a `/`-separated relative
/// path, and then the same semantics a read of the laid-out file has.
///
/// That last part is the whole difficulty, and three things follow from it:
/// the extraction writes every entry in order, so a name appearing twice
/// ends up holding the LAST one's bytes; `extract_zip` turns a symlink entry
/// into a real symlink, so reading it yields its target's bytes; and on a
/// case-insensitive filesystem — macOS and Windows — a read of
/// `src/AcmeBundle.php` finds an entry named `src/acmebundle.php`. A match
/// that only differs in case is accepted after an exact one fails: on a
/// case-sensitive host that can only over-answer, and the caller's
/// over-answer is a needless hand-over.
///
/// `None` when the archive holds no such file: a directory entry, a missing
/// name, or a symlink whose target is absent or escapes the package (an
/// archive `extract_zip` would refuse outright).
pub fn read_zip_entry(zip_bytes: &[u8], rel: &str) -> Result<Option<Vec<u8>>> {
    let dest = Path::new("<archive>");
    let mut archive =
        zip::ZipArchive::new(std::io::Cursor::new(zip_bytes)).map_err(Error::zip(dest))?;
    let mut entries: Vec<(PathBuf, bool, u32)> = Vec::with_capacity(archive.len());
    for i in 0..archive.len() {
        let entry = archive.by_index(i).map_err(Error::zip(dest))?;
        entries.push((
            entry_path(&entry, dest)?,
            entry.is_dir(),
            entry.unix_mode().unwrap_or(0),
        ));
    }
    let strip = root_strip_of(entries.iter().map(|(p, d, _)| (p.as_path(), *d)));
    // Later entries win, as the extraction's writes do.
    let mut by_path: Vec<(Vec<std::ffi::OsString>, usize, bool)> = Vec::new();
    for (i, (path, dir, mode)) in entries.iter().enumerate() {
        if *dir {
            continue;
        }
        let Some(comps) = stripped(path, strip.as_deref()) else {
            continue;
        };
        let link = mode & S_IFMT == S_IFLNK;
        if let Some(slot) = by_path.iter_mut().find(|(c, _, _)| *c == comps) {
            *slot = (comps, i, link);
        } else {
            by_path.push((comps, i, link));
        }
    }
    let find = |wanted: &[std::ffi::OsString]| -> Option<(usize, bool)> {
        by_path
            .iter()
            .find(|(c, _, _)| c == wanted)
            .or_else(|| {
                by_path
                    .iter()
                    .find(|(c, _, _)| same_ignoring_case(c, wanted))
            })
            .map(|(_, i, link)| (*i, *link))
    };
    let mut wanted = components(Path::new(rel));
    // `ELOOP`: a symlink chain the filesystem would refuse to follow.
    for _ in 0..8 {
        let Some((index, link)) = find(&wanted) else {
            return Ok(None);
        };
        let bytes = read_zip_index(&mut archive, index, dest, rel)?;
        if !link {
            return Ok(Some(bytes));
        }
        let Ok(target) = String::from_utf8(bytes) else {
            return Ok(None);
        };
        let Some(next) = resolve_link(&wanted, &target) else {
            return Ok(None);
        };
        wanted = next;
    }
    Ok(None)
}

fn read_zip_index(
    archive: &mut zip::ZipArchive<std::io::Cursor<&[u8]>>,
    index: usize,
    dest: &Path,
    rel: &str,
) -> Result<Vec<u8>> {
    let mut entry = archive.by_index(index).map_err(Error::zip(dest))?;
    let hint = entry.size();
    let mut allowance = MAX_UNCOMPRESSED;
    read_within(&mut entry, &mut allowance, hint)
        .map_err(Error::io(dest))?
        .ok_or_else(|| Error::HostileArchive {
            dest: dest.to_path_buf(),
            reason: format!("entry {rel} is absurdly large"),
        })
}

/// A symlink's target, resolved lexically against the link's own path, as
/// the filesystem resolves it. `None` when it leaves the package root, which
/// `check_symlink_target` refuses at extraction time, or when it is absolute.
fn resolve_link(link: &[std::ffi::OsString], target: &str) -> Option<Vec<std::ffi::OsString>> {
    let target = Path::new(target);
    if target.is_absolute() {
        return None;
    }
    let mut out: Vec<std::ffi::OsString> = link[..link.len().saturating_sub(1)].to_vec();
    for c in target.components() {
        match c {
            Component::Normal(n) => out.push(n.to_owned()),
            Component::CurDir => {}
            Component::ParentDir => {
                out.pop()?;
            }
            _ => return None,
        }
    }
    (!out.is_empty()).then_some(out)
}

/// The same for a tar dist. The stream is not seekable and the strip needs
/// every path, so the archive is read once, keeping every candidate — the
/// last one wins, as the extraction's writes do. A symlink is NOT followed
/// here: `PharData::extractTo`, which `TarDownloader` uses, writes a symlink
/// entry as an empty file, and an empty file answers like `None` to the one
/// question this serves.
pub fn read_tar_entry(tgz_bytes: &[u8], rel: &str) -> Result<Option<Vec<u8>>> {
    let dest = Path::new("<archive>");
    let wanted = components(Path::new(rel));
    let mut paths: Vec<(PathBuf, bool)> = Vec::new();
    let mut found: Vec<(PathBuf, Vec<u8>)> = Vec::new();
    let decoder = flate2::read::GzDecoder::new(tgz_bytes);
    let mut archive = tar::Archive::new(decoder);
    let mut allowance = MAX_UNCOMPRESSED;
    for entry in archive.entries().map_err(Error::io(dest))? {
        let mut entry = entry.map_err(Error::io(dest))?;
        let kind = entry.header().entry_type();
        let path = {
            let raw = entry.path_bytes();
            let text = std::str::from_utf8(&raw).map_err(|_| Error::HostileArchive {
                dest: dest.to_path_buf(),
                reason: "entry path is not UTF-8".to_owned(),
            })?;
            let p = Path::new(text);
            if p.is_absolute()
                || !p
                    .components()
                    .all(|c| matches!(c, Component::Normal(_) | Component::CurDir))
            {
                return Err(Error::HostileArchive {
                    dest: dest.to_path_buf(),
                    reason: format!("invalid entry path: {text:?}"),
                });
            }
            p.to_path_buf()
        };
        paths.push((path.clone(), kind.is_dir()));
        if !kind.is_file() {
            continue;
        }
        // The strip is only known at the end, so every name that could be
        // the one — stripped or not, whatever its case — is kept.
        if matches_stripped(&path, 0, &wanted) || matches_stripped(&path, 1, &wanted) {
            let hint = entry.size();
            let buf = read_within(&mut entry, &mut allowance, hint)
                .map_err(Error::io(dest))?
                .ok_or_else(|| Error::HostileArchive {
                    dest: dest.to_path_buf(),
                    reason: "decompressed size beyond the limit".to_owned(),
                })?;
            found.push((path, buf));
        }
    }
    let strip = root_strip_of(paths.iter().map(|(p, d)| (p.as_path(), *d)));
    // Last match wins; an exact one wins over one that only differs in case.
    let pick = |exact: bool| {
        found.iter().rev().find(|(p, _)| {
            stripped(p, strip.as_deref()).is_some_and(|c| {
                if exact {
                    c == wanted
                } else {
                    same_ignoring_case(&c, &wanted)
                }
            })
        })
    };
    Ok(pick(true).or_else(|| pick(false)).map(|(_, b)| b.clone()))
}

/// An entry path's components once the single root is taken off; `None` when
/// the entry is thrown away (beside the root, or the root itself). Mirrors
/// `strip_root`, on components rather than on a `Path`.
fn stripped(path: &Path, root: Option<&std::ffi::OsStr>) -> Option<Vec<std::ffi::OsString>> {
    let got = components(path);
    match root {
        None => (!got.is_empty()).then(|| got.clone()),
        Some(root) => (got.len() > 1 && got[0] == root).then(|| got[1..].to_vec()),
    }
}

fn components(p: &Path) -> Vec<std::ffi::OsString> {
    p.components()
        .filter_map(|c| match c {
            Component::Normal(n) => Some(n.to_owned()),
            _ => None,
        })
        .collect()
}

/// The same path but for ASCII case, which is what a case-insensitive
/// filesystem folds in every name a PHP class file can have.
fn same_ignoring_case(a: &[std::ffi::OsString], b: &[std::ffi::OsString]) -> bool {
    a.len() == b.len()
        && a.iter()
            .zip(b)
            .all(|(x, y)| match (x.to_str(), y.to_str()) {
                (Some(x), Some(y)) => x.eq_ignore_ascii_case(y),
                _ => x == y,
            })
}

/// Whether a path, with its first `drop` components taken off, is the wanted
/// one. Used before the single root is known, with both 0 and 1, to keep every
/// candidate; the root's real name settles it afterwards.
fn matches_stripped(path: &Path, drop: usize, wanted: &[std::ffi::OsString]) -> bool {
    let got = components(path);
    got.len() > drop && {
        let c = &got[drop..];
        c == wanted || same_ignoring_case(c, wanted)
    }
}

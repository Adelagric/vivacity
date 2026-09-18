//! One parse per JSON file per process: `installed.json` and the global
//! `config.json` are read by the scope analysis, the layout, the
//! transaction, the installer and the autoloader in the same run. The
//! parsed value is kept by path and validated on each read against the
//! file's (mtime ns, size) — a file rewritten mid-run (installed.json by
//! the installer) is parsed again; a file removed comes back as absent.

use serde_json::Value;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};

type Identity = (u128, u64);
type Entries = HashMap<PathBuf, (Identity, Option<Arc<Value>>)>;

fn identity(path: &Path) -> Option<Identity> {
    let meta = std::fs::metadata(path).ok()?;
    let mtime = meta
        .modified()
        .ok()?
        .duration_since(std::time::UNIX_EPOCH)
        .ok()?
        .as_nanos();
    Some((mtime, meta.len()))
}

fn cache() -> &'static Mutex<Entries> {
    static CACHE: OnceLock<Mutex<Entries>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

/// The parsed JSON of `path`: `None` when the file is absent, unreadable
/// or not JSON (the same answer every caller gave those cases).
pub fn read(path: &Path) -> Option<Arc<Value>> {
    let id = identity(path)?;
    if let Ok(map) = cache().lock() {
        if let Some((cached_id, value)) = map.get(path) {
            if *cached_id == id {
                return value.clone();
            }
        }
    }
    let value = std::fs::read_to_string(path)
        .ok()
        .and_then(|t| serde_json::from_str::<Value>(&t).ok())
        .map(Arc::new);
    if let Ok(mut map) = cache().lock() {
        map.insert(path.to_path_buf(), (id, value.clone()));
    }
    value
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reparses_when_the_file_changes_and_forgets_when_it_goes() {
        let dir = tempfile::tempdir().expect("tempdir");
        let f = dir.path().join("x.json");
        std::fs::write(&f, r#"{"a":1}"#).expect("write");
        assert_eq!(read(&f).expect("v")["a"], 1);
        // Same content length, different bytes, later mtime.
        std::thread::sleep(std::time::Duration::from_millis(20));
        std::fs::write(&f, r#"{"a":2}"#).expect("write");
        assert_eq!(read(&f).expect("v")["a"], 2);
        std::fs::remove_file(&f).expect("rm");
        assert!(read(&f).is_none());
        std::fs::write(&f, "not json").expect("write");
        assert!(read(&f).is_none());
    }
}

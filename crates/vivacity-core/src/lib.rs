//! vivacity-core: manifests, platform, fetch and installation.

pub mod binproxy;
pub mod clone;
pub mod constraint;
pub mod content_hash;
pub mod error;
pub mod extract;
pub mod fetch;
pub mod glob;
pub mod installer;
pub mod installers;
pub mod layout;
pub mod lock;
pub mod path_install;
pub mod pathutil;
pub mod pest_plugin;
pub mod phpcs_installer;
pub mod phpjson;
pub mod phpserialize;
pub mod platform;
pub mod root_version;
pub mod runtime_stub;
pub mod scope;
pub mod state;
pub mod store;
pub mod version;

pub use error::{Error, Result};

/// `random_bytes($n)` for the few places Composer draws randomness (the
/// APCu prefix): the OS entropy source, or the hasher seed as a fallback.
pub fn random_bytes(n: usize) -> Vec<u8> {
    let mut out = vec![0u8; n];
    #[cfg(unix)]
    {
        use std::io::Read as _;
        if let Ok(mut f) = std::fs::File::open("/dev/urandom") {
            if f.read_exact(&mut out).is_ok() {
                return out;
            }
        }
    }
    use std::hash::{BuildHasher as _, Hasher as _};
    let mut i = 0;
    while i < n {
        let word = std::collections::hash_map::RandomState::new()
            .build_hasher()
            .finish();
        for b in word.to_le_bytes() {
            if i < n {
                out[i] = b;
                i += 1;
            }
        }
    }
    out
}

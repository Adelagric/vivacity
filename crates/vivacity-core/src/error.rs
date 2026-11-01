use std::path::{Path, PathBuf};

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("failed to read {path}: {source}")]
    ReadFile {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("I/O error at {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("invalid JSON in {context}: {source}")]
    Json {
        context: String,
        #[source]
        source: serde_json::Error,
    },

    #[error("invalid zip archive for {dest}: {source}")]
    Zip {
        dest: PathBuf,
        #[source]
        source: zip::result::ZipError,
    },

    #[error("hostile archive refused for {dest}: {reason}")]
    HostileArchive { dest: PathBuf, reason: String },

    /// The same refusal, once the caller knows which package the archive
    /// belongs to: the path the extractor can name is the store's own
    /// temporary directory, which tells a reader nothing.
    #[error("hostile archive refused for {name} ({version}): {reason}")]
    HostileDist {
        name: String,
        version: String,
        reason: String,
    },

    /// A diagnostic of vivacity's (Composer writes curl's words, not
    /// these): both fields are shown through
    /// [`mask_url_secrets`](crate::fetch::mask_url_secrets), whatever built
    /// the error — reqwest's message names the request URL again.
    #[error(
        "HTTP failure for {}: {}",
        crate::fetch::mask_url_secrets(.url),
        crate::fetch::mask_url_secrets(.message)
    )]
    Http { url: String, message: String },

    /// A dist whose sha1 is not the lock's `dist.shasum`: `FileDownloader`'s
    /// `UnexpectedValueException` (FileDownloader.php l. 211-213), exit 1;
    /// `url` is the dist URL already through `Url::sanitize`.
    #[error("The checksum verification of the file failed (downloaded from {url})")]
    ShasumMismatch { url: String },

    #[error("cannot encode non-finite float ({0}) as JSON (PHP json_encode would fail too)")]
    NonFiniteFloat(f64),

    /// Plugin emulation that cannot be reproduced byte for byte, detected
    /// before any change to vendor/: the CLI delegates to Composer.
    #[error("{0}")]
    Unsupported(String),

    /// A `TransportException` whose message is Composer's word for word
    /// (`secure-http` refusing a redirect target): exit 100.
    #[error("{0}")]
    Transport(String),

    /// A refusal Composer states in its own words (`PathDownloader`):
    /// printed as is, exit 1.
    #[error("{0}")]
    Refused(String),
}

impl Error {
    pub fn io(path: &Path) -> impl FnOnce(std::io::Error) -> Error + '_ {
        move |source| Error::Io {
            path: path.to_path_buf(),
            source,
        }
    }

    pub fn zip(dest: &Path) -> impl FnOnce(zip::result::ZipError) -> Error + '_ {
        move |source| Error::Zip {
            dest: dest.to_path_buf(),
            source,
        }
    }
}

pub type Result<T> = std::result::Result<T, Error>;

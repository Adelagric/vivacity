//! Dist downloads, interoperable with Composer's cache: same layout
//! (`<cache>/files/<vendor>/<pkg>/<sha1-of-url>.zip`), both read AND fed, so
//! a cache warmed by one serves the other. Minimal v1 auth: `github-oauth`,
//! `http-basic`, `bearer` (the COMPOSER_HOME auth.json, the project's, then
//! COMPOSER_AUTH, as Composer merges them). The lock's shasum, when present, is checked on
//! download AND when reading back from the cache (meta-analysis F7: a shared
//! cache is read back with suspicion).

use crate::error::{Error, Result};
use serde_json::Value;
use sha1::{Digest, Sha1};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

#[derive(Debug, Default, Clone)]
pub struct Auth {
    pub github_oauth: BTreeMap<String, String>,
    pub http_basic: BTreeMap<String, (String, String)>,
    pub bearer: BTreeMap<String, String>,
}

impl Auth {
    /// Merges in `Factory::createComposer`'s order, each layer over the
    /// previous one: the COMPOSER_HOME auth.json, the project's auth.json,
    /// then COMPOSER_AUTH (`loadComposerAuthEnv`, merged last).
    pub fn load(project_dir: &Path) -> Auth {
        Auth::load_from(
            composer_home().as_deref(),
            project_dir,
            std::env::var("COMPOSER_AUTH").ok().as_deref(),
        )
    }

    fn load_from(home: Option<&Path>, project_dir: &Path, composer_auth: Option<&str>) -> Auth {
        let mut auth = Auth::default();
        // Through the run's copies (`global_json`): the startup reads them
        // already (`update-with-minimal-changes`, `secure-http`).
        if let Some(home) = home {
            auth.merge_json_file(home);
        }
        auth.merge_json_file(project_dir);
        if let Some(env) = composer_auth {
            if let Ok(v) = serde_json::from_str::<Value>(env) {
                auth.merge_value(&v);
            }
        }
        auth
    }

    /// `dir`'s auth.json, when it is there and valid JSON.
    fn merge_json_file(&mut self, dir: &Path) {
        with_global_json(dir, "auth.json", |v| {
            if let Some(v) = v {
                self.merge_value(v);
            }
        });
    }

    fn merge_value(&mut self, v: &Value) {
        if let Some(map) = v.get("github-oauth").and_then(Value::as_object) {
            for (host, tok) in map {
                if let Some(t) = tok.as_str() {
                    self.github_oauth
                        .insert(host.to_ascii_lowercase(), t.to_owned());
                }
            }
        }
        if let Some(map) = v.get("bearer").and_then(Value::as_object) {
            for (host, tok) in map {
                if let Some(t) = tok.as_str() {
                    self.bearer.insert(host.to_ascii_lowercase(), t.to_owned());
                }
            }
        }
        if let Some(map) = v.get("http-basic").and_then(Value::as_object) {
            for (host, creds) in map {
                if let (Some(u), Some(p)) = (
                    creds.get("username").and_then(Value::as_str),
                    creds.get("password").and_then(Value::as_str),
                ) {
                    self.http_basic
                        .insert(host.to_ascii_lowercase(), (u.to_owned(), p.to_owned()));
                }
            }
        }
    }

    /// Value of the Authorization header Composer sends with this URL, if
    /// any: one credential per origin, a (username, password) pair as
    /// `BaseIO::loadConfiguration` stores it (github-oauth `(token,
    /// x-oauth-basic)`, then http-basic, then bearer `(token, bearer)`, the
    /// last one loaded winning), found by `AuthHelper::findAuthOrigin`,
    /// rendered by `AuthHelper::addAuthenticationOptions` from the pair's
    /// values. A github.com token goes to `api.github.com` only — never to
    /// codeload.github.com (its own origin) nor to github.com itself. Not
    /// rendered as Composer does: GitLab tokens, Bitbucket `x-token-auth`,
    /// `custom-headers`, `client-certificate` (basic credentials here).
    /// Called per request: a redirect inherits reqwest's rule (the header
    /// is dropped across hosts), where Composer computes the target's own.
    pub fn authorization_for_url(&self, url: &str) -> Option<String> {
        let origin = composer_origin(url)?;
        let has = |o: &str| {
            self.bearer.contains_key(o)
                || self.http_basic.contains_key(o)
                || self.github_oauth.contains_key(o)
        };
        let origin = if has(&origin) {
            origin
        } else {
            match origin.as_str() {
                "api.bitbucket.org" | "api.github.com" => {
                    let canonical = origin.trim_start_matches("api.").to_owned();
                    if has(&canonical) {
                        canonical
                    } else {
                        return None;
                    }
                }
                _ => return None,
            }
        };
        let (user, password): (&str, &str) = if let Some(t) = self.bearer.get(&origin) {
            (t, "bearer")
        } else if let Some((u, p)) = self.http_basic.get(&origin) {
            (u, p)
        } else {
            (self.github_oauth.get(&origin)?, "x-oauth-basic")
        };
        if password == "bearer" {
            return Some(format!("Bearer {user}"));
        }
        if origin == "github.com" && password == "x-oauth-basic" {
            // `{^https?://api\.github\.com/}`, on the URL as written.
            let api = url
                .strip_prefix("https://")
                .or_else(|| url.strip_prefix("http://"))
                .is_some_and(|rest| rest.starts_with("api.github.com/"));
            return api.then(|| format!("token {user}"));
        }
        // Anything else — a GitHub Enterprise token included — as basic
        // credentials.
        use base64::Engine as _;
        let encoded =
            base64::engine::general_purpose::STANDARD.encode(format!("{user}:{password}"));
        Some(format!("Basic {encoded}"))
    }
}

/// `Url::getOrigin`: the host, with its port when the URL carries one;
/// every `*.github.com` but codeload.github.com is `github.com`,
/// repo.packagist.org is `packagist.org`. (PHP's `parse_url` keeps an
/// explicit default port that the `url` crate drops: `https://h:443/` is
/// `h:443` for Composer, `h` here. And Composer compares hosts and the
/// keys of auth.json as written, where vivacity lowercases both.)
fn composer_origin(url: &str) -> Option<String> {
    let parsed = reqwest::Url::parse(url).ok()?;
    let host = parsed.host_str()?;
    let origin = match parsed.port() {
        Some(port) => format!("{host}:{port}"),
        None => host.to_owned(),
    };
    if origin.ends_with(".github.com") && origin != "codeload.github.com" {
        return Some("github.com".to_owned());
    }
    if origin == "repo.packagist.org" {
        return Some("packagist.org".to_owned());
    }
    Some(origin)
}

/// What `Factory` and `Config` read of the machine to place Composer's
/// directories: the environment, a few directory probes, the OS. A trait so
/// that the tests can play Windows, a Linux with `/etc/xdg`, or a missing
/// `HOME`, on any host.
pub(crate) trait Host {
    /// `Platform::getEnv`: `None` when unset (PHP's `false`), the value
    /// otherwise, empty included.
    fn env(&self, name: &str) -> Option<String>;
    /// Some environment variable whose name starts with `XDG_` exists.
    fn any_xdg_env(&self) -> bool;
    fn is_dir(&self, path: &str) -> bool;
    /// `Platform::isWindows`.
    fn windows(&self) -> bool;
    /// `PHP_OS === 'Darwin'`.
    fn darwin(&self) -> bool;
    /// `config.<key>` of the global layer under `<home>`
    /// ([`global_config_layer`]).
    fn global_config(&self, home: &str, key: &str) -> std::result::Result<Option<Value>, String>;
    /// `Cache::isEnabled`'s test: `(is_dir || mkdir(…, 0777, true)) &&
    /// is_writable`, without creating anything — a directory that can be
    /// written, or whose nearest existing ancestor is a directory that can.
    fn dir_writable_or_creatable(&self, path: &str) -> bool;
}

/// The running process.
struct RealHost;

impl Host for RealHost {
    fn env(&self, name: &str) -> Option<String> {
        std::env::var_os(name).map(|v| v.to_string_lossy().into_owned())
    }
    fn any_xdg_env(&self) -> bool {
        std::env::vars_os().any(|(k, _)| k.to_string_lossy().starts_with("XDG_"))
    }
    fn is_dir(&self, path: &str) -> bool {
        Path::new(path).is_dir()
    }
    fn windows(&self) -> bool {
        cfg!(windows)
    }
    fn darwin(&self) -> bool {
        cfg!(target_os = "macos")
    }
    fn global_config(&self, home: &str, key: &str) -> std::result::Result<Option<Value>, String> {
        let config = global_json(Path::new(home), "config.json");
        let auth = global_json(Path::new(home), "auth.json");
        layer_of(
            home,
            key,
            config,
            auth,
            std::env::var("COMPOSER_AUTH").ok().as_deref(),
        )
    }
    fn dir_writable_or_creatable(&self, path: &str) -> bool {
        let path = Path::new(path);
        if path.is_dir() {
            return crate::io::writable(path);
        }
        // Something else there (a file, a dangling link): `mkdir` fails.
        if std::fs::symlink_metadata(path).is_ok() {
            return false;
        }
        match path.ancestors().skip(1).find(|a| a.exists()) {
            Some(existing) => existing.is_dir() && crate::io::writable(existing),
            None => false,
        }
    }
}

/// A JSON file of Composer's configuration as a run reads it: `Ok(None)`
/// when it is absent, `Err` (the message) when it cannot be read or is not
/// valid JSON. Read once per run and path ([`forget_global_files`] starts a
/// run): the cache directory, the merged repositories and `secure-http` all
/// ask for the global config.json and auth.json, which used to be read
/// again for each.
pub fn global_json(path_dir: &Path, name: &str) -> std::result::Result<Option<Value>, String> {
    let path = path_dir.join(name);
    let mut memo = GLOBAL_FILES
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if let Some((_, v)) = memo.iter().find(|(p, _)| *p == path) {
        return v.clone();
    }
    // Absent is nothing; present but unreadable stops Composer
    // (`JsonFile::read`), like invalid JSON.
    let v = match std::fs::read_to_string(&path) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e.to_string()),
        Ok(text) => serde_json::from_str(&text)
            .map(Some)
            .map_err(|e| e.to_string()),
    };
    memo.push((path, v.clone()));
    v
}

/// [`global_json`] without the copy: `f` sees the run's parsed file (`None`
/// when absent or not valid JSON).
pub fn with_global_json<R>(path_dir: &Path, name: &str, f: impl FnOnce(Option<&Value>) -> R) -> R {
    let path = path_dir.join(name);
    {
        let memo = GLOBAL_FILES
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some((_, v)) = memo.iter().find(|(p, _)| *p == path) {
            return f(v.as_ref().ok().and_then(Option::as_ref));
        }
    }
    let v = global_json(path_dir, name);
    f(v.as_ref().ok().and_then(Option::as_ref))
}

/// The files [`global_json`] read, by path.
type JsonMemo = Vec<(PathBuf, std::result::Result<Option<Value>, String>)>;
static GLOBAL_FILES: std::sync::Mutex<JsonMemo> = std::sync::Mutex::new(Vec::new());

/// The global config.json and auth.json of `Config::get('home')`, as
/// [`global_json`] keeps them; `Err` when the home cannot be resolved or a
/// file is not valid JSON (Composer stops).
pub fn global_config_files() -> std::result::Result<(Option<Value>, Option<Value>), String> {
    let home = config_home(&RealHost)?;
    let invalid = |name: &str| {
        format!(
            "{} is not valid JSON",
            Path::new(&home).join(name).display()
        )
    };
    let config =
        global_json(Path::new(&home), "config.json").map_err(|_| invalid("config.json"))?;
    let auth = global_json(Path::new(&home), "auth.json").map_err(|_| invalid("auth.json"))?;
    Ok((config, auth))
}

/// The archives this run put in Composer's files cache, from every path
/// that downloads (the install, Flex reading dists before the lock is
/// written): a run that hands over takes them back, or Composer would find
/// them and print no `  - Downloading` line for them
/// (FileDownloader.php:180-193). A channel: sending takes no lock.
fn cached_dists() -> &'static (
    std::sync::mpsc::Sender<PathBuf>,
    std::sync::Mutex<std::sync::mpsc::Receiver<PathBuf>>,
) {
    static CACHED: std::sync::OnceLock<(
        std::sync::mpsc::Sender<PathBuf>,
        std::sync::Mutex<std::sync::mpsc::Receiver<PathBuf>>,
    )> = std::sync::OnceLock::new();
    CACHED.get_or_init(|| {
        let (tx, rx) = std::sync::mpsc::channel();
        (tx, std::sync::Mutex::new(rx))
    })
}

fn note_cached_dist(path: &Path) {
    let _ = cached_dists().0.send(path.to_path_buf());
}

/// The archives noted since the last call ([`cached_dists`]).
pub fn take_cached_dists() -> Vec<PathBuf> {
    cached_dists()
        .1
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .try_iter()
        .collect()
}

/// Removes the archives this run put in Composer's files cache
/// ([`take_cached_dists`]), before the run is handed to Composer. `Err`
/// (the path and why) when one cannot be removed: Composer would then find
/// it and print one `  - Downloading` line fewer than it does from a clean
/// cache, so the hand-over could not be made identical — the caller stops
/// with that error instead.
pub fn take_back_cached_dists() -> std::result::Result<(), String> {
    for path in take_cached_dists() {
        match std::fs::remove_file(&path) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => {
                return Err(format!(
                    "cannot take {} back from Composer's files cache before handing the run over ({e}); remove it and run again",
                    path.display()
                ))
            }
        }
    }
    Ok(())
}

/// A new run: the configuration files are read again.
pub fn forget_global_files() {
    // (And the archives a previous run cached are its own.)
    let _ = take_cached_dists();
    *COMPOSER_HOME
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner) = None;
    GLOBAL_FILES
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .clear();
}

/// `Factory::loadComposerAuthEnv`: COMPOSER_AUTH unset or empty is nothing
/// (Factory.php:686); anything else must decode to a JSON object — a value
/// that does not decode stops Composer, one that is not an object fails
/// the auth schema, which Composer reports (twice: the function runs for
/// `createConfig` and again for `createComposer`). `Err` for both.
pub fn composer_auth_env(raw: Option<&str>) -> std::result::Result<Option<Value>, String> {
    let Some(text) = raw.filter(|t| !t.is_empty()) else {
        return Ok(None);
    };
    match serde_json::from_str::<Value>(text) {
        Ok(v @ Value::Object(_)) => Ok(Some(v)),
        Ok(_) => Err("COMPOSER_AUTH is not a JSON object".to_owned()),
        Err(_) => Err("COMPOSER_AUTH is not valid JSON".to_owned()),
    }
}

/// The global layer of `Config::get($key)` as `Factory::createConfig`
/// merges it: `config.<key>` of `<home>/config.json`, then the top level of
/// `<home>/auth.json` and of COMPOSER_AUTH (both merged under `config`).
/// `Err` where Composer would read something vivacity does not: a file or
/// COMPOSER_AUTH that is not valid JSON (Composer stops), or `key` set by
/// auth.json or COMPOSER_AUTH.
#[cfg(test)]
fn global_config_layer(
    home: &str,
    key: &str,
    composer_auth: Option<&str>,
) -> std::result::Result<Option<Value>, String> {
    let read = |name: &str| -> std::result::Result<Option<Value>, String> {
        match std::fs::read_to_string(Path::new(home).join(name)) {
            Err(_) => Ok(None),
            Ok(text) => serde_json::from_str(&text)
                .map(Some)
                .map_err(|e| e.to_string()),
        }
    };
    layer_of(
        home,
        key,
        read("config.json"),
        read("auth.json"),
        composer_auth,
    )
}

/// [`global_config_layer`] on files already read.
fn layer_of(
    home: &str,
    key: &str,
    config: std::result::Result<Option<Value>, String>,
    auth: std::result::Result<Option<Value>, String>,
    composer_auth: Option<&str>,
) -> std::result::Result<Option<Value>, String> {
    let invalid =
        |name: &str| format!("{} is not valid JSON", Path::new(home).join(name).display());
    let config = config.map_err(|_| invalid("config.json"))?;
    let auth = auth.map_err(|_| invalid("auth.json"))?;
    let env_auth = composer_auth_env(composer_auth)?;
    for (source, layer) in [("auth.json", &auth), ("COMPOSER_AUTH", &env_auth)] {
        if layer.as_ref().and_then(|l| l.get(key)).is_some() {
            return Err(format!("{key} in {source}"));
        }
    }
    Ok(config
        .as_ref()
        .and_then(|c| c.get("config"))
        .and_then(|c| c.get(key))
        .cloned())
}

/// A cache directory set in the project's composer.json (`config.cache-dir`,
/// `cache-files-dir`, `cache-repo-dir`), which `Config::get` would take over
/// the global value: not reproduced, named for the hand-over.
pub fn project_cache_config(manifest: &Value) -> Option<String> {
    let config = manifest.get("config")?;
    ["cache-dir", "cache-files-dir", "cache-repo-dir"]
        .into_iter()
        .find(|key| config.get(key).is_some())
        .map(|key| format!("config {key} in composer.json"))
}

/// PHP truthiness of an environment value: set, not empty, not `"0"`.
fn truthy(v: Option<String>) -> Option<String> {
    v.filter(|s| !s.is_empty() && s != "0")
}

/// `strtr($s, '\\', '/')` then `rtrim(…, '/')`.
fn slashes_trimmed(s: &str) -> String {
    s.replace('\\', "/").trim_end_matches('/').to_owned()
}

/// `Factory::useXdg`: any `XDG_*` environment variable, or a `/etc/xdg`
/// directory.
fn use_xdg(h: &dyn Host) -> bool {
    h.any_xdg_env() || h.is_dir("/etc/xdg")
}

/// `Factory::getUserDir`: HOME, separators normalised and trailing ones
/// trimmed; Composer stops without it.
fn user_dir(h: &dyn Host) -> std::result::Result<String, String> {
    truthy(h.env("HOME"))
        .map(|home| slashes_trimmed(&home))
        .ok_or_else(|| "neither HOME nor COMPOSER_HOME is set, where Composer stops".to_owned())
}

/// `Factory::getHomeDir` (docs/reference/Factory.php): COMPOSER_HOME, else
/// on Windows `%APPDATA%/Composer` (Composer stops without APPDATA), else
/// the first existing directory among `$XDG_CONFIG_HOME/composer` (if XDG
/// is in use) and `~/.composer`, else the first candidate.
fn home_dir(h: &dyn Host) -> std::result::Result<String, String> {
    if let Some(home) = truthy(h.env("COMPOSER_HOME")) {
        return Ok(home);
    }
    if h.windows() {
        return truthy(h.env("APPDATA"))
            .map(|a| format!("{}/Composer", slashes_trimmed(&a)))
            .ok_or_else(|| {
                "neither APPDATA nor COMPOSER_HOME is set, where Composer stops".to_owned()
            });
    }
    let user = user_dir(h)?;
    let mut dirs: Vec<String> = Vec::new();
    if use_xdg(h) {
        let xdg = truthy(h.env("XDG_CONFIG_HOME")).unwrap_or_else(|| format!("{user}/.config"));
        dirs.push(format!("{xdg}/composer"));
    }
    dirs.push(format!("{user}/.composer"));
    let first = dirs.first().cloned().unwrap_or_default();
    Ok(dirs.into_iter().find(|d| h.is_dir(d)).unwrap_or(first))
}

/// `Platform::expandPath`: a leading `~/` becomes the user's
/// directory; a leading `$VAR` or `%VAR%` becomes the variable's value (the
/// empty string when unset; `$HOME` falls back to USERPROFILE on Windows).
/// `Err` where Composer would ask the password database for the user's
/// directory (no HOME at all), which is not reproduced.
fn expand_path(h: &dyn Host, path: &str) -> std::result::Result<String, String> {
    if let Some(rest) = path.strip_prefix('~') {
        // `'#^~[\\/]#'` is a PHP single-quoted string: the class is `[\/]`,
        // a slash only.
        if rest.starts_with('/') {
            // `Platform::getUserDirectory`: HOME as it is, set even empty.
            let user = match h.env("HOME") {
                Some(home) => home,
                None => h
                    .env("USERPROFILE")
                    .filter(|_| h.windows())
                    .ok_or_else(|| "`~` in the cache directory without HOME".to_owned())?,
            };
            return Ok(format!("{user}{rest}"));
        }
        return Ok(path.to_owned());
    }
    let percent = match path.chars().next() {
        Some('$') => false,
        Some('%') => true,
        _ => return Ok(path.to_owned()),
    };
    let body = &path[1..];
    // `\w++` without the `u` modifier: ASCII word characters, possessive.
    let len = body
        .bytes()
        .take_while(|b| b.is_ascii_alphanumeric() || *b == b'_')
        .count();
    if len == 0 {
        return Ok(path.to_owned());
    }
    let var = &body[..len];
    let rest = if percent {
        match body[len..].strip_prefix('%') {
            Some(rest) => rest,
            None => return Ok(path.to_owned()),
        }
    } else {
        &body[len..]
    };
    let value = if h.windows() && var == "HOME" {
        truthy(h.env("HOME")).or_else(|| h.env("USERPROFILE"))
    } else {
        h.env(var)
    };
    Ok(format!("{}{rest}", value.unwrap_or_default()))
}

/// `Factory::getCacheDir`, the default `cache-dir`: COMPOSER_CACHE_DIR;
/// else `$COMPOSER_HOME/cache`; Windows -> `%LOCALAPPDATA%/Composer` (else
/// `<home>/cache`); Darwin -> `~/Library/Caches/composer`;
/// `~/.composer/cache` if the home is `~/.composer` and it exists; XDG ->
/// `$XDG_CACHE_HOME/composer`; else `<home>/cache`. Strings are joined as
/// Composer joins them (`/tmp/h/` gives `/tmp/h//cache`).
fn factory_cache_dir(h: &dyn Host, home: &str) -> std::result::Result<String, String> {
    if let Some(dir) = truthy(h.env("COMPOSER_CACHE_DIR")) {
        return Ok(dir);
    }
    if let Some(home_env) = truthy(h.env("COMPOSER_HOME")) {
        return Ok(format!("{home_env}/cache"));
    }
    if h.windows() {
        let dir = match truthy(h.env("LOCALAPPDATA")) {
            Some(local) => format!("{local}/Composer"),
            None => format!("{home}/cache"),
        };
        return Ok(slashes_trimmed(&dir));
    }
    let user = user_dir(h)?;
    if h.darwin() {
        // Composer also moves a legacy `<home>/cache` there: not reproduced
        // (it only ever moves a directory vivacity does not read).
        return Ok(format!("{user}/Library/Caches/composer"));
    }
    if home == format!("{user}/.composer") && h.is_dir(&format!("{home}/cache")) {
        return Ok(format!("{home}/cache"));
    }
    if use_xdg(h) {
        let xdg = truthy(h.env("XDG_CACHE_HOME")).unwrap_or_else(|| format!("{user}/.cache"));
        return Ok(format!("{xdg}/composer"));
    }
    Ok(format!("{home}/cache"))
}

/// A `{$key}` reference, which `Config::process` would substitute.
fn has_config_reference(s: &str) -> bool {
    // `#\{\$(.+)\}#`: `{$`, at least one character, then a `}`.
    s.match_indices("{$")
        .any(|(at, _)| s[at + 2..].chars().skip(1).any(|c| c == '}'))
}

/// `Config::get('home')`: the home, expanded, trailing separators trimmed —
/// the directory whose config.json and auth.json Composer reads.
fn config_home(h: &dyn Host) -> std::result::Result<String, String> {
    let home = expand_path(h, &home_dir(h)?)?;
    if has_config_reference(&home) {
        return Err(format!("home {home} holds a {{$…}} reference"));
    }
    Ok(home.trim_end_matches(['/', '\\']).to_owned())
}

/// The raw value `Config::get($key)` starts from, for a `*-dir` key whose
/// environment variable is `env`: the variable when set (even empty), else
/// config.json's value, else `default` (the value `Factory` merged).
fn config_dir_value(
    h: &dyn Host,
    env: &str,
    key: &str,
    default: impl FnOnce() -> std::result::Result<String, String>,
) -> std::result::Result<String, String> {
    if let Some(v) = h.env(env) {
        return Ok(v);
    }
    match h.global_config(&config_home(h)?, key)? {
        Some(Value::String(s)) => Ok(s),
        Some(other) => Err(format!("config {key} {other} in config.json")),
        None => default(),
    }
}

/// `Config::get('cache-dir', Config::RELATIVE_PATHS)` — what `composer
/// config --global cache-dir` prints: COMPOSER_CACHE_DIR when set (even
/// empty), else config.json's `cache-dir`, else [`factory_cache_dir`];
/// trailing `/` and `\` trimmed, then `~`, `$VAR` and `%VAR%` expanded. A
/// relative value stays relative. `Err` (the reason) where Composer stops
/// (no APPDATA/HOME nor COMPOSER_HOME) or where vivacity does not reproduce
/// it (a `{$key}` reference, a config.json value that is not a string).
fn cache_dir_setting_with(h: &dyn Host) -> std::result::Result<String, String> {
    // `Factory::createConfig` asks for the home first, whatever comes next.
    let home = home_dir(h)?;
    let raw = config_dir_value(h, "COMPOSER_CACHE_DIR", "cache-dir", || {
        factory_cache_dir(h, &home)
    })?;
    if has_config_reference(&raw) {
        return Err(format!("cache-dir {raw} holds a {{$…}} reference"));
    }
    expand_path(h, raw.trim_end_matches(['/', '\\']))
}

/// `Config::realpath`'s test for a path it keeps as it is: `/…`, a drive
/// letter, a `scheme://`, a UNC `\\…`.
fn is_config_absolute(path: &str) -> bool {
    let b = path.as_bytes();
    if b.first() == Some(&b'/') || path.starts_with("\\\\") {
        return true;
    }
    if b.len() >= 2 && b[0].is_ascii_alphabetic() && b[1] == b':' {
        return true;
    }
    let scheme = path
        .bytes()
        .take_while(|c| c.is_ascii_alphanumeric() || *c == b'.')
        .count();
    scheme > 0 && path[scheme..].starts_with("://")
}

/// `Config::realpath`: an absolute path as it is, anything else under the
/// base directory (Composer's working directory), joined with `/`.
fn config_realpath(path: &str, base: &str) -> String {
    if is_config_absolute(path) {
        path.to_owned()
    } else {
        format!("{base}/{path}")
    }
}

/// Why a command cannot use Composer's cache directory natively, if it
/// cannot: Composer would stop, would write into the working directory (an
/// empty value) or a `0` directory, or would place `files/` or `repo/` on
/// a path of their own (`reads_cache`: the command downloads or resolves).
fn cache_dir_hand_over_with(h: &dyn Host, reads_cache: bool) -> Option<String> {
    let setting = match cache_dir_setting_with(h) {
        Ok(s) => s,
        Err(why) => return Some(why),
    };
    if setting.is_empty() {
        return Some(
            "an empty cache-dir, which Composer resolves to the working directory".to_owned(),
        );
    }
    if setting == "0" {
        return Some("cache-dir 0, which Composer reads as the directory ./0".to_owned());
    }
    if reads_cache {
        for (env, key) in [
            ("COMPOSER_CACHE_FILES_DIR", "cache-files-dir"),
            ("COMPOSER_CACHE_REPO_DIR", "cache-repo-dir"),
        ] {
            if h.env(env).is_some() {
                return Some(format!("{env} is set"));
            }
            if let Ok(home) = config_home(h) {
                match h.global_config(&home, key) {
                    Ok(None) => {}
                    Ok(Some(_)) => return Some(format!("config {key} in config.json")),
                    Err(why) => return Some(why),
                }
            }
        }
    }
    None
}

/// `Config::get('home')`: the directory of Composer's global config.json,
/// auth.json and composer.json (`Factory::getHomeDir`, expanded and
/// trimmed). `None` where Composer stops.
pub fn composer_home() -> Option<PathBuf> {
    // Resolved once per run (`forget_global_files`): the trigger of the
    // output hold, the configuration and auth readers all ask.
    let mut memo = COMPOSER_HOME
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if let Some(home) = memo.as_ref() {
        return home.clone();
    }
    let home = config_home(&RealHost).ok().map(PathBuf::from);
    *memo = Some(home.clone());
    home
}

/// [`composer_home`] for the run (`None`: not resolved yet).
static COMPOSER_HOME: std::sync::Mutex<Option<Option<PathBuf>>> = std::sync::Mutex::new(None);

/// `composer config --global cache-dir`: the cache directory as Composer
/// prints it (`Config::RELATIVE_PATHS`). `Err` (the reason) where Composer
/// stops or vivacity does not reproduce the value.
pub fn cache_dir_setting() -> std::result::Result<String, String> {
    cache_dir_setting_with(&RealHost)
}

/// Whether the cache directory is relative, so that Composer would resolve
/// it against its working directory (`--working-dir` included).
pub fn cache_dir_is_relative() -> bool {
    cache_dir_setting().is_ok_and(|s| !is_config_absolute(&s))
}

/// Why a command must go to Composer because of the cache directory, if it
/// must ([`cache_dir_hand_over_with`]); decided before anything is written.
pub fn cache_dir_hand_over(reads_cache: bool) -> Option<String> {
    cache_dir_hand_over_with(&RealHost, reads_cache)
}

/// `Config::get('cache-dir')`, the directory Composer's caches live under:
/// [`cache_dir_setting`] resolved against the working directory. The
/// commands hand the cases it does not reproduce to Composer first
/// ([`cache_dir_hand_over`]); reached anyway (a library caller), it falls
/// back to `<cwd>/cache`.
pub fn composer_cache_dir() -> PathBuf {
    let cwd = std::env::current_dir().unwrap_or_default();
    let base = cwd.to_string_lossy();
    let setting = cache_dir_setting().unwrap_or_else(|_| "cache".to_owned());
    PathBuf::from(config_realpath(&setting, &base))
}

/// Cache path of a dist, identical to Composer's: sha1 of the FULL URL
/// (deliberate in Composer: prevents cross-repository poisoning), key
/// sanitised to `[a-z0-9._/-]`.
pub fn dist_cache_path(cache_root: &Path, name: &str, url: &str) -> PathBuf {
    dist_cache_path_of(cache_root, name, url, "zip")
}

/// `FileDownloader`'s cache key: `<name>/<sha1(url)>.<dist type>` — a
/// `tar` dist is cached as `.tar`, so Composer and vivacity find each
/// other's files.
pub fn dist_cache_path_of(cache_root: &Path, name: &str, url: &str, dist_type: &str) -> PathBuf {
    let mut h = Sha1::new();
    h.update(url.as_bytes());
    let sha: String = h.finalize().iter().map(|b| format!("{b:02x}")).collect();
    let sane_name: String = name
        .chars()
        .map(|c| {
            let c = c.to_ascii_lowercase();
            if c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, '.' | '_' | '/' | '-') {
                c
            } else {
                '-'
            }
        })
        .collect();
    cache_root
        .join("files")
        .join(sane_name)
        .join(format!("{sha}.{dist_type}"))
}

fn sha1_hex(bytes: &[u8]) -> String {
    let mut h = Sha1::new();
    h.update(bytes);
    h.finalize().iter().map(|b| format!("{b:02x}")).collect()
}

/// `hash_file('sha1', …)`, read in chunks; `None` when the file cannot be
/// read.
fn sha1_file(path: &Path) -> Option<String> {
    use std::io::Read;
    let mut f = std::fs::File::open(path).ok()?;
    let mut h = Sha1::new();
    let mut buf = vec![0u8; 64 * 1024];
    loop {
        match f.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => h.update(&buf[..n]),
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
            Err(_) => return None,
        }
    }
    Some(h.finalize().iter().map(|b| format!("{b:02x}")).collect())
}

/// Whether Composer **downloads** a dist rather than copying it from its
/// files cache: `FileDownloader::download` (FileDownloader.php l. 176-198)
/// copies it only when
/// `$this->cache !== null && ($checksum === null || $checksum === '' ||
/// $checksum === $this->cache->sha1($cacheKey)) && $this->cache->copyTo(…)`
/// — a cache this run has (`files_cache`, [`FilesCache`]), a file at the key
/// ([`dist_cache_path_of`]), and, when the lock gives a `dist.shasum`, that
/// file's sha1 equal to it. Otherwise the `  - Downloading` line and the
/// request. The one predicate for both: the line ([`crate::installer`]'s
/// caller) and the request ([`Fetcher::dist_bytes_decided`]) follow what it
/// answered once.
///
/// Not reproduced here, handed over or consigned elsewhere: the key of a
/// preferred mirror (handed over before anything), `updateDistReference`
/// (the key on the URL as the lock writes it, point 14), a custom cache key
/// from a `PRE_FILE_DOWNLOAD` listener (no plugin runs), and the garbage
/// collection Composer draws once in 51 runs (`Cache::gcIsNecessary`),
/// whose run is the one this answers for.
pub fn composer_downloads(
    cache_path: &Path,
    expected_sha1: Option<&str>,
    files_cache: bool,
) -> bool {
    if !files_cache {
        return true;
    }
    match expected_sha1 {
        None => !cache_path.is_file(),
        Some(exp) => sha1_file(cache_path).is_none_or(|actual| actual != exp),
    }
}

/// Composer's files cache for a run, as `Factory::createDownloadManager`
/// (Factory.php l. 504-510) and `Cache` (Cache.php l. 50-108) make it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FilesCache {
    /// Read and written as usual.
    Enabled,
    /// No cache (`cache-files-ttl`, else `cache-ttl`, at 0), or one
    /// `Cache::isUsable` refuses (`$null`, `nul`, `NUL`, `/dev/null`): every
    /// dist is downloaded, none is written, and nothing is said.
    Disabled,
}

/// `Cache::isUsable`: `'{(^|[\\/])(\$null|nul|NUL|/dev/null)([\\/]|$)}'`.
fn cache_dir_usable(path: &str) -> bool {
    let b = path.as_bytes();
    let sep = |c: u8| c == b'/' || c == b'\\';
    for start in 0..=b.len() {
        if start > 0 && !sep(b[start - 1]) {
            continue;
        }
        for word in ["$null", "nul", "NUL", "/dev/null"] {
            let end = start + word.len();
            if b[start..].starts_with(word.as_bytes()) && (end == b.len() || sep(b[end])) {
                return false;
            }
        }
    }
    true
}

/// The files cache of this run ([`FilesCache`]), or why the command must go
/// to Composer: a cache in read-only mode (`cache-read-only` or
/// `COMPOSER_CACHE_READ_ONLY`, `Config.php` l. 400-412 — vivacity writes
/// the repository caches as well), a lifetime that is not an integer, one of
/// these keys where vivacity does not read it (the global auth.json,
/// COMPOSER_AUTH, the project's auth.json), or a `files/` directory that can
/// neither be written nor created (Composer warns « Cannot create cache
/// directory … » and goes on without it, Cache.php l. 90-108). Decided from
/// the configuration before anything is written; `project_config` is the
/// `config` of composer.json.
pub fn files_cache(
    project_config: Option<&Value>,
    project_auth: Option<&Value>,
) -> std::result::Result<FilesCache, String> {
    files_cache_with(&RealHost, project_config, project_auth)
}

fn files_cache_with(
    h: &dyn Host,
    project_config: Option<&Value>,
    project_auth: Option<&Value>,
) -> std::result::Result<FilesCache, String> {
    let home = config_home(h)?;
    // `Config::merge`: the last layer that sets the key wins — the global
    // config.json (the auth layers are refused by `global_config`), then
    // composer.json, then the project's auth.json (refused).
    let layered = |key: &str| -> std::result::Result<Option<Value>, String> {
        let mut value = h.global_config(&home, key)?;
        if let Some(v) = project_config.and_then(|c| c.get(key)) {
            value = Some(v.clone());
        }
        if project_auth.is_some_and(|a| a.get(key).is_some()) {
            return Err(format!("{key} in the project's auth.json"));
        }
        Ok(value)
    };
    // `getComposerEnv` first, then the layers; `$val !== 'false' && (bool) $val`.
    let read_only = match h.env("COMPOSER_CACHE_READ_ONLY") {
        Some(v) => !(v.is_empty() || v == "0" || v == "false"),
        None => match layered("cache-read-only")? {
            None | Some(Value::Bool(false)) => false,
            Some(Value::Bool(true)) => true,
            Some(other) => return Err(format!("cache-read-only {other}")),
        },
    };
    if read_only {
        return Err("a read-only cache (cache-read-only)".to_owned());
    }
    // `cache-files-ttl` when set, else `cache-ttl` (Config.php l. 428-463),
    // `max(0, (int) …)`; only an integer is taken as it is.
    let integer = |key: &str, v: Value| -> std::result::Result<i64, String> {
        v.as_i64().ok_or_else(|| format!("{key} {v}"))
    };
    let ttl = match layered("cache-files-ttl")? {
        Some(v) => integer("cache-files-ttl", v)?,
        None => match layered("cache-ttl")? {
            Some(v) => integer("cache-ttl", v)?,
            None => 15_552_000,
        },
    };
    let cwd = std::env::current_dir().unwrap_or_default();
    let files = format!(
        "{}/files",
        config_realpath(&cache_dir_setting_with(h)?, &cwd.to_string_lossy())
    );
    // The cache is built only when the lifetime is positive; `isUsable` is
    // asked in its constructor, before any write test.
    if ttl <= 0 || !cache_dir_usable(&files) {
        return Ok(FilesCache::Disabled);
    }
    if !h.dir_writable_or_creatable(&files) {
        return Err(format!(
            "the cache directory {files} can neither be written nor created"
        ));
    }
    Ok(FilesCache::Enabled)
}

/// `Url::sanitize` (Url.php l. 130-148): an `access_token` query value
/// masked, a password replaced by `***`, a user name that looks like a
/// token shortened (`Url::sanitizeUsername`).
pub fn sanitize_url(url: &str) -> String {
    static TOKEN: std::sync::OnceLock<pcre2::bytes::Regex> = std::sync::OnceLock::new();
    static CREDS: std::sync::OnceLock<pcre2::bytes::Regex> = std::sync::OnceLock::new();
    let token = compiled(&TOKEN, r"([&?]access_token=)[^&]+", false);
    let mut out = String::new();
    let mut last = 0;
    for m in token.find_iter(url.as_bytes()).flatten() {
        out.push_str(&url[last..m.start()]);
        let matched = &url[m.start()..m.end()];
        let eq = matched.find('=').map(|i| i + 1).unwrap_or(matched.len());
        out.push_str(&matched[..eq]);
        out.push_str("***");
        last = m.end();
    }
    out.push_str(&url[last..]);
    let url = out;
    let creds = compiled(
        &CREDS,
        r"(?:(?P<prefix>[a-z0-9][a-z0-9+.-]*://)|\A)(?P<user>[^:/\s?#]*)(?::(?P<password>[^\s/?#]+))?@",
        true,
    );
    match creds.captures(url.as_bytes()) {
        Ok(Some(caps)) => {
            let whole = caps.get(0).map(|m| (m.start(), m.end())).unwrap_or((0, 0));
            let prefix = caps.get(1).map(|m| &url[m.start()..m.end()]).unwrap_or("");
            let user = caps.get(2).map(|m| &url[m.start()..m.end()]).unwrap_or("");
            let user = sanitize_username(user);
            let replacement = if caps.get(3).is_some_and(|m| m.end() > m.start()) {
                format!("{prefix}{user}:***@")
            } else {
                format!("{prefix}{user}@")
            };
            format!("{}{}{}", &url[..whole.0], replacement, &url[whole.1..])
        }
        _ => url,
    }
}

/// `Url::sanitizeUsername`.
fn sanitize_username(user: &str) -> String {
    const NON_SECRET: &[&str] = &[
        "gitlab-ci-token",
        "x-token-auth",
        "oauth2",
        "x-oauth-basic",
        "git",
        "user",
        "token",
    ];
    static GH: std::sync::OnceLock<pcre2::bytes::Regex> = std::sync::OnceLock::new();
    if NON_SECRET.contains(&user) {
        return user.to_owned();
    }
    let gh = compiled(&GH, r"^(?:ghp|gho|ghu|ghs|ghr|github_pat)_", false);
    if gh.is_match(user.as_bytes()).unwrap_or(false) || user.len() >= 12 {
        // `substr($user, 0, 3)` counts bytes; cut at a character boundary
        // at or before the third one (PHP would split the character).
        let cut = (0..=user.len().min(3))
            .rev()
            .find(|&i| user.is_char_boundary(i))
            .unwrap_or(0);
        return format!("{}***", &user[..cut]);
    }
    user.to_owned()
}

/// A pattern compiled once.
fn compiled(
    cell: &'static std::sync::OnceLock<pcre2::bytes::Regex>,
    pattern: &str,
    caseless: bool,
) -> &'static pcre2::bytes::Regex {
    cell.get_or_init(|| {
        pcre2::bytes::RegexBuilder::new()
            .caseless(caseless)
            .build(pattern)
            .unwrap_or_else(|e| panic!("regex `{pattern}`: {e}"))
    })
}

/// Response of a conditional metadata GET.
#[derive(Debug, Clone)]
pub enum MetadataResponse {
    NotModified,
    NotFound,
    Body {
        bytes: Vec<u8>,
        last_modified: Option<String>,
    },
}

pub struct Fetcher {
    client: reqwest::Client,
    cache_root: PathBuf,
    auth: Auth,
    /// The floor of an attempt's total time ([`TOTAL_TIMEOUT_FLOOR`];
    /// shorter in tests).
    total_floor: std::time::Duration,
    /// `ini_get("default_socket_timeout")` of the php Composer would run,
    /// asked only once an attempt has lasted `total_floor`.
    socket_timeout: fn() -> Option<u64>,
    /// Whether Composer has a files cache this run ([`FilesCache`]): when it
    /// has none, nothing is read from `files/` nor written there.
    files_cache: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Provenance {
    Cache,
    Network,
}

/// Composer's retry rule, ported from `CurlDownloader` (2.10.3, lines 384-398
/// and 465-478):
/// - a transport error is retried only for curl errno 6 (host not resolved),
///   7 (cannot connect), 28 (timeout), 16 / 92 (http2), or 56 / 35 with
///   "connection reset by peer" — in reqwest's terms, no response came back and
///   the cause is a connect, a timeout, or the body being cut;
/// - a *status* is retried only when it is one of 423, 425, 500, 502, 503, 504,
///   507, 510, or a 400 from `codeload.github.com`. A 404, a 401 or a 403 is
///   therefore final, where vivacity used to try three times — slower, and it
///   said "after 3 attempts" about a verdict that never moves;
/// - at most three retries, with Composer's own delays:
///   `restartJobWithDelay` sleeps not at all, then 100 ms, then 500 ms.
const MAX_RETRIES: u32 = 3;

/// `CURLOPT_CONNECTTIMEOUT, 10` (CurlDownloader.php l. 221): curl counts the
/// name resolution, the TCP connection and the TLS handshake in it, and so
/// does reqwest's `connect_timeout` with the rustls connector — measured,
/// not assumed (`the_connect_timeout_covers_the_tls_handshake`): without it,
/// a server that accepts TCP and never answers the ClientHello held
/// `install` forever (cypht's composer.tiki.org, 2026-10-31).
const CONNECT_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);

/// `CURLOPT_TIMEOUT, max((int) ini_get("default_socket_timeout"), 300)`
/// (l. 222): the whole attempt — connection, headers and body — and a
/// timed-out GET is retried like any curl error 28. A caller's
/// `http.timeout` replaces it (the advisories API: 10 s, [`Fetcher::post_form`]).
const TOTAL_TIMEOUT_FLOOR: std::time::Duration = std::time::Duration::from_secs(300);

/// `default_socket_timeout` as the php that runs Composer reads it: the php
/// of the platform probe (`VIVACITY_PHP`, else `php` from the PATH — the one
/// `#!/usr/bin/env php` picks for the `composer` the fallback runs). Asked at
/// most once per process, and only by an attempt that has already waited
/// [`TOTAL_TIMEOUT_FLOOR`]: no process on any other path. `None` when php
/// does not answer — the floor then stands.
fn php_socket_timeout() -> Option<u64> {
    static VALUE: std::sync::OnceLock<Option<u64>> = std::sync::OnceLock::new();
    *VALUE.get_or_init(|| {
        use std::io::Read;
        let php = std::env::var("VIVACITY_PHP").unwrap_or_else(|_| "php".to_owned());
        let mut child = std::process::Command::new(php)
            // A marker before the value: a startup warning printed on stdout
            // is not mistaken for it.
            .args([
                "-r",
                "echo 'socket-timeout:', (int) ini_get('default_socket_timeout');",
            ])
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null())
            .spawn()
            .ok()?;
        // Bounded: a php that does not answer (a shim waiting for input, a
        // wrapper) must not undo the timeout it is asked about. It runs on
        // the worker that waited 300 s already, once per process.
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        loop {
            match child.try_wait() {
                Ok(Some(_)) => break,
                Ok(None) if std::time::Instant::now() < deadline => {
                    std::thread::sleep(std::time::Duration::from_millis(20));
                }
                _ => {
                    let _ = child.kill();
                    let _ = child.wait();
                    return None;
                }
            }
        }
        let mut text = String::new();
        child.stdout.take()?.read_to_string(&mut text).ok()?;
        socket_timeout_of(&text)
    })
}

/// The value php printed after its marker. `(int)` of a negative value is
/// negative: `max(…, 300)` keeps the floor, so does `None`.
fn socket_timeout_of(stdout: &str) -> Option<u64> {
    let (_, value) = stdout.rsplit_once("socket-timeout:")?;
    u64::try_from(value.trim().parse::<i64>().ok()?).ok()
}

/// `FileDownloader::download`'s own loop (FileDownloader.php l. 135,
/// 249-290): an archive whose request ended on a 500, 502, 503 or 504 —
/// after `CurlDownloader`'s own retries — is asked for again, three times,
/// 500 ms apart. Any other failure is final there: a curl error's
/// `TransportException` carries the code 400 (TransportException.php), which
/// sets `$retries` to 0.
const DIST_ROUNDS: u32 = 3;

/// Part of the message of every failure that ran out of time — the
/// connection or the whole attempt: curl's error 28, after which Composer
/// prints its offline hint (Application.php l. 597-608).
pub const TIMED_OUT: &str = "operation timed out";

/// A reqwest error as the message says it: a timeout says so (reqwest's own
/// text does not name the cause).
fn describe(e: &reqwest::Error) -> String {
    if e.is_timeout() {
        format!("{e}: {TIMED_OUT}")
    } else {
        e.to_string()
    }
}

fn dist_round_is_retryable(status: Option<u16>) -> bool {
    matches!(status, Some(500 | 502 | 503 | 504))
}

fn status_is_retryable(status: u16, url: &str) -> bool {
    if matches!(status, 423 | 425 | 500 | 502 | 503 | 504 | 507 | 510) {
        return true;
    }
    status == 400
        && reqwest::Url::parse(url)
            .ok()
            .as_ref()
            .and_then(|u| u.host_str())
            == Some("codeload.github.com")
}

/// `true` when the request came back without any response at all and the cause
/// is one curl would retry.
fn transport_is_retryable(e: &reqwest::Error) -> bool {
    e.status().is_none() && (e.is_connect() || e.is_timeout() || e.is_request() || e.is_body())
}

/// The delay before retry number `retries` (1-based), as
/// `CurlDownloader::restartJobWithDelay` sleeps it.
fn retry_delay(retries: u32) -> std::time::Duration {
    std::time::Duration::from_millis(match retries {
        0 | 1 => 0,
        2 => 100,
        _ => 500,
    })
}

/// The message, saying how many attempts there really were — claiming three
/// when the verdict was final after one is the kind of detail that sends a
/// reader looking for a network problem that never existed.
fn attempts_message(retries: u32, last: &str) -> String {
    if retries == 0 {
        last.to_owned()
    } else {
        format!("failed after {} attempts: {last}", retries + 1)
    }
}

/// How a failed attempt is reported: the message, and whether trying again can
/// change the answer.
struct Attempt {
    message: String,
    retryable: bool,
    /// A redirect `secure-http` stopped: never retried.
    insecure_redirect: Option<InsecureRedirect>,
    /// The HTTP status the attempt ended on, when one came back.
    status: Option<u16>,
}

impl Attempt {
    /// An attempt that outlived its total time: curl error 28, retried.
    fn timed_out(after: std::time::Duration) -> Attempt {
        Attempt {
            message: format!("{TIMED_OUT} after {} milliseconds", after.as_millis()),
            retryable: true,
            insecure_redirect: None,
            status: None,
        }
    }
}

/// A redirect `secure-http` stops (the request's error source).
/// `CurlDownloader::handleRedirect` restarts the job on the new URL, and
/// `initDownload` checks it: refused, the `TransportException` names the
/// target; read from where vivacity does not, the run is Composer's.
#[derive(Debug, Clone)]
enum InsecureRedirect {
    /// Composer's message for the target.
    Refused(String),
    /// Why the run goes to Composer.
    HandOver(String),
}

impl std::fmt::Display for InsecureRedirect {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            InsecureRedirect::Refused(m) | InsecureRedirect::HandOver(m) => f.write_str(m),
        }
    }
}

impl std::error::Error for InsecureRedirect {}

impl InsecureRedirect {
    /// The command's error for a metadata request: Composer's transport
    /// failure (caught where Composer catches it, a hand-over where it
    /// does not — `session_kind`), or the hand-over.
    fn into_error(self) -> Error {
        match self {
            InsecureRedirect::Refused(m) => Error::Transport(m),
            InsecureRedirect::HandOver(m) => Error::Unsupported(m),
        }
    }

    /// The command's error for a dist download: a hand-over either way.
    /// Composer's refusal of the target is a `TransportException` that
    /// `DownloadManager` turns into « Failed to download … from dist », then
    /// a try from source when there is one, or the exception box
    /// (DownloadManager.php:218-245) — after the « Downloading » lines; not
    /// reproduced.
    fn into_dist_error(self) -> Error {
        match self {
            InsecureRedirect::Refused(m) => {
                Error::Unsupported(format!("a redirect secure-http refuses ({m})"))
            }
            InsecureRedirect::HandOver(m) => Error::Unsupported(m),
        }
    }
}

/// The refused redirect behind a reqwest error, if that is what it is.
fn insecure_redirect(e: &reqwest::Error) -> Option<InsecureRedirect> {
    let mut source = std::error::Error::source(e);
    while let Some(s) = source {
        if let Some(r) = s.downcast_ref::<InsecureRedirect>() {
            return Some(r.clone());
        }
        source = s.source();
    }
    None
}

/// What the redirect policy does with a target `secure-http` objects to.
fn redirect_verdict(target: &reqwest::Url) -> Option<InsecureRedirect> {
    use crate::secure_http::{classify, refusal, Verdict};
    let url = target.as_str();
    match classify(url) {
        Verdict::Allowed => None,
        Verdict::Warn { scheme, host } => {
            crate::secure_http::note_redirect(&scheme, &host);
            None
        }
        // A target with credentials would need `Url::sanitize` in the
        // message: Composer's, then.
        Verdict::Refuse if target.username().is_empty() && target.password().is_none() => {
            Some(InsecureRedirect::Refused(refusal(url)))
        }
        Verdict::Refuse => Some(InsecureRedirect::HandOver(
            "a redirect to an insecure URL with credentials, which secure-http refuses".to_owned(),
        )),
        Verdict::HandOver(reason) => Some(InsecureRedirect::HandOver(format!(
            "a redirect to an insecure URL, with {reason}"
        ))),
    }
}

/// `secure-http` once more where the request leaves: the commands decide
/// before anything is written ([`crate::secure_http`]); a request they did
/// not foresee is handed to Composer rather than sent.
fn refuse_insecure(url: &str) -> Result<()> {
    use crate::secure_http::{classify, Verdict};
    let why = match classify(url) {
        Verdict::Refuse => "which secure-http refuses".to_owned(),
        Verdict::HandOver(reason) => format!("with {reason}"),
        Verdict::Allowed | Verdict::Warn { .. } => return Ok(()),
    };
    Err(Error::Unsupported(format!(
        "{} would be fetched over an insecure protocol, {why}",
        url.split(['?', '#']).next().unwrap_or(url)
    )))
}

/// The client every request goes through: Composer's redirect rule and its
/// connection timeout.
fn client(
    redirects: reqwest::redirect::Policy,
    connect_timeout: std::time::Duration,
) -> Result<reqwest::Client> {
    reqwest::Client::builder()
        .user_agent(format!("vivacity/{}", env!("CARGO_PKG_VERSION")))
        .redirect(redirects)
        .connect_timeout(connect_timeout)
        .build()
        .map_err(|e| Error::Http {
            url: "client".to_owned(),
            message: e.to_string(),
        })
}

impl Fetcher {
    pub fn new(cache_root: PathBuf, auth: Auth) -> Result<Fetcher> {
        // `CurlDownloader::handleRedirect` starts a new request, which
        // `prohibitUrlByConfig` checks ([`redirect_verdict`]); at most
        // `CurlDownloader::$maxRedirects` (20) are followed.
        let redirects = reqwest::redirect::Policy::custom(|attempt| {
            if let Some(stop) = redirect_verdict(attempt.url()) {
                attempt.error(stop)
            } else if attempt.previous().len() > 20 {
                attempt.error("too many redirects")
            } else {
                attempt.follow()
            }
        });
        let client = client(redirects, CONNECT_TIMEOUT)?;
        Ok(Fetcher {
            client,
            cache_root,
            auth,
            total_floor: TOTAL_TIMEOUT_FLOOR,
            socket_timeout: php_socket_timeout,
            files_cache: true,
        })
    }

    /// This fetcher for a run where Composer has no files cache
    /// ([`FilesCache::Disabled`]): every archive is fetched, none is cached.
    pub fn without_files_cache(mut self) -> Fetcher {
        self.files_cache = false;
        self
    }

    /// One attempt within its total time: `max(default_socket_timeout,
    /// 300)`. Not reqwest's own `timeout`, which cannot be lengthened once
    /// the request is sent: the attempt is awaited for the floor, and only
    /// then — when php says the setting is longer — for the rest.
    async fn within_total<T>(
        &self,
        attempt: impl std::future::Future<Output = std::result::Result<T, Attempt>>,
    ) -> std::result::Result<T, Attempt> {
        let mut attempt = std::pin::pin!(attempt);
        if let Ok(r) = tokio::time::timeout(self.total_floor, &mut attempt).await {
            return r;
        }
        let total = (self.socket_timeout)()
            .map(std::time::Duration::from_secs)
            .unwrap_or_default()
            .max(self.total_floor);
        let rest = total - self.total_floor;
        if !rest.is_zero() {
            if let Ok(r) = tokio::time::timeout(rest, &mut attempt).await {
                return r;
            }
        }
        Err(Attempt::timed_out(total))
    }

    /// The directory Composer's caches live under, as this fetcher reads
    /// them (`files/` holds the archives).
    pub fn cache_root(&self) -> &Path {
        &self.cache_root
    }

    /// Whether Composer has a files cache this run ([`Fetcher::without_files_cache`]).
    pub fn files_cache(&self) -> bool {
        self.files_cache
    }

    /// Bytes of the dist: from Composer's files cache when Composer would
    /// copy it from there ([`composer_downloads`]), fetched otherwise
    /// (Composer's attempts and delays), the cache fed through temp+rename.
    pub async fn dist_bytes(
        &self,
        name: &str,
        url: &str,
        expected_sha1: Option<&str>,
        offline: bool,
    ) -> Result<(Vec<u8>, Provenance)> {
        self.dist_bytes_of(name, url, "zip", expected_sha1, offline)
            .await
    }

    /// `dist_bytes` for a given dist type (the cache file's extension).
    pub async fn dist_bytes_of(
        &self,
        name: &str,
        url: &str,
        dist_type: &str,
        expected_sha1: Option<&str>,
        offline: bool,
    ) -> Result<(Vec<u8>, Provenance)> {
        let cache_path = dist_cache_path_of(&self.cache_root, name, url, dist_type);
        let download = composer_downloads(&cache_path, expected_sha1, self.files_cache);
        self.dist_bytes_decided(name, url, dist_type, expected_sha1, offline, download)
            .await
    }

    /// The copy Composer's files cache holds of a dist, and nothing else: no
    /// request, no local read, nothing written — what an unchanged package
    /// (one Composer leaves alone) may borrow to fill the store. `None` when
    /// there is no copy, or none that matches `expected_sha1`.
    pub fn cached_dist_bytes(
        &self,
        name: &str,
        url: &str,
        dist_type: &str,
        expected_sha1: Option<&str>,
    ) -> Option<Vec<u8>> {
        if !self.files_cache {
            return None;
        }
        let bytes =
            std::fs::read(dist_cache_path_of(&self.cache_root, name, url, dist_type)).ok()?;
        match expected_sha1 {
            Some(exp) if sha1_hex(&bytes) != exp => None,
            _ => Some(bytes),
        }
    }

    /// The bytes of a dist whose source is already decided: `download`
    /// false, the copy in Composer's files cache (the caller ran
    /// [`composer_downloads`] on it, which is not run again); true, the
    /// request `FileDownloader` makes. A cached copy gone since the decision
    /// is downloaded, as Composer's `copyTo` failing would.
    ///
    /// What the request leaves is Composer's (FileDownloader.php l. 200-247):
    /// the archive is written to the files cache (`$accept`) **before** its
    /// checksum is verified, so a mismatching download stays there; a stale
    /// copy whose checksum did not match is not removed beforehand, only
    /// overwritten by a successful download. A dist on this filesystem
    /// (`file://`, a plain path) is read through `RemoteFilesystem`, which
    /// `COMPOSER_DISABLE_NETWORK` does not stop (HttpDownloader.php
    /// l. 236-264, 328-338); any other request offline is refused with
    /// Composer's words (status 499: no new attempt, no other URL,
    /// FileDownloader.php l. 272-275).
    pub async fn dist_bytes_decided(
        &self,
        name: &str,
        url: &str,
        dist_type: &str,
        expected_sha1: Option<&str>,
        offline: bool,
        download: bool,
    ) -> Result<(Vec<u8>, Provenance)> {
        let cache_path = dist_cache_path_of(&self.cache_root, name, url, dist_type);
        if !download && self.files_cache {
            if let Ok(bytes) = std::fs::read(&cache_path) {
                return Ok((bytes, Provenance::Cache));
            }
        }
        let verified = |bytes: Vec<u8>| -> Result<(Vec<u8>, Provenance)> {
            if let Some(exp) = expected_sha1 {
                let actual = sha1_hex(&bytes);
                if actual != exp {
                    return Err(Error::ShasumMismatch {
                        name: name.to_owned(),
                        expected: exp.to_owned(),
                        actual,
                    });
                }
            }
            Ok((bytes, Provenance::Network))
        };

        // A dist that lives on this filesystem is read, not downloaded: that
        // is what Composer does with it, and refusing it lost the whole
        // `artifact` repository family (offline installs, private builds).
        if let Some(path) = Self::local_dist_path(url) {
            let bytes = std::fs::read(&path).map_err(|e| Error::Http {
                url: url.to_owned(),
                message: format!("{} could not be read: {e}", path.display()),
            })?;
            self.cache_dist(&cache_path, &bytes);
            return verified(bytes);
        }
        if offline {
            return Err(Error::Transport(format!(
                "Network disabled, request canceled: {}",
                sanitize_url(url)
            )));
        }

        refuse_insecure(url)?;
        let mut retries = 0u32;
        let mut rounds = 0u32;
        let mut in_round = 0u32;
        let last_err = loop {
            match self.within_total(self.try_download(url)).await {
                Ok(bytes) => {
                    self.cache_dist(&cache_path, &bytes);
                    return verified(bytes);
                }
                Err(Attempt {
                    insecure_redirect: Some(stop),
                    ..
                }) => return Err(stop.into_dist_error()),
                Err(a) => {
                    if a.retryable && in_round < MAX_RETRIES {
                        retries += 1;
                        in_round += 1;
                        tokio::time::sleep(retry_delay(in_round)).await;
                    } else if dist_round_is_retryable(a.status) && rounds < DIST_ROUNDS {
                        // `FileDownloader`: a new job, whose curl retries
                        // start from zero again.
                        retries += 1;
                        rounds += 1;
                        in_round = 0;
                        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
                    } else {
                        break a.message;
                    }
                }
            }
        };
        Err(Error::Http {
            url: url.to_owned(),
            message: attempts_message(retries, &last_err),
        })
    }

    /// Metadata GET (packages.json, p2 files): `Ok(None)` on 404 (unknown
    /// package, tolerated by Composer), error otherwise; 3 attempts on
    /// transport errors.
    pub async fn metadata_bytes(&self, url: &str) -> Result<Option<Vec<u8>>> {
        match self.metadata_fetch(url, None).await? {
            MetadataResponse::Body { bytes, .. } => Ok(Some(bytes)),
            MetadataResponse::NotFound | MetadataResponse::NotModified => Ok(None),
        }
    }

    /// `application/x-www-form-urlencoded` POST (the security advisories
    /// API: `packages[]=...`): 10 s in all, Composer's `http.timeout`
    /// (ComposerRepository.php l. 777), which replaces the total time of
    /// [`within_total`](Self::within_total); a single attempt, as
    /// `CurlDownloader` retries a GET only; 404 -> `NotFound`.
    pub async fn post_form(&self, url: &str, body: &str) -> Result<MetadataResponse> {
        refuse_insecure(url)?;
        let mut req = self
            .client
            .post(url)
            .header(
                reqwest::header::CONTENT_TYPE,
                "application/x-www-form-urlencoded",
            )
            .timeout(std::time::Duration::from_secs(10))
            .body(body.to_owned());
        if let Some(authz) = self.auth.authorization_for_url(url) {
            req = req.header(reqwest::header::AUTHORIZATION, authz);
        }
        let resp = req.send().await.map_err(|e| match insecure_redirect(&e) {
            Some(stop) => stop.into_error(),
            None => Error::Http {
                url: url.to_owned(),
                message: describe(&e),
            },
        })?;
        if resp.status() == reqwest::StatusCode::NOT_FOUND {
            return Ok(MetadataResponse::NotFound);
        }
        let resp = resp.error_for_status().map_err(|e| Error::Http {
            url: url.to_owned(),
            message: describe(&e),
        })?;
        let bytes = resp.bytes().await.map_err(|e| Error::Http {
            url: url.to_owned(),
            message: describe(&e),
        })?;
        Ok(MetadataResponse::Body {
            bytes: bytes.to_vec(),
            last_modified: None,
        })
    }

    /// Conditional metadata GET: `If-Modified-Since` when the cache has a
    /// date, 304 -> `NotModified`, 404 -> `NotFound`, else the body and the
    /// `Last-Modified` header; 3 attempts on transport errors.
    pub async fn metadata_fetch(
        &self,
        url: &str,
        if_modified_since: Option<&str>,
    ) -> Result<MetadataResponse> {
        refuse_insecure(url)?;
        let mut retries = 0u32;
        let last_err = loop {
            let attempt = match self
                .within_total(self.metadata_attempt(url, if_modified_since))
                .await
            {
                Ok(response) => return Ok(response),
                Err(a) => a,
            };
            if let Some(stop) = attempt.insecure_redirect {
                return Err(stop.into_error());
            }
            if !attempt.retryable || retries >= MAX_RETRIES {
                break attempt.message;
            }
            retries += 1;
            tokio::time::sleep(retry_delay(retries)).await;
        };
        Err(Error::Http {
            url: url.to_owned(),
            message: attempts_message(retries, &last_err),
        })
    }

    /// One conditional GET, headers and body.
    async fn metadata_attempt(
        &self,
        url: &str,
        if_modified_since: Option<&str>,
    ) -> std::result::Result<MetadataResponse, Attempt> {
        let mut req = self.client.get(url);
        if let Some(authz) = self.auth.authorization_for_url(url) {
            req = req.header(reqwest::header::AUTHORIZATION, authz);
        }
        if let Some(ims) = if_modified_since {
            req = req.header(reqwest::header::IF_MODIFIED_SINCE, ims);
        }
        let resp = req.send().await.map_err(|e| Attempt {
            retryable: transport_is_retryable(&e),
            insecure_redirect: insecure_redirect(&e),
            message: describe(&e),
            status: None,
        })?;
        if resp.status() == reqwest::StatusCode::NOT_FOUND {
            return Ok(MetadataResponse::NotFound);
        }
        if resp.status() == reqwest::StatusCode::NOT_MODIFIED {
            return Ok(MetadataResponse::NotModified);
        }
        let last_modified = resp
            .headers()
            .get(reqwest::header::LAST_MODIFIED)
            .and_then(|v| v.to_str().ok())
            .map(str::to_owned);
        let resp = resp.error_for_status().map_err(|e| Attempt {
            retryable: e
                .status()
                .is_some_and(|s| status_is_retryable(s.as_u16(), url)),
            insecure_redirect: None,
            message: describe(&e),
            status: e.status().map(|s| s.as_u16()),
        })?;
        match resp.bytes().await {
            Ok(b) => Ok(MetadataResponse::Body {
                bytes: b.to_vec(),
                last_modified,
            }),
            Err(e) => Err(Attempt {
                retryable: transport_is_retryable(&e),
                insecure_redirect: None,
                message: describe(&e),
                status: None,
            }),
        }
    }

    /// Writes a dist into Composer's own files cache, through a temporary
    /// name so a killed process never leaves a half-written entry behind.
    fn cache_dist(&self, cache_path: &Path, bytes: &[u8]) {
        if !self.files_cache {
            return;
        }
        if let Some(parent) = cache_path.parent() {
            if std::fs::create_dir_all(parent).is_ok() {
                let tmp = cache_path.with_extension("zip.vivacity-tmp");
                if std::fs::write(&tmp, bytes).is_ok() && std::fs::rename(&tmp, cache_path).is_ok()
                {
                    note_cached_dist(cache_path);
                }
            }
        }
    }

    /// The filesystem path a dist URL names, when it names one. Composer's
    /// `HttpDownloader::addJob` hands anything that is not `http(s)://` to
    /// `RemoteFilesystem`, which opens it with PHP's stream layer — so a
    /// `file://` URL and a plain path both work there, and a plain path is
    /// exactly what an `artifact` repository writes into the lock. Nothing is
    /// percent-decoded, as PHP's file wrapper decodes nothing either.
    fn local_dist_path(url: &str) -> Option<PathBuf> {
        if let Some(rest) = url.strip_prefix("file://") {
            // The third slash belongs to the path (`file:///srv/x` is
            // `/srv/x`) except when a drive letter follows it, the Windows
            // form (`file:///C:/x` is `C:/x`). A host part
            // (`file://host/share`) is left to the caller's error, as PHP
            // leaves it to the stream wrapper.
            let path = match rest.strip_prefix('/') {
                Some(after) if after.chars().nth(1) == Some(':') => after,
                _ => rest,
            };
            return Some(PathBuf::from(path));
        }
        if url.contains("://") {
            return None;
        }
        Some(PathBuf::from(url))
    }

    async fn try_download(&self, url: &str) -> std::result::Result<Vec<u8>, Attempt> {
        let fail = |e: reqwest::Error, url: &str| Attempt {
            insecure_redirect: insecure_redirect(&e),
            message: describe(&e),
            retryable: match e.status() {
                Some(s) => status_is_retryable(s.as_u16(), url),
                None => transport_is_retryable(&e),
            },
            status: e.status().map(|s| s.as_u16()),
        };
        let mut req = self.client.get(url);
        if let Some(authz) = self.auth.authorization_for_url(url) {
            req = req.header(reqwest::header::AUTHORIZATION, authz);
        }
        let resp = req.send().await.map_err(|e| fail(e, url))?;
        let resp = resp.error_for_status().map_err(|e| fail(e, url))?;
        Ok(resp.bytes().await.map_err(|e| fail(e, url))?.to_vec())
    }
}

#[cfg(test)]
mod tests {
    /// `FileDownloader::download` l. 180: a copy at the key serves unless the
    /// lock's checksum says otherwise, or the run has no files cache.
    #[test]
    fn composer_downloads_what_its_files_cache_does_not_serve() {
        let dir = tempfile::tempdir().expect("tmp");
        let at = dir.path().join("files/acme/a/k.zip");
        assert!(composer_downloads(&at, None, true), "absent");
        std::fs::create_dir_all(at.parent().expect("parent")).expect("mkdir");
        std::fs::write(&at, b"zip bytes").expect("write");
        let good = sha1_hex(b"zip bytes");
        assert!(!composer_downloads(&at, None, true), "present, no checksum");
        assert!(
            !composer_downloads(&at, Some(&good), true),
            "checksum matches"
        );
        assert!(
            composer_downloads(&at, Some(&sha1_hex(b"other")), true),
            "checksum differs"
        );
        assert!(composer_downloads(&at, None, false), "no files cache");
        // A directory at the key: `copyTo` cannot copy it.
        let d = dir.path().join("files/acme/b/k.zip");
        std::fs::create_dir_all(&d).expect("mkdir");
        assert!(composer_downloads(&d, None, true));
    }

    #[test]
    fn cache_dir_usable_is_composers_is_usable() {
        for bad in [
            "/dev/null/files",
            "/dev/null",
            "nul/files",
            "C:\\x\\NUL\\files",
            "/a/$null/files",
            "/a/nul",
        ] {
            assert!(!cache_dir_usable(bad), "{bad}");
        }
        for good in [
            "/home/u/.cache/composer/files",
            "/a/nulx/files",
            "/a/xnul/files",
            "/a/Nul/files",
            "/dev/nullx/files",
        ] {
            assert!(cache_dir_usable(good), "{good}");
        }
    }

    #[test]
    fn the_files_cache_of_a_run() {
        let base = || FakeHost::linux("/home/u").with("COMPOSER_CACHE_DIR", "/c");
        let global = |key: &str, v: Value| {
            let mut h = base();
            h.config_json.insert(format!("/home/u/.composer|{key}"), v);
            h
        };
        let cfg = |v: Value| v;
        assert_eq!(
            files_cache_with(&base(), None, None),
            Ok(FilesCache::Enabled)
        );
        // COMPOSER_CACHE_READ_ONLY: `!== 'false' && (bool)`.
        for (v, read_only) in [
            ("1", true),
            ("yes", true),
            ("false", false),
            ("0", false),
            ("", false),
        ] {
            assert_eq!(
                files_cache_with(&base().with("COMPOSER_CACHE_READ_ONLY", v), None, None).is_err(),
                read_only,
                "{v:?}"
            );
        }
        assert!(
            files_cache_with(&global("cache-read-only", Value::Bool(true)), None, None).is_err()
        );
        assert_eq!(
            files_cache_with(&global("cache-read-only", Value::Bool(false)), None, None),
            Ok(FilesCache::Enabled)
        );
        assert!(
            files_cache_with(&global("cache-read-only", Value::from("false")), None, None).is_err()
        );
        // The project's composer.json wins over the global layer.
        assert_eq!(
            files_cache_with(
                &global("cache-read-only", Value::Bool(true)),
                Some(&cfg(serde_json::json!({"cache-read-only": false}))),
                None
            ),
            Ok(FilesCache::Enabled)
        );
        // The lifetimes: `cache-files-ttl`, else `cache-ttl`; 0 = no cache.
        let project = |v: Value| files_cache_with(&base(), Some(&v), None);
        assert_eq!(
            project(serde_json::json!({"cache-files-ttl": 0})),
            Ok(FilesCache::Disabled)
        );
        assert_eq!(
            project(serde_json::json!({"cache-ttl": 0})),
            Ok(FilesCache::Disabled)
        );
        assert_eq!(
            project(serde_json::json!({"cache-ttl": -5})),
            Ok(FilesCache::Disabled)
        );
        assert_eq!(
            project(serde_json::json!({"cache-ttl": 0, "cache-files-ttl": 60})),
            Ok(FilesCache::Enabled)
        );
        assert!(project(serde_json::json!({"cache-files-ttl": "60"})).is_err());
        assert!(project(serde_json::json!({"cache-ttl": null})).is_err());
        assert_eq!(
            files_cache_with(&global("cache-ttl", Value::from(0)), None, None),
            Ok(FilesCache::Disabled)
        );
        // Where vivacity does not read the key: handed over.
        assert!(files_cache_with(
            &base(),
            None,
            Some(&serde_json::json!({"cache-files-ttl": 0}))
        )
        .is_err());
        let mut h = base();
        h.config_errors.insert(
            "/home/u/.composer|cache-ttl".to_owned(),
            "cache-ttl in COMPOSER_AUTH".to_owned(),
        );
        assert!(files_cache_with(&h, None, None).is_err());
        // `Cache::isUsable`, silently; a directory that cannot be made, out.
        assert_eq!(
            files_cache_with(
                &FakeHost::linux("/home/u").with("COMPOSER_CACHE_DIR", "/dev/null"),
                None,
                None
            ),
            Ok(FilesCache::Disabled)
        );
        let mut h = base();
        h.unwritable.push("/c/files".to_owned());
        assert!(files_cache_with(&h, None, None).is_err());
        // … unless there is no cache to write to anyway.
        assert_eq!(
            files_cache_with(&h, Some(&serde_json::json!({"cache-files-ttl": 0})), None),
            Ok(FilesCache::Disabled)
        );
    }

    #[test]
    fn a_files_directory_is_writable_or_creatable() {
        let dir = tempfile::tempdir().expect("tmp");
        let at = |rel: &str| dir.path().join(rel).to_string_lossy().into_owned();
        assert!(
            RealHost.dir_writable_or_creatable(&at("cache/files")),
            "creatable"
        );
        std::fs::create_dir_all(dir.path().join("cache/files")).expect("mkdir");
        assert!(
            RealHost.dir_writable_or_creatable(&at("cache/files")),
            "there"
        );
        std::fs::write(dir.path().join("plain"), b"x").expect("write");
        assert!(
            !RealHost.dir_writable_or_creatable(&at("plain")),
            "a file in the way"
        );
        assert!(
            !RealHost.dir_writable_or_creatable(&at("plain/files")),
            "a file above"
        );
    }

    #[test]
    fn a_long_user_name_is_shortened_without_splitting_a_character() {
        assert_eq!(
            sanitize_url("https://ààààààà@host/x.zip"),
            "https://à***@host/x.zip"
        );
        assert_eq!(
            sanitize_url("https://abcdefghijklm@host/x.zip"),
            "https://abc***@host/x.zip"
        );
        assert_eq!(
            sanitize_url("https://u:p@host/x.zip?access_token=t&a=1"),
            "https://u:***@host/x.zip?access_token=***&a=1"
        );
    }

    /// `RemoteFilesystem` reads a local dist even with the network disabled;
    /// a request is refused in Composer's words (`HttpDownloader::startJob`),
    /// the URL sanitised.
    #[test]
    fn offline_a_local_dist_is_read_and_a_request_refused() {
        let dir = tempfile::tempdir().expect("tmp");
        let src = dir.path().join("a.zip");
        std::fs::write(&src, b"zip bytes").expect("write");
        let fetcher = Fetcher::new(dir.path().join("cache"), Auth::default()).expect("fetcher");
        let runtime = tokio::runtime::Runtime::new().expect("runtime");
        let local = src.to_string_lossy().into_owned();
        let (bytes, _) = runtime
            .block_on(fetcher.dist_bytes_decided("acme/a", &local, "zip", None, true, true))
            .expect("read offline");
        assert_eq!(bytes, b"zip bytes");
        assert!(dist_cache_path_of(&dir.path().join("cache"), "acme/a", &local, "zip").is_file());
        let err = runtime
            .block_on(fetcher.dist_bytes_decided(
                "acme/b",
                "https://user:secret@example.invalid/b.zip",
                "zip",
                None,
                true,
                true,
            ))
            .expect_err("refused");
        assert_eq!(
            err.to_string(),
            "Network disabled, request canceled: https://user:***@example.invalid/b.zip"
        );
    }

    /// `$accept` writes the download into the cache before the checksum is
    /// verified (FileDownloader.php l. 239-242, then 213-215); a stale copy
    /// is overwritten, not removed first.
    #[test]
    fn a_mismatching_dist_is_cached_before_it_fails() {
        let dir = tempfile::tempdir().expect("tmp");
        let src = dir.path().join("a.zip");
        std::fs::write(&src, b"new bytes").expect("write");
        let cache = dir.path().join("cache");
        let fetcher = Fetcher::new(cache.clone(), Auth::default()).expect("fetcher");
        let runtime = tokio::runtime::Runtime::new().expect("runtime");
        let url = src.to_string_lossy().into_owned();
        let at = dist_cache_path_of(&cache, "acme/a", &url, "zip");
        std::fs::create_dir_all(at.parent().expect("parent")).expect("mkdir");
        std::fs::write(&at, b"stale").expect("write");
        let err = runtime
            .block_on(fetcher.dist_bytes_of(
                "acme/a",
                &url,
                "zip",
                Some(&sha1_hex(b"locked")),
                false,
            ))
            .expect_err("checksum");
        assert!(matches!(err, Error::ShasumMismatch { .. }), "{err}");
        assert_eq!(std::fs::read(&at).expect("cached"), b"new bytes");
    }

    /// Without a files cache, nothing is read from it nor written to it.
    #[test]
    fn without_a_files_cache_nothing_is_read_or_written() {
        let dir = tempfile::tempdir().expect("tmp");
        let src = dir.path().join("a.zip");
        std::fs::write(&src, b"fresh").expect("write");
        let cache = dir.path().join("cache");
        let url = src.to_string_lossy().into_owned();
        let at = dist_cache_path_of(&cache, "acme/a", &url, "zip");
        std::fs::create_dir_all(at.parent().expect("parent")).expect("mkdir");
        std::fs::write(&at, b"cached").expect("write");
        let fetcher = Fetcher::new(cache, Auth::default())
            .expect("fetcher")
            .without_files_cache();
        let runtime = tokio::runtime::Runtime::new().expect("runtime");
        let (bytes, provenance) = runtime
            .block_on(fetcher.dist_bytes_of("acme/a", &url, "zip", None, false))
            .expect("fetched");
        assert_eq!(
            (bytes.as_slice(), provenance),
            (&b"fresh"[..], Provenance::Network)
        );
        assert_eq!(std::fs::read(&at).expect("untouched"), b"cached");
    }

    /// Every archive a run writes to Composer's files cache is noted, from
    /// whichever caller downloads it (the install, Flex's dist reader): a
    /// hand-over takes them back.
    #[test]
    fn an_archive_written_to_the_files_cache_is_noted() {
        let dir = tempfile::tempdir().expect("tmp");
        let src = dir.path().join("a.zip");
        std::fs::write(&src, b"zip bytes").expect("write");
        let fetcher = Fetcher::new(dir.path().join("cache"), Auth::default()).expect("fetcher");
        let runtime = tokio::runtime::Runtime::new().expect("runtime");
        let url = src.to_string_lossy().into_owned();
        runtime
            .block_on(fetcher.dist_bytes_of("acme/a", &url, "zip", None, false))
            .expect("read");
        let cached = dist_cache_path_of(&dir.path().join("cache"), "acme/a", &url, "zip");
        assert!(cached.exists());
        // Other tests may note theirs at the same time: this one is there.
        assert!(take_cached_dists().contains(&cached));
    }

    use super::*;

    /// A machine described in the test: its environment, the directories
    /// that exist, its OS.
    #[derive(Default)]
    struct FakeHost {
        env: BTreeMap<String, String>,
        dirs: Vec<String>,
        windows: bool,
        darwin: bool,
        config_json: BTreeMap<String, Value>,
        /// `<home>|<key>` whose global layer cannot be reproduced.
        config_errors: BTreeMap<String, String>,
        /// Directories that can neither be written nor created.
        unwritable: Vec<String>,
    }

    impl FakeHost {
        fn linux(home: &str) -> FakeHost {
            FakeHost::default().with("HOME", home)
        }
        fn with(mut self, k: &str, v: &str) -> FakeHost {
            self.env.insert(k.to_owned(), v.to_owned());
            self
        }
        fn dir(mut self, d: &str) -> FakeHost {
            self.dirs.push(d.to_owned());
            self
        }
    }

    impl Host for FakeHost {
        fn env(&self, name: &str) -> Option<String> {
            self.env.get(name).cloned()
        }
        fn any_xdg_env(&self) -> bool {
            self.env.keys().any(|k| k.starts_with("XDG_"))
        }
        fn is_dir(&self, path: &str) -> bool {
            self.dirs.iter().any(|d| d == path)
        }
        fn windows(&self) -> bool {
            self.windows
        }
        fn darwin(&self) -> bool {
            self.darwin
        }
        fn global_config(
            &self,
            home: &str,
            key: &str,
        ) -> std::result::Result<Option<Value>, String> {
            let at = format!("{home}|{key}");
            match self.config_errors.get(&at) {
                Some(e) => Err(e.clone()),
                None => Ok(self.config_json.get(&at).cloned()),
            }
        }
        fn dir_writable_or_creatable(&self, path: &str) -> bool {
            !self.unwritable.iter().any(|d| d == path)
        }
    }

    fn setting(h: &FakeHost) -> std::result::Result<String, String> {
        cache_dir_setting_with(h)
    }

    #[test]
    fn the_cache_dir_is_config_get_on_top_of_factory() {
        // Each expectation is what `composer config --global cache-dir`
        // printed on Composer 2.10.3 (harness/composer-cache-dir.sh).
        let mac = |k: &str, v: &str| {
            let mut h = FakeHost::linux("/Users/u").with(k, v);
            h.darwin = true;
            h
        };
        // Trailing separators go, both kinds, any number.
        assert_eq!(
            setting(&mac("COMPOSER_CACHE_DIR", "/tmp/zz/")).unwrap(),
            "/tmp/zz"
        );
        assert_eq!(
            setting(&mac("COMPOSER_CACHE_DIR", "/tmp/zz\\/")).unwrap(),
            "/tmp/zz"
        );
        // `~/`, `$VAR`, `%VAR%` expand; an unset variable is the empty string.
        assert_eq!(
            setting(&mac("COMPOSER_CACHE_DIR", "~/zz")).unwrap(),
            "/Users/u/zz"
        );
        assert_eq!(
            setting(&mac("COMPOSER_CACHE_DIR", "$HOME/zz")).unwrap(),
            "/Users/u/zz"
        );
        assert_eq!(
            setting(&mac("COMPOSER_CACHE_DIR", "%HOME%/zz")).unwrap(),
            "/Users/u/zz"
        );
        assert_eq!(
            setting(&mac("COMPOSER_CACHE_DIR", "$NOPE/zz")).unwrap(),
            "/zz"
        );
        // Only a leading `~/`; `%VAR` without its closing `%` stays.
        assert_eq!(setting(&mac("COMPOSER_CACHE_DIR", "~zz")).unwrap(), "~zz");
        assert_eq!(setting(&mac("COMPOSER_CACHE_DIR", "~/")).unwrap(), "~");
        assert_eq!(
            setting(&mac("COMPOSER_CACHE_DIR", "%HOME/zz")).unwrap(),
            "%HOME/zz"
        );
        // Relative stays relative here (RELATIVE_PATHS), set-but-empty is
        // taken as it is, and so is `0` (Config::get tests `!== false`).
        assert_eq!(
            setting(&mac("COMPOSER_CACHE_DIR", "rel/x")).unwrap(),
            "rel/x"
        );
        assert_eq!(setting(&mac("COMPOSER_CACHE_DIR", "")).unwrap(), "");
        assert_eq!(setting(&mac("COMPOSER_CACHE_DIR", "/")).unwrap(), "");
        assert_eq!(setting(&mac("COMPOSER_CACHE_DIR", "0")).unwrap(), "0");
        // COMPOSER_HOME: `<home>/cache` as written, then expanded.
        assert_eq!(
            setting(&mac("COMPOSER_HOME", "/tmp/hh/")).unwrap(),
            "/tmp/hh//cache"
        );
        assert_eq!(
            setting(&mac("COMPOSER_HOME", "~/hh")).unwrap(),
            "/Users/u/hh/cache"
        );
        // `0` is PHP-falsy for Factory: as if unset.
        assert_eq!(
            setting(&mac("COMPOSER_HOME", "0")).unwrap(),
            "/Users/u/Library/Caches/composer"
        );
        // A `{$key}` reference is Config::process's: not reproduced.
        assert!(setting(&mac("COMPOSER_CACHE_DIR", "{$home}/c")).is_err());
    }

    #[test]
    fn linux_follows_xdg_etc_xdg_and_the_legacy_directory() {
        let h = FakeHost::linux("/home/u");
        assert_eq!(setting(&h).unwrap(), "/home/u/.composer/cache");
        // `/etc/xdg` alone turns XDG on (Factory::useXdg).
        let h = FakeHost::linux("/home/u").dir("/etc/xdg");
        assert_eq!(setting(&h).unwrap(), "/home/u/.cache/composer");
        assert_eq!(home_dir(&h).unwrap(), "/home/u/.config/composer");
        // Any XDG_* variable does too; XDG_CACHE_HOME is taken as written.
        let h = FakeHost::linux("/home/u").with("XDG_CACHE_HOME", "/x/");
        assert_eq!(setting(&h).unwrap(), "/x//composer");
        let h = FakeHost::linux("/home/u").with("XDG_RUNTIME_DIR", "/run/u");
        assert_eq!(setting(&h).unwrap(), "/home/u/.cache/composer");
        // An existing ~/.composer/cache wins while the home is ~/.composer.
        let h = FakeHost::linux("/home/u")
            .dir("/etc/xdg")
            .dir("/home/u/.composer")
            .dir("/home/u/.composer/cache");
        assert_eq!(setting(&h).unwrap(), "/home/u/.composer/cache");
        // HOME's trailing slash goes (getUserDir).
        let h = FakeHost::linux("/home/u/");
        assert_eq!(setting(&h).unwrap(), "/home/u/.composer/cache");
        // No HOME and no COMPOSER_HOME: Composer stops.
        assert!(setting(&FakeHost::default()).is_err());
        assert!(home_dir(&FakeHost::default()).is_err());
    }

    #[test]
    fn windows_paths_and_the_missing_appdata() {
        let win = || FakeHost {
            windows: true,
            ..FakeHost::default()
        };
        let h = win()
            .with("APPDATA", "C:\\Users\\u\\AppData\\Roaming\\")
            .with("LOCALAPPDATA", "C:\\Users\\u\\AppData\\Local");
        assert_eq!(setting(&h).unwrap(), "C:/Users/u/AppData/Local/Composer");
        assert_eq!(home_dir(&h).unwrap(), "C:/Users/u/AppData/Roaming/Composer");
        let h = win().with("APPDATA", "C:\\Users\\u\\AppData\\Roaming");
        assert_eq!(
            setting(&h).unwrap(),
            "C:/Users/u/AppData/Roaming/Composer/cache"
        );
        // A COMPOSER_CACHE_DIR keeps its backslashes, trailing ones aside.
        let h = win()
            .with("APPDATA", "C:\\a")
            .with("COMPOSER_CACHE_DIR", "D:\\cc\\");
        assert_eq!(setting(&h).unwrap(), "D:\\cc");
        // `$HOME` falls back to USERPROFILE on Windows only.
        let h = win()
            .with("APPDATA", "C:\\a")
            .with("USERPROFILE", "C:\\Users\\u")
            .with("COMPOSER_CACHE_DIR", "$HOME/cc");
        assert_eq!(setting(&h).unwrap(), "C:\\Users\\u/cc");
        // Neither APPDATA nor COMPOSER_HOME: getHomeDir throws, even with
        // LOCALAPPDATA or COMPOSER_CACHE_DIR set.
        assert!(setting(&win().with("LOCALAPPDATA", "C:\\l")).is_err());
        assert!(setting(&win().with("COMPOSER_CACHE_DIR", "C:\\c")).is_err());
        assert!(setting(&win().with("COMPOSER_HOME", "C:\\h")).is_ok());
    }

    #[test]
    fn the_global_config_json_is_read_under_the_processed_home() {
        let mut h = FakeHost::linux("/home/u").with("COMPOSER_HOME", "~/h/");
        h.config_json
            .insert("/home/u/h|cache-dir".to_owned(), Value::from("/srv/cc/"));
        assert_eq!(setting(&h).unwrap(), "/srv/cc");
        // The environment still wins over config.json.
        let h2 = FakeHost {
            config_json: h.config_json.clone(),
            ..FakeHost::linux("/home/u")
                .with("COMPOSER_HOME", "~/h/")
                .with("COMPOSER_CACHE_DIR", "/env")
        };
        assert_eq!(setting(&h2).unwrap(), "/env");
        // A value that is not a string is not reproduced.
        h.config_json
            .insert("/home/u/h|cache-dir".to_owned(), Value::from(3));
        assert!(setting(&h).is_err());
    }

    #[test]
    fn a_relative_cache_dir_resolves_against_the_working_directory() {
        // Config::realpath: absolute forms kept, the rest under the base.
        assert_eq!(config_realpath("/a", "/w"), "/a");
        assert_eq!(config_realpath("C:\\a", "/w"), "C:\\a");
        assert_eq!(config_realpath("c:/a", "/w"), "c:/a");
        assert_eq!(config_realpath("\\\\srv\\a", "/w"), "\\\\srv\\a");
        assert_eq!(config_realpath("s3://b", "/w"), "s3://b");
        assert_eq!(config_realpath("rel/x", "/w"), "/w/rel/x");
        assert_eq!(config_realpath("~", "/w"), "/w/~");
        assert_eq!(config_realpath("", "/w"), "/w/");
        assert!(!is_config_absolute("rel"));
        assert!(is_config_absolute("/rel"));
    }

    #[test]
    fn what_vivacity_hands_to_composer() {
        let h = |k: &str, v: &str| FakeHost::linux("/home/u").with(k, v);
        assert_eq!(
            cache_dir_hand_over_with(&FakeHost::linux("/home/u"), true),
            None
        );
        assert_eq!(
            cache_dir_hand_over_with(&h("COMPOSER_CACHE_DIR", "/c/"), true),
            None
        );
        for v in ["", "/", "0"] {
            assert!(
                cache_dir_hand_over_with(&h("COMPOSER_CACHE_DIR", v), false).is_some(),
                "{v:?}"
            );
        }
        // A cache sub-directory of its own: commands that read the cache only.
        for k in ["COMPOSER_CACHE_FILES_DIR", "COMPOSER_CACHE_REPO_DIR"] {
            assert!(cache_dir_hand_over_with(&h(k, "/f"), true).is_some(), "{k}");
            assert_eq!(cache_dir_hand_over_with(&h(k, "/f"), false), None, "{k}");
        }
        let mut cfg = FakeHost::linux("/home/u");
        cfg.config_json.insert(
            "/home/u/.composer|cache-files-dir".to_owned(),
            Value::from("/f"),
        );
        assert!(cache_dir_hand_over_with(&cfg, true).is_some());
        // Composer stops: the hand-over lets it say so.
        assert!(cache_dir_hand_over_with(&FakeHost::default(), false).is_some());
    }

    #[test]
    fn only_a_tilde_slash_expands() {
        // `'#^~[\\/]#'` in single quotes is the PCRE `[\/]`: a slash only.
        let h = FakeHost::linux("/home/u").with("COMPOSER_CACHE_DIR", "~\\zz");
        assert_eq!(setting(&h).unwrap(), "~\\zz");
    }

    #[test]
    fn a_global_layer_vivacity_does_not_read_is_handed_over() {
        // An unreadable config.json, or a cache key at the top of auth.json
        // or COMPOSER_AUTH (Factory merges both under `config`).
        let mut h = FakeHost::linux("/home/u");
        h.config_errors.insert(
            "/home/u/.composer|cache-dir".to_owned(),
            "cache-dir in auth.json".to_owned(),
        );
        assert!(setting(&h).is_err());
        assert!(cache_dir_hand_over_with(&h, false).is_some());
        let mut h = FakeHost::linux("/home/u");
        h.config_errors.insert(
            "/home/u/.composer|cache-files-dir".to_owned(),
            "cache-files-dir in COMPOSER_AUTH".to_owned(),
        );
        assert!(setting(&h).is_ok());
        assert!(cache_dir_hand_over_with(&h, true).is_some());
    }

    #[test]
    fn the_global_layer_is_read_as_factory_merges_it() {
        let dir = tempfile::tempdir().expect("tmp");
        let home = dir.path().to_str().expect("utf8");
        let layer = |config: Option<&str>, auth: Option<&str>, env: Option<&str>| {
            let _ = std::fs::remove_file(dir.path().join("config.json"));
            let _ = std::fs::remove_file(dir.path().join("auth.json"));
            if let Some(c) = config {
                std::fs::write(dir.path().join("config.json"), c).expect("write");
            }
            if let Some(a) = auth {
                std::fs::write(dir.path().join("auth.json"), a).expect("write");
            }
            global_config_layer(home, "cache-dir", env)
        };
        assert_eq!(layer(None, None, None), Ok(None));
        assert_eq!(
            layer(Some(r#"{"config":{"cache-dir":"/c"}}"#), None, None),
            Ok(Some(Value::from("/c")))
        );
        // auth.json and COMPOSER_AUTH as CI sets them: no effect.
        assert_eq!(
            layer(
                Some(r#"{"config":{"cache-dir":"/c"}}"#),
                Some(r#"{"github-oauth":{"github.com":"t"}}"#),
                Some(r#"{"http-basic":{}}"#)
            ),
            Ok(Some(Value::from("/c")))
        );
        assert!(layer(Some("{not json"), None, None).is_err());
        assert!(layer(None, Some("{not json"), None).is_err());
        assert!(layer(None, None, Some("{not json")).is_err());
        assert!(layer(None, Some(r#"{"cache-dir":"/a"}"#), None).is_err());
        assert!(layer(None, None, Some(r#"{"cache-dir":"/a"}"#)).is_err());
        // An empty COMPOSER_AUTH is ignored.
        assert_eq!(layer(None, None, Some("")), Ok(None));
        // `"0"` is not unset in 2.10.3 (`false === $env || '' === $env`):
        // it decodes to an integer, which the auth schema refuses.
        assert!(layer(None, None, Some("0")).is_err());
        assert!(layer(None, None, Some("[]")).is_err());
    }

    #[test]
    fn a_project_cache_setting_is_handed_over() {
        let m = |c: Value| serde_json::json!({ "config": c });
        assert_eq!(project_cache_config(&serde_json::json!({})), None);
        assert_eq!(
            project_cache_config(&m(serde_json::json!({"vendor-dir": "v"}))),
            None
        );
        for key in ["cache-dir", "cache-files-dir", "cache-repo-dir"] {
            assert!(
                project_cache_config(&m(serde_json::json!({ key: "/x" }))).is_some(),
                "{key}"
            );
        }
    }

    #[test]
    fn cache_layout_matches_composer() {
        // Checked against Composer's real cache in M0: the key is
        // files/<name>/<sha1(url)>.zip.
        let p = dist_cache_path(
            Path::new("/c"),
            "monolog/monolog",
            "https://api.github.com/repos/Seldaek/monolog/zipball/abc",
        );
        // `join` separates with `\` on Windows: compare in normalized form.
        let s = p.to_string_lossy().replace('\\', "/");
        assert!(s.starts_with("/c/files/monolog/monolog/"));
        assert!(s.ends_with(".zip"));
        assert_eq!(sha1_hex(b"abc"), "a9993e364706816aba3e25717850c26c9cd0d89d");
    }

    #[test]
    fn the_retry_rule_is_composers() {
        // The list `CurlDownloader` retries on, and nothing else: a 404, a 401
        // or a 403 is final, which is why a missing dist no longer takes three
        // tries to say so.
        for s in [423, 425, 500, 502, 503, 504, 507, 510] {
            assert!(status_is_retryable(s, "https://repo.example/x"), "{s}");
        }
        for s in [400, 401, 403, 404, 405, 409, 410, 418, 451, 501, 505] {
            assert!(!status_is_retryable(s, "https://repo.example/x"), "{s}");
        }
        // The one host-specific exception Composer carries.
        assert!(status_is_retryable(
            400,
            "https://codeload.github.com/acme/widget/zip/abc"
        ));
        assert!(!status_is_retryable(400, "https://github.com/acme/widget"));
    }

    /// A listener that accepts every connection and never writes a byte:
    /// an `https://` URL waits for a ServerHello, an `http://` one for a
    /// response. With `answer`, each connection gets it once, then silence.
    fn silent_listener(answer: Option<&'static [u8]>) -> u16 {
        use std::io::{Read, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind");
        let port = listener.local_addr().expect("addr").port();
        std::thread::spawn(move || {
            let mut held = Vec::new();
            for mut stream in listener.incoming().flatten() {
                if let Some(bytes) = answer {
                    let mut request = [0u8; 4096];
                    let _ = stream.read(&mut request);
                    let _ = stream.write_all(bytes);
                }
                held.push(stream);
            }
        });
        port
    }

    impl Fetcher {
        /// A fetcher whose attempts last `floor`, and `socket_timeout`'s
        /// answer when it is longer.
        fn with_total(floor: std::time::Duration, socket_timeout: fn() -> Option<u64>) -> Fetcher {
            let mut f = Fetcher::new(
                std::env::temp_dir().join("vivacity-fetch-test"),
                Auth::default(),
            )
            .expect("fetcher");
            f.total_floor = floor;
            f.socket_timeout = socket_timeout;
            f
        }
    }

    /// The experiment behind [`CONNECT_TIMEOUT`], kept as a test: with the
    /// reqwest, hyper-rustls and rustls of the lock (ring provider), the
    /// connection timeout stops a TLS handshake the server never answers —
    /// the TCP connection itself is up, as the `http://` request to the same
    /// listener shows by outliving it.
    #[cfg(feature = "rustls-tls-ring")]
    #[test]
    fn the_connect_timeout_covers_the_tls_handshake() {
        let port = silent_listener(None);
        let runtime = tokio::runtime::Runtime::new().expect("runtime");
        let limit = std::time::Duration::from_millis(500);
        let c = client(reqwest::redirect::Policy::none(), limit).expect("client");
        let t0 = std::time::Instant::now();
        // Bounded here, so that a client without the timeout fails the test
        // instead of hanging it.
        let e = runtime
            .block_on(async {
                tokio::time::timeout(
                    limit * 6,
                    c.get(format!("https://127.0.0.1:{port}/")).send(),
                )
                .await
            })
            .expect("the connection timeout fired before the test's own limit")
            .expect_err("no ServerHello ever comes");
        let elapsed = t0.elapsed();
        assert!(e.is_connect() && e.is_timeout(), "{e:?}");
        assert!(elapsed >= limit && elapsed < limit * 6, "{elapsed:?}");
        assert!(describe(&e).ends_with(TIMED_OUT), "{}", describe(&e));
        let plain = runtime.block_on(async {
            tokio::time::timeout(limit * 3, c.get(format!("http://127.0.0.1:{port}/")).send()).await
        });
        assert!(
            plain.is_err(),
            "connected over TCP, the response never comes"
        );
    }

    /// The total time covers the body: headers and the start of a body,
    /// then silence, is curl error 28 after the total — retried, as a GET.
    #[test]
    fn an_attempt_is_bounded_by_its_total_time_body_included() {
        let port = silent_listener(Some(
            b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 100\r\n\r\n{\"pa",
        ));
        let floor = std::time::Duration::from_millis(300);
        let fetcher = Fetcher::with_total(floor, || None);
        let runtime = tokio::runtime::Runtime::new().expect("runtime");
        let t0 = std::time::Instant::now();
        let url = format!("http://127.0.0.1:{port}/packages.json");
        let e = runtime
            .block_on(async {
                tokio::time::timeout(
                    std::time::Duration::from_secs(10),
                    fetcher.metadata_fetch(&url, None),
                )
                .await
            })
            .expect("the total time fired before the test's own limit")
            .expect_err("the body never ends");
        let elapsed = t0.elapsed();
        // Four attempts of 300 ms, 0 + 100 + 500 ms between them.
        assert!(
            elapsed >= std::time::Duration::from_millis(1800),
            "{elapsed:?}"
        );
        assert!(elapsed < std::time::Duration::from_secs(6), "{elapsed:?}");
        assert!(
            e.to_string()
                .ends_with("failed after 4 attempts: operation timed out after 300 milliseconds"),
            "{e}"
        );
    }

    /// `max((int) ini_get("default_socket_timeout"), 300)`: a longer
    /// setting lengthens the wait, a shorter one leaves the floor.
    #[test]
    fn default_socket_timeout_lengthens_the_total_never_shortens_it() {
        let runtime = tokio::runtime::Runtime::new().expect("runtime");
        let floor = std::time::Duration::from_millis(200);
        for (setting, total) in [
            ((|| Some(1)) as fn() -> Option<u64>, 1000u128),
            (|| Some(0), 200),
            (|| None, 200),
        ] {
            let fetcher = Fetcher::with_total(floor, setting);
            let t0 = std::time::Instant::now();
            let r = runtime.block_on(
                fetcher.within_total(std::future::pending::<std::result::Result<(), Attempt>>()),
            );
            let elapsed = t0.elapsed().as_millis();
            let a = r.expect_err("never answers");
            assert!(a.retryable);
            assert_eq!(
                a.message,
                format!("operation timed out after {total} milliseconds")
            );
            assert!(
                elapsed >= total && elapsed < total + 500,
                "{elapsed} for {total}"
            );
        }
        // A request that answers within the floor is not held.
        let fetcher = Fetcher::with_total(floor, || panic!("not asked"));
        assert!(runtime
            .block_on(fetcher.within_total(async { Ok::<_, Attempt>(7) }))
            .is_ok());
    }

    #[test]
    fn the_socket_timeout_is_read_after_its_marker() {
        assert_eq!(socket_timeout_of("socket-timeout:60"), Some(60));
        assert_eq!(
            socket_timeout_of("PHP Warning:  Module \"x\" is already loaded\nsocket-timeout:600"),
            Some(600)
        );
        assert_eq!(socket_timeout_of("socket-timeout:-1"), None);
        assert_eq!(socket_timeout_of("60"), None);
        assert_eq!(socket_timeout_of(""), None);
    }

    /// `FileDownloader`'s own rounds: 500, 502, 503 and 504 only.
    #[test]
    fn only_a_server_error_starts_a_new_download_round() {
        for s in [500, 502, 503, 504] {
            assert!(dist_round_is_retryable(Some(s)), "{s}");
        }
        for s in [None, Some(404), Some(423), Some(507), Some(510)] {
            assert!(!dist_round_is_retryable(s), "{s:?}");
        }
        assert_eq!(DIST_ROUNDS, 3);
    }

    #[test]
    fn the_retry_delays_are_composers() {
        // `restartJobWithDelay`: nothing, then 100 ms, then 500 ms.
        assert_eq!(retry_delay(1).as_millis(), 0);
        assert_eq!(retry_delay(2).as_millis(), 100);
        assert_eq!(retry_delay(3).as_millis(), 500);
        assert_eq!(retry_delay(4).as_millis(), 500);
        // Three retries at most, so four attempts, and the message says so
        // rather than claiming three when the verdict was final at once.
        assert_eq!(MAX_RETRIES, 3);
        assert_eq!(attempts_message(0, "gone"), "gone");
        assert_eq!(attempts_message(3, "gone"), "failed after 4 attempts: gone");
    }

    #[test]
    fn a_dist_url_that_names_a_local_file_is_recognised() {
        let p = |u: &str| Fetcher::local_dist_path(u).map(|p| p.to_string_lossy().into_owned());
        // What an `artifact` repository writes into the lock: a plain path.
        assert_eq!(
            p("/srv/artifacts/acme-widget-1.0.0.zip"),
            Some("/srv/artifacts/acme-widget-1.0.0.zip".to_owned())
        );
        // A `file://` URL, the other form Composer opens.
        assert_eq!(
            p("file:///srv/a%20b/x.zip"),
            // Nothing is decoded, as PHP's file wrapper decodes nothing.
            Some("/srv/a%20b/x.zip".to_owned())
        );
        // The Windows form of the same thing: the third slash is the
        // separator before the drive letter, not part of the path.
        assert_eq!(p("file:///C:/srv/x.zip"), Some("C:/srv/x.zip".to_owned()));
        // Everything else stays a download.
        assert_eq!(p("https://repo.example/x.zip"), None);
        assert_eq!(p("http://repo.example/x.zip"), None);
        assert_eq!(p("ssh://git@example/x.git"), None);
    }

    /// Where Composer sends what (`Url::getOrigin`,
    /// `AuthHelper::findAuthOrigin`, `AuthHelper::addAuthenticationOptions`,
    /// one credential per origin, `BaseIO::loadConfiguration` order).
    #[test]
    fn auth_header_selection() {
        let mut auth = Auth::default();
        auth.github_oauth
            .insert("github.com".into(), "ghtok".into());
        auth.github_oauth
            .insert("ghe.example.com".into(), "enttok".into());
        auth.bearer
            .insert("repo.example.com".into(), "beartok".into());
        auth.http_basic
            .insert("basic.example.com".into(), ("user".into(), "pass".into()));
        let h = |url: &str| auth.authorization_for_url(url);

        // The github.com token goes to the API only, http included
        // (`{^https?://api\.github\.com/}`).
        assert_eq!(
            h("https://api.github.com/repos/a/b/zipball/c").as_deref(),
            Some("token ghtok")
        );
        assert_eq!(
            h("http://api.github.com/repos/a/b/zipball/c").as_deref(),
            Some("token ghtok")
        );
        // codeload is its own origin; github.com and the other
        // `*.github.com` are origin github.com but not the API.
        assert_eq!(h("https://codeload.github.com/a/b/legacy.zip/c"), None);
        assert_eq!(h("https://github.com/a/b/archive/c.zip"), None);
        assert_eq!(h("https://objects.github.com/x.zip"), None);
        // A port is part of the origin: `api.github.com:8443` is not github.com.
        assert_eq!(h("https://api.github.com:8443/repos/a/b/zipball/c"), None);
        // A GitHub Enterprise domain: the token as basic credentials.
        assert_eq!(
            h("https://ghe.example.com/api/v3/repos/a/b/zipball/c").as_deref(),
            Some("Basic ZW50dG9rOngtb2F1dGgtYmFzaWM=")
        );
        assert_eq!(
            h("https://repo.example.com/x.zip").as_deref(),
            Some("Bearer beartok")
        );
        assert_eq!(
            h("https://basic.example.com/x.zip").as_deref(),
            Some("Basic dXNlcjpwYXNz")
        );
        assert_eq!(h("https://unknown.example.com/x.zip"), None);
        assert_eq!(h("not a url"), None);
    }

    #[test]
    fn auth_one_credential_per_origin() {
        // Same origin, three kinds: the last loaded wins (bearer).
        let mut auth = Auth::default();
        auth.github_oauth.insert("h.example".into(), "g".into());
        auth.http_basic
            .insert("h.example".into(), ("u".into(), "p".into()));
        auth.bearer.insert("h.example".into(), "b".into());
        assert_eq!(
            auth.authorization_for_url("https://h.example/x").as_deref(),
            Some("Bearer b")
        );
        // http-basic over github-oauth, on the API too.
        let mut auth = Auth::default();
        auth.github_oauth.insert("github.com".into(), "g".into());
        auth.http_basic
            .insert("github.com".into(), ("u".into(), "p".into()));
        assert_eq!(
            auth.authorization_for_url("https://api.github.com/x")
                .as_deref(),
            Some("Basic dTpw")
        );
        // repo.packagist.org is origin packagist.org; api.bitbucket.org
        // falls back to bitbucket.org.
        let mut auth = Auth::default();
        auth.http_basic
            .insert("packagist.org".into(), ("u".into(), "p".into()));
        auth.http_basic
            .insert("bitbucket.org".into(), ("u".into(), "p".into()));
        assert_eq!(
            auth.authorization_for_url("https://repo.packagist.org/p2/a/b.json")
                .as_deref(),
            Some("Basic dTpw")
        );
        assert_eq!(
            auth.authorization_for_url("https://api.bitbucket.org/2.0/x")
                .as_deref(),
            Some("Basic dTpw")
        );
        assert_eq!(auth.authorization_for_url("https://repo.example/x"), None);
        // The pair's values decide, not the key it came from: http-basic
        // with Composer's GitHub or bearer passwords.
        let mut auth = Auth::default();
        auth.http_basic
            .insert("github.com".into(), ("t".into(), "x-oauth-basic".into()));
        auth.http_basic
            .insert("b.example".into(), ("t".into(), "bearer".into()));
        assert_eq!(
            auth.authorization_for_url("https://api.github.com/x")
                .as_deref(),
            Some("token t")
        );
        assert_eq!(
            auth.authorization_for_url("https://github.com/a/b/archive/c.zip"),
            None
        );
        assert_eq!(
            auth.authorization_for_url("https://b.example/x").as_deref(),
            Some("Bearer t")
        );
    }

    /// `Factory::createComposer`: the COMPOSER_HOME auth.json, then the
    /// project's auth.json, then COMPOSER_AUTH (`loadComposerAuthEnv`, last).
    #[test]
    fn auth_sources_load_in_composers_order() {
        let home = tempfile::tempdir().expect("tmp");
        let project = tempfile::tempdir().expect("tmp");
        std::fs::write(
            home.path().join("auth.json"),
            r#"{"github-oauth":{"github.com":"home","a.example":"home"}}"#,
        )
        .unwrap();
        std::fs::write(
            project.path().join("auth.json"),
            r#"{"github-oauth":{"github.com":"project","b.example":"project"}}"#,
        )
        .unwrap();
        let env = r#"{"github-oauth":{"github.com":"env"}}"#;
        let auth = Auth::load_from(Some(home.path()), project.path(), Some(env));
        assert_eq!(auth.github_oauth["github.com"], "env");
        assert_eq!(auth.github_oauth["a.example"], "home");
        assert_eq!(auth.github_oauth["b.example"], "project");
        // Without the variable, the project's file wins over the home one.
        let auth = Auth::load_from(Some(home.path()), project.path(), None);
        assert_eq!(auth.github_oauth["github.com"], "project");
        // An empty variable adds nothing (a malformed one stops Composer:
        // `global_config_layer` hands such a run over before this is read).
        let auth = Auth::load_from(None, project.path(), Some(""));
        assert_eq!(auth.github_oauth["github.com"], "project");
    }

    #[test]
    fn composer_auth_env_shape_is_parsed() {
        let mut auth = Auth::default();
        auth.merge_value(&serde_json::json!({
            "github-oauth": {"github.com": "t1"},
            "http-basic": {"h": {"username": "u", "password": "p"}},
            "bearer": {"b": "tk"}
        }));
        assert_eq!(auth.github_oauth.len(), 1);
        assert_eq!(auth.http_basic.len(), 1);
        assert_eq!(auth.bearer.len(), 1);
    }
}

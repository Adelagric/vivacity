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
        if let Some(home) = home {
            auth.merge_json_file(&home.join("auth.json"));
        }
        auth.merge_json_file(&project_dir.join("auth.json"));
        if let Some(env) = composer_auth {
            if let Ok(v) = serde_json::from_str::<Value>(env) {
                auth.merge_value(&v);
            }
        }
        auth
    }

    fn merge_json_file(&mut self, path: &Path) {
        if let Ok(text) = std::fs::read_to_string(path) {
            if let Ok(v) = serde_json::from_str::<Value>(&text) {
                self.merge_value(&v);
            }
        }
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
        global_config_layer(home, key, std::env::var("COMPOSER_AUTH").ok().as_deref())
    }
}

/// The global layer of `Config::get($key)` as `Factory::createConfig`
/// merges it: `config.<key>` of `<home>/config.json`, then the top level of
/// `<home>/auth.json` and of COMPOSER_AUTH (both merged under `config`).
/// `Err` where Composer would read something vivacity does not: a file or
/// COMPOSER_AUTH that is not valid JSON (Composer stops), or `key` set by
/// auth.json or COMPOSER_AUTH.
fn global_config_layer(
    home: &str,
    key: &str,
    composer_auth: Option<&str>,
) -> std::result::Result<Option<Value>, String> {
    let read = |name: &str| -> std::result::Result<Option<Value>, String> {
        let path = Path::new(home).join(name);
        match std::fs::read_to_string(&path) {
            Err(_) => Ok(None),
            Ok(text) => serde_json::from_str(&text)
                .map(Some)
                .map_err(|_| format!("{} is not valid JSON", path.display())),
        }
    };
    let config = read("config.json")?;
    let auth = read("auth.json")?;
    // `if ($composerAuthEnv)`: PHP truthiness.
    let env_auth = match composer_auth.filter(|a| !a.is_empty() && *a != "0") {
        Some(text) => Some(
            serde_json::from_str::<Value>(text)
                .map_err(|_| "COMPOSER_AUTH is not valid JSON".to_owned())?,
        ),
        None => None,
    };
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

/// `config.<key>` of the global layer as `Config::get` sees it: config.json,
/// `Err` where auth.json or COMPOSER_AUTH set the key, a file is not valid
/// JSON, or the home cannot be resolved ([`global_config_layer`]).
pub fn global_config_value_checked(key: &str) -> std::result::Result<Option<Value>, String> {
    let host = RealHost;
    let home = config_home(&host)?;
    host.global_config(&home, key)
}

/// `Config::get('home')`: the directory of Composer's global config.json,
/// auth.json and composer.json (`Factory::getHomeDir`, expanded and
/// trimmed). `None` where Composer stops.
pub fn composer_home() -> Option<PathBuf> {
    config_home(&RealHost).ok().map(PathBuf::from)
}

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
    /// The command's error for it: Composer's transport failure, or the
    /// hand-over.
    fn into_error(self) -> Error {
        match self {
            InsecureRedirect::Refused(m) => Error::Transport(m),
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
        let client = reqwest::Client::builder()
            .user_agent(format!("vivacity/{}", env!("CARGO_PKG_VERSION")))
            .redirect(redirects)
            .build()
            .map_err(|e| Error::Http {
                url: "client".to_owned(),
                message: e.to_string(),
            })?;
        Ok(Fetcher {
            client,
            cache_root,
            auth,
        })
    }

    /// Bytes of the dist: cache first (shasum re-checked), network otherwise
    /// (3 attempts, backoff), cache fed through temp+rename.
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
        if let Ok(bytes) = std::fs::read(&cache_path) {
            match expected_sha1 {
                Some(exp) if sha1_hex(&bytes) != exp => {
                    // Corrupted/poisoned cache entry: throw it away.
                    let _ = std::fs::remove_file(&cache_path);
                }
                _ => return Ok((bytes, Provenance::Cache)),
            }
        }
        if offline {
            return Err(Error::Http {
                url: url.to_owned(),
                message: format!("missing cache for {name} in offline mode"),
            });
        }

        // A dist that lives on this filesystem is read, not downloaded: that
        // is what Composer does with it, and refusing it lost the whole
        // `artifact` repository family (offline installs, private builds).
        if let Some(path) = Self::local_dist_path(url) {
            let bytes = std::fs::read(&path).map_err(|e| Error::Http {
                url: url.to_owned(),
                message: format!("{} could not be read: {e}", path.display()),
            })?;
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
            self.cache_dist(&cache_path, &bytes);
            return Ok((bytes, Provenance::Network));
        }

        refuse_insecure(url)?;
        let mut retries = 0u32;
        let last_err = loop {
            match self.try_download(url).await {
                Ok(bytes) => {
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
                    self.cache_dist(&cache_path, &bytes);
                    return Ok((bytes, Provenance::Network));
                }
                Err(Attempt {
                    insecure_redirect: Some(stop),
                    ..
                }) => return Err(stop.into_error()),
                Err(a) => {
                    if !a.retryable || retries >= MAX_RETRIES {
                        break a.message;
                    }
                    retries += 1;
                    tokio::time::sleep(retry_delay(retries)).await;
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
    /// API: `packages[]=...`), 10 s timeout like Composer, a single attempt;
    /// 404 -> `NotFound`.
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
                message: e.to_string(),
            },
        })?;
        if resp.status() == reqwest::StatusCode::NOT_FOUND {
            return Ok(MetadataResponse::NotFound);
        }
        let resp = resp.error_for_status().map_err(|e| Error::Http {
            url: url.to_owned(),
            message: e.to_string(),
        })?;
        let bytes = resp.bytes().await.map_err(|e| Error::Http {
            url: url.to_owned(),
            message: e.to_string(),
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
            let mut req = self.client.get(url);
            if let Some(authz) = self.auth.authorization_for_url(url) {
                req = req.header(reqwest::header::AUTHORIZATION, authz);
            }
            if let Some(ims) = if_modified_since {
                req = req.header(reqwest::header::IF_MODIFIED_SINCE, ims);
            }
            let attempt = match req.send().await {
                Ok(resp) => {
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
                    match resp.error_for_status() {
                        Ok(resp) => match resp.bytes().await {
                            Ok(b) => {
                                return Ok(MetadataResponse::Body {
                                    bytes: b.to_vec(),
                                    last_modified,
                                })
                            }
                            Err(e) => Attempt {
                                retryable: transport_is_retryable(&e),
                                insecure_redirect: None,
                                message: e.to_string(),
                            },
                        },
                        Err(e) => Attempt {
                            retryable: e
                                .status()
                                .is_some_and(|s| status_is_retryable(s.as_u16(), url)),
                            insecure_redirect: None,
                            message: e.to_string(),
                        },
                    }
                }
                Err(e) => Attempt {
                    retryable: transport_is_retryable(&e),
                    insecure_redirect: insecure_redirect(&e),
                    message: e.to_string(),
                },
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

    /// Writes a dist into Composer's own files cache, through a temporary
    /// name so a killed process never leaves a half-written entry behind.
    fn cache_dist(&self, cache_path: &Path, bytes: &[u8]) {
        if let Some(parent) = cache_path.parent() {
            if std::fs::create_dir_all(parent).is_ok() {
                let tmp = cache_path.with_extension("zip.vivacity-tmp");
                if std::fs::write(&tmp, bytes).is_ok() {
                    let _ = std::fs::rename(&tmp, cache_path);
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
            message: e.to_string(),
            retryable: match e.status() {
                Some(s) => status_is_retryable(s.as_u16(), url),
                None => transport_is_retryable(&e),
            },
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
        // An empty COMPOSER_AUTH is PHP-falsy: ignored.
        assert_eq!(layer(None, None, Some("")), Ok(None));
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

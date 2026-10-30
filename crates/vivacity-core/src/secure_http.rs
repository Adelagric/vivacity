//! `secure-http`: `Config::prohibitUrlByConfig` (Config.php:692-738) and the
//! setting it reads (Config.php:415-425).
//!
//! Composer checks every URL it is about to request — `CurlDownloader`
//! (:181) for repository metadata and dists, `RemoteFilesystem` (:287),
//! `Git::runCommand` (:109). For a URL of scheme `http`, `git`, `ftp` or
//! `svn`, `secure-http` true throws a `TransportException` (« Your
//! configuration does not allow connections to … »), false writes « Warning:
//! Accessing <host> over <scheme> which is an insecure protocol. », once per
//! host and per `Config` — that is per `Composer` instance: `require` and
//! `remove` create a second one after editing composer.json
//! ([`new_composer`]). `HttpDownloader::startJob` answers before curl when the
//! network is disabled (HttpDownloader.php:328-338): no refusal and no
//! warning offline. A dist the files cache serves makes no request at all.
//!
//! This module decides; the callers print, at the place Composer's request
//! would have printed, or hand the run to Composer. Nothing here reads the
//! disk until an insecure URL shows up: the common path (every URL `https`)
//! is a string test.

use serde_json::Value;
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;

/// What a request to one URL does in Composer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Verdict {
    /// Not an insecure URL (or the network is disabled): nothing.
    Allowed,
    /// `secure-http` false: the warning is due for this host, unless the
    /// `Config` already wrote it ([`warn_once`]).
    Warn { scheme: String, host: String },
    /// `secure-http` true: Composer throws a `TransportException`, whose
    /// message is [`refusal`] of the sanitized URL.
    Refuse,
    /// Composer would read the setting from where vivacity does not
    /// (auth.json, `COMPOSER_AUTH`, `disable-tls`): the reason to hand over.
    HandOver(String),
}

struct State {
    /// Whether a run set the state ([`begin`]): a library caller that never
    /// did keeps the old behaviour, nothing refused nor warned.
    begun: bool,
    /// The project whose composer.json `config` is read, lazily.
    project: Option<PathBuf>,
    /// `--offline` (vivacity's transports honour it; `COMPOSER_DISABLE_NETWORK`
    /// alone they do not, so it does not count here either).
    offline: bool,
    /// The setting, once read: `Ok(secure)` or the hand-over reason.
    setting: Option<Result<bool, String>>,
    /// `Config::$warnedHosts`.
    warned: BTreeSet<String>,
    /// Hosts reached by a redirect while `secure-http` is false, in order:
    /// Composer warns about them as `CurlDownloader` restarts the job, in
    /// its wait loop ([`take_redirect_warnings`]).
    redirects: Vec<(String, String)>,
}

static STATE: Mutex<State> = Mutex::new(State {
    begun: false,
    project: None,
    offline: false,
    setting: None,
    warned: BTreeSet::new(),
    redirects: Vec::new(),
});

fn state() -> std::sync::MutexGuard<'static, State> {
    STATE.lock().unwrap_or_else(|e| e.into_inner())
}

/// A new run: the project it reads, whether the network is off; nothing
/// warned yet, the setting not read yet.
pub fn begin(project: Option<&Path>, offline: bool) {
    let mut s = state();
    s.begun = true;
    s.project = project.map(Path::to_path_buf);
    s.offline = offline;
    s.setting = None;
    s.warned.clear();
    s.redirects.clear();
    REDIRECTED.store(false, Ordering::Release);
}

/// A new `Composer` instance (and so a new `Config`): `require` and
/// `remove` create one after editing composer.json. The hosts it warned
/// about are forgotten.
pub fn new_composer() {
    state().warned.clear();
}

/// The scheme (lowercased) and host (as written) of a URL
/// `prohibitUrlByConfig` objects to, if it does: `http`, `git` or `ftp`, in
/// any case (`svn` left out, below). The host is `parse_url`'s:
/// between `//` and the path, without user info or port.
pub fn insecure(url: &str) -> Option<(String, Option<String>)> {
    let (scheme, rest) = url.split_once("://")?;
    // `svn` would need `secure-svn-domains` and its own message; vivacity
    // never requests one (svn is Composer's), so it is not listed.
    let scheme = ["http", "git", "ftp"]
        .into_iter()
        .find(|s| scheme.eq_ignore_ascii_case(s))?
        .to_owned();
    let authority = &rest[..rest.find(['/', '?', '#']).unwrap_or(rest.len())];
    let host_port = authority.rsplit_once('@').map_or(authority, |(_, h)| h);
    let host = if host_port.starts_with('[') {
        host_port
            .find(']')
            .map(|end| host_port[..=end].to_owned())
            .unwrap_or_else(|| host_port.to_owned())
    } else {
        host_port.split(':').next().unwrap_or("").to_owned()
    };
    // `FILTER_VALIDATE_URL` wants a host; `http://` without one is still
    // checked (`^https?://`), and `parse_url` then gives no host.
    if host.is_empty() {
        return (scheme == "http").then_some((scheme, None));
    }
    Some((scheme, Some(host)))
}

/// `CurlDownloader::initDownload` skips the check for Packagist's v1
/// provider URLs with a `$` placeholder (CurlDownloader.php:179).
fn curl_exempt(url: &str) -> bool {
    let rest = url
        .strip_prefix("http://repo.packagist.org/p/")
        .or_else(|| url.strip_prefix("http://packagist.org/p/"));
    rest.is_some() && (url.contains('$') || url.contains("%24"))
}

/// The `TransportException` message for a refused URL (sanitized by the
/// caller, as `Url::sanitize`).
pub fn refusal(sanitized_url: &str) -> String {
    format!(
        "Your configuration does not allow connections to {sanitized_url}. See https://getcomposer.org/doc/06-config.md#secure-http for details."
    )
}

/// The warning line for one host.
pub fn warning(scheme: &str, host: &str) -> String {
    format!("Warning: Accessing {host} over {scheme} which is an insecure protocol.")
}

/// What a request to `url` (an HTTP request: `CurlDownloader`'s exemption
/// applies) does. No lock is taken and nothing is read for a secure URL.
pub fn classify(url: &str) -> Verdict {
    let Some((scheme, host)) = insecure(url) else {
        return Verdict::Allowed;
    };
    if curl_exempt(url) {
        return Verdict::Allowed;
    }
    let mut s = state();
    if !s.begun || s.offline {
        return Verdict::Allowed;
    }
    let setting = match &s.setting {
        Some(v) => v.clone(),
        None => {
            let v = read_setting(s.project.as_deref());
            s.setting = Some(v.clone());
            v
        }
    };
    match setting {
        Err(reason) => Verdict::HandOver(reason),
        Ok(true) => Verdict::Refuse,
        Ok(false) => match host {
            Some(host) => Verdict::Warn { scheme, host },
            None => Verdict::Allowed,
        },
    }
}

/// The warning for `host`, unless this `Config` already wrote it; records
/// it as written.
pub fn warn_once(scheme: &str, host: &str) -> Option<String> {
    state()
        .warned
        .insert(host.to_owned())
        .then(|| warning(scheme, host))
}

/// A redirect to `url`, which `secure-http` false lets through: its host is
/// warned about later, where Composer's wait loop would ([`take_redirect_warnings`]).
pub fn note_redirect(scheme: &str, host: &str) {
    state().redirects.push((scheme.to_owned(), host.to_owned()));
    REDIRECTED.store(true, Ordering::Release);
}

/// Whether [`note_redirect`] ran since the last [`take_redirect_warnings`]:
/// the common path (no redirect to an insecure host) takes no lock.
static REDIRECTED: AtomicBool = AtomicBool::new(false);

/// The warnings due for the redirects noted since the last call, in order,
/// each host once per `Config`.
pub fn take_redirect_warnings() -> Vec<String> {
    if !REDIRECTED.swap(false, Ordering::AcqRel) {
        return Vec::new();
    }
    let mut s = state();
    let pending = std::mem::take(&mut s.redirects);
    pending
        .into_iter()
        .filter_map(|(scheme, host)| {
            s.warned
                .insert(host.clone())
                .then(|| warning(&scheme, &host))
        })
        .collect()
}

/// [`classify`] then, for a warning, [`warn_once`]: what one request does,
/// as the line to write now (`Ok(Some)`), nothing, or the refusal / the
/// hand-over reason (`Err`).
pub fn request(url: &str) -> Result<Option<String>, Verdict> {
    match classify(url) {
        Verdict::Allowed => Ok(None),
        Verdict::Warn { scheme, host } => Ok(warn_once(&scheme, &host)),
        other => Err(other),
    }
}

/// The run's setting: `Ok(secure-http)` as `Config::get('secure-http')`
/// returns it, or why Composer would read it from where vivacity does not.
/// Read only once an insecure URL shows up.
fn read_setting(project: Option<&Path>) -> Result<bool, String> {
    // COMPOSER_AUTH is merged under `config` last of all, after the project
    // (Factory.php:344): a setting there wins over everything, unread here.
    let env_auth = std::env::var("COMPOSER_AUTH").unwrap_or_default();
    if !env_auth.is_empty() && env_auth != "0" {
        match serde_json::from_str::<Value>(&env_auth) {
            Ok(v) => {
                for key in ["secure-http", "disable-tls"] {
                    if v.get(key).is_some() {
                        return Err(format!("{key} in COMPOSER_AUTH"));
                    }
                }
            }
            Err(_) => return Err("COMPOSER_AUTH is not valid JSON".to_owned()),
        }
    }
    let manifest = project.and_then(|p| crate::jsonfile::read(&p.join("composer.json")));
    let project_config = manifest.as_ref().and_then(|m| m.get("config"));
    // The project's auth.json is merged under `config` after composer.json
    // (Factory.php:334-338): a setting there is not read here.
    if let Some(auth) = project.and_then(|p| crate::jsonfile::read(&p.join("auth.json"))) {
        for key in ["secure-http", "disable-tls"] {
            if auth.get(key).is_some() {
                return Err(format!("{key} in auth.json"));
            }
        }
    }
    let global = |key: &str| crate::fetch::global_config_value_checked(key);
    let disable_tls = match project_config.and_then(|c| c.get("disable-tls")) {
        Some(v) => Some(v.clone()),
        None => global("disable-tls")?,
    };
    // `disable-tls` true turns `secure-http` off, and does more (no TLS, a
    // warning of its own at `Factory::createHttpDownloader`): not ported.
    if disable_tls.as_ref().is_some_and(config_bool) {
        return Err("disable-tls is set".to_owned());
    }
    let value = match project_config.and_then(|c| c.get("secure-http")) {
        Some(v) => Some(v.clone()),
        None => global("secure-http")?,
    };
    Ok(value.as_ref().is_none_or(config_bool))
}

/// `$this->config[$key] !== 'false' && (bool) $this->config[$key]`.
fn config_bool(v: &Value) -> bool {
    match v {
        Value::Null => false,
        Value::Bool(b) => *b,
        Value::Number(n) => n.as_f64().is_some_and(|f| f != 0.0),
        Value::String(s) => !(s.is_empty() || s == "0" || s == "false"),
        Value::Array(a) => !a.is_empty(),
        Value::Object(o) => !o.is_empty(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_schemes_and_hosts_prohibit_url_by_config_reads() {
        let h = |u: &str| insecure(u);
        assert_eq!(
            h("http://127.0.0.1:8080/packages.json"),
            Some(("http".into(), Some("127.0.0.1".into())))
        );
        assert_eq!(
            h("HTTP://User:pw@Example.COM/x.zip"),
            Some(("http".into(), Some("Example.COM".into())))
        );
        assert_eq!(
            h("git://github.com/a/b.git"),
            Some(("git".into(), Some("github.com".into())))
        );
        assert_eq!(
            h("http://[::1]:80/x"),
            Some(("http".into(), Some("[::1]".into())))
        );
        assert_eq!(h("https://example.com/x"), None);
        assert_eq!(h("svn://example.com/x"), None);
        assert_eq!(h("file:///tmp/x"), None);
        assert_eq!(h("/tmp/x.zip"), None);
        assert_eq!(h("ssh://git@example.com/x"), None);
    }

    #[test]
    fn packagist_v1_provider_placeholders_are_exempt() {
        assert!(curl_exempt(
            "http://repo.packagist.org/p/%package%$%hash%.json"
        ));
        assert!(curl_exempt("http://packagist.org/p/provider-a%24b.json"));
        assert!(!curl_exempt("http://repo.packagist.org/p/a/b.json"));
        assert!(!curl_exempt("http://example.org/p/$x"));
    }

    #[test]
    fn the_setting_is_read_as_config_get_reads_it() {
        assert!(config_bool(&Value::Bool(true)));
        assert!(!config_bool(&Value::String("false".into())));
        assert!(!config_bool(&Value::String("0".into())));
        assert!(config_bool(&Value::String("no".into())));
        assert!(!config_bool(&Value::Bool(false)));
        assert!(!config_bool(&serde_json::json!(0)));
    }

    /// The one test that drives the process-wide state (the others do not
    /// touch it): a run, its `Config`'s warned hosts, a second `Composer`.
    #[test]
    fn a_run_warns_once_per_host_and_config() {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(
            dir.path().join("composer.json"),
            r#"{"config": {"secure-http": false, "disable-tls": false}}"#,
        )
        .expect("manifest");
        begin(Some(dir.path()), false);
        let url = "http://h.test/x.zip";
        assert_eq!(
            classify(url),
            Verdict::Warn {
                scheme: "http".into(),
                host: "h.test".into()
            }
        );
        assert_eq!(request(url), Ok(Some(warning("http", "h.test"))));
        assert_eq!(request("http://h.test/y.zip"), Ok(None));
        new_composer();
        assert_eq!(request(url), Ok(Some(warning("http", "h.test"))));
        assert_eq!(classify("https://h.test/x.zip"), Verdict::Allowed);
        // Offline: no request, so neither warning nor refusal.
        begin(Some(dir.path()), true);
        assert_eq!(classify(url), Verdict::Allowed);
        std::fs::write(
            dir.path().join("composer.json"),
            r#"{"config": {"disable-tls": false}}"#,
        )
        .expect("manifest");
        // A new run reads the setting again (the default, unless the
        // machine's global config says otherwise).
        begin(Some(dir.path()), false);
        assert!(matches!(
            classify(url),
            Verdict::Refuse | Verdict::Warn { .. } | Verdict::HandOver(_)
        ));
        std::fs::write(dir.path().join("auth.json"), r#"{"secure-http": false}"#).expect("auth");
        begin(Some(dir.path()), false);
        assert_eq!(
            classify(url),
            Verdict::HandOver("secure-http in auth.json".into())
        );
        begin(None, false);
    }

    #[test]
    fn the_messages_are_composers() {
        assert_eq!(
            refusal("http://h/packages.json"),
            "Your configuration does not allow connections to http://h/packages.json. See https://getcomposer.org/doc/06-config.md#secure-http for details."
        );
        assert_eq!(
            warning("http", "h"),
            "Warning: Accessing h over http which is an insecure protocol."
        );
    }
}

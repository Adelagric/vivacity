//! Emulation of drupol/composer-packages 2.0.0 (MIT, Pol Dellaiera;
//! docs/reference/plugins/composer-packages/, NOTICE.md): at
//! `POST_AUTOLOAD_DUMP` its `Plugin::regeneration` writes five classes
//! under `<install path>/build/` — `Packages`, `Types`, `Directories`,
//! `Versions`, `Dependencies` — from the lock data (`packages`, then
//! `packages-dev`, whatever `--no-dev` says), through five Twig templates
//! (copied in `assets/composer-packages/`, rendered by the small subset of
//! Twig they use, [`render_template`]).
//!
//! What each class holds (`src/Exporter/*.php`):
//! - `Packages`: `name => lock entry`, sorted by name (`ksort`), and a regex
//!   per vendor prefix;
//! - `Types`: the four core types, then the others met, each with its
//!   entries;
//! - `Directories`: `getName()` (lower case) `=> getInstallPath()` —
//!   `realpath(vendor-dir)/<pretty name>[/<target-dir>]`, `NULL` for a
//!   metapackage (and a `symfony-pack` under Flex, whose installer extends
//!   the metapackage one);
//! - `Versions`: the names in lock order, the regex from the sorted versions;
//! - `Dependencies`: `getName() =>` the keys of `getRequires()` (lower case)
//!   but `php` and `ext-*`.
//!
//! `GENERATED_AT = time()` is the only thing two runs of Composer disagree
//! on. The checks in [`lock_issue`] name what this port does not
//! reproduce: those runs go to Composer before anything is written.

use crate::error::{Error, Result};
use serde_json::{Map, Value};
use std::path::{Path, PathBuf};

pub const PLUGIN_NAME: &str = "drupol/composer-packages";

/// The classes in `ClassGenerator::regenerateClasses` order, with their
/// template.
const CLASSES: [(&str, &str); 5] = [
    (
        "Packages",
        include_str!("../assets/composer-packages/packages.twig"),
    ),
    (
        "Types",
        include_str!("../assets/composer-packages/types.twig"),
    ),
    (
        "Directories",
        include_str!("../assets/composer-packages/directories.twig"),
    ),
    (
        "Versions",
        include_str!("../assets/composer-packages/versions.twig"),
    ),
    (
        "Dependencies",
        include_str!("../assets/composer-packages/dependencies.twig"),
    ),
];

/// What the plugin reads besides the lock.
pub struct Input<'a> {
    /// `getComposer()->getPackage()->getName()`: lower case, `__root__`
    /// without a name.
    pub root_name: &'a str,
    /// `realpath(vendor-dir)` as PHP computes it ([`php_realpath`]).
    pub vendor_real: &'a str,
    /// Flex is active: `symfony-pack` packages have no install path.
    pub flex_packs: bool,
    /// `time()`.
    pub generated_at: i64,
}

/// The entries of `packages` then `packages-dev`, as `array_merge` gives
/// them. Only call after [`lock_issue`] returned `None`.
fn entries(lock: &Value) -> Vec<&Map<String, Value>> {
    ["packages", "packages-dev"]
        .iter()
        .filter_map(|k| lock.get(*k).and_then(Value::as_array))
        .flatten()
        .filter_map(Value::as_object)
        .collect()
}

fn str_of<'a>(p: &'a Map<String, Value>, key: &str) -> &'a str {
    p.get(key).and_then(Value::as_str).unwrap_or_default()
}

/// A decimal integer string within PHP's range, which an array key turns
/// into an integer.
fn int_like(key: &str) -> bool {
    key.parse::<i64>().is_ok_and(|i| i.to_string() == key)
}

/// Any string (key or value) with a NUL byte, or a negative zero (`-0` is
/// the integer 0 for PHP, `-0.0` a float: the parsed value cannot tell).
fn unported_value(v: &Value) -> Option<&'static str> {
    match v {
        Value::String(s) if s.contains('\0') => Some("a NUL byte"),
        Value::Number(n)
            if n.as_f64() == Some(0.0) && n.as_f64().is_some_and(f64::is_sign_negative) =>
        {
            Some("a negative zero")
        }
        Value::Array(a) => a.iter().find_map(unported_value),
        Value::Object(o) => o.iter().find_map(|(k, v)| {
            if k.contains('\0') {
                Some("a NUL byte")
            } else {
                unported_value(v)
            }
        }),
        _ => None,
    }
}

/// Why the plugin's run on this lock data is not reproduced, if it is not:
/// what makes it throw (a missing key, a name without its `/`, a type that
/// is not a string), what PHP would turn into an integer key, and the
/// values this port does not write as `var_export` does.
pub fn lock_issue(lock: &Value) -> Option<String> {
    let mut names = std::collections::BTreeSet::new();
    for key in ["packages", "packages-dev"] {
        let Some(list) = lock.get(key).and_then(Value::as_array) else {
            return Some(format!(
                "the lock's `{key}` is not a list (the plugin's array_merge fails)"
            ));
        };
        for p in list {
            let Some(p) = p.as_object() else {
                return Some(format!("an entry of `{key}` is not an object"));
            };
            let Some(name) = p.get("name").and_then(Value::as_str) else {
                return Some(format!("an entry of `{key}` has no string name"));
            };
            if name.matches('/').count() != 1 {
                return Some(format!("the name `{name}` does not have exactly one `/`"));
            }
            if !names.insert(name.to_ascii_lowercase()) {
                return Some(format!("the name `{name}` is in the lock twice"));
            }
            // (A lock has no `version_normalized`: `ArrayLoader` normalizes
            // `version`, as `getLockedRepository` already did.)
            for field in ["version", "type"] {
                if p.get(field).and_then(Value::as_str).is_none() {
                    return Some(format!("{name} has no string `{field}`"));
                }
            }
            if int_like(str_of(p, "type")) {
                return Some(format!(
                    "the type of {name} is numeric (an integer key of Types)"
                ));
            }
            if p.get("target-dir").is_some_and(|t| !t.is_string()) {
                return Some(format!("the target-dir of {name} is not a string"));
            }
            if let Some(what) = unported_value(&Value::Object(p.clone())) {
                return Some(format!("{name} carries {what} (var_export not reproduced)"));
            }
        }
    }
    None
}

/// `Package::getTargetDir()`: `.` and `..` segments collapsed to `/`, the
/// leading `/` removed (a trailing one stays).
fn target_dir(raw: &str) -> String {
    let re = pcre2::bytes::RegexBuilder::new()
        .extended(true)
        .build(r"(?:^|[\\/]+) \.\.? (?:[\\/]+|$) (?:\.\.? (?:[\\/]+|$) )*")
        .expect("getTargetDir's regex");
    let bytes = raw.as_bytes();
    let mut out: Vec<u8> = Vec::with_capacity(bytes.len());
    let mut last = 0;
    for m in re.find_iter(bytes).flatten() {
        out.extend_from_slice(&bytes[last..m.start()]);
        out.push(b'/');
        last = m.end();
    }
    out.extend_from_slice(&bytes[last..]);
    let s = String::from_utf8(out).unwrap_or_default();
    s.trim_start_matches('/').to_owned()
}

/// `InstallationManager::getInstallPath` for the installers this port
/// knows: none for a metapackage (or a pack under Flex), else
/// LibraryInstaller's — `$targetDir ? '/'.$targetDir : ''`, where `"0"` is
/// false.
fn install_path(p: &Map<String, Value>, input: &Input<'_>) -> Value {
    let ty = str_of(p, "type");
    if ty == "metapackage" || (input.flex_packs && ty == "symfony-pack") {
        return Value::Null;
    }
    let mut path = format!("{}/{}", input.vendor_real, str_of(p, "name"));
    if let Some(raw) = p.get("target-dir").and_then(Value::as_str) {
        let td = target_dir(raw);
        if !td.is_empty() && td != "0" {
            path.push('/');
            path.push_str(&td);
        }
    }
    Value::String(path)
}

/// One regex group per vendor prefix, in first-seen order:
/// `'(?i:<prefix>)(?|' => ['/?(?i:<bundle>) (*MARK:<mark>)|', …]`.
fn regex_groups<'a>(items: impl Iterator<Item = (&'a str, String)>, dash: bool) -> Value {
    let mut groups: Map<String, Value> = Map::new();
    for (name, mark) in items {
        let (prefix, bundle) = name.split_once('/').unwrap_or((name, ""));
        let bundle = if dash {
            bundle.replace('-', "-?")
        } else {
            bundle.to_owned()
        };
        let line = format!("/?(?i:{bundle}) (*MARK:{mark})|");
        match groups
            .entry(format!("(?i:{prefix})(?|"))
            .or_insert_with(|| Value::Array(Vec::new()))
        {
            Value::Array(a) => a.push(Value::String(line)),
            _ => unreachable!(),
        }
    }
    Value::Object(groups)
}

/// Each exporter's `exportToArray()`, as Twig variables.
fn variables(class: &str, lock: &Value, input: &Input<'_>) -> Map<String, Value> {
    let all = entries(lock);
    let mut vars = Map::new();
    match class {
        "Packages" => {
            let mut sorted: Vec<&Map<String, Value>> = all.clone();
            sorted.sort_by(|a, b| {
                str_of(a, "name")
                    .as_bytes()
                    .cmp(str_of(b, "name").as_bytes())
            });
            let packages: Map<String, Value> = sorted
                .iter()
                .map(|p| (str_of(p, "name").to_owned(), Value::Object((*p).clone())))
                .collect();
            let regex = regex_groups(
                sorted
                    .iter()
                    .map(|p| (str_of(p, "name"), str_of(p, "name").to_owned())),
                true,
            );
            vars.insert("packages".into(), Value::Object(packages));
            vars.insert("regex".into(), regex);
        }
        "Types" => {
            let mut types: Map<String, Value> =
                ["library", "project", "metapackage", "composer-plugin"]
                    .iter()
                    .map(|t| ((*t).to_owned(), Value::Array(Vec::new())))
                    .collect();
            for p in &all {
                match types
                    .entry(str_of(p, "type").to_owned())
                    .or_insert_with(|| Value::Array(Vec::new()))
                {
                    Value::Array(a) => a.push(Value::Object((*p).clone())),
                    _ => unreachable!(),
                }
            }
            vars.insert("types".into(), Value::Object(types));
        }
        "Directories" => {
            let directories: Map<String, Value> = all
                .iter()
                .map(|p| {
                    (
                        str_of(p, "name").to_ascii_lowercase(),
                        install_path(p, input),
                    )
                })
                .collect();
            let regex = regex_groups(
                directories
                    .iter()
                    .map(|(n, d)| (n.as_str(), d.as_str().unwrap_or_default().to_owned())),
                true,
            );
            vars.insert("directories".into(), Value::Object(directories));
            vars.insert("regex".into(), regex);
        }
        "Versions" => {
            let names: Vec<Value> = all
                .iter()
                .map(|p| Value::String(str_of(p, "name").to_owned()))
                .collect();
            let mut versions: Vec<(&str, &str)> = all
                .iter()
                .map(|p| (str_of(p, "name"), str_of(p, "version")))
                .collect();
            versions.sort_by(|a, b| a.0.as_bytes().cmp(b.0.as_bytes()));
            let regex = regex_groups(versions.iter().map(|(n, v)| (*n, (*v).to_owned())), false);
            vars.insert("packageNames".into(), Value::Array(names));
            vars.insert("regex".into(), regex);
        }
        "Dependencies" => {
            let mut deps: Map<String, Value> = Map::new();
            for p in &all {
                let name = str_of(p, "name").to_ascii_lowercase();
                // `ArrayLoader::parseLinks`: an array only, string
                // constraints only, targets in lower case (a later one
                // keeps the first one's place); `$packageDeps += …` then
                // `$carry += …` merge a name met twice.
                let mut keys: Vec<String> = match deps.get(&name) {
                    Some(Value::Array(a)) => a
                        .iter()
                        .map(|v| match v {
                            Value::String(s) => s.clone(),
                            other => other.to_string(),
                        })
                        .collect(),
                    _ => Vec::new(),
                };
                let links: Vec<(String, &Value)> = match p.get("require") {
                    Some(Value::Object(o)) => o.iter().map(|(k, v)| (k.clone(), v)).collect(),
                    Some(Value::Array(a)) => a
                        .iter()
                        .enumerate()
                        .map(|(i, v)| (i.to_string(), v))
                        .collect(),
                    _ => Vec::new(),
                };
                for (target, constraint) in links {
                    if !constraint.is_string() {
                        continue;
                    }
                    let target = target.to_ascii_lowercase();
                    if target == "php" || target.starts_with("ext-") || keys.contains(&target) {
                        continue;
                    }
                    keys.push(target);
                }
                // An integer-like target is an integer key, and value.
                let values = keys
                    .into_iter()
                    .map(|k| match k.parse::<i64>() {
                        Ok(i) if int_like(&k) => Value::from(i),
                        _ => Value::String(k),
                    })
                    .collect();
                deps.insert(name, Value::Array(values));
            }
            let regex = regex_groups(deps.keys().map(|n| (n.as_str(), n.clone())), true);
            vars.insert("packageDeps".into(), Value::Object(deps));
            vars.insert("regex".into(), regex);
        }
        _ => unreachable!(),
    }
    vars.insert("generatedAt".into(), Value::from(input.generated_at));
    vars.insert(
        "rootPackageName".into(),
        Value::String(input.root_name.to_owned()),
    );
    vars
}

/// The five classes, in the order they are written.
pub fn render(lock: &Value, input: &Input<'_>) -> Result<Vec<(&'static str, String)>> {
    CLASSES
        .iter()
        .map(|(class, template)| {
            let vars = variables(class, lock, input);
            render_template(template, &vars)
                .map(|code| (*class, code))
                .map_err(|e| Error::Unsupported(format!("{PLUGIN_NAME} {class}: {e}")))
        })
        .collect()
}

/// `Exporter::exportToFile` for each class: a temporary file next to it,
/// `chmod 0664`, `rename` — never a write in place (the copy under
/// vendor/ shares its files with the store).
pub fn write(build_dir: &Path, classes: &[(&str, String)]) -> Result<()> {
    for (class, code) in classes {
        let dest = build_dir.join(format!("{class}.php"));
        let tmp = build_dir.join(format!("{class}.php_tmp{}", std::process::id()));
        std::fs::write(&tmp, code).map_err(Error::io(&tmp))?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o664))
                .map_err(Error::io(&tmp))?;
        }
        std::fs::rename(&tmp, &dest).map_err(Error::io(&dest))?;
    }
    Ok(())
}

/// PHP's `realpath()` (`tsrm_realpath`): `.` and `..` resolved, symbolic
/// links followed, every other component kept as given — where
/// `std::fs::canonicalize` would also take the case stored on disk
/// (macOS).
pub fn php_realpath(path: &Path) -> std::io::Result<PathBuf> {
    php_realpath_depth(path, 0)
}

fn php_realpath_depth(path: &Path, depth: usize) -> std::io::Result<PathBuf> {
    use std::path::Component;
    if depth > 40 {
        return Err(std::io::Error::other("too many levels of symbolic links"));
    }
    let path = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()?.join(path)
    };
    let mut out = PathBuf::from("/");
    for comp in path.components() {
        match comp {
            Component::RootDir | Component::Prefix(_) | Component::CurDir => {}
            Component::ParentDir => {
                out.pop();
            }
            Component::Normal(name) => {
                let next = out.join(name);
                let meta = std::fs::symlink_metadata(&next)?;
                out = if meta.file_type().is_symlink() {
                    let target = std::fs::read_link(&next)?;
                    php_realpath_depth(&out.join(target), depth + 1)?
                } else {
                    next
                };
            }
        }
    }
    Ok(out)
}

/// `htmlspecialchars($s, ENT_QUOTES | ENT_SUBSTITUTE, 'UTF-8')`: Twig's
/// `html` escaping strategy, the environment's default.
fn escape_html(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#039;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            c => out.push(c),
        }
    }
    out
}

/// `Name::camelize`: runs of characters outside `[a-z0-9]` (case-blind,
/// byte-wise) become a space, `ucwords`, the spaces removed, `lcfirst`.
fn camelize(s: &str) -> String {
    let mut words = String::new();
    let mut in_gap = false;
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() {
            words.push(b as char);
            in_gap = false;
        } else if !in_gap {
            words.push(' ');
            in_gap = true;
        }
    }
    let mut out = String::with_capacity(words.len());
    let mut upper = true;
    for c in words.chars() {
        if c == ' ' {
            upper = true;
        } else {
            out.push(if upper { c.to_ascii_uppercase() } else { c });
            upper = false;
        }
    }
    let mut chars = out.chars();
    match chars.next() {
        Some(first) => first.to_ascii_lowercase().to_string() + chars.as_str(),
        None => String::new(),
    }
}

/// A parsed template: the subset of Twig the five templates use.
enum Node {
    Text(String),
    /// `{{ name }}` or `{{ name|filter }}`.
    Print(String, Option<String>),
    /// `{% for key, value in seq %}` / `{% for value in seq %}`.
    For(Option<String>, String, String, Vec<Node>),
}

/// `{% for key, value in seq %}`'s three names.
type ForHead = (Option<String>, String, String);

/// Twig's lexer for these templates: text, `{{ … }}`, `{% … %}` — and the
/// newline right after a block tag removed (`%}\n?`), the indentation
/// before it kept.
fn parse(template: &str) -> std::result::Result<Vec<Node>, String> {
    // One frame per open `for` (its key, value and sequence names), the
    // template itself at the bottom.
    let mut stack: Vec<(Option<ForHead>, Vec<Node>)> = vec![(None, Vec::new())];
    let mut rest = template;
    loop {
        let next = [rest.find("{{"), rest.find("{%"), rest.find("{#")]
            .into_iter()
            .flatten()
            .min();
        let Some(at) = next else {
            if !rest.is_empty() {
                stack
                    .last_mut()
                    .expect("stack")
                    .1
                    .push(Node::Text(rest.to_owned()));
            }
            break;
        };
        if at > 0 {
            stack
                .last_mut()
                .expect("stack")
                .1
                .push(Node::Text(rest[..at].to_owned()));
        }
        let tag = &rest[at..];
        if tag.starts_with("{#") {
            return Err("comments are not supported".into());
        }
        let close = if tag.starts_with("{{") { "}}" } else { "%}" };
        let end = tag.find(close).ok_or("unclosed tag")?;
        let inner = tag[2..end].trim();
        if inner.starts_with('-')
            || inner.ends_with('-')
            || inner.starts_with('~')
            || inner.ends_with('~')
        {
            return Err("whitespace control is not supported".into());
        }
        rest = &tag[end + 2..];
        if close == "}}" {
            let (name, filter) = match inner.split_once('|') {
                Some((n, f)) => (n.trim().to_owned(), Some(f.trim().to_owned())),
                None => (inner.to_owned(), None),
            };
            stack
                .last_mut()
                .expect("stack")
                .1
                .push(Node::Print(name, filter));
            continue;
        }
        rest = rest.strip_prefix('\n').unwrap_or(rest);
        if inner == "endfor" {
            let (head, body) = stack.pop().ok_or("endfor without for")?;
            let (key, value, seq) = head.ok_or("endfor without for")?;
            stack
                .last_mut()
                .ok_or("endfor without for")?
                .1
                .push(Node::For(key, value, seq, body));
        } else if let Some(rest_for) = inner.strip_prefix("for ") {
            let (vars, seq) = rest_for.split_once(" in ").ok_or("malformed for")?;
            let (key, value) = match vars.split_once(',') {
                Some((k, v)) => (Some(k.trim().to_owned()), v.trim().to_owned()),
                None => (None, vars.trim().to_owned()),
            };
            stack.push((Some((key, value, seq.trim().to_owned())), Vec::new()));
        } else {
            return Err(format!("unsupported tag `{inner}`"));
        }
    }
    let (head, nodes) = stack.pop().ok_or("empty")?;
    if head.is_some() || !stack.is_empty() {
        return Err("unclosed for".into());
    }
    Ok(nodes)
}

/// The value of `{{ … }}` as PHP prints it: strings escaped, integers in
/// decimal, `null` empty.
fn print_value(v: &Value) -> std::result::Result<String, String> {
    match v {
        Value::String(s) => Ok(escape_html(s)),
        Value::Number(n) if n.is_i64() => Ok(n.to_string()),
        Value::Null => Ok(String::new()),
        other => Err(format!("cannot print {other}")),
    }
}

fn eval(
    nodes: &[Node],
    scope: &[Map<String, Value>],
    out: &mut String,
) -> std::result::Result<(), String> {
    let lookup = |name: &str| -> std::result::Result<Value, String> {
        scope
            .iter()
            .rev()
            .find_map(|s| s.get(name).cloned())
            .ok_or_else(|| format!("unknown variable `{name}`"))
    };
    for node in nodes {
        match node {
            Node::Text(t) => out.push_str(t),
            Node::Print(name, None) => out.push_str(&print_value(&lookup(name)?)?),
            Node::Print(name, Some(filter)) => {
                let v = lookup(name)?;
                match filter.as_str() {
                    // `VarExportExtension`: `var_export($data, true)`, safe.
                    "export" => out.push_str(&crate::runtime_stub::php_var_export(&v, 0)),
                    // `CamelCaseExtension`, escaped like any output.
                    "camelize" => {
                        let s = match &v {
                            Value::String(s) => s.clone(),
                            other => return Err(format!("camelize of {other}")),
                        };
                        out.push_str(&escape_html(&camelize(&s)));
                    }
                    f => return Err(format!("unsupported filter `{f}`")),
                }
            }
            Node::For(key, value, seq, body) => {
                let items: Vec<(Value, Value)> = match lookup(seq)? {
                    Value::Object(o) => o.into_iter().map(|(k, v)| (Value::String(k), v)).collect(),
                    Value::Array(a) => a
                        .into_iter()
                        .enumerate()
                        .map(|(i, v)| (Value::from(i as i64), v))
                        .collect(),
                    other => return Err(format!("cannot iterate {other}")),
                };
                for (k, v) in items {
                    let mut frame = Map::new();
                    if let Some(key) = key {
                        frame.insert(key.clone(), k);
                    }
                    frame.insert(value.clone(), v);
                    let mut inner: Vec<Map<String, Value>> = scope.to_vec();
                    inner.push(frame);
                    eval(body, &inner, out)?;
                }
            }
        }
    }
    Ok(())
}

/// One template rendered with its variables.
fn render_template(
    template: &str,
    vars: &Map<String, Value>,
) -> std::result::Result<String, String> {
    let nodes = parse(template)?;
    let mut out = String::new();
    eval(&nodes, std::slice::from_ref(vars), &mut out)?;
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn camelize_like_the_plugin() {
        // php -r 'require "src/Utils/Name.php"; echo Name::camelize(…);'
        assert_eq!(camelize("symfony/polyfill-php72"), "symfonyPolyfillPhp72");
        assert_eq!(camelize("composer-plugin"), "composerPlugin");
        assert_eq!(camelize("-foo_bar"), "fooBar");
        assert_eq!(camelize("été"), "t");
        assert_eq!(camelize("o'type&<x>"), "oTypeX");
        assert_eq!(camelize("3f/pcre"), "3fPcre");
        assert_eq!(camelize(""), "");
    }

    #[test]
    fn target_dir_like_package_get_target_dir() {
        assert_eq!(target_dir("Acme/Target"), "Acme/Target");
        assert_eq!(target_dir("/a/b/"), "a/b/");
        assert_eq!(target_dir("a/../b"), "a/b");
        assert_eq!(target_dir("../../x"), "x");
        assert_eq!(target_dir("a\\.\\b"), "a/b");
    }

    #[test]
    fn twig_subset_keeps_the_indentation_before_a_block_tag() {
        let vars: Map<String, Value> = serde_json::from_value(json!({
            "regex": {"(?i:a)(?|": ["/?(?i:b) (*MARK:a/b)|", "/?(?i:c) (*MARK:a/c)|"]},
            "n": "x'&<y>",
        }))
        .unwrap();
        let t = "R = '#^(?|\n{% for package, bundles in regex %}\n        {{ package }}\n    {% for packageName in bundles %}\n        {{ packageName }}\n    {% endfor %}\n    )|\n{% endfor %}\n    )$#x';\n{{ n }}|{{ n|camelize }}\n";
        assert_eq!(
            render_template(t, &vars).unwrap(),
            "R = '#^(?|\n        (?i:a)(?|\n            /?(?i:b) (*MARK:a/b)|\n            /?(?i:c) (*MARK:a/c)|\n        )|\n    )$#x';\nx&#039;&amp;&lt;y&gt;|xY\n"
        );
    }

    #[test]
    fn the_five_templates_parse() {
        for (class, template) in CLASSES {
            assert!(parse(template).is_ok(), "{class}");
        }
    }

    #[test]
    fn lock_issues() {
        let entry = |name: &str| json!({"name": name, "version": "1.0.0", "type": "library"});
        let ok = json!({"packages": [entry("a/b")], "packages-dev": []});
        assert_eq!(lock_issue(&ok), None);
        assert!(lock_issue(&json!({"packages": [entry("a/b")]})).is_some());
        assert!(lock_issue(&json!({"packages": [entry("ab")], "packages-dev": []})).is_some());
        assert!(
            lock_issue(&json!({"packages": [entry("a/b")], "packages-dev": [entry("A/B")]}))
                .is_some()
        );
        let mut numeric = entry("a/b");
        numeric["type"] = json!("12");
        assert!(lock_issue(&json!({"packages": [numeric], "packages-dev": []})).is_some());
        let mut nul = entry("a/b");
        nul["description"] = json!("a\u{0}b");
        assert!(lock_issue(&json!({"packages": [nul], "packages-dev": []})).is_some());
        let neg: Value = serde_json::from_str(
            r#"{"packages": [{"name": "a/b", "version": "1", "version_normalized": "1.0.0.0", "type": "library", "extra": {"z": -0.0}}], "packages-dev": []}"#,
        )
        .unwrap();
        assert!(lock_issue(&neg).is_some());
    }

    #[test]
    fn dependencies_and_directories() {
        let lock = json!({
            "packages": [
                {"name": "a/b-c", "version": "1.0.0", "version_normalized": "1.0.0.0", "type": "library",
                 "require": {"php": ">=7", "ext-json": "*", "Lib-ICU": "*", "x/Y": "^1", "123": "*", "bad": 1}, "target-dir": "T/"},
                {"name": "m/meta", "version": "1.0.0", "version_normalized": "1.0.0.0", "type": "metapackage"},
            ],
            "packages-dev": [],
        });
        let input = Input {
            root_name: "r/r",
            vendor_real: "/p/vendor",
            flex_packs: false,
            generated_at: 7,
        };
        let deps = variables("Dependencies", &lock, &input);
        assert_eq!(
            deps["packageDeps"],
            json!({"a/b-c": ["lib-icu", "x/y", 123], "m/meta": []})
        );
        // A name with capitals: `getName()` keys, the pretty name in the
        // path (`getPrettyName()`), the raw name in Packages and Versions.
        let caps = json!({"packages": [{"name": "Up/Case", "version": "1.0.0", "type": "library"}], "packages-dev": []});
        assert_eq!(
            variables("Directories", &caps, &input)["directories"],
            json!({"up/case": "/p/vendor/Up/Case"})
        );
        assert_eq!(
            variables("Dependencies", &caps, &input)["regex"],
            json!({"(?i:up)(?|": ["/?(?i:case) (*MARK:up/case)|"]})
        );
        assert_eq!(
            variables("Versions", &caps, &input)["regex"],
            json!({"(?i:Up)(?|": ["/?(?i:Case) (*MARK:1.0.0)|"]})
        );
        assert!(variables("Packages", &caps, &input)["packages"]
            .get("Up/Case")
            .is_some());
        let dirs = variables("Directories", &lock, &input);
        assert_eq!(
            dirs["directories"],
            json!({"a/b-c": "/p/vendor/a/b-c/T/", "m/meta": null})
        );
        assert_eq!(
            dirs["regex"],
            json!({"(?i:a)(?|": ["/?(?i:b-?c) (*MARK:/p/vendor/a/b-c/T/)|"], "(?i:m)(?|": ["/?(?i:meta) (*MARK:)|"]})
        );
    }

    #[cfg(unix)]
    #[test]
    fn php_realpath_follows_links_and_keeps_the_given_case() {
        let tmp = tempfile::tempdir().unwrap();
        let base = crate::pathutil::canonicalize(tmp.path()).unwrap();
        std::fs::create_dir_all(base.join("real/vendor")).unwrap();
        std::os::unix::fs::symlink(base.join("real"), base.join("link")).unwrap();
        assert_eq!(
            php_realpath(&base.join("link/./vendor/../vendor")).unwrap(),
            base.join("real/vendor")
        );
        assert!(php_realpath(&base.join("missing")).is_err());
    }
}

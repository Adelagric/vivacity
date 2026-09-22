//! `bamarni/composer-bin-plugin` (docs/reference/plugins/composer-bin-plugin/,
//! MIT), read at 1.9.1: on the `COMMAND` event and again on
//! `POST_AUTOLOAD_DUMP` it reads `extra.bamarni-bin`, prints one
//! `[bamarni-bin] …` deprecation line on stderr per setting left at a
//! default that 2.x flips (`bin-links` true, `forward-command` false),
//! and — only when `forward-command` is `true` and the command is
//! `install` or `update` — forwards the command to every `vendor-bin/*`
//! namespace (nested installs). The forwarding is not emulated: such a
//! project is handed to Composer; everything else is the lines.

use serde_json::Value;

pub const PLUGIN_NAME: &str = "bamarni/composer-bin-plugin";
const EXTRA_KEY: &str = "bamarni-bin";

/// `Config::__construct` over the root's `extra`: `Err` with the plugin's
/// message when a setting has the wrong type (Composer aborts).
#[derive(Debug)]
pub struct BinConfig {
    pub forward_command: bool,
    pub deprecations: Vec<String>,
}

pub fn config(root_extra: Option<&Value>) -> Result<BinConfig, String> {
    let user = root_extra
        .and_then(|e| e.get(EXTRA_KEY))
        .and_then(Value::as_object);
    let get_type = |v: &Value| match v {
        Value::Null => "null",
        Value::Bool(_) => "bool",
        Value::Number(n) if n.is_i64() || n.is_u64() => "int",
        Value::Number(_) => "float",
        Value::String(_) => "string",
        Value::Array(_) | Value::Object(_) => "array",
    };
    let bin_links = user.and_then(|u| u.get("bin-links"));
    let bin_links_value = match bin_links {
        None => true,
        Some(Value::Bool(b)) => *b,
        Some(other) => {
            return Err(format!(
            "Expected setting \"extra.{EXTRA_KEY}.bin-links\" to be a boolean value. Got \"{}\".",
            get_type(other)
        ))
        }
    };
    let mut deprecations = Vec::new();
    if bin_links_value && bin_links.is_none() {
        deprecations.push(format!(
            "The setting \"extra.{EXTRA_KEY}.bin-links\" will be set to \"false\" from 2.x onwards. If you wish to keep it to \"true\", you need to set it explicitly."
        ));
    }
    if let Some(t) = user.and_then(|u| u.get("target-directory")) {
        if !t.is_string() {
            return Err(format!(
                "Expected setting \"extra.{EXTRA_KEY}.target-directory\" to be a string. Got \"{}\".",
                get_type(t)
            ));
        }
    }
    let forward = user.and_then(|u| u.get("forward-command"));
    let forward_command = match forward {
        None => false,
        Some(Value::Bool(b)) => *b,
        Some(other) => {
            return Err(format!(
                "Expected setting \"extra.{EXTRA_KEY}.forward-command\" to be a boolean value. Got \"{}\".",
                get_type(other)
            ))
        }
    };
    if !forward_command && forward.is_none() {
        deprecations.push(format!(
            "The setting \"extra.{EXTRA_KEY}.forward-command\" will be set to \"true\" from 2.x onwards. If you wish to keep it to \"false\", you need to set it explicitly."
        ));
    }
    Ok(BinConfig {
        forward_command,
        deprecations,
    })
}

/// `Logger::logStandard`: the lines as they reach stderr.
pub fn print_deprecations(cfg: &BinConfig) {
    for d in &cfg.deprecations {
        eprintln!("[bamarni-bin] {d}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn defaults_forward_and_types() {
        let c = config(None).expect("defaults");
        assert!(!c.forward_command);
        assert_eq!(c.deprecations.len(), 2);
        let c = config(Some(
            &json!({"bamarni-bin": {"bin-links": true, "forward-command": false}}),
        ))
        .expect("explicit");
        assert!(c.deprecations.is_empty());
        let c = config(Some(&json!({"bamarni-bin": {"bin-links": false}}))).expect("one");
        assert_eq!(c.deprecations.len(), 1);
        assert!(c.deprecations[0].contains("forward-command"));
        assert!(
            config(Some(&json!({"bamarni-bin": {"forward-command": true}})))
                .expect("fwd")
                .forward_command
        );
        let e = config(Some(&json!({"bamarni-bin": {"forward-command": "yes"}}))).unwrap_err();
        assert_eq!(e, "Expected setting \"extra.bamarni-bin.forward-command\" to be a boolean value. Got \"string\".");
    }
}

//! API keys and the Kerf Cloud token kept in the operating system's own store, the macOS Keychain,
//! Windows Credential Manager, or the Secret Service on Linux, rather than in the editor's plain
//! settings or a file (`cloud_session.rs`).
//!
//! `KERF_SECRETS=off` turns the store off, so a test never reads or writes a developer's real
//! keychain; the editor then keeps keys in its settings, and the token in its file, as it does
//! wherever the store cannot be reached.

use serde_json::{json, Value};

/// The service every key is kept under.
const SERVICE: &str = "Industrial Kerf";

fn off() -> bool {
    std::env::var_os("KERF_SECRETS").is_some_and(|value| value == "off" || value == "0")
}

fn entry(name: &str, enabled: bool) -> Result<keyring::Entry, String> {
    if !enabled {
        return Err("the system keychain is off (KERF_SECRETS=off)".into());
    }
    if name.is_empty() {
        return Err("a secret needs a name".into());
    }
    keyring::Entry::new(SERVICE, name).map_err(|error| error.to_string())
}

/// The secret kept as `name`, `None` when there is none.
fn get(name: &str, enabled: bool) -> Result<Option<String>, String> {
    match entry(name, enabled)?.get_password() {
        Ok(value) => Ok(Some(value)),
        Err(keyring::Error::NoEntry) => Ok(None),
        Err(error) => Err(error.to_string()),
    }
}

/// Keep `value` as `name`.
fn set(name: &str, value: &str, enabled: bool) -> Result<(), String> {
    entry(name, enabled)?.set_password(value).map_err(|error| error.to_string())
}

/// Forget `name`; forgetting what is not there succeeds.
fn delete(name: &str, enabled: bool) -> Result<(), String> {
    match entry(name, enabled)?.delete_credential() {
        Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
        Err(error) => Err(error.to_string()),
    }
}

/// A store of named secrets: the system's keychain, or, in a test, one in memory.
pub(crate) trait Keychain {
    /// The secret kept as `name`, `None` when there is none; an error when the store cannot be
    /// reached.
    fn get(&self, name: &str) -> Result<Option<String>, String>;
    fn set(&self, name: &str, value: &str) -> Result<(), String>;
    /// Forgetting what is not there succeeds.
    fn delete(&self, name: &str) -> Result<(), String>;
}

/// The operating system's keychain, unless `KERF_SECRETS=off`.
pub(crate) struct SystemKeychain {
    enabled: bool,
}

impl SystemKeychain {
    pub(crate) fn new() -> Self {
        Self { enabled: !off() }
    }

    /// The keychain as `KERF_SECRETS=off` leaves it: refusing everything.
    #[cfg(test)]
    pub(crate) fn turned_off() -> Self {
        Self { enabled: false }
    }
}

impl Keychain for SystemKeychain {
    fn get(&self, name: &str) -> Result<Option<String>, String> {
        get(name, self.enabled)
    }

    fn set(&self, name: &str, value: &str) -> Result<(), String> {
        set(name, value, self.enabled)
    }

    fn delete(&self, name: &str) -> Result<(), String> {
        delete(name, self.enabled)
    }
}

/// `{op: get|set|delete, name, value?}` -> `{ok: true, value?}` (value null when get finds none),
/// or `{error}` when the store cannot be used.
pub(crate) fn request(request: &Value) -> Value {
    request_with(request, !off())
}

fn request_with(request: &Value, enabled: bool) -> Value {
    let name = request["name"].as_str().unwrap_or_default();
    let result = match request["op"].as_str() {
        Some("get") => get(name, enabled).map(|value| json!({"ok": true, "value": value})),
        Some("set") => match request["value"].as_str() {
            Some(value) => set(name, value, enabled).map(|()| json!({"ok": true})),
            None => Err("set needs a value".into()),
        },
        Some("delete") => delete(name, enabled).map(|()| json!({"ok": true})),
        _ => Err("op is get, set, or delete".into()),
    };
    result.unwrap_or_else(|error| json!({"error": error}))
}

#[cfg(test)]
mod tests {
    use super::*;

    // Only with the store off: a test must not touch a real keychain.
    #[test]
    fn requests_are_checked_and_the_store_can_be_turned_off() {
        assert_eq!(
            request_with(&json!({"op": "list"}), false),
            json!({"error": "op is get, set, or delete"})
        );
        let refused = request_with(&json!({"op": "get", "name": "agent/anthropic/apiKey"}), false);
        assert!(
            refused["error"].as_str().is_some_and(|error| error.contains("KERF_SECRETS=off")),
            "{refused}"
        );
        assert!(request_with(&json!({"op": "set", "name": "x"}), false)["error"].is_string());
        assert!(request_with(&json!({"op": "get", "name": ""}), true)["error"].is_string());
    }

    /// The real store: a key kept, read back, and forgotten. It writes to this machine's keychain, so
    /// it is run by hand: `cargo test -p kerf-qt-bridge --lib real_keychain -- --ignored`.
    #[test]
    #[ignore = "writes to this machine's keychain"]
    fn a_key_round_trips_through_the_real_keychain() {
        let name = format!("kerf-test-{}", std::process::id());
        let kept = request_with(&json!({"op": "set", "name": name, "value": "sk-test"}), true);
        assert_eq!(kept, json!({"ok": true}));
        assert_eq!(
            request_with(&json!({"op": "get", "name": name}), true),
            json!({"ok": true, "value": "sk-test"})
        );
        assert_eq!(request_with(&json!({"op": "delete", "name": name}), true), json!({"ok": true}));
        assert_eq!(
            request_with(&json!({"op": "get", "name": name}), true),
            json!({"ok": true, "value": null})
        );
    }
}

//! API keys and the Kerf Cloud token kept in the operating system's own store, the macOS Keychain,
//! Windows Credential Manager, or the Secret Service on Linux, rather than in a plain settings
//! file. The editor, `kerf` and `kerf-mcp` reach it here, so a key kept in one is the key the
//! others read.
//!
//! `KERF_SECRETS=off` turns the store off, so a test never reads or writes a developer's real
//! keychain, and so does [`turn_off`] (the editor's `--settings-dir`); the editor then keeps keys
//! in its settings, and the token in its file, as it does wherever the store cannot be reached.

use std::sync::atomic::{AtomicBool, Ordering};

/// The service every key is kept under.
const SERVICE: &str = "Industrial Kerf";

/// The store is off for this process: [`turn_off`] said so.
static OFF: AtomicBool = AtomicBool::new(false);

/// Turn the store off for this process from here on, as `KERF_SECRETS=off` does, leaving its
/// environment as it is.
pub fn turn_off() {
    OFF.store(true, Ordering::SeqCst);
}

/// Whether the store is off for this process: [`turn_off`], or `KERF_SECRETS=off` (or `0`).
pub fn off() -> bool {
    OFF.load(Ordering::SeqCst)
        || std::env::var_os("KERF_SECRETS").is_some_and(|value| value == "off" || value == "0")
}

/// A store of named secrets: the system's keychain, or, in a test, one in memory.
pub trait Keychain {
    /// The secret kept as `name`, `None` when there is none; an error when the store cannot be
    /// reached.
    fn get(&self, name: &str) -> Result<Option<String>, String>;
    fn set(&self, name: &str, value: &str) -> Result<(), String>;
    /// Forgetting what is not there succeeds.
    fn delete(&self, name: &str) -> Result<(), String>;
}

/// The operating system's keychain, unless `KERF_SECRETS=off` or [`turn_off`].
pub struct SystemKeychain {
    enabled: bool,
}

impl SystemKeychain {
    /// The keychain, off where [`off`] says.
    pub fn new() -> Self {
        Self { enabled: !off() }
    }

    /// The keychain on or off as `enabled` says, whatever the environment: off for a test that
    /// must not touch a real one, on for the one that tests it.
    pub fn enabled(enabled: bool) -> Self {
        Self { enabled }
    }

    fn entry(&self, name: &str) -> Result<keyring::Entry, String> {
        if !self.enabled {
            return Err("the system keychain is off (KERF_SECRETS=off)".into());
        }
        if name.is_empty() {
            return Err("a secret needs a name".into());
        }
        keyring::Entry::new(SERVICE, name).map_err(|error| error.to_string())
    }
}

impl Default for SystemKeychain {
    fn default() -> Self {
        Self::new()
    }
}

impl Keychain for SystemKeychain {
    fn get(&self, name: &str) -> Result<Option<String>, String> {
        match self.entry(name)?.get_password() {
            Ok(value) => Ok(Some(value)),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(error) => Err(error.to_string()),
        }
    }

    fn set(&self, name: &str, value: &str) -> Result<(), String> {
        self.entry(name)?
            .set_password(value)
            .map_err(|error| error.to_string())
    }

    fn delete(&self, name: &str) -> Result<(), String> {
        match self.entry(name)?.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(error) => Err(error.to_string()),
        }
    }
}

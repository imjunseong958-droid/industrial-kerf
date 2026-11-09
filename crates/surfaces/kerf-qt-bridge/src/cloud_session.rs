//! The Kerf Cloud session (`docs/cloud.md`): the service's URL in `~/.kerf/cloud.json`, and
//! the account token in the system's keychain, beside the API keys (`secrets.rs`).
//!
//! Where no keychain can be reached (`KERF_SECRETS=off`, or a Linux without a Secret Service)
//! the token stays in that file, which only its owner may read (0600), as every token did
//! before. A token found in the file while the keychain can be reached moves into it, and the
//! file keeps only the URL; so a token kept by an older Kerf moves the first time it is read.

use std::path::Path;

use kerf_cloud::{CloudError, Store};

use crate::secrets::Keychain;

/// The token's name in the keychain.
pub(crate) const TOKEN: &str = "cloud/token";

/// The session kept at `path`: the file's URL, and the token from the file or the keychain.
pub(crate) fn load(path: &Path, keychain: &dyn Keychain) -> Store {
    let mut store = Store::load(path);
    if let Some(token) = store.token().map(str::to_owned) {
        // A token is only written to the file when the keychain could not take it, or was by a
        // Kerf from before the keychain, so it is the newest: the keychain takes it, and the file
        // keeps the rest. Should the file not be rewritten, the next read moves it again.
        if keychain.set(TOKEN, &token).is_ok() {
            let mut rest = store.clone();
            rest.token = None;
            let _ = rest.save(path);
        }
        return store;
    }
    if let Ok(Some(token)) = keychain.get(TOKEN) {
        store.token = Some(token);
    }
    store
}

/// Keeps `token` as the session's, or forgets the session's (`None`): in the keychain where it
/// can be reached, else in the file.
pub(crate) fn keep_token(
    path: &Path,
    keychain: &dyn Keychain,
    token: Option<&str>,
) -> Result<(), CloudError> {
    let before = Store::load(path);
    let mut file = before.clone();
    match token.filter(|token| !token.trim().is_empty()) {
        Some(token) => match keychain.set(TOKEN, token) {
            Ok(()) => file.token = None,
            Err(_) => file.token = Some(token.to_owned()),
        },
        None => {
            // Where the keychain cannot be reached it holds nothing to forget.
            let _ = keychain.delete(TOKEN);
            file.token = None;
        }
    }
    // Only a change is written: a session kept in the keychain, with the URL from
    // KERF_CLOUD_URL, leaves no file at all.
    if file != before {
        file.save(path)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::secrets::SystemKeychain;
    use std::cell::{Cell, RefCell};
    use std::collections::HashMap;

    /// A keychain in memory, counting what was written to it.
    #[derive(Default)]
    struct Memory {
        secrets: RefCell<HashMap<String, String>>,
        writes: Cell<usize>,
    }

    impl Keychain for Memory {
        fn get(&self, name: &str) -> Result<Option<String>, String> {
            Ok(self.secrets.borrow().get(name).cloned())
        }

        fn set(&self, name: &str, value: &str) -> Result<(), String> {
            self.writes.set(self.writes.get() + 1);
            self.secrets.borrow_mut().insert(name.to_owned(), value.to_owned());
            Ok(())
        }

        fn delete(&self, name: &str) -> Result<(), String> {
            self.secrets.borrow_mut().remove(name);
            Ok(())
        }
    }

    fn on_disk(path: &Path) -> Option<serde_json::Value> {
        std::fs::read_to_string(path)
            .ok()
            .map(|text| serde_json::from_str(&text).unwrap())
    }

    #[test]
    fn with_a_keychain_the_token_never_reaches_the_file() {
        let home = tempfile::tempdir().unwrap();
        let path = home.path().join(".kerf/cloud.json");
        let keychain = Memory::default();
        // The URL from KERF_CLOUD_URL: nothing to put in a file.
        keep_token(&path, &keychain, Some("tok-1")).unwrap();
        assert_eq!(on_disk(&path), None);
        assert_eq!(keychain.get(TOKEN).unwrap().as_deref(), Some("tok-1"));
        assert_eq!(load(&path, &keychain).token(), Some("tok-1"));

        // The URL in the file stays there, and the token stays out.
        Store { url: Some("https://cloud.example".into()), token: None }.save(&path).unwrap();
        keep_token(&path, &keychain, Some("tok-2")).unwrap();
        assert_eq!(on_disk(&path), Some(serde_json::json!({"url": "https://cloud.example"})));
        let store = load(&path, &keychain);
        assert_eq!(store.token(), Some("tok-2"));
        assert!(store.service_url().is_some());

        keep_token(&path, &keychain, None).unwrap();
        assert_eq!(keychain.get(TOKEN).unwrap(), None);
        assert_eq!(load(&path, &keychain).token(), None);
    }

    #[test]
    fn a_token_file_moves_into_the_keychain_once() {
        let home = tempfile::tempdir().unwrap();
        let path = home.path().join(".kerf/cloud.json");
        // As an older Kerf kept it.
        Store { url: Some("https://cloud.example".into()), token: Some("tok-old".into()) }
            .save(&path)
            .unwrap();
        let keychain = Memory::default();

        assert_eq!(load(&path, &keychain).token(), Some("tok-old"));
        assert_eq!(keychain.get(TOKEN).unwrap().as_deref(), Some("tok-old"));
        assert_eq!(on_disk(&path), Some(serde_json::json!({"url": "https://cloud.example"})));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&path).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o600, "{mode:o}");
        }

        // Read again, it comes from the keychain, which is not written again.
        assert_eq!(load(&path, &keychain).token(), Some("tok-old"));
        assert_eq!(keychain.writes.get(), 1);
    }

    // KERF_SECRETS=off is a keychain that cannot be reached, as a Linux without a Secret
    // Service is: the token is kept in the file, readable by its owner only, as before.
    #[test]
    fn without_a_keychain_the_token_stays_in_the_file() {
        let home = tempfile::tempdir().unwrap();
        let path = home.path().join(".kerf/cloud.json");
        let keychain = SystemKeychain::turned_off();

        keep_token(&path, &keychain, Some("tok-file")).unwrap();
        assert_eq!(on_disk(&path), Some(serde_json::json!({"token": "tok-file"})));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&path).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o600, "{mode:o}");
        }
        // Nothing moves it, and it is still read.
        assert_eq!(load(&path, &keychain).token(), Some("tok-file"));
        assert_eq!(on_disk(&path), Some(serde_json::json!({"token": "tok-file"})));

        keep_token(&path, &keychain, None).unwrap();
        assert_eq!(on_disk(&path), Some(serde_json::json!({})));
        assert_eq!(load(&path, &keychain).token(), None);
    }

    // Signed in while the keychain could not be reached, then read where it can: the file's
    // token is the newer, and replaces the keychain's.
    #[test]
    fn a_token_kept_in_the_file_meanwhile_is_the_one_used() {
        let home = tempfile::tempdir().unwrap();
        let path = home.path().join(".kerf/cloud.json");
        let keychain = Memory::default();
        keychain.set(TOKEN, "tok-earlier").unwrap();
        keep_token(&path, &SystemKeychain::turned_off(), Some("tok-later")).unwrap();

        assert_eq!(load(&path, &keychain).token(), Some("tok-later"));
        assert_eq!(keychain.get(TOKEN).unwrap().as_deref(), Some("tok-later"));
        assert_eq!(on_disk(&path), Some(serde_json::json!({})));
    }

    #[test]
    fn an_empty_token_signs_out() {
        let home = tempfile::tempdir().unwrap();
        let path = home.path().join(".kerf/cloud.json");
        let keychain = Memory::default();
        keep_token(&path, &keychain, Some("tok")).unwrap();
        keep_token(&path, &keychain, Some("  ")).unwrap();
        assert_eq!(load(&path, &keychain).token(), None);
    }

    /// The real store: the token kept, read back, and forgotten. It writes to this machine's
    /// keychain, so it is run by hand:
    /// `cargo test -p kerf-qt-bridge --lib cloud_session::tests::real -- --ignored`.
    #[test]
    #[ignore = "writes to this machine's keychain"]
    fn real_keychain_keeps_the_token_out_of_the_file() {
        let home = tempfile::tempdir().unwrap();
        let path = home.path().join(".kerf/cloud.json");
        let keychain = SystemKeychain::new();
        keep_token(&path, &keychain, Some("tok-real")).unwrap();
        assert_eq!(on_disk(&path), None);
        assert_eq!(load(&path, &keychain).token(), Some("tok-real"));

        // Windows' Credential Manager refuses a token longer than 1280 UTF-16 units, so that one
        // stays whole in the file, and is the one read, not the shorter one still in the keychain.
        let long = "t".repeat(3000);
        keep_token(&path, &keychain, Some(&long)).unwrap();
        let in_file = on_disk(&path).and_then(|file| file["token"].as_str().map(str::to_owned));
        assert_eq!(in_file.as_deref(), cfg!(windows).then_some(long.as_str()));
        assert_eq!(load(&path, &keychain).token(), Some(long.as_str()));

        keep_token(&path, &keychain, None).unwrap();
        assert_eq!(on_disk(&path).and_then(|file| file.get("token").cloned()), None);
        assert_eq!(load(&path, &keychain).token(), None);
    }
}

//! Kerf Cloud: the hosted model behind the paid tier. The editor is free;
//! a subscription buys requests to the hosted model, metered and checked by
//! the service. Entitlements live on the server, since a check in an
//! open-source client is no check at all. The client only reflects them.
//!
//! The service is not deployed yet. This crate holds the client side of its
//! contract (`docs/cloud.md`): where the service is, the signed-in session,
//! the account, and what the account allows. Until a service URL is
//! configured, Kerf Cloud reports itself unavailable and people use their own
//! API key (Settings → Advanced).

use serde::{Deserialize, Serialize};
use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

/// Overrides the service URL, for development and staging.
pub const URL_ENV: &str = "KERF_CLOUD_URL";

/// The service URL and session, in `~/.kerf/cloud.json`, outside projects.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Store {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    /// The account token the service issued.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub token: Option<String>,
}

impl Store {
    pub fn default_path() -> Option<PathBuf> {
        std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".kerf/cloud.json"))
    }

    /// The store at `path`; a missing or unreadable file is an empty store.
    pub fn load(path: &Path) -> Self {
        fs::read_to_string(path)
            .ok()
            .and_then(|text| serde_json::from_str(&text).ok())
            .unwrap_or_default()
    }

    pub fn save(&self, path: &Path) -> Result<(), CloudError> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(CloudError::io)?;
        }
        let text = serde_json::to_string_pretty(self).expect("store serializes");
        // Write beside, then rename, so a crash never leaves half a token.
        let temporary = path.with_extension("json.tmp");
        fs::write(&temporary, text).map_err(CloudError::io)?;
        restrict(&temporary);
        fs::rename(&temporary, path).map_err(CloudError::io)
    }

    /// The service URL: the environment override, else the stored one.
    pub fn service_url(&self) -> Option<String> {
        std::env::var(URL_ENV)
            .ok()
            .or_else(|| self.url.clone())
            .map(|url| url.trim().trim_end_matches('/').to_string())
            .filter(|url| !url.is_empty())
    }

    pub fn token(&self) -> Option<&str> {
        self.token
            .as_deref()
            .filter(|token| !token.trim().is_empty())
    }
}

#[cfg(unix)]
fn restrict(path: &Path) {
    use std::os::unix::fs::PermissionsExt;
    let _ = fs::set_permissions(path, fs::Permissions::from_mode(0o600));
}

#[cfg(not(unix))]
fn restrict(_: &Path) {}

/// What the service reports about the signed-in account.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Account {
    pub email: String,
    /// The plan's display name, such as `Free` or `Pro`.
    pub plan: String,
    /// The plan includes hosted model requests.
    pub ai: bool,
    /// Requests left in this period; `None` when unmetered.
    #[serde(default)]
    pub credits: Option<u64>,
}

/// Whether hosted requests can be made now, and if not, why.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Access {
    Ready,
    /// No service URL is configured.
    Unavailable,
    SignedOut,
    /// Signed in, but the service could not be reached.
    Unreachable,
    /// The plan does not include hosted requests.
    NotIncluded,
    OutOfCredits,
}

impl Access {
    pub fn of(account: &Account) -> Self {
        if !account.ai {
            Self::NotIncluded
        } else if account.credits == Some(0) {
            Self::OutOfCredits
        } else {
            Self::Ready
        }
    }

    pub fn reason(self) -> Option<&'static str> {
        match self {
            Self::Ready => None,
            Self::Unavailable => {
                Some("Kerf Cloud isn't available yet. Add your own API key in Settings → Advanced.")
            }
            Self::SignedOut => Some("Sign in to Kerf Cloud in Settings"),
            Self::Unreachable => Some("Kerf Cloud can't be reached right now"),
            Self::NotIncluded => Some("Your plan doesn't include Kerf Cloud requests"),
            Self::OutOfCredits => Some("You've used this period's Kerf Cloud requests"),
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum CloudError {
    Unavailable,
    SignedOut,
    /// The service refused the token.
    Unauthorized,
    Service(String),
    Io(String),
}

impl CloudError {
    fn io(error: std::io::Error) -> Self {
        Self::Io(error.to_string())
    }
}

impl fmt::Display for CloudError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unavailable => f.write_str(Access::Unavailable.reason().unwrap_or_default()),
            Self::SignedOut => f.write_str(Access::SignedOut.reason().unwrap_or_default()),
            Self::Unauthorized => {
                f.write_str("Kerf Cloud did not accept this sign-in; sign in again")
            }
            Self::Service(message) => write!(f, "Kerf Cloud: {message}"),
            Self::Io(message) => write!(f, "Could not save the Kerf Cloud session: {message}"),
        }
    }
}

impl std::error::Error for CloudError {}

/// The account for `token`, from `GET {url}/v1/account`.
pub fn fetch_account(url: &str, token: &str) -> Result<Account, CloudError> {
    let agent = ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(15)))
        .http_status_as_error(false)
        .build()
        .new_agent();
    let mut response = agent
        .get(format!("{url}/v1/account"))
        .header("Authorization", format!("Bearer {token}"))
        .call()
        .map_err(|error| CloudError::Service(error.to_string()))?;
    match response.status().as_u16() {
        200 => response
            .body_mut()
            .read_json::<Account>()
            .map_err(|error| CloudError::Service(format!("unreadable account: {error}"))),
        401 | 403 => Err(CloudError::Unauthorized),
        status => Err(CloudError::Service(format!(
            "account request failed ({status})"
        ))),
    }
}

/// The hosted model endpoint: the Anthropic Messages protocol under
/// `{url}/v1`, authenticated with the account token.
pub fn model_base_url(url: &str) -> String {
    format!("{url}/v1")
}

/// The service URL and token an agent session needs, or why there are none.
pub fn model_credentials(store: &Store) -> Result<(String, String), CloudError> {
    let url = store.service_url().ok_or(CloudError::Unavailable)?;
    let token = store.token().ok_or(CloudError::SignedOut)?;
    Ok((model_base_url(&url), token.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::thread;

    /// A one-request service answering with `status` and `body`, returning
    /// the request it saw.
    fn serve(status: &'static str, body: &'static str) -> (String, thread::JoinHandle<String>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let handle = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = [0u8; 4096];
            let read = stream.read(&mut request).unwrap();
            write!(
                stream,
                "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            )
            .unwrap();
            String::from_utf8_lossy(&request[..read]).into_owned()
        });
        (url, handle)
    }

    #[test]
    fn accounts_are_read_with_the_token() {
        let (url, server) = serve(
            "200 OK",
            r#"{"email":"lim@example.com","plan":"Pro","ai":true,"credits":1200}"#,
        );
        let account = fetch_account(&url, "tok-1").unwrap();
        assert_eq!(account.plan, "Pro");
        assert_eq!(Access::of(&account), Access::Ready);
        let request = server.join().unwrap();
        assert!(request.starts_with("GET /v1/account "));
        assert!(request
            .to_ascii_lowercase()
            .contains("authorization: bearer tok-1"));
    }

    #[test]
    fn refused_tokens_sign_out() {
        let (url, server) = serve("401 Unauthorized", "{}");
        assert_eq!(fetch_account(&url, "old"), Err(CloudError::Unauthorized));
        server.join().unwrap();
    }

    #[test]
    fn access_follows_the_plan_and_credits() {
        let mut account = Account {
            email: "a@b".into(),
            plan: "Free".into(),
            ai: false,
            credits: Some(0),
        };
        assert_eq!(Access::of(&account), Access::NotIncluded);
        account.ai = true;
        assert_eq!(Access::of(&account), Access::OutOfCredits);
        account.credits = None;
        assert_eq!(Access::of(&account), Access::Ready);
        assert!(Access::Unavailable
            .reason()
            .unwrap()
            .contains("Settings → Advanced"));
    }

    #[test]
    fn the_session_is_kept_outside_projects_and_drives_the_model_endpoint() {
        let directory = std::env::temp_dir().join(format!("kerf-cloud-{}", std::process::id()));
        let path = directory.join("cloud.json");
        let mut store = Store::load(&path);
        assert_eq!(store, Store::default());
        if std::env::var_os(URL_ENV).is_none() {
            assert_eq!(model_credentials(&store), Err(CloudError::Unavailable));
            store.url = Some("https://cloud.example/".into());
            assert_eq!(model_credentials(&store), Err(CloudError::SignedOut));
            store.token = Some("tok".into());
            assert_eq!(
                model_credentials(&store),
                Ok(("https://cloud.example/v1".into(), "tok".into()))
            );
        }
        store.token = Some("tok".into());
        store.save(&path).unwrap();
        assert_eq!(Store::load(&path), store);
        let _ = fs::remove_dir_all(directory);
    }
}

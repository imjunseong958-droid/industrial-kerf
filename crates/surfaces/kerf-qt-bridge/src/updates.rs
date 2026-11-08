//! Whether a newer Kerf has been released on GitHub. Kerf only asks and says so; downloading and
//! installing are the person's, from the release page.
//!
//! 0.x releases are pre-releases, which GitHub's "latest release" leaves out, so the newest is
//! found among the recent ones. `KERF_UPDATE_CHECK=off` asks nothing, as the tests run.

use serde_json::{json, Value};
use std::time::Duration;

/// Kerf's releases, newest first.
const RELEASES: &str =
    "https://api.github.com/repos/imjunseong958-droid/industrial-kerf/releases?per_page=20";

/// `{current, latest?, newer, url?}`, or `{error}` when GitHub could not be asked.
pub(crate) fn check() -> Value {
    if std::env::var_os("KERF_UPDATE_CHECK").is_some_and(|value| value == "off" || value == "0") {
        return json!({"error": "the update check is off (KERF_UPDATE_CHECK=off)"});
    }
    match fetch() {
        Ok(releases) => newest(&releases, env!("CARGO_PKG_VERSION")),
        Err(error) => json!({"error": error}),
    }
}

fn fetch() -> Result<Value, String> {
    let tls = ureq::tls::TlsConfig::builder()
        .root_certs(ureq::tls::RootCerts::PlatformVerifier)
        .build();
    let agent = ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(10)))
        .tls_config(tls)
        .build()
        .new_agent();
    agent
        .get(RELEASES)
        .header("Accept", "application/vnd.github+json")
        .header("User-Agent", concat!("industrial-kerf/", env!("CARGO_PKG_VERSION")))
        .call()
        .map_err(|error| format!("could not ask GitHub for releases: {error}"))?
        .body_mut()
        .read_json::<Value>()
        .map_err(|error| format!("GitHub's answer could not be read: {error}"))
}

/// `1.2.3` from `v1.2.3` or `1.2.3`; anything after a `-` or `+` is dropped, so a release
/// candidate counts as its release.
fn version(text: &str) -> Option<(u64, u64, u64)> {
    let core = text.trim().trim_start_matches('v').split(['-', '+']).next()?;
    let mut parts = core.split('.').map(|part| part.parse::<u64>().ok());
    let version = (parts.next()??, parts.next().unwrap_or(Some(0))?, parts.next().unwrap_or(Some(0))?);
    parts.next().is_none().then_some(version)
}

/// The newest published release in `releases` (GitHub's list), against `current`.
pub(crate) fn newest(releases: &Value, current: &str) -> Value {
    let found = releases
        .as_array()
        .into_iter()
        .flatten()
        .filter(|release| !release["draft"].as_bool().unwrap_or(false))
        .filter_map(|release| {
            let tag = release["tag_name"].as_str()?;
            Some((version(tag)?, tag, release["html_url"].as_str().unwrap_or_default()))
        })
        .max_by_key(|(version, _, _)| *version);
    let Some((latest, tag, url)) = found else {
        return json!({"current": current, "newer": false});
    };
    json!({
        "current": current,
        "latest": tag.trim_start_matches('v'),
        "newer": version(current).is_some_and(|current| latest > current),
        "url": url,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn release(tag: &str, draft: bool) -> Value {
        json!({"tag_name": tag, "draft": draft, "html_url": format!("https://example.com/{tag}")})
    }

    #[test]
    fn the_newest_published_release_is_found_and_compared() {
        let releases = json!([
            release("v0.2.0", false),
            release("v0.10.0", false),
            release("v0.11.0", true),
            release("nightly", false),
        ]);
        // 0.10 is newer than 0.2 though it sorts before it as text; a draft is not out.
        assert_eq!(
            newest(&releases, "0.2.0"),
            json!({"current": "0.2.0", "latest": "0.10.0", "newer": true, "url": "https://example.com/v0.10.0"})
        );
        assert_eq!(newest(&releases, "0.10.0")["newer"], json!(false));
        assert_eq!(newest(&releases, "1.0.0")["newer"], json!(false));
        assert_eq!(newest(&json!([]), "0.1.0"), json!({"current": "0.1.0", "newer": false}));
    }

    #[test]
    fn versions_read_as_numbers() {
        assert_eq!(version("v1.2.3"), Some((1, 2, 3)));
        assert_eq!(version("0.1"), Some((0, 1, 0)));
        assert_eq!(version("v2.0.0-rc.1"), Some((2, 0, 0)));
        assert_eq!(version("1.2.3.4"), None);
        assert_eq!(version("nightly"), None);
    }

    /// GitHub itself. It asks the network, so it is run by hand:
    /// `cargo test -p kerf-qt-bridge --lib real_releases -- --ignored --nocapture`.
    #[test]
    #[ignore = "asks api.github.com"]
    fn github_answers_with_the_real_releases() {
        let answer = check();
        println!("{answer}");
        assert!(answer["error"].is_null(), "{answer}");
        assert_eq!(answer["current"], json!(env!("CARGO_PKG_VERSION")));
    }
}

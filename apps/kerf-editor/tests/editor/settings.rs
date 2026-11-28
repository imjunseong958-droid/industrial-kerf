//! The Settings window: its pages, providers and keys as it shows them.

use kerf_bridge::event::{CloudAccess, CloudAccount, CloudStatus, ListedModel};
use kerf_editor::settings::{
    account_status, cursor_imports, generation_opener, mcp_server_config, model_summary,
    read_failure, releases_url, validate_mcp_config, McpClient,
};
use proptest::prelude::*;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};

// ---------------------------------------------------------------- Settings ▸ Generation

#[test]
fn the_generation_file_opens_itself_and_a_missing_one_its_folder() {
    let file = Path::new("/home/me/.kerf/generation.json");
    assert_eq!(
        generation_opener(file, true),
        ("Open generation.json", file.to_path_buf())
    );
    assert_eq!(
        generation_opener(file, false),
        ("Open Its Folder", PathBuf::from("/home/me/.kerf"))
    );
}

proptest! {
    #[test]
    fn what_the_generation_button_opens_is_the_file_or_the_folder_it_goes_in(
        parts in proptest::collection::vec("[a-z0-9_]{1,8}", 1..5),
        exists in proptest::bool::ANY,
    ) {
        let folder: PathBuf = std::iter::once("/".to_owned()).chain(parts).collect();
        let file = folder.join("generation.json");
        let (words, opened) = generation_opener(&file, exists);
        if exists {
            prop_assert_eq!(words, "Open generation.json");
            prop_assert_eq!(opened, file);
        } else {
            prop_assert_eq!(words, "Open Its Folder");
            prop_assert_eq!(opened.join("generation.json"), file);
        }
    }
}

// ---------------------------------------------------------------- Settings ▸ MCP

#[test]
fn both_cursor_files_are_offered_whether_there_or_not() {
    let scratch = tempdir("cursor-both");
    let project = scratch.join("film");
    let home = scratch.join("home");
    // The project's file is there and the home one is not: both are still
    // offered, so the home one can be loaded while the project has one.
    std::fs::create_dir_all(project.join(".cursor")).unwrap();
    std::fs::write(project.join(".cursor/mcp.json"), "{\"mcpServers\": {}}").unwrap();
    let imports = cursor_imports(&project, Some(&home));
    assert_eq!(
        imports,
        vec![
            (
                "Load from Project's Cursor",
                project.join(".cursor/mcp.json")
            ),
            ("Load from Home Cursor", home.join(".cursor/mcp.json")),
        ]
    );
    // Neither there: the same two.
    let bare = scratch.join("bare");
    assert_eq!(cursor_imports(&bare, Some(&home)).len(), 2);
    // No home folder known: only the project's.
    assert_eq!(
        cursor_imports(&project, None),
        vec![(
            "Load from Project's Cursor",
            project.join(".cursor/mcp.json")
        )]
    );
    let _ = std::fs::remove_dir_all(&scratch);
}

#[test]
fn a_cursor_file_that_does_not_read_says_where_and_why() {
    let scratch = tempdir("cursor-unread");
    let missing = scratch.join(".cursor/mcp.json");
    let error = std::fs::read_to_string(&missing).unwrap_err();
    let said = read_failure("~/.cursor/mcp.json", &error);
    assert!(
        said.starts_with("Could not read ~/.cursor/mcp.json: "),
        "{said}"
    );
    // The system's words, without its number, which tells the person nothing.
    assert!(!said.contains("os error"), "{said}");
    assert!(said.len() > "Could not read ~/.cursor/mcp.json: ".len());
    // A folder where the file should be does not read either.
    std::fs::create_dir_all(&missing).unwrap();
    let error = std::fs::read_to_string(&missing).unwrap_err();
    let said = read_failure("x", &error);
    assert!(said.starts_with("Could not read x: "), "{said}");
    assert!(!said.contains("os error"), "{said}");
    let _ = std::fs::remove_dir_all(&scratch);
}

#[test]
fn an_error_without_a_number_is_said_whole() {
    let error = std::io::Error::other("permission withheld");
    assert_eq!(
        read_failure("p", &error),
        "Could not read p: permission withheld"
    );
}

// ---------------------------------------------------------------- About

proptest! {
    #[test]
    fn the_releases_page_is_the_repositorys_own(
        owner in "[a-z0-9-]{1,12}", name in "[a-z0-9-]{1,12}", slashes in 0usize..3,
    ) {
        let repository = format!("https://github.com/{owner}/{name}{}", "/".repeat(slashes));
        let url = releases_url(&repository);
        prop_assert_eq!(url, format!("https://github.com/{owner}/{name}/releases"));
    }
}

/// A folder of its own under the system's temporary folder.
fn tempdir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "kerf-dialogs-{name}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|time| time.as_nanos())
            .unwrap_or_default()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// What a `cloud` event says: the access, the account and the message.
fn cloud(
    access: CloudAccess,
    account: Option<(&str, &str, Option<u64>)>,
    message: Option<&str>,
) -> CloudStatus {
    CloudStatus {
        access,
        account: account.map(|(email, plan, credits)| CloudAccount {
            email: email.into(),
            plan: plan.into(),
            ai: true,
            credits,
        }),
        message: message.map(str::to_owned),
    }
}

#[test]
fn the_account_says_where_it_stands_and_what_can_be_done() {
    let unheard = account_status(None);
    assert!(unheard.words.starts_with("Not available yet"));
    assert!(!unheard.sign_in && !unheard.sign_out);
    let signed_out = account_status(Some(&cloud(CloudAccess::SignedOut, None, None)));
    assert_eq!(
        signed_out.words,
        "Signed out. Paste your Kerf sign-in token to sign in"
    );
    assert!(signed_out.sign_in && !signed_out.sign_out);
    let ready = account_status(Some(&cloud(
        CloudAccess::Ready,
        Some(("ed@kerf.dev", "Pro", Some(41))),
        None,
    )));
    assert_eq!(
        ready.words,
        "Signed in as ed@kerf.dev · Pro plan · 41 requests left"
    );
    assert!(ready.sign_out && !ready.sign_in);
    let spent = account_status(Some(&cloud(
        CloudAccess::OutOfCredits,
        Some(("ed@kerf.dev", "Free", Some(0))),
        None,
    )));
    assert!(spent
        .words
        .ends_with("\nYou've used this period's requests"));
    let away = account_status(Some(&cloud(
        CloudAccess::Unreachable,
        None,
        Some("timed out"),
    )));
    assert_eq!(away.words, "Kerf can't be reached right now\ntimed out");
    assert!(away.sign_in);
}

#[test]
fn mcp_servers_are_checked_as_kerf_chat_needs_them() {
    assert_eq!(validate_mcp_config("{\"mcpServers\": {}}"), Ok(()));
    let good = r#"{"mcpServers": {"fs": {"command": "npx", "args": ["-y", "fs"],
        "env": {"ROOT": "/"}, "tools": null}}}"#;
    assert_eq!(validate_mcp_config(good), Ok(()));
    let said = |text: &str| validate_mcp_config(text).unwrap_err();
    assert!(said("{").starts_with("Invalid JSON"));
    assert_eq!(said("[]"), "The JSON root must be an object.");
    assert_eq!(said("{}"), "mcpServers must be an object.");
    assert_eq!(
        said(r#"{"mcpServers": {"a": 1}}"#),
        "Each server must be an object."
    );
    assert!(said(r#"{"mcpServers": {"a": {"url": "https://x"}}}"#)
        .starts_with("Each server needs a nonempty local command string."));
    assert_eq!(
        said(r#"{"mcpServers": {"a": {"command": "x", "args": "-y"}}}"#),
        "args must be an array of strings."
    );
    assert_eq!(
        said(r#"{"mcpServers": {"a": {"command": "x", "tools": [1]}}}"#),
        "tools must contain only strings."
    );
    assert_eq!(
        said(r#"{"mcpServers": {"a": {"command": "x", "env": {"A": 1}}}}"#),
        "env must contain only string values."
    );
}

#[test]
fn kerfs_own_mcp_config_is_ready_to_paste_in_each_client() {
    let args = vec!["--dir".to_owned(), "/films/Atlas \"Cut\"".to_owned()];
    for client in [McpClient::Cursor, McpClient::Claude] {
        let config: Value =
            serde_json::from_str(&mcp_server_config(client, "/bin/kerf-mcp", &args)).unwrap();
        assert_eq!(config["mcpServers"]["kerf"]["command"], "/bin/kerf-mcp");
        assert_eq!(config["mcpServers"]["kerf"]["args"], json!(args));
        assert_eq!(
            validate_mcp_config(&config.to_string()),
            Ok(()),
            "Kerf chat could take it too"
        );
    }
    let codex = mcp_server_config(McpClient::Codex, "/bin/kerf-mcp", &args);
    assert_eq!(
        codex,
        "[mcp_servers.kerf]\ncommand = \"/bin/kerf-mcp\"\nargs = [\"--dir\", \"/films/Atlas \\\"Cut\\\"\"]"
    );
}

#[test]
fn a_model_is_said_in_a_line() {
    let model = ListedModel {
        name: "wan".into(),
        size: Some("832x480".into()),
        rate: Some(16.0),
        seconds: Some(5.0),
        reads: Some(vec!["guide".into(), "ref".into()]),
        ..ListedModel::default()
    };
    assert_eq!(
        model_summary(&model),
        "832x480 · 16 fps · up to 5 s · reads guide, ref"
    );
    let rate = ListedModel {
        rate: Some(23.976),
        ..ListedModel::default()
    };
    assert_eq!(model_summary(&rate), "23.976 fps");
    let bare = ListedModel {
        name: "x".into(),
        ..ListedModel::default()
    };
    assert_eq!(model_summary(&bare), "says nothing of what it takes");
}

//! Settings kept across versions and windows, and what Settings and the
//! launch say: Kerf's account, the MCP servers both ways, the shot models,
//! a newer release, a crash, a link that cannot be followed.

use kerf_editor::app::{component_name, crash_notice_text};
use kerf_editor::links::{is_link, openable, parse};
use kerf_editor::palette::unavailable;
use kerf_editor::prefs::{merge, migrate, ApiMode, Prefs, Provider, UPDATE_EVERY_MS, VERSION};
use kerf_editor::settings::{
    account_status, mcp_server_config, model_summary, validate_mcp_config, McpClient,
};
use proptest::prelude::*;
use serde_json::{json, Map, Value};
use std::path::{Path, PathBuf};

fn fields(value: Value) -> Map<String, Value> {
    match value {
        Value::Object(fields) => fields,
        _ => panic!("not an object"),
    }
}

fn read(path: &Path) -> Value {
    serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
}

#[test]
fn an_old_file_is_migrated_and_stamped_and_keeps_what_it_had() {
    let folder = tempfile::tempdir().unwrap();
    let path = folder.path().join("editor.json");
    std::fs::write(
        &path,
        r#"{"provider": "openai", "openaiBaseUrl": "https://gw/v1", "sidebarWidth": 300}"#,
    )
    .unwrap();
    let prefs = Prefs::load_from(&path);
    assert_eq!(prefs.version, VERSION);
    assert_eq!(prefs.provider, Provider::Openai);
    assert_eq!(prefs.openai_base_url, "https://gw/v1");
    let written = read(&path);
    assert_eq!(
        written["version"],
        json!(VERSION),
        "migrated as it was read"
    );
    assert_eq!(written["sidebarWidth"], json!(300), "as it was written");
    let mut stamped = fields(written);
    assert!(
        !migrate(&mut stamped),
        "a migrated file is not migrated again"
    );
}

#[test]
fn a_newer_kerfs_file_is_left_as_it_is_but_for_what_this_one_changed() {
    let folder = tempfile::tempdir().unwrap();
    let path = folder.path().join("editor.json");
    let newer = json!({
        "version": VERSION + 1,
        "provider": "a-provider-from-later",
        "face": "a-face-from-later",
        "somethingNew": {"kept": true},
        "keymap": "premiere",
        "sidebarWidth": 310,
    });
    std::fs::write(&path, newer.to_string()).unwrap();
    let mut prefs = Prefs::load_from(&path);
    // What this Kerf cannot read is its default, alone.
    assert_eq!(prefs.version, VERSION + 1);
    assert_eq!(prefs.keymap, kerf_editor::prefs::Keymap::Premiere);
    assert_eq!(prefs.sidebar_width, 310.0);
    assert_eq!(read(&path), newer, "reading never rewrites a newer file");
    prefs.sidebar_width = 333.0;
    prefs.save_to(&path).unwrap();
    let saved = read(&path);
    assert_eq!(saved["sidebarWidth"], json!(333.0));
    for kept in ["version", "provider", "face", "somethingNew", "keymap"] {
        assert_eq!(
            saved[kept], newer[kept],
            "{kept} kept as the newer Kerf wrote it"
        );
    }
}

#[test]
fn one_bad_value_resets_only_itself() {
    let read = Prefs::from_fields(&fields(json!({
        "provider": 7,
        "permissions": "auto",
        "openaiApiMode": "responses",
        "recent": ["/films/a"],
        "checkUpdates": "yes",
    })));
    assert_eq!(read.permissions, kerf_editor::prefs::Permissions::Auto);
    assert_eq!(read.openai_api_mode, ApiMode::Responses);
    assert_eq!(read.recent, [PathBuf::from("/films/a")]);
    assert!(read.check_updates, "a bad value is its default");
}

#[test]
fn a_save_in_one_window_never_undoes_anothers() {
    let folder = tempfile::tempdir().unwrap();
    let path = folder.path().join("editor.json");
    Prefs::default().save_to(&path).unwrap();
    let mut first = Prefs::load_from(&path);
    let mut second = Prefs::load_from(&path);
    first.provider = Provider::Cloud;
    first.opened(Path::new("/films/first"));
    first
        .kept_keys
        .insert("agent/openai/apiKey".into(), "sk-1".into());
    first.save_to(&path).unwrap();
    // The second window still holds what it read, and saves its own change.
    second.sidebar_width = 400.0;
    second.opened(Path::new("/films/second"));
    second
        .kept_keys
        .insert("agent/anthropic/apiKey".into(), "sk-ant".into());
    second.save_to(&path).unwrap();
    let both = Prefs::load_from(&path);
    assert_eq!(
        both.provider,
        Provider::Cloud,
        "the first window's choice stays"
    );
    assert_eq!(both.sidebar_width, 400.0);
    assert!(both.recent.contains(&PathBuf::from("/films/first")));
    assert_eq!(both.recent[0], PathBuf::from("/films/second"));
    assert_eq!(both.kept_keys.len(), 2, "each window's key is kept");
    // And the second window takes up the first's choice, keeping its own
    // that it has not saved.
    second.face = kerf_editor::theme::Face::Inter;
    second.refresh_from(&path);
    assert_eq!(second.provider, Provider::Cloud);
    assert_eq!(second.face, kerf_editor::theme::Face::Inter);
    second.save_to(&path).unwrap();
    assert_eq!(
        Prefs::load_from(&path).face,
        kerf_editor::theme::Face::Inter
    );
}

#[test]
fn a_copy_read_before_there_was_a_file_does_not_undo_a_later_save() {
    let folder = tempfile::tempdir().unwrap();
    let path = folder.path().join("editor.json");
    // The launcher reads no file; then the crash notice says so, and saves.
    let mut launcher = Prefs::load_from(&path);
    let mut notice = Prefs::load_from(&path);
    notice.crashes_seen_ms = 42;
    notice.save_to(&path).unwrap();
    launcher.provider = Provider::Openai;
    launcher.save_to(&path).unwrap();
    let read = Prefs::load_from(&path);
    assert_eq!(read.crashes_seen_ms, 42);
    assert_eq!(read.provider, Provider::Openai);
    assert_eq!(read.version, VERSION);
}

#[test]
fn a_removed_key_or_project_stays_removed() {
    let folder = tempfile::tempdir().unwrap();
    let path = folder.path().join("editor.json");
    let mut prefs = Prefs::default();
    prefs.kept_keys.insert("a".into(), "1".into());
    prefs.opened(Path::new("/films/gone"));
    prefs.save_to(&path).unwrap();
    let mut again = Prefs::load_from(&path);
    again.kept_keys.clear();
    again.recent.clear();
    again.save_to(&path).unwrap();
    let read = Prefs::load_from(&path);
    assert!(read.kept_keys.is_empty());
    assert!(read.recent.is_empty());
}

fn small_value() -> impl Strategy<Value = Value> {
    prop_oneof![
        any::<bool>().prop_map(Value::from),
        (0u32..5).prop_map(Value::from),
        "[a-c]{0,2}".prop_map(Value::from),
    ]
}

fn small_fields() -> impl Strategy<Value = Map<String, Value>> {
    proptest::collection::btree_map("[a-e]", small_value(), 0..5)
        .prop_map(|map| map.into_iter().collect())
}

proptest! {
    /// What this copy changed is written; what it did not change is the
    /// file's, whoever wrote it; saving twice is saving once.
    #[test]
    fn a_save_writes_only_what_this_copy_changed(
        on_disk in small_fields(),
        known in small_fields(),
        mine in small_fields(),
    ) {
        let merged = merge(on_disk.clone(), Some(&known), &mine);
        for (name, value) in &mine {
            if known.get(name) != Some(value) {
                prop_assert_eq!(merged.get(name), Some(value));
            } else {
                prop_assert_eq!(merged.get(name), on_disk.get(name));
            }
        }
        for (name, value) in &on_disk {
            if !mine.contains_key(name) {
                prop_assert_eq!(merged.get(name), Some(value));
            }
        }
        prop_assert_eq!(merge(merged.clone(), Some(&mine), &mine), merged.clone());
        // Knowing nothing of the file, every field of this copy is written.
        let whole = merge(on_disk, None, &mine);
        for (name, value) in &mine {
            prop_assert_eq!(whole.get(name), Some(value));
        }
    }

    /// Prefs written and read again are the same prefs.
    #[test]
    fn prefs_round_trip_through_their_fields(width in 200.0f32..800.0, version in 0u64..1000) {
        let prefs = Prefs {
            sidebar_width: width.round(),
            mcp_config_version: version,
            provider: Provider::ClaudeCode,
            claude_code_model: "opus".into(),
            ..Prefs::default()
        };
        let written = fields(serde_json::to_value(&prefs).unwrap());
        prop_assert_eq!(Prefs::from_fields(&written), prefs);
    }
}

#[test]
fn update_notices_ask_at_most_every_20_hours_and_never_offer_a_skipped_release() {
    let mut prefs = Prefs {
        update_checked_ms: 1_000,
        ..Prefs::default()
    };
    assert!(!prefs.update_due(1_000 + UPDATE_EVERY_MS - 1));
    assert!(prefs.update_due(1_000 + UPDATE_EVERY_MS));
    prefs.check_updates = false;
    assert!(!prefs.update_due(u64::MAX), "off, it never asks");
    let newer =
        json!({"current": "0.1.4", "latest": "0.2.0", "newer": true, "url": "https://r/0.2.0"});
    assert_eq!(
        prefs.update_offer(&newer),
        Some(("0.2.0".to_owned(), "https://r/0.2.0".to_owned()))
    );
    prefs.update_skipped = "0.2.0".into();
    assert_eq!(prefs.update_offer(&newer), None);
    assert_eq!(
        prefs.update_offer(&json!({"current": "0.2.0", "newer": false})),
        None
    );
    assert_eq!(prefs.update_offer(&json!({"error": "offline"})), None);
}

#[test]
fn a_crash_is_told_of_once() {
    let listed = json!({"folder": "/c", "reports": [
        {"component": "kerfd", "atMs": 3000.0, "message": "index out of bounds\nat …"},
        {"component": "editor", "atMs": 2000.0, "message": ""},
        {"component": "kerf", "atMs": 1000.0, "message": ""},
    ]});
    let mut prefs = Prefs {
        crashes_seen_ms: 1000,
        ..Prefs::default()
    };
    let unseen = prefs.unseen_crashes(&listed);
    assert_eq!(unseen.len(), 2);
    let text = crash_notice_text(&unseen);
    assert!(text.starts_with(
        "Kerf stopped unexpectedly 2 times since it last said so. The latest was in Kerf's project service (kerfd)."
    ));
    assert!(text.contains("“index out of bounds”"), "{text}");
    assert!(text.contains("crash folder, on this computer only"));
    let one = crash_notice_text(&unseen[..1]);
    assert!(one.starts_with(
        "Kerf stopped unexpectedly: Kerf's project service (kerfd) hit an error it could not recover from."
    ));
    prefs.saw_crashes(&unseen);
    assert_eq!(prefs.crashes_seen_ms, 3000);
    assert!(prefs.unseen_crashes(&listed).is_empty());
    assert_eq!(component_name("kerf"), "the kerf command");
    assert_eq!(component_name("editor"), "the editor");
}

#[test]
fn the_account_says_where_it_stands_and_what_can_be_done() {
    let unheard = account_status(None);
    assert!(unheard.words.starts_with("Not available yet"));
    assert!(!unheard.sign_in && !unheard.sign_out);
    let signed_out = account_status(Some(&json!({"access": "signedOut"})));
    assert_eq!(
        signed_out.words,
        "Signed out. Paste your Kerf sign-in token to sign in"
    );
    assert!(signed_out.sign_in && !signed_out.sign_out);
    let ready = account_status(Some(&json!({"access": "ready",
        "account": {"email": "ed@kerf.dev", "plan": "Pro", "ai": true, "credits": 41}})));
    assert_eq!(
        ready.words,
        "Signed in as ed@kerf.dev · Pro plan · 41 requests left"
    );
    assert!(ready.sign_out && !ready.sign_in);
    let spent = account_status(Some(&json!({"access": "outOfCredits",
        "account": {"email": "ed@kerf.dev", "plan": "Free", "ai": true, "credits": 0}})));
    assert!(spent
        .words
        .ends_with("\nYou've used this period's requests"));
    let away = account_status(Some(
        &json!({"access": "unreachable", "message": "timed out"}),
    ));
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
    let model = json!({"name": "wan", "size": "832x480", "rate": 16.0, "seconds": 5.0,
        "reads": ["guide", "ref"]});
    assert_eq!(
        model_summary(&model),
        "832x480 · 16 fps · up to 5 s · reads guide, ref"
    );
    assert_eq!(model_summary(&json!({"rate": 23.976})), "23.976 fps");
    assert_eq!(
        model_summary(&json!({"name": "x"})),
        "says nothing of what it takes"
    );
}

#[test]
fn a_link_with_a_fragment_still_reads_and_a_gone_project_is_said() {
    let link = parse("kerf://atlas-cut/rev3@00:00:01:00#top").unwrap();
    assert_eq!(link.project, "atlas-cut");
    assert_eq!(link.timecode.as_deref(), Some("00:00:01:00"));
    let folder = tempfile::tempdir().unwrap();
    let project = folder.path().join("Atlas Cut");
    std::fs::create_dir_all(&project).unwrap();
    let said = openable(&project).unwrap_err();
    assert!(
        said.starts_with("The project “Atlas Cut” is no longer at "),
        "{said}"
    );
    std::fs::write(project.join("project.kerf"), "{}").unwrap();
    assert_eq!(openable(&project), Ok(()));
}

#[test]
fn a_command_that_cannot_run_says_why() {
    let states = json!({"edit.undo": "Nothing to undo", "view.sidebar": null});
    assert_eq!(unavailable(&states, "edit.undo"), Some("Nothing to undo"));
    assert_eq!(unavailable(&states, "view.sidebar"), None);
    assert_eq!(unavailable(&states, "not.listed"), None);
    // The registry's own say, with no project connected.
    let states = kerf_bridge::commands_states(&json!({"connected": false}));
    assert!(unavailable(&states, "edit.undo").is_some());
    assert_eq!(unavailable(&states, "app.preferences"), None);
}

#[test]
fn a_folder_named_in_any_script_is_not_a_link_and_does_not_stop_kerf() {
    // Byte 7 falls inside a character in each.
    for argument in [
        "my-영상",
        "프로젝트",
        "kerf:/é",
        "./Ünïcödé",
        "日本語のフォルダ",
    ] {
        assert!(!is_link(argument), "{argument}");
        assert!(parse(argument).is_err(), "{argument}");
    }
    assert!(is_link("KERF://프로젝트/rev2"));
    assert_eq!(parse("kerf://프로젝트/rev2").unwrap().project, "프로젝트");
}

proptest! {
    /// Whatever comes on the command line or from the system is asked
    /// about: a link is one that starts kerf://, in any case.
    #[test]
    fn any_text_is_a_link_or_not_without_failing(
        head in "(?i:kerf://)?",
        rest in any::<String>(),
    ) {
        let text = format!("{head}{rest}");
        prop_assert_eq!(is_link(&text), text.to_ascii_lowercase().starts_with("kerf://"));
        if !is_link(&text) {
            prop_assert!(parse(&text).is_err());
        }
    }
}

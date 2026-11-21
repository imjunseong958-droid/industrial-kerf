//! Platform and services: links outside ASCII and the link inbox, the Qt
//! editor's settings taken up, the faces and who answers by default, the
//! master output kept per project, the startup options' pieces, and the
//! Collab chip's and Export's words and commands.

use kerf_editor::app::{parse_window_size, path_with_tool_folders};
use kerf_editor::export::{
    export_into, master_goes, master_makes, master_request, request, Quality,
};
use kerf_editor::finish::{
    deliver_said, preflight_hidden, preflight_rows, publish_command, publish_name,
    pull_request_command, pull_request_said, pull_request_title, sync_lines, sync_said, PublishWay,
};
use kerf_editor::links::{
    hand_off, inbox_address, inbox_links, inbox_message, listen, make, parse, resolve, slug,
    InboxAddress,
};
use kerf_editor::prefs::{
    default_provider, qt_master_key, qt_settings_ini, MasterOutput, Permissions, Prefs, Provider,
};
use proptest::prelude::*;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::time::Duration;

// ---------------------------------------------------------------- links

#[test]
fn a_browsers_punycode_link_finds_a_project_named_outside_ascii() {
    let folder = PathBuf::from("/films/가편집 컷");
    assert_eq!(slug("가편집 컷"), "가편집-컷");
    // A browser writes the host in its ASCII form.
    let link = parse("kerf://xn----2x5es30fxse1xg/rev3@00:00:01:00").unwrap();
    assert_eq!(link.project, "가편집-컷");
    assert_eq!(link.revision, Some(3));
    assert_eq!(
        resolve(&link, std::slice::from_ref(&folder)).unwrap(),
        folder
    );
    // Upper case ASCII form, as some write it, too.
    let link = parse("kerf://XN----2X5ES30FXSE1XG/rev3").unwrap();
    assert_eq!(
        resolve(&link, std::slice::from_ref(&folder)).unwrap(),
        folder
    );
}

#[test]
fn a_name_in_nfd_finds_its_nfc_folder() {
    // "Café Été" written decomposed (as macOS file names may be) and composed.
    let decomposed = "Cafe\u{301} E\u{301}te\u{301}";
    let composed = "Caf\u{e9} \u{c9}t\u{e9}";
    assert_eq!(slug(decomposed), slug(composed));
    assert_eq!(slug(composed), "caf\u{e9}-\u{e9}t\u{e9}");
    let folder = PathBuf::from(format!("/films/{decomposed}"));
    let link = parse(&format!("kerf://{}/rev1", slug(composed))).unwrap();
    assert_eq!(
        resolve(&link, std::slice::from_ref(&folder)).unwrap(),
        folder
    );
}

#[test]
fn a_long_name_makes_a_slug_short_enough_for_one_label() {
    let long = "a".repeat(80);
    assert_eq!(slug(&long).len(), 63);
    let hangul = "편집".repeat(40);
    let made = slug(&hangul);
    assert!(!made.is_empty());
    assert!(idna::domain_to_ascii(&made).unwrap().len() <= 63);
    // The link to such a folder still finds it.
    let folder = PathBuf::from(format!("/films/{hangul}"));
    let link = parse(&make(&folder, 2, None, 25.0).unwrap()).unwrap();
    assert_eq!(
        resolve(&link, std::slice::from_ref(&folder)).unwrap(),
        folder
    );
}

proptest! {
    #[test]
    fn a_slug_is_its_own_slug(name in "\\PC{0,40}") {
        let once = slug(&name);
        prop_assert_eq!(slug(&once), once.clone());
        prop_assert!(!once.starts_with('-') && !once.ends_with('-') && !once.contains("--"));
    }

    #[test]
    fn a_made_link_reads_back_to_its_folder(name in "[\\p{L}\\p{N} _.-]{1,30}", revision in 1u64..10_000) {
        let folder = PathBuf::from("/films").join(&name);
        if let Some(link) = make(&folder, revision, Some(12.5), 25.0) {
            let read = parse(&link).unwrap();
            prop_assert_eq!(read.revision, Some(revision));
            prop_assert_eq!(resolve(&read, std::slice::from_ref(&folder)).unwrap(), folder);
        }
    }

    #[test]
    fn the_inbox_reads_back_the_links_it_was_given(
        links in prop::collection::vec("(kerf://[a-z0-9-]{1,12}/rev[0-9]{1,4}|https://[a-z]{1,8}|[ a-z]{0,6})", 0..6)
    ) {
        let wanted: Vec<String> = links
            .iter()
            .map(|link| link.trim().to_owned())
            .filter(|link| kerf_editor::links::is_link(link))
            .collect();
        match inbox_message(&links) {
            None => prop_assert!(wanted.is_empty()),
            Some(message) => {
                prop_assert_eq!(inbox_links(&message), Some(wanted));
                // Until its empty line it is not whole.
                prop_assert_eq!(inbox_links(&message[..message.len() - 1]), None);
            }
        }
    }
}

fn received(rx: &mpsc::Receiver<Vec<String>>) -> Vec<String> {
    rx.recv_timeout(Duration::from_secs(3)).unwrap_or_default()
}

#[cfg(unix)]
#[test]
fn an_editor_started_for_a_link_hands_it_to_the_one_listening() {
    let runtime = tempfile::tempdir().unwrap();
    let address = inbox_address("abc123", Some(runtime.path()), None).unwrap();
    assert!(matches!(&address, InboxAddress::Socket(path) if path.starts_with(runtime.path())));
    let links = vec!["kerf://atlas/rev3@00:00:02:00".to_owned()];
    // Nobody listens yet: the link is not taken.
    assert!(!hand_off(&address, &links, Duration::from_millis(300)));
    let (tx, rx) = mpsc::channel();
    let inbox = listen(&address, move |links| {
        let _ = tx.send(links);
    })
    .expect("listens");
    // A second editor finds the first one there and does not listen.
    assert!(listen(&address, |_| {}).is_none());
    let mut given = links.clone();
    given.push("not a link".into());
    assert!(hand_off(&address, &given, Duration::from_secs(2)));
    assert_eq!(received(&rx), links);
    // Gone with the editor, and taken over by the next.
    drop(inbox);
    assert!(!hand_off(&address, &links, Duration::from_millis(300)));
}

#[cfg(unix)]
#[test]
fn a_socket_an_editor_that_crashed_left_is_taken_over() {
    let runtime = tempfile::tempdir().unwrap();
    let address = inbox_address("dead", Some(runtime.path()), None).unwrap();
    let InboxAddress::Socket(path) = &address else {
        panic!("a socket");
    };
    // A socket nobody answers on.
    drop(std::os::unix::net::UnixListener::bind(path).unwrap());
    assert!(path.exists());
    let (tx, rx) = mpsc::channel();
    let _inbox = listen(&address, move |links| {
        let _ = tx.send(links);
    })
    .expect("takes it over");
    assert!(hand_off(
        &address,
        &["kerf://b/rev1".into()],
        Duration::from_secs(2)
    ));
    assert_eq!(received(&rx), ["kerf://b/rev1"]);
}

#[test]
fn a_port_inbox_takes_only_who_knows_its_token() {
    let settings = tempfile::tempdir().unwrap();
    let address = inbox_address("p0rt", None, Some(settings.path())).unwrap();
    let InboxAddress::Port(file) = &address else {
        panic!("a port");
    };
    let (tx, rx) = mpsc::channel();
    let _inbox = listen(&address, move |links| {
        let _ = tx.send(links);
    })
    .expect("listens");
    assert!(listen(&address, |_| {}).is_none(), "one inbox at a time");
    assert!(hand_off(
        &address,
        &["kerf://c/rev2".into()],
        Duration::from_secs(2)
    ));
    assert_eq!(received(&rx), ["kerf://c/rev2"]);
    // The wrong token is turned away.
    let text = std::fs::read_to_string(file).unwrap();
    let (port, _) = text.trim().split_once(' ').unwrap();
    std::fs::write(file, format!("{port} wrong\n")).unwrap();
    assert!(!hand_off(
        &address,
        &["kerf://c/rev2".into()],
        Duration::from_millis(500)
    ));
}

#[test]
fn an_inbox_needs_an_id_and_a_private_place() {
    let place = tempfile::tempdir().unwrap();
    assert_eq!(inbox_address("", Some(place.path()), None), None);
    assert_eq!(inbox_address("../x", Some(place.path()), None), None);
    assert_eq!(inbox_address("abc", None, None), None);
}

// ---------------------------------------------------------------- startup

#[test]
fn window_sizes_read_as_the_qt_editor_took_them() {
    assert_eq!(parse_window_size("1600x1000"), Some((1600.0, 1000.0)));
    assert_eq!(parse_window_size(" 800X600 "), Some((800.0, 600.0)));
    for bad in ["", "1600", "x1000", "0x10", "ax b", "1600x1000x2", "-5x5"] {
        assert_eq!(parse_window_size(bad), None, "{bad}");
    }
}

#[test]
fn macos_path_gains_homebrews_and_local_tools_once() {
    let home = Path::new("/Users/jun");
    assert_eq!(
        path_with_tool_folders("/usr/bin:/bin", home),
        "/usr/bin:/bin:/opt/homebrew/bin:/opt/homebrew/sbin:/usr/local/bin:/Users/jun/.local/bin"
    );
    let full = path_with_tool_folders("/usr/local/bin::/opt/homebrew/bin", home);
    assert_eq!(
        full,
        "/usr/local/bin:/opt/homebrew/bin:/opt/homebrew/sbin:/Users/jun/.local/bin"
    );
    assert_eq!(path_with_tool_folders(&full, home), full, "nothing twice");
}

// ---------------------------------------------------------------- who answers, faces

#[test]
fn with_no_key_kept_kerfs_hosted_model_answers() {
    assert_eq!(default_provider(None, false, false), Provider::Cloud);
    assert_eq!(default_provider(None, true, false), Provider::Anthropic);
    assert_eq!(default_provider(None, true, true), Provider::Anthropic);
    assert_eq!(default_provider(None, false, true), Provider::Openai);
    assert_eq!(
        default_provider(Some(Provider::ClaudeCode), true, true),
        Provider::ClaudeCode
    );
}

#[test]
fn a_key_kept_in_the_file_keeps_its_provider_until_one_is_chosen() {
    if std::env::var_os("KERF_PROVIDER").is_some()
        || std::env::var_os("ANTHROPIC_API_KEY").is_some()
        || std::env::var_os("OPENAI_API_KEY").is_some()
    {
        return;
    }
    let read = Prefs::from_fields(
        json!({"keptKeys": {"agent/openai/apiKey": "sk-1"}})
            .as_object()
            .unwrap(),
    );
    assert_eq!(read.provider, Provider::Openai);
    let none = Prefs::from_fields(json!({}).as_object().unwrap());
    assert_eq!(none.provider, Provider::Cloud);
    let chosen = Prefs::from_fields(json!({"provider": "anthropic"}).as_object().unwrap());
    assert_eq!(chosen.provider, Provider::Anthropic);
}

// ---------------------------------------------------------------- the Qt editor's settings

#[test]
fn the_qt_editors_settings_are_taken_up_once_with_settings_dir() {
    let folder = tempfile::tempdir().unwrap();
    let qt = folder.path().join("Industrial Kerf");
    std::fs::create_dir_all(&qt).unwrap();
    std::fs::write(
        qt.join("Industrial Kerf.ini"),
        "[layout]\nsidebarWidth=333\n[agent]\npermissions=auto\n",
    )
    .unwrap();
    let path = folder.path().join("editor.json");
    std::env::set_var("KERF_EDITOR_PREFS", &path);
    std::env::set_var("KERF_SECRETS", "off");
    let taken = kerf_editor::prefs::import_qt_once(Some(folder.path()));
    assert_eq!(taken, ["agent/permissions", "layout/sidebarWidth"]);
    let prefs = Prefs::load_from(&path);
    assert_eq!(prefs.sidebar_width, 333.0);
    assert_eq!(prefs.permissions, Permissions::Auto);
    // Once: a file of its own is never overwritten.
    std::fs::write(
        qt.join("Industrial Kerf.ini"),
        "[layout]\nsidebarWidth=200\n",
    )
    .unwrap();
    assert!(kerf_editor::prefs::import_qt_once(Some(folder.path())).is_empty());
    assert_eq!(Prefs::load_from(&path).sidebar_width, 333.0);
}

/// A string as QSettings writes it in INI (iniEscapedString), for the
/// round trip below.
fn qt_escaped(text: &str) -> String {
    let needs_quotes = text.contains([',', ';', '='])
        || text.starts_with(char::is_whitespace)
        || text.ends_with(char::is_whitespace)
        || text.starts_with('"');
    let mut out = String::new();
    if needs_quotes {
        out.push('"');
    }
    for ch in text.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            other => out.push(other),
        }
    }
    if needs_quotes {
        out.push('"');
    }
    out
}

proptest! {
    #[test]
    fn an_ini_value_reads_back_as_qsettings_wrote_it(
        text in "[^@\\r\\x00]{0,24}",
        list in prop::collection::vec("[a-zA-Z0-9/ ._,\"]{1,12}", 2..5),
    ) {
        let one = qt_settings_ini(&format!("[a]\nkey={}\n", qt_escaped(&text)));
        prop_assert_eq!(&one["a/key"], &Value::String(text.clone()));
        let written: Vec<String> = list.iter().map(|item| qt_escaped(item)).collect();
        let many = qt_settings_ini(&format!("[a]\nkey={}\n", written.join(", ")));
        prop_assert_eq!(&many["a/key"], &json!(list));
    }
}

// ---------------------------------------------------------------- the master output

#[test]
fn the_master_goes_to_deliveries_unless_the_project_keeps_another_place() {
    let project = Path::new("/films/atlas");
    assert_eq!(
        master_request(Some(project), None),
        json!({"type": "export", "quality": "master", "output": "deliveries/", "name": "atlas_master"})
    );
    let kept = MasterOutput {
        folder: Some("/Volumes/out/".into()),
        name: Some("  Atlas Final ".into()),
        codec: Some("hevc".into()),
    };
    assert_eq!(
        master_request(Some(project), Some(&kept)),
        json!({"type": "export", "quality": "master", "codec": "hevc", "output": "/Volumes/out/", "name": "Atlas Final"})
    );
    assert_eq!(
        master_goes(Some(project), Some(&kept)).0,
        "/Volumes/out/Atlas Final.mp4"
    );
    // ProRes is a master's own codec and goes without saying; an empty
    // folder is where kerfd puts renders.
    let own = MasterOutput {
        folder: Some(String::new()),
        name: None,
        codec: Some("prores_hq".into()),
    };
    assert_eq!(
        master_request(Some(project), Some(&own)),
        json!({"type": "export", "quality": "master", "name": "atlas_master"})
    );
    assert_eq!(master_goes(Some(project), Some(&own)).0, "atlas_master.mov");
    assert_eq!(
        master_request(None, None),
        json!({"type": "export", "quality": "master"})
    );
}

/// A project that keeps its master in `folder` as `name`, in `codec`.
fn keeps(folder: Option<&str>, name: Option<&str>, codec: Option<&str>) -> MasterOutput {
    MasterOutput {
        folder: folder.map(str::to_owned),
        name: name.map(str::to_owned),
        codec: codec.map(str::to_owned),
    }
}

#[test]
fn every_way_of_exporting_the_master_sends_where_the_project_keeps_it() {
    let project = Path::new("/films/harbour");
    let mut prefs = Prefs::default();
    // Nothing kept: deliveries/, as the toolbar sent it before.
    assert_eq!(
        request(Some(project), &prefs, "master"),
        kerf_editor::session::export_request(Some(project), "master")
    );
    prefs.master_outputs.insert(
        kerf_editor::prefs::project_key(project),
        keeps(Some("masters"), Some("Harbour final"), Some("h264")),
    );
    assert_eq!(
        request(Some(project), &prefs, "master"),
        json!({"type": "export", "quality": "master", "codec": "h264", "output": "masters", "name": "Harbour final"})
    );
    // A review copy, a full render and a preview are not the master's.
    for quality in ["review", "full", "preview"] {
        assert_eq!(
            request(Some(project), &prefs, quality),
            kerf_editor::session::export_request(Some(project), quality)
        );
    }
    // Another project keeps nothing.
    assert_eq!(
        request(Some(Path::new("/films/atlas")), &prefs, "master")["output"],
        "deliveries/"
    );
}

#[test]
fn the_master_says_what_it_is_made_as_and_where_it_goes() {
    let project = Some(Path::new("/films/harbour"));
    let hdr = keeps(Some("masters"), None, Some("hevc_hdr"));
    assert_eq!(
        master_goes(project, Some(&hdr)),
        (
            "masters/harbour_master.mp4".to_owned(),
            "HEVC 10-bit HDR in MP4, with its sound".to_owned()
        )
    );
    assert_eq!(master_makes(Some(&hdr)), "HEVC 10-bit HDR");
    assert_eq!(master_makes(None), Quality::Master.makes());
    assert_eq!(
        master_makes(Some(&keeps(None, None, Some("prores_hq")))),
        "ProRes 422 HQ"
    );
    assert_eq!(
        master_makes(Some(&keeps(None, None, Some("h264")))),
        "H.264"
    );
    // A codec kerfd renders no master in is not called ProRes.
    let odd = keeps(None, None, Some("av1"));
    assert_eq!(master_makes(Some(&odd)), "av1");
    let (goes, makes) = master_goes(project, Some(&odd));
    assert_eq!(goes, "deliveries/harbour_master");
    assert!(makes.starts_with("av1"), "{makes}");
    assert!(!makes.contains("ProRes"), "{makes}");
}

#[test]
fn export_says_where_one_press_puts_the_chosen_quality() {
    let project = Some(Path::new("/films/harbour"));
    let kept = keeps(Some("masters"), None, None);
    assert_eq!(
        export_into(Quality::Master, project, None).as_deref(),
        Some("Into deliveries/")
    );
    assert_eq!(
        export_into(Quality::Master, project, Some(&kept)).as_deref(),
        Some("Into masters/")
    );
    assert_eq!(
        export_into(Quality::Draft, project, Some(&kept)).as_deref(),
        Some("Into deliveries/")
    );
    // Where kerfd puts renders has no folder to name.
    let anywhere = keeps(Some(""), None, None);
    assert_eq!(export_into(Quality::Master, project, Some(&anywhere)), None);
    assert_eq!(export_into(Quality::Draft, None, None), None);
}

#[test]
fn deliver_says_where_the_master_and_the_review_copy_go() {
    let project = Some(Path::new("/films/harbour"));
    assert_eq!(
        deliver_said(project, None),
        "The master and a review copy go to deliveries/ in the project, numbered when one is there already."
    );
    assert_eq!(
        deliver_said(project, Some(&keeps(Some("masters"), None, None))),
        "The master goes to masters/ and a review copy to deliveries/ in the project, each numbered when one is there already."
    );
    assert_eq!(
        deliver_said(project, Some(&keeps(Some("/Volumes/out/"), None, None))),
        "The master goes to /Volumes/out/ and a review copy to deliveries/ in the project, each numbered when one is there already."
    );
    assert_eq!(
        deliver_said(project, Some(&keeps(Some(""), None, None))),
        "The master goes where Kerf puts renders and a review copy to deliveries/ in the project, each numbered when one is there already."
    );
}

proptest! {
    #[test]
    fn the_master_request_names_what_the_deliver_row_says(
        project in "[a-zA-Z0-9 _-]{1,20}",
        folder in prop::option::of("[a-z/]{0,12}"),
        name in prop::option::of("[a-zA-Z0-9 ]{0,12}"),
        codec in prop::option::of(prop::sample::select(vec!["prores_hq", "h264", "hevc", "hevc_hdr", ""])),
    ) {
        let path = PathBuf::from("/films").join(project.trim());
        prop_assume!(path.file_name().is_some());
        let kept = MasterOutput {
            folder,
            name,
            codec: codec.map(str::to_owned),
        };
        let command = master_request(Some(&path), Some(&kept));
        let (goes, _) = master_goes(Some(&path), Some(&kept));
        // The file the row names is the request's name, in the codec's container.
        let extension = match command["codec"].as_str() {
            None => "mov",
            Some(_) => "mp4",
        };
        let file = format!("{}.{extension}", command["name"].as_str().unwrap());
        prop_assert!(goes.ends_with(&file), "{} does not end with {}", goes, file);
        match command["output"].as_str() {
            Some(folder) => prop_assert!(goes.starts_with(folder)),
            None => prop_assert_eq!(&goes, &file),
        }
        // ProRes goes without saying; kerfd's default for a master.
        prop_assert_ne!(command["codec"].as_str(), Some("prores_hq"));
        // With nothing kept, the toolbar's request is the master's.
        prop_assert_eq!(
            master_request(Some(&path), None),
            kerf_editor::session::export_request(Some(&path), "master")
        );
    }
}

#[test]
fn a_master_output_is_kept_by_the_projects_absolute_path() {
    let mut prefs = Prefs::default();
    let output = MasterOutput {
        name: Some("Final".into()),
        ..MasterOutput::default()
    };
    prefs.master_outputs.insert(
        kerf_editor::prefs::project_key(Path::new("/films/two/../atlas/")),
        output.clone(),
    );
    assert_eq!(
        prefs.master_output(Path::new("/films/atlas")),
        Some(&output)
    );
    assert_eq!(
        qt_master_key(Path::new("/films/My Film")).as_deref(),
        Some("qt:My Film~58872c9953bd")
    );
}

// ---------------------------------------------------------------- Collab's words

#[test]
fn publish_starts_from_the_folders_name_and_shares_three_ways() {
    assert_eq!(publish_name("My Film (v2)"), "My-Film-v2-");
    assert_eq!(publish_name("atlas.cut_1"), "atlas.cut_1");
    assert_eq!(publish_name("가편집"), "-");
    assert_eq!(
        publish_command(PublishWay::GitHub, " atlas ", false, ""),
        Some(json!({"type": "collab_publish", "github": {"private": false, "name": "atlas"}}))
    );
    assert_eq!(
        publish_command(PublishWay::GitHub, "", true, ""),
        Some(json!({"type": "collab_publish", "github": {"private": true}}))
    );
    assert_eq!(publish_command(PublishWay::Remote, "x", true, "  "), None);
    assert_eq!(
        publish_command(PublishWay::Remote, "x", true, " git@gitlab.com:j/a.git "),
        Some(json!({"type": "collab_publish", "remote": "git@gitlab.com:j/a.git"}))
    );
    assert_eq!(
        publish_command(PublishWay::Local, "x", true, "y"),
        Some(json!({"type": "collab_publish"}))
    );
}

#[test]
fn a_pull_request_is_titled_from_its_branch_and_said_with_its_url() {
    assert_eq!(pull_request_title("trim-the_opening"), "Trim the opening");
    assert_eq!(pull_request_title(""), "");
    assert_eq!(pull_request_command("  ", "body"), None);
    assert_eq!(
        pull_request_command("Trim", "  "),
        Some(json!({"type": "collab_pull_request", "title": "Trim"}))
    );
    assert_eq!(
        pull_request_command("Trim", "Shorter\nopening"),
        Some(json!({"type": "collab_pull_request", "title": "Trim", "body": "Shorter\nopening"}))
    );
    assert_eq!(
        pull_request_said("https://github.com/j/a/pull/4", true),
        "Opened a pull request: https://github.com/j/a/pull/4"
    );
    assert_eq!(
        pull_request_said("https://github.com/j/a/compare/trim?expand=1", false),
        "Opened GitHub's page for a new pull request: https://github.com/j/a/compare/trim?expand=1"
    );
    assert_eq!(
        pull_request_said("https://github.com/j/a/pull/3", false),
        "This branch already has a pull request: https://github.com/j/a/pull/3"
    );
}

#[test]
fn what_a_sync_brought_is_said_and_left_in_activity() {
    let report = json!({
        "incoming": [{"author": "jun", "subject": "Trimmed c3"}, {"author": "mi", "subject": "Title"}],
        "conflicts": [
            {"path": "project.kerf", "what": "c3's out point", "kept": "yours"},
            {"path": "", "what": "the title's words", "kept": "theirs"},
        ],
        "renamed": [["c4", "c7"]],
        "pushed": true,
    });
    assert_eq!(
        sync_lines(&report),
        [
            "jun · Trimmed c3",
            "mi · Title",
            "Kept yours · project.kerf: c3's out point",
            "Kept theirs · the title's words",
            "Renumbered · c4 → c7",
        ]
    );
    assert_eq!(
        sync_said(&report, false).as_deref(),
        Some("Synced: 2 commits in, pushed, 1 change kept yours, 1 change kept theirs")
    );
    let quiet = json!({"incoming": [], "conflicts": [], "pushed": false});
    assert_eq!(
        sync_said(&quiet, false),
        None,
        "a quiet live cycle says nothing"
    );
    assert_eq!(
        sync_said(&quiet, true).as_deref(),
        Some("Synced: up to date")
    );
}

// ---------------------------------------------------------------- Preflight

fn row(code: &str, passed: bool) -> Value {
    json!({"code": code, "name": code.to_uppercase(), "passed": passed, "value": "1", "limit": "≤ 2"})
}

proptest! {
    #[test]
    fn preflight_shows_every_failure_it_can_then_passes(passes in prop::collection::vec(any::<bool>(), 0..14)) {
        let summary = json!({"rows": passes
            .iter()
            .enumerate()
            .map(|(index, passed)| row(&format!("c{index}"), *passed))
            .collect::<Vec<_>>()});
        let failed = passes.iter().filter(|passed| !**passed).count();
        let rows = preflight_rows(&summary, 4, 6);
        let shown_failed = rows.iter().filter(|row| !row.passed).count();
        prop_assert_eq!(shown_failed, failed.min(6));
        prop_assert_eq!(preflight_hidden(&summary, 6), failed.saturating_sub(6));
        prop_assert_eq!(rows.len(), shown_failed.max(4.min(passes.len())));
        // In the summary's order.
        let order: Vec<usize> = rows.iter().map(|row| row.code[1..].parse().unwrap()).collect();
        let mut sorted = order.clone();
        sorted.sort_unstable();
        prop_assert_eq!(order, sorted);
    }
}

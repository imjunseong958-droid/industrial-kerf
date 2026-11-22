//! kerf:// links.

use kerf_editor::links::{
    hand_off, inbox_address, inbox_links, inbox_message, is_link, listen, make, openable, parse,
    resolve, seconds, slug, InboxAddress,
};
use proptest::prelude::*;
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::time::Duration;

#[test]
fn a_link_names_a_project_a_revision_and_a_moment() {
    let link = parse("kerf://atlas-cut/rev12@00:01:02:12?branch=edit&head=abc").unwrap();
    assert_eq!(link.project, "atlas-cut");
    assert_eq!(link.revision, Some(12));
    assert_eq!(link.timecode.as_deref(), Some("00:01:02:12"));
    assert_eq!(link.branch.as_deref(), Some("edit"));
    assert_eq!(seconds(&link, 24.0), Some(62.5));
    let bare = parse("KERF://Atlas-Cut").unwrap();
    assert_eq!(
        (bare.project.as_str(), bare.revision, bare.timecode),
        ("atlas-cut", None, None)
    );
}

#[test]
fn what_is_not_a_link_says_why() {
    assert!(parse("https://example.com").is_err());
    assert!(parse("kerf:///rev1").unwrap_err().contains("no project"));
    assert!(parse("kerf://a/twelve")
        .unwrap_err()
        .contains("not a revision"));
    assert!(parse("kerf://a/rev1@1:2")
        .unwrap_err()
        .contains("not a timecode"));
    assert!(!is_link("kerf:"));
}

#[test]
fn a_link_finds_its_project_among_those_known_and_never_guesses() {
    let known = vec![
        PathBuf::from("/films/Atlas Cut"),
        PathBuf::from("/films/Café Cut"),
        PathBuf::from("/b/twin"),
        PathBuf::from("/c/Twin"),
    ];
    assert_eq!(
        resolve(&parse("kerf://atlas-cut/rev1").unwrap(), &known).unwrap(),
        Path::new("/films/Atlas Cut")
    );
    assert_eq!(
        resolve(&parse("kerf://caf%C3%A9-cut").unwrap(), &known).unwrap(),
        Path::new("/films/Café Cut")
    );
    assert!(resolve(&parse("kerf://unknown").unwrap(), &known)
        .unwrap_err()
        .contains("No project"));
    assert!(resolve(&parse("kerf://twin").unwrap(), &known)
        .unwrap_err()
        .contains("More than one"));
}

proptest! {
    /// A link made for a project reads back to that project, revision and frame.
    #[test]
    fn a_made_link_reads_back(name in "[A-Za-z0-9 ]{1,20}", revision in 0u64..100_000, frames in 0u32..500_000) {
        prop_assume!(!slug(&name).is_empty());
        let folder = PathBuf::from("/projects").join(&name);
        let fps = 24.0;
        let at = frames as f64 / fps;
        let made = make(&folder, revision, Some(at), fps).unwrap();
        let link = parse(&made).unwrap();
        prop_assert_eq!(link.revision, Some(revision));
        prop_assert_eq!(resolve(&link, std::slice::from_ref(&folder)).unwrap(), folder);
        let back = seconds(&link, fps).unwrap();
        prop_assert!((back - at).abs() < 0.5 / fps);
    }
}

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

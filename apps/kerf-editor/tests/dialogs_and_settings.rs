//! Dialogs, Settings and the window's standing problems, through their
//! public pieces: what Settings ▸ Models' button opens, the two Cursor
//! imports and a file that does not read, About's releases page, what Esc
//! does in a launcher opened from an editor, what Return does in Cast and
//! Styles, a failed sync kept until a later one starts or succeeds, the
//! toolbar's red line, and no "Runtime is disconnected" bar under the
//! disconnected one.

use kerf_editor::finish::SyncFailure;
use kerf_editor::launcher::{launcher_escape, LauncherEscape};
use kerf_editor::settings::{cursor_imports, models_opener, read_failure, releases_url};
use kerf_editor::store::Connection;
use kerf_editor::workspace::{bar_takes_error, standing_problem, sync_failed, StandingInput};
use proptest::prelude::*;
use std::path::{Path, PathBuf};

const DISCONNECTED: &str = "Runtime is disconnected; reconnect to continue";

// ---------------------------------------------------------------- Settings ▸ Models

#[test]
fn the_models_file_opens_itself_and_a_missing_one_its_folder() {
    let file = Path::new("/home/me/.config/kerf/models.json");
    assert_eq!(
        models_opener(file, true),
        ("Open Models File", file.to_path_buf())
    );
    assert_eq!(
        models_opener(file, false),
        ("Open Its Folder", PathBuf::from("/home/me/.config/kerf"))
    );
}

proptest! {
    #[test]
    fn what_the_models_button_opens_is_the_file_or_the_folder_it_goes_in(
        parts in proptest::collection::vec("[a-z0-9_]{1,8}", 1..5),
        exists in proptest::bool::ANY,
    ) {
        let folder: PathBuf = std::iter::once("/".to_owned()).chain(parts).collect();
        let file = folder.join("models.json");
        let (words, opened) = models_opener(&file, exists);
        if exists {
            prop_assert_eq!(words, "Open Models File");
            prop_assert_eq!(opened, file);
        } else {
            prop_assert_eq!(words, "Open Its Folder");
            prop_assert_eq!(opened.join("models.json"), file);
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
    // The system's words, without its number (as Qt's errorString).
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

// ---------------------------------------------------------------- the launcher

#[test]
fn esc_gives_a_launcher_opened_from_an_editor_back() {
    use LauncherEscape::*;
    // A form open goes first, wherever the launcher came from.
    assert_eq!(launcher_escape(true, false, true), PutFormAway);
    assert_eq!(launcher_escape(true, false, false), PutFormAway);
    // A form whose task runs stays.
    assert_eq!(launcher_escape(true, true, true), Nothing);
    // No form: from an editor, it is given up; the first-run one stays.
    assert_eq!(launcher_escape(false, false, true), GiveBack);
    assert_eq!(launcher_escape(false, true, true), GiveBack);
    assert_eq!(launcher_escape(false, false, false), Nothing);
    assert_eq!(launcher_escape(false, true, false), Nothing);
}

proptest! {
    #[test]
    fn the_launcher_the_editor_starts_with_is_never_given_up(
        form in proptest::bool::ANY, busy in proptest::bool::ANY,
    ) {
        prop_assert_ne!(launcher_escape(form, busy, false), LauncherEscape::GiveBack);
    }
}

// ---------------------------------------------------------------- a failed sync

#[test]
fn a_failed_sync_stands_until_a_later_one_starts_or_succeeds() {
    let mut sync = SyncFailure::default();
    assert!(sync.failed("could not reach origin", false));
    assert_eq!(sync.standing(), Some("could not reach origin"));
    assert_eq!(sync.last(), Some("could not reach origin"));
    // A later sync starts: it stands no longer, but the chip still knows why
    // the last one failed until one succeeds.
    sync.started();
    assert_eq!(sync.standing(), None);
    assert_eq!(sync.last(), Some("could not reach origin"));
    // It fails the same way again: Live Sync says it only once, but it
    // stands again.
    assert!(!sync.failed("could not reach origin", false));
    assert_eq!(sync.standing(), Some("could not reach origin"));
    // A person asking hears it each time.
    assert!(sync.failed("could not reach origin", true));
    // Another reason is said.
    assert!(sync.failed("remote refused the push", false));
    sync.done();
    assert_eq!(sync.standing(), None);
    assert_eq!(sync.last(), None);
    // After a success the same reason is new again.
    assert!(sync.failed("could not reach origin", false));
}

#[derive(Clone, Debug)]
enum Step {
    Started,
    Failed(u8, bool),
    Done,
}

fn step() -> impl Strategy<Value = Step> {
    prop_oneof![
        Just(Step::Started),
        (0u8..3, proptest::bool::ANY).prop_map(|(why, asked)| Step::Failed(why, asked)),
        Just(Step::Done),
    ]
}

proptest! {
    #[test]
    fn a_sync_failure_follows_qts_rules(steps in proptest::collection::vec(step(), 0..40)) {
        let mut sync = SyncFailure::default();
        // The model: why the last failed (until done), whether it stands.
        let mut last: Option<String> = None;
        for step in steps {
            let standing = match step {
                Step::Started => {
                    sync.started();
                    false
                }
                Step::Failed(why, asked) => {
                    let why = format!("reason {why}");
                    let said = sync.failed(&why, asked);
                    prop_assert_eq!(said, asked || last.as_deref() != Some(why.as_str()));
                    last = Some(why);
                    true
                }
                Step::Done => {
                    sync.done();
                    last = None;
                    false
                }
            };
            prop_assert_eq!(sync.last(), last.as_deref());
            prop_assert_eq!(sync.standing(), last.as_deref().filter(|_| standing));
            // What stands is always why the last one failed.
            if let Some(why) = sync.standing() {
                prop_assert_eq!(Some(why), sync.last());
            }
        }
    }
}

// ---------------------------------------------------------------- the toolbar's red line

fn input<'a>(
    connection: &'a Connection,
    error: Option<&'a str>,
    conflicted: bool,
    sync_failure: Option<&'a str>,
) -> StandingInput<'a> {
    StandingInput {
        has_project: true,
        connection,
        error,
        conflicted,
        sync_failure,
    }
}

#[test]
fn the_red_line_says_a_failed_sync_after_its_bar_is_dismissed() {
    let connected = Connection::Connected;
    // The bar shows: the line says its first line.
    let bar = sync_failed("could not reach origin");
    assert_eq!(bar, "Sync failed: could not reach origin");
    assert_eq!(
        standing_problem(&input(
            &connected,
            Some(&bar),
            false,
            Some("could not reach origin")
        )),
        Some("Sync failed: could not reach origin".into())
    );
    // Dismissed: the line still says it, until a later sync starts.
    assert_eq!(
        standing_problem(&input(
            &connected,
            None,
            false,
            Some("could not reach origin")
        )),
        Some("Sync failed: could not reach origin".into())
    );
    assert_eq!(
        standing_problem(&input(&connected, None, false, None)),
        None
    );
}

#[test]
fn the_red_line_keeps_qts_order() {
    let connected = Connection::Connected;
    let failed = Connection::Failed("gone".into());
    let connecting = Connection::Connecting;
    let why = Some("no route");
    assert_eq!(
        standing_problem(&input(&failed, Some("an error"), true, why)),
        Some("Offline".into())
    );
    assert_eq!(
        standing_problem(&input(&connecting, Some("an error"), true, why)),
        Some("Connecting…".into())
    );
    assert_eq!(
        standing_problem(&input(&connected, Some("an error\nmore"), true, why)),
        Some("an error".into())
    );
    assert_eq!(
        standing_problem(&input(&connected, None, true, why)),
        Some("A merge is unfinished; finish it with git".into())
    );
    assert_eq!(
        standing_problem(&input(&connected, None, false, why)),
        Some("Sync failed: no route".into())
    );
    // No project: nothing stands in the way of one.
    let mut none = input(&failed, Some("an error"), true, why);
    none.has_project = false;
    assert_eq!(standing_problem(&none), None);
}

#[test]
fn a_failed_sync_reads_as_a_person_does() {
    assert_eq!(
        sync_failed("error[git.fetch]: could not read from remote"),
        "Sync failed: could not read from remote"
    );
    // A long failure (git's lines) is one line in the toolbar.
    let connected = Connection::Connected;
    assert_eq!(
        standing_problem(&input(
            &connected,
            None,
            false,
            Some("fatal: '/gone' does not appear to be a git repository\nfatal: Could not read")
        )),
        Some("Sync failed: fatal: '/gone' does not appear to be a git repository".into())
    );
}

// ---------------------------------------------------------------- disconnected

#[test]
fn runtime_is_disconnected_has_no_bar_while_kerfd_is_away() {
    assert!(!bar_takes_error(
        DISCONNECTED,
        &Connection::Failed("gone".into())
    ));
    assert!(!bar_takes_error(DISCONNECTED, &Connection::Connecting));
    // Reached again meanwhile: it is said.
    assert!(bar_takes_error(DISCONNECTED, &Connection::Connected));
    // Any other error has its bar whatever the connection.
    assert!(bar_takes_error(
        "Clip not found",
        &Connection::Failed("gone".into())
    ));
    assert!(bar_takes_error("Clip not found", &Connection::Connecting));
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

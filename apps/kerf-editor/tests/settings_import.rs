//! Its own test binary: the test sets KERF_EDITOR_PREFS and KERF_SECRETS for the whole
//! process, which the editor's other tests would read.

use kerf_editor::prefs::{Permissions, Prefs};

// ----------------------------------------------- the settings of the editor before 0.1.5 (Qt)

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

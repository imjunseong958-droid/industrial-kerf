//! What the fifth wave's integration wired across the groups' files: Kerf's
//! own mark on the Auto mode's chip, and the sizes and holds the groups kept
//! locally now named in the theme.

use gpui::AssetSource;
use kerf_editor::chat::{permission_look, FOLDER_CHIP_MOST};
use kerf_editor::icons::{Assets, Icon};
use kerf_editor::prefs::Permissions;
use kerf_editor::store::{preview_notice, PREVIEW_NOTICE_HOLD};
use kerf_editor::theme;
use serde_json::json;
use std::time::Duration;

#[test]
fn auto_carries_kerf_s_own_mark_in_the_gpu_s_amber() {
    assert_eq!(
        permission_look(Permissions::Auto),
        (Icon::Kerf, Some(theme::activity::gpu()))
    );
    // Each mode looks its own.
    let looks = [
        Permissions::Ask,
        Permissions::AcceptEdits,
        Permissions::Auto,
    ]
    .map(|mode| permission_look(mode).0);
    assert_eq!(looks, [Icon::Lock, Icon::Check, Icon::Kerf]);
}

#[test]
fn kerf_s_mark_is_served_as_qt_drew_it() {
    assert!(Icon::ALL.contains(&Icon::Kerf));
    assert_eq!(Icon::Kerf.name(), "kerf");
    let served = Assets
        .load(&Icon::Kerf.path())
        .unwrap()
        .expect("the mark is served");
    let svg = String::from_utf8(served.into_owned()).unwrap();
    // Qt's g-kerf, filled: a mask in the chip's ink.
    assert!(svg.contains(r#"d="M3 3.5h4v5.5h2V3.5h4v9H3z""#), "{svg}");
    assert!(svg.contains(r#"stroke="none""#));
}

#[test]
fn the_preview_s_short_hold_is_the_theme_s() {
    assert_eq!(PREVIEW_NOTICE_HOLD, theme::motion::NOTICE_SHORT_HOLD);
    assert_eq!(theme::motion::NOTICE_SHORT_HOLD, Duration::from_secs(4));
    // Shorter than a notice's own hold, as Qt's status bar held it.
    assert!(theme::motion::NOTICE_SHORT_HOLD < theme::motion::NOTICE_HOLD);
    let ready = json!({"type": "preview", "status": "ready"});
    assert_eq!(
        preview_notice(&ready).and_then(|(_, hold)| hold),
        Some(theme::motion::NOTICE_SHORT_HOLD)
    );
}

#[test]
fn the_composer_s_and_the_ask_s_sizes_are_the_theme_s() {
    assert_eq!(FOLDER_CHIP_MOST, theme::layout::COMPOSER_FOLDER_MOST);
    assert_eq!(
        kerf_editor::ask::SCOPE_MENU_WIDTH,
        theme::layout::ASK_SCOPE_MENU_WIDTH
    );
    // The Scope menu is no narrower than any menu, nor wider than the card.
    const {
        assert!(theme::layout::ASK_SCOPE_MENU_WIDTH >= theme::layout::MENU_LEAST);
        assert!(theme::layout::ASK_SCOPE_MENU_WIDTH <= theme::layout::ASK_WIDTH);
    }
}

#[test]
fn notices_stand_above_the_composer_and_what_stands_over_it() {
    use kerf_editor::workspace::notice_bottom;
    // The composer with Kerf's next steps over it starts 160 up a window
    // 1000 tall: a notice's foot stands NOTICE_GAP above that.
    assert_eq!(
        notice_bottom(1000.0, Some(840.0)),
        160.0 + theme::layout::NOTICE_GAP
    );
    // Before the composer has been laid out, where notices always stood.
    assert_eq!(notice_bottom(1000.0, None), theme::layout::NOTICE_FOOT);
}

proptest::proptest! {
    /// Wherever the composer's stack starts, a notice's foot is above it,
    /// by the gap, and inside the window; a taller stack lifts it as much.
    #[test]
    fn a_notice_never_covers_the_composer_s_stack(
        height in 200.0f32..2400.0,
        share in 0.0f32..1.0,
        grow in 0.0f32..200.0,
    ) {
        use kerf_editor::workspace::notice_bottom;
        let top = height * share;
        let bottom = notice_bottom(height, Some(top));
        let foot = height - bottom;
        proptest::prop_assert!((top - foot - theme::layout::NOTICE_GAP).abs() < 1e-3);
        proptest::prop_assert!(bottom >= theme::layout::NOTICE_GAP);
        let higher = (top - grow).max(0.0);
        let lifted = notice_bottom(height, Some(higher));
        proptest::prop_assert!((lifted - bottom - (top - higher)).abs() < 1e-3);
    }
}

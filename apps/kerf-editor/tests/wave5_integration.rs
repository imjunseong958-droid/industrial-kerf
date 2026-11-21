//! What the fifth wave's integration wired across the groups' files: Kerf's
//! own mark on the Auto mode's chip, and the sizes and holds the groups kept
//! locally now named in the theme.

use kerf_editor::theme;

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

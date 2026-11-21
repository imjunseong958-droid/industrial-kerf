//! The clip Inspector's choices, through their public functions: which
//! choices are a row of segments and which open the kit's menu, the menu's
//! rows (the one chosen ticked, a letter going to the option it starts),
//! which way the menu opens from its button and how it keeps inside the
//! window, how far the Inspector scrolls to show a choice the keys come
//! to, when an open menu goes as the Inspector scrolls under it, and what
//! choosing a row sends once a snapshot has set another option meanwhile.

use kerf_editor::inspector::{
    choice_is_compact, choice_menu_opens_up, choice_menu_shift, choice_menu_stays, choice_rows,
    choice_sends, scroll_to_show,
};
use kerf_editor::theme::{layout, space};
use kerf_editor::ui::menu_letter;
use proptest::prelude::*;

fn owned(options: &[&str]) -> Vec<String> {
    options.iter().map(|option| (*option).to_owned()).collect()
}

/// What a choice's menu keeps from its button and from the window's edge.
const MARGIN: f32 = space::S4 + space::S8;

#[test]
fn a_few_short_options_are_segments_and_the_rest_open_a_menu() {
    assert!(choice_is_compact(&owned(&["left", "center", "right"])));
    assert!(choice_is_compact(&owned(&["in", "out"])));
    // A fourth option, or one longer than eight letters, opens a menu.
    assert!(!choice_is_compact(&owned(&[
        "normal", "add", "screen", "multiply"
    ])));
    assert!(!choice_is_compact(&owned(&["linear", "character"])));
}

#[test]
fn the_menu_says_each_option_in_words_and_ticks_the_one_chosen() {
    let rows = choice_rows(
        &owned(&["normal", "soft_light", "hard_light"]),
        "soft_light",
    );
    assert_eq!(
        rows,
        vec![
            ("Normal".to_owned(), false),
            ("Soft light".to_owned(), true),
            ("Hard light".to_owned(), false),
        ]
    );
    // Nothing chosen yet (a value the catalog no longer offers): none ticked.
    assert!(choice_rows(&owned(&["a", "b"]), "c")
        .iter()
        .all(|(_, ticked)| !ticked));
}

fn option_name() -> impl Strategy<Value = String> {
    "[a-z]{1,10}(_[a-z]{1,6})?"
}

proptest! {
    #[test]
    fn compact_is_at_most_three_options_of_at_most_eight_letters(
        options in prop::collection::vec(option_name(), 0..7),
    ) {
        let short = options.iter().all(|option| option.chars().count() <= 8);
        prop_assert_eq!(choice_is_compact(&options), options.len() <= 3 && short);
    }

    #[test]
    fn a_row_is_ticked_exactly_where_its_option_is_the_one_chosen(
        options in prop::collection::vec(option_name(), 1..12),
        pick in any::<prop::sample::Index>(),
    ) {
        let chosen = options[pick.index(options.len())].clone();
        let rows = choice_rows(&options, &chosen);
        prop_assert_eq!(rows.len(), options.len());
        for ((words, ticked), option) in rows.iter().zip(&options) {
            prop_assert_eq!(*ticked, *option == chosen);
            prop_assert!(!words.trim().is_empty());
        }
        prop_assert!(rows.iter().any(|(_, ticked)| *ticked));
    }

    /// The menu's letter jump: the first letter of an option goes to a row
    /// that starts with it, the option's own or one before it in the cycle.
    #[test]
    fn a_letter_goes_to_an_option_starting_with_it(
        options in prop::collection::vec(option_name(), 1..12),
        pick in any::<prop::sample::Index>(),
    ) {
        let rows = choice_rows(&options, "");
        let words: Vec<&str> = rows.iter().map(|(words, _)| words.as_str()).collect();
        let enabled = vec![true; words.len()];
        let wanted = &options[pick.index(options.len())];
        let letter = wanted.chars().next().unwrap();
        let at = menu_letter(&words, &enabled, None, letter);
        prop_assert!(at.is_some());
        let at = at.unwrap();
        prop_assert!(options[at].starts_with(letter));
    }

    #[test]
    fn it_opens_over_its_button_only_where_under_it_is_short_and_over_it_roomier(
        top in 0.0f32..1200.0,
        tall in 16.0f32..40.0,
        height in 40.0f32..900.0,
        window in 200.0f32..1400.0,
    ) {
        let bottom = top + tall;
        let below = window - bottom - MARGIN;
        let above = top - MARGIN;
        let up = choice_menu_opens_up(top, bottom, height, window);
        if below >= height {
            prop_assert!(!up);
        }
        if up {
            prop_assert!(above > below);
        }
        if below < height && above > below {
            prop_assert!(up);
        }
    }
}

proptest! {
    /// Scrolled to show it, a button that fits the view lies whole inside it,
    /// clear of the fades, wherever the list could reach; one in sight stays.
    #[test]
    fn a_button_scrolled_to_shows_whole_and_one_in_sight_stays(
        view_top in 0.0f32..300.0,
        view_tall in 120.0f32..800.0,
        at in -2000.0f32..2000.0,
        tall in 16.0f32..40.0,
        offset in -1500.0f32..0.0,
    ) {
        let view = (view_top, view_top + view_tall);
        let item = (at, at + tall);
        let most = 4000.0;
        let shown = scroll_to_show(item, view, offset, most);
        prop_assert!((-most..=0.0).contains(&shown));
        let moved = shown - offset;
        let (top, bottom) = (item.0 + moved, item.1 + moved);
        let inside = item.0 >= view.0 + layout::TOP_FADE
            && item.1 <= view.1 - layout::BOTTOM_FADE;
        if inside {
            prop_assert_eq!(shown, offset);
        } else if shown > -most && shown < 0.0 {
            prop_assert!(top >= view.0 + layout::TOP_FADE - 0.01);
            prop_assert!(bottom <= view.1 - layout::BOTTOM_FADE + 0.01);
        }
    }
}

proptest! {
    /// Moved left as it must, the menu ends inside the window's margin
    /// where it fits, and never starts outside the margin on the left.
    #[test]
    fn the_menu_moved_left_stays_inside_the_window(
        window in 300.0f32..2000.0,
        at in 0.0f32..1.0,
        width in 180.0f32..600.0,
    ) {
        let left = space::S8 + at * (window - 2.0 * space::S8);
        let shift = choice_menu_shift(left, width, window);
        prop_assert!(shift >= 0.0);
        prop_assert!(left - shift >= space::S8 - 0.01);
        if width <= window - 2.0 * space::S8 {
            prop_assert!(left - shift + width <= window - space::S8 + 0.01);
        }
    }
}

proptest! {
    /// A menu stays open exactly while the body is scrolled as it was when
    /// it opened and some of its button shows in the body's view.
    #[test]
    fn a_menu_stays_only_unscrolled_over_a_button_in_sight(
        view_top in 0.0f32..300.0,
        view_tall in 120.0f32..800.0,
        at in -1500.0f32..1500.0,
        tall in 16.0f32..40.0,
        opened in -1500.0f32..0.0,
        rolled in -400.0f32..400.0,
    ) {
        let view = (view_top, view_top + view_tall);
        let button = (at, at + tall);
        let in_sight = button.1 > view.0 && button.0 < view.1;
        prop_assert_eq!(choice_menu_stays(opened, opened, button, view), in_sight);
        if rolled.abs() >= 1.0 {
            prop_assert!(!choice_menu_stays(opened, opened + rolled, button, view));
        }
    }

    /// Worked out on what the choice holds now, a menu's ticked row is the
    /// one row that sends nothing: any other sends, whichever was ticked as
    /// the menu opened; keyed, none sends.
    #[test]
    fn the_row_ticked_now_is_the_one_that_sends_nothing(
        options in prop::collection::vec(option_name(), 2..12),
        now in any::<prop::sample::Index>(),
        keyed in any::<bool>(),
    ) {
        let held = &options[now.index(options.len())];
        for ((_, ticked), option) in choice_rows(&options, held).iter().zip(&options) {
            prop_assert_eq!(choice_sends(option, held, keyed), !keyed && !*ticked);
        }
    }
}

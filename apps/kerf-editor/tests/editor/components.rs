//! The interface kit: buttons, switches, segmented controls, menus, selectable text, and the theme's measures, colours and faces.

use crate::support::{bounds, window};
use gpui::{px, size};
use kerf_editor::input::Unit;
use kerf_editor::selectable::{copied, stretch, within, Chosen, Spot};
use kerf_editor::theme;
use kerf_editor::theme::{layout, Face, ReadingFace};
use kerf_editor::ui::{
    kept_path, menu_height, menu_letter, menu_step, scroll_fades, segment_step, submenu_place,
    SubmenuPlace,
};
use proptest::prelude::*;

// ------------------------------------------------------------ menus and segments

proptest! {
    #[test]
    fn the_arrows_light_only_what_can_be_chosen_and_go_round(
        enabled in proptest::collection::vec(any::<bool>(), 0..10),
        from in proptest::option::of(0usize..10),
        down in any::<bool>(),
    ) {
        let from = from.filter(|at| *at < enabled.len());
        let next = menu_step(&enabled, from, down);
        prop_assert_eq!(next.is_some(), enabled.iter().any(|on| *on));
        if let Some(next) = next {
            prop_assert!(enabled[next]);
            // As many steps as there are rows that can be chosen come back.
            let count = enabled.iter().filter(|on| **on).count();
            let mut at = Some(next);
            for _ in 0..count {
                at = menu_step(&enabled, at, down);
            }
            prop_assert_eq!(at, Some(next));
        }
    }

    #[test]
    fn a_letter_goes_to_a_row_starting_with_it(
        words in proptest::collection::vec("[a-dA-D][a-z]{0,4}", 1..8),
        enabled in proptest::collection::vec(any::<bool>(), 8),
        letter in "[a-d]",
    ) {
        let rows: Vec<&str> = words.iter().map(String::as_str).collect();
        let letter = letter.chars().next().unwrap();
        let found = menu_letter(&rows, &enabled[..rows.len()], None, letter);
        let any = rows.iter().zip(&enabled).any(|(row, on)| {
            *on && row.to_lowercase().starts_with(letter)
        });
        prop_assert_eq!(found.is_some(), any);
        if let Some(at) = found {
            prop_assert!(enabled[at] && rows[at].to_lowercase().starts_with(letter));
        }
    }
}

#[test]
fn a_letter_typed_again_goes_on_to_the_next_row_with_it() {
    let rows = ["Rename", "Archive", "Reveal", "Delete…"];
    let enabled = [true, true, true, true];
    assert_eq!(menu_letter(&rows, &enabled, None, 'r'), Some(0));
    assert_eq!(menu_letter(&rows, &enabled, Some(0), 'r'), Some(2));
    assert_eq!(menu_letter(&rows, &enabled, Some(2), 'R'), Some(0));
    assert_eq!(menu_letter(&rows, &enabled, Some(2), 'x'), None);
}

#[test]
fn the_arrows_in_a_segmented_control_skip_what_cannot_be_chosen() {
    let enabled = [true, false, true, true];
    assert_eq!(segment_step(&enabled, 0, true), Some(2));
    assert_eq!(segment_step(&enabled, 2, false), Some(0));
    assert_eq!(segment_step(&enabled, 3, true), None);
    assert_eq!(segment_step(&enabled, 0, false), None);
}

// ------------------------------------------------------------ lists and panels

#[test]
fn a_list_fades_where_a_row_is_cut_and_while_there_is_more() {
    let rows = [(100.0, 140.0), (140.0, 180.0), (180.0, 220.0)];
    // At its top, with more below.
    assert_eq!(scroll_fades(0.0, 60.0, 100.0, &rows), (false, true));
    // Scrolled into the middle of the first row.
    assert_eq!(scroll_fades(20.0, 60.0, 100.0, &rows), (true, true));
    // Scrolled exactly to a row's top: nothing cut.
    assert_eq!(scroll_fades(40.0, 60.0, 100.0, &rows), (false, true));
    // At the end, a row cut at the top; and at the end on a row's top.
    assert_eq!(scroll_fades(60.0, 60.0, 100.0, &rows), (true, false));
    assert_eq!(scroll_fades(80.0, 80.0, 100.0, &rows), (false, false));
    // Painted content with no rows: any scroll cuts it.
    assert_eq!(scroll_fades(5.0, 60.0, 100.0, &[]), (true, true));
}

// ------------------------------------------------------------ type and colour

#[test]
fn figures_are_tabular_and_mono_has_the_plain_zero() {
    let text = theme::font_features(false);
    let mono = theme::font_features(true);
    assert!(text.tag_value_list().contains(&("tnum".to_owned(), 1)));
    assert!(mono.tag_value_list().contains(&("tnum".to_owned(), 1)));
    assert!(mono.tag_value_list().contains(&("zero".to_owned(), 0)));
    assert!(!text.tag_value_list().iter().any(|(tag, _)| tag == "zero"));
    assert_eq!(
        theme::font(theme::UI_FAMILY, gpui::FontWeight::NORMAL, false).features,
        text
    );
}

#[test]
fn the_interface_and_the_conversation_have_faces_of_their_own() {
    assert!(Face::INTERFACE.contains(&Face::System));
    assert!(!Face::INTERFACE.contains(&Face::Serif));
    // What was saved as the one serif choice is IBM Plex reading in serif.
    assert_eq!(Face::Serif.split(), (Face::Plex, ReadingFace::Serif));
    assert_eq!(Face::Inter.split(), (Face::Inter, ReadingFace::Same));
    assert_eq!(ReadingFace::Serif.family(Face::Inter), theme::SERIF_FAMILY);
    assert_eq!(ReadingFace::Same.family(Face::Inter), theme::INTER_FAMILY);
    assert_eq!(Face::System.mono(), theme::JETBRAINS_FAMILY);
    assert!(!Face::System.ui().is_empty());
    assert_eq!(
        serde_json::to_string(&ReadingFace::Same).unwrap(),
        "\"same\""
    );
    assert_eq!(
        serde_json::from_str::<Face>("\"system\"").unwrap(),
        Face::System
    );
    assert_eq!(ReadingFace::Same.label(), "Same as interface");
    assert_eq!(Face::System.label(), "System");
}

#[test]
fn the_nine_marker_colours_are_their_own_and_read_on_the_ruler() {
    let colours: Vec<gpui::Rgba> = theme::marker::NAMES
        .iter()
        .map(|name| theme::marker::named(Some(name)))
        .collect();
    for (at, colour) in colours.iter().enumerate() {
        for other in &colours[at + 1..] {
            assert_ne!(colour, other, "{}", theme::marker::NAMES[at]);
        }
        for ground in [theme::chrome(), theme::well(), theme::surround()] {
            assert!(
                theme::contrast(*colour, ground) >= 3.0,
                "{} on the ruler",
                theme::marker::NAMES[at]
            );
        }
    }
    // No colour, or one not of the nine, is green, as Qt drew it.
    assert_eq!(theme::marker::named(None), theme::marker::green());
    assert_eq!(theme::marker::named(Some("teal")), theme::marker::green());
    assert_eq!(theme::marker::named(Some(" Cyan ")), theme::marker::cyan());
}

// ------------------------------------------------------------ choosing words

#[test]
fn a_drag_chooses_from_where_it_went_down_across_pieces() {
    let mut chosen = Chosen::pressed("answer", 0, 2..2, Unit::Letter);
    assert!(chosen.is_empty());
    chosen.drag_to(Spot::new(2, 3), (Spot::new(2, 3), Spot::new(2, 3)));
    assert_eq!(chosen.span(), (Spot::new(0, 2), Spot::new(2, 3)));
    assert_eq!(chosen.within(0, 10), Some(2..10));
    assert_eq!(chosen.within(1, 7), Some(0..7));
    assert_eq!(chosen.within(2, 9), Some(0..3));
    assert_eq!(chosen.within(3, 9), None);
    // Dragged back before where it went down, it reaches back from there.
    chosen.drag_to(Spot::new(0, 0), (Spot::new(0, 0), Spot::new(0, 0)));
    assert_eq!(chosen.span(), (Spot::new(0, 0), Spot::new(0, 2)));
}

#[test]
fn a_drag_after_a_double_click_goes_by_whole_words() {
    // "Cut these together": "these" chosen, then the pointer on "Cut".
    let mut chosen = Chosen::pressed("request", 0, 4..9, Unit::Word);
    chosen.drag_to(Spot::new(0, 1), (Spot::new(0, 0), Spot::new(0, 3)));
    assert_eq!(chosen.span(), (Spot::new(0, 0), Spot::new(0, 9)));
    // On to "together": the first word stays whole.
    chosen.drag_to(Spot::new(0, 12), (Spot::new(0, 10), Spot::new(0, 18)));
    assert_eq!(chosen.span(), (Spot::new(0, 4), Spot::new(0, 18)));
}

#[test]
fn what_is_copied_keeps_the_breaks_between_pieces() {
    let pieces = [
        (0, "I cut 3 clips together.", "\n\n"),
        (1, "one", "\n\n"),
        (2, "two", "\n"),
    ];
    assert_eq!(
        copied(&pieces, (Spot::new(0, 6), Spot::new(2, 3))),
        "3 clips together.\n\none\ntwo"
    );
    assert_eq!(copied(&pieces, (Spot::new(1, 1), Spot::new(1, 3))), "ne");
    assert_eq!(copied(&pieces, (Spot::new(0, 3), Spot::new(0, 3))), "");
}

proptest! {
    #[test]
    fn a_span_covers_whole_pieces_between_its_ends(
        lens in prop::collection::vec(0usize..40, 1..6),
        a in (0usize..6, 0usize..50),
        b in (0usize..6, 0usize..50),
    ) {
        let (start, end) = {
            let (a, b) = (Spot::new(a.0, a.1), Spot::new(b.0, b.1));
            if a <= b { (a, b) } else { (b, a) }
        };
        for (piece, len) in lens.iter().enumerate() {
            match within((start, end), piece, *len) {
                Some(range) => {
                    prop_assert!(range.start < range.end && range.end <= *len);
                    if piece > start.piece && piece < end.piece {
                        prop_assert_eq!(range, 0..*len);
                    }
                }
                // A piece between the ends is covered whole, if it has words.
                None if piece > start.piece && piece < end.piece => prop_assert_eq!(*len, 0),
                None => {}
            }
        }
    }

    #[test]
    fn copying_everything_joins_every_piece(words in prop::collection::vec("[a-z ]{0,12}", 1..6)) {
        let pieces: Vec<(usize, &str, &str)> =
            words.iter().enumerate().map(|(at, words)| (at, words.as_str(), "\n")).collect();
        let last = pieces.len() - 1;
        let all = copied(&pieces, (Spot::new(0, 0), Spot::new(last, words[last].len())));
        let expected: Vec<&str> = words.iter().map(String::as_str).filter(|words| !words.is_empty()).collect();
        prop_assert_eq!(all, expected.join("\n"));
    }

    #[test]
    fn a_word_drag_always_keeps_the_first_word(
        first in (0usize..4, 0usize..20, 0usize..10),
        here in (0usize..4, 0usize..20, 0usize..10),
    ) {
        let first = (Spot::new(first.0, first.1), Spot::new(first.0, first.1 + first.2));
        let here = (Spot::new(here.0, here.1), Spot::new(here.0, here.1 + here.2));
        let (anchor, head) = stretch(first, here);
        let (start, end) = if anchor <= head { (anchor, head) } else { (head, anchor) };
        prop_assert!(start <= first.0 && end >= first.1);
        prop_assert!(start <= here.0 || end >= here.1);
    }
}

/// Every icon the views draw is served to GPUI under a name of its own.
#[test]
fn every_icon_is_served_under_its_own_name() {
    use gpui::AssetSource;
    use kerf_editor::icons::{Assets, Icon};
    let mut names = std::collections::HashSet::new();
    for icon in Icon::ALL {
        assert!(names.insert(icon.name()), "{} twice", icon.name());
        let served = Assets.load(&icon.path()).unwrap().expect("served");
        let svg = String::from_utf8(served.into_owned()).unwrap();
        assert!(svg.starts_with("<svg") && svg.ends_with("</svg>"));
    }
    for icon in [
        Icon::List,
        Icon::Grid,
        Icon::Audio,
        Icon::Dice,
        Icon::ZoomIn,
        Icon::ZoomOut,
    ] {
        assert!(Icon::ALL.contains(&icon), "{} is not served", icon.name());
    }
}

// ------------------------------------------------------------ submenus

#[test]
fn a_submenu_low_in_the_window_opens_upward_its_foot_on_the_rows() {
    // Collaborate ▸ in the app's menu over the account, in a window 1000
    // tall: its row near the foot.
    let panel = bounds(8.0, 560.0, 240.0, 420.0);
    let row = bounds(13.0, 900.0, 230.0, layout::MENU_ROW);
    let rows: Vec<kerf_editor::ui::MenuItem> = (0..8)
        .map(|at| kerf_editor::ui::MenuItem::new(format!("Row {at}")))
        .collect();
    let sub = size(px(240.0), px(menu_height(&rows)));
    assert_eq!(
        submenu_place(row, panel, sub, window(1600.0, 1000.0)),
        SubmenuPlace {
            left: false,
            up: true
        }
    );
    // High in the window it opens down, its first row level with the row.
    let row = bounds(13.0, 100.0, 230.0, layout::MENU_ROW);
    assert_eq!(
        submenu_place(row, panel, sub, window(1600.0, 1000.0)),
        SubmenuPlace::default()
    );
    // At the window's right, it opens to the left.
    let panel = bounds(1400.0, 40.0, 190.0, 300.0);
    let row = bounds(1405.0, 80.0, 180.0, layout::MENU_ROW);
    assert_eq!(
        submenu_place(row, panel, sub, window(1600.0, 1000.0)),
        SubmenuPlace {
            left: true,
            up: false
        }
    );
}

proptest! {
    /// A submenu opens where it fits; where it fits on neither side, on the
    /// side with more room.
    #[test]
    fn a_submenu_opens_where_there_is_room(
        panel_left in 0f32..1500.0,
        row_top in 0f32..960.0,
        rows in 1usize..20,
        width in 180f32..320.0,
    ) {
        let window = window(1600.0, 1000.0);
        let panel = bounds(panel_left, row_top - 40.0, width, 400.0);
        let row = bounds(panel_left + 5.0, row_top, width - 10.0, layout::MENU_ROW);
        let items: Vec<kerf_editor::ui::MenuItem> = (0..rows).map(|at| kerf_editor::ui::MenuItem::new(format!("{at}"))).collect();
        let tall = menu_height(&items);
        let sub = size(px(width), px(tall));
        let place = submenu_place(row, panel, sub, window);
        let edge = layout::TIP_EDGE;
        let right_room = 1600.0 - edge - (panel_left + width - 1.0);
        let left_room = panel_left + 1.0 - edge;
        let below = 1000.0 - edge - (row_top - 5.0);
        let above = row_top + layout::MENU_ROW + 5.0 - edge;
        if place.left {
            prop_assert!(width > right_room && left_room > right_room);
        } else {
            prop_assert!(width <= right_room || left_room <= right_room);
        }
        if place.up {
            prop_assert!(tall > below && above > below);
        } else {
            prop_assert!(tall <= below || above <= below);
        }
    }
}

#[test]
fn a_menu_is_as_tall_as_its_rows_and_the_lines_between_their_groups() {
    let items = vec![
        kerf_editor::ui::MenuItem::new("Insert"),
        kerf_editor::ui::MenuItem::new("Overwrite"),
        kerf_editor::ui::MenuItem::new("Replace Media…").separated(),
        kerf_editor::ui::MenuItem::new("Import Media…").separated(),
    ];
    // Its padding and edge, four rows, two lines.
    assert_eq!(
        menu_height(&items),
        4.0 * 2.0 + 2.0 + 4.0 * layout::MENU_ROW + 2.0 * 9.0
    );
    assert_eq!(menu_height(&[]), 10.0);
}

fn items_of(shape: &[(bool, usize)]) -> Vec<kerf_editor::ui::MenuItem> {
    shape
        .iter()
        .enumerate()
        .map(|(at, (enabled, children))| {
            let item = kerf_editor::ui::MenuItem::new(format!("{at}")).disabled(!enabled);
            if *children > 0 {
                item.submenu(
                    (0..*children)
                        .map(|c| kerf_editor::ui::MenuItem::new(format!("{at}.{c}")))
                        .collect(),
                )
            } else {
                item
            }
        })
        .collect()
}

proptest! {
    /// When a menu's rows change while it is open, what stays lit is a row
    /// there that can be chosen, at each level that still opens.
    #[test]
    fn what_stays_lit_is_still_there_and_can_be_chosen(
        shape in proptest::collection::vec((any::<bool>(), 0usize..4), 0..8),
        path in proptest::collection::vec(proptest::option::of(0usize..10), 1..4),
    ) {
        let items = items_of(&shape);
        let kept = kept_path(&items, &path);
        prop_assert!(!kept.is_empty() && kept.len() <= path.len());
        let mut rows = items.as_slice();
        for (level, lit) in kept.iter().enumerate() {
            if let Some(at) = lit {
                prop_assert!(*at < rows.len());
                prop_assert!(rows[*at].enabled());
                prop_assert_eq!(Some(*at), path[level]);
            }
            if level + 1 < kept.len() {
                let at = lit.expect("a level opens only under a lit row");
                rows = shape_children(&items, &kept[..=level]);
                prop_assert!(!rows.is_empty(), "{at}");
            }
        }
    }
}

/// The rows under the lit rows of `path`, from the top.
fn shape_children<'a>(
    items: &'a [kerf_editor::ui::MenuItem],
    path: &[Option<usize>],
) -> &'a [kerf_editor::ui::MenuItem] {
    let mut rows = items;
    for lit in path {
        let Some(item) = lit.and_then(|at| rows.get(at)) else {
            return &[];
        };
        rows = item.rows();
    }
    rows
}

#[test]
fn a_row_gone_or_greyed_is_lit_no_more_and_takes_its_submenu_with_it() {
    let before = items_of(&[(true, 0), (true, 2), (true, 0)]);
    assert_eq!(
        kept_path(&before, &[Some(1), Some(1)]),
        vec![Some(1), Some(1)]
    );
    let greyed = items_of(&[(true, 0), (false, 2), (true, 0)]);
    assert_eq!(kept_path(&greyed, &[Some(1), Some(1)]), vec![None]);
    let fewer = items_of(&[(true, 0)]);
    assert_eq!(kept_path(&fewer, &[Some(2)]), vec![None]);
}

//! The timeline's strip transport, the notices' own holds, `--screenshot`'s
//! pixels, and the Qt editor's QVariant settings (`@Variant`, `@DateTime`,
//! a plist's `<data>`) read as Qt reads them and taken up.

use kerf_editor::app::{x11_rgba, X11Pixels};
use kerf_editor::prefs::{
    import_qt, qt_graph_key, qt_settings_ini, qt_settings_plist, qt_settings_reg, qt_variant,
    Prefs, QDATASTREAM_4_0, QDATASTREAM_5_6,
};
use kerf_editor::store::EditorState;
use kerf_editor::theme::{layout, motion};
use kerf_editor::timeline::{strip_has_transport, StripFit};
use proptest::prelude::*;
use serde_json::{json, Map, Value};
use std::collections::{BTreeMap, HashMap};
use std::path::Path;
use std::time::Duration;

// ---------------------------------------------------------------- the strip's transport

#[test]
fn the_strip_carries_the_transport_only_while_the_picture_is_closed() {
    assert!(strip_has_transport(Some(false)));
    assert!(!strip_has_transport(Some(true)));
    // Until the window says, the stage has it.
    assert!(!strip_has_transport(None));
}

/// A strip as wide as the window's at 1200 points with the picture pane
/// closed, and its pieces: what the rest needs, the cut's length.
const STRIP_REST: f32 = 300.0;
const STRIP_LENGTH: f32 = 78.0;

#[test]
fn a_narrow_strip_lets_go_of_the_slider_then_the_ends_then_the_length() {
    let all = StripFit::ALL;
    let need = |fit: StripFit| STRIP_REST + fit.width(true, STRIP_LENGTH);
    // Room for all of it: all of it.
    assert_eq!(all.within(need(all), STRIP_REST, true, STRIP_LENGTH), all);
    // A point short: the slider goes, − and + stay.
    let fit = all.within(need(all) - 1.0, STRIP_REST, true, STRIP_LENGTH);
    assert_eq!(fit, StripFit::STEPS[1]);
    assert!(!fit.slider && fit.transport_ends && fit.length);
    // Then the transport's ends, then the cut's length.
    let fit = all.within(
        need(StripFit::STEPS[1]) - 1.0,
        STRIP_REST,
        true,
        STRIP_LENGTH,
    );
    assert_eq!(fit, StripFit::STEPS[2]);
    let fit = all.within(
        need(StripFit::STEPS[2]) - 1.0,
        STRIP_REST,
        true,
        STRIP_LENGTH,
    );
    assert_eq!(fit, StripFit::STEPS[3]);
    // Narrower than even that: all let go, and the rest is cut off.
    assert_eq!(
        all.within(10.0, STRIP_REST, true, STRIP_LENGTH),
        StripFit::STEPS[3]
    );
    // Without the transport its ends take no room: from the slider on, the
    // length is next.
    assert_eq!(
        StripFit::STEPS[1].width(false, STRIP_LENGTH),
        StripFit::STEPS[2].width(false, STRIP_LENGTH)
    );
    // Back with room: shown again only with some to spare.
    let tight = StripFit::STEPS[3].within(need(all), STRIP_REST, true, STRIP_LENGTH);
    assert_eq!(tight, StripFit::STEPS[1]);
    let spare = need(all) + layout::STRIP_REGROW;
    assert_eq!(
        StripFit::STEPS[3].within(spare, STRIP_REST, true, STRIP_LENGTH),
        all
    );
}

fn any_strip_fit() -> impl Strategy<Value = StripFit> {
    (0usize..4).prop_map(|step| StripFit::STEPS[step])
}

proptest! {
    #[test]
    fn a_strip_settles_on_the_most_that_fits(
        now in any_strip_fit(),
        room in 0f32..1200.0,
        more in 0f32..400.0,
        rest in 0f32..900.0,
        transport in any::<bool>(),
        length in 0f32..120.0,
    ) {
        let fit = now.within(room, rest, transport, length);
        let step = StripFit::STEPS.iter().position(|step| *step == fit).unwrap();
        // It fits, unless nothing does and all of it is let go.
        prop_assert!(rest + fit.width(transport, length) <= room || step == 3);
        // Nothing more would: what it shows more than now needs the spare too.
        let now_step = StripFit::STEPS.iter().position(|step| *step == now).unwrap();
        for (earlier, other) in StripFit::STEPS.iter().enumerate().take(step) {
            let spare = if earlier < now_step { layout::STRIP_REGROW } else { 0.0 };
            prop_assert!(rest + other.width(transport, length) + spare > room);
        }
        // Drawn again at the same width, it stays as it is: no showing and
        // hiding by turns.
        prop_assert_eq!(fit.within(room, rest, transport, length), fit);
        // More room never shows less.
        let wider = now.within(room + more, rest, transport, length);
        let wider_step = StripFit::STEPS.iter().position(|step| *step == wider).unwrap();
        prop_assert!(wider_step <= step);
    }
}

// ---------------------------------------------------------------- notices

#[test]
fn a_notice_with_a_hold_of_its_own_stays_for_it() {
    // Qt held the sync's summary and its pull request's line 8 s.
    assert_eq!(motion::NOTICE_LONG_HOLD, Duration::from_secs(8));
    assert!(motion::NOTICE_LONG_HOLD > motion::NOTICE_HOLD);
    let mut state = EditorState::default();
    state.notify_held(
        "Synced: 2 commits in, pushed",
        false,
        Some(motion::NOTICE_LONG_HOLD),
    );
    state.notify("Rendered master", false);
    state.notify("The project was not pushed", true);
    // Past the window's holds, an error's too: the one held longer stays.
    assert!(state.expire_notices(Duration::ZERO, Duration::ZERO));
    let left: Vec<&str> = state
        .notices
        .iter()
        .map(|notice| notice.text.as_str())
        .collect();
    assert_eq!(left, ["Synced: 2 commits in, pushed"]);
    // A hold of its own shorter than the window's goes first.
    state.notify_held("Gone at once", false, Some(Duration::ZERO));
    assert!(state.expire_notices(Duration::from_secs(3600), Duration::from_secs(3600)));
    assert!(state
        .notices
        .iter()
        .all(|notice| notice.text != "Gone at once"));
}

#[test]
fn a_notice_without_a_hold_keeps_the_windows() {
    let mut state = EditorState::default();
    state.notify("Rendered master", false);
    state.notify("It failed", true);
    let hold = Duration::from_secs(6);
    let error_hold = Duration::from_secs(12);
    assert_eq!(state.notices[0].hold, None);
    assert_eq!(state.notices[0].held_for(hold, error_hold), hold);
    assert_eq!(state.notices[1].held_for(hold, error_hold), error_hold);
}

#[test]
fn a_cast_saved_is_held_as_qt_held_it() {
    let mut state = EditorState::default();
    state.apply(&json!({"type": "cast", "saved": true, "toMake": [{"media": "shot-2"}]}));
    let notice = state.notices.last().expect("a notice");
    assert_eq!(notice.text, "Saved the cast; shot-2 wants making again");
    assert_eq!(notice.hold, Some(motion::NOTICE_LONG_HOLD));
}

// ---------------------------------------------------------------- --screenshot's pixels

/// The usual X server layout: 32 bits a pixel, BGRX in memory.
const XRGB: X11Pixels = X11Pixels {
    bits_per_pixel: 32,
    scanline_pad: 32,
    red_mask: 0x00ff_0000,
    green_mask: 0x0000_ff00,
    blue_mask: 0x0000_00ff,
    msb_first: false,
};

#[test]
fn x11_pixels_of_other_layouts_are_read() {
    // 24 bits a pixel, rows padded to 32 bits: 2 pixels take 8 bytes a row.
    let packed = X11Pixels {
        bits_per_pixel: 24,
        ..XRGB
    };
    let data = [
        0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0xaa, 0xbb, // row 0
        0x10, 0x20, 0x30, 0x40, 0x50, 0x60, 0xcc, 0xdd, // row 1
    ];
    let picture = x11_rgba(&data, 2, 2, packed).unwrap();
    assert_eq!(picture.get_pixel(0, 0).0, [0x03, 0x02, 0x01, 255]);
    assert_eq!(picture.get_pixel(1, 1).0, [0x60, 0x50, 0x40, 255]);
    // 16 bits, 5-6-5, most significant byte first.
    let wide = X11Pixels {
        bits_per_pixel: 16,
        scanline_pad: 16,
        red_mask: 0xf800,
        green_mask: 0x07e0,
        blue_mask: 0x001f,
        msb_first: true,
    };
    let picture = x11_rgba(&[0xf8, 0x00, 0x07, 0xe0, 0x00, 0x1f], 3, 1, wide).unwrap();
    assert_eq!(picture.get_pixel(0, 0).0, [255, 0, 0, 255]);
    assert_eq!(picture.get_pixel(1, 0).0, [0, 255, 0, 255]);
    assert_eq!(picture.get_pixel(2, 0).0, [0, 0, 255, 255]);
    // Too few bytes, or a depth it does not know: nothing.
    assert!(x11_rgba(&data[..10], 2, 2, packed).is_none());
    let odd = X11Pixels {
        bits_per_pixel: 8,
        ..XRGB
    };
    assert!(x11_rgba(&[0; 16], 4, 4, odd).is_none());
}

proptest! {
    #[test]
    fn x11_pixels_read_back_as_they_were(
        width in 1u32..9,
        rows in prop::collection::vec(prop::collection::vec(any::<[u8; 3]>(), 8), 1..6),
        msb_first in any::<bool>(),
    ) {
        let height = rows.len() as u32;
        let format = X11Pixels { msb_first, ..XRGB };
        let mut data = Vec::new();
        for row in &rows {
            for [red, green, blue] in row.iter().take(width as usize) {
                let pixel = u32::from(*red) << 16 | u32::from(*green) << 8 | u32::from(*blue);
                let bytes = if msb_first { pixel.to_be_bytes() } else { pixel.to_le_bytes() };
                data.extend_from_slice(&bytes);
            }
        }
        let picture = x11_rgba(&data, width, height, format).unwrap();
        for (y, row) in rows.iter().enumerate() {
            for (x, [red, green, blue]) in row.iter().take(width as usize).enumerate() {
                prop_assert_eq!(
                    picture.get_pixel(x as u32, y as u32).0,
                    [*red, *green, *blue, 255]
                );
            }
        }
    }
}

// ---------------------------------------------------------------- the Qt editor's QVariants

/// What Qt 6.12's QSettings wrote (PyQt6, `QSettings(…, IniFormat)`): the
/// keys the Qt editor kept as QVariants, and the plain `@` forms.
const QT_INI: &str = r#"[graph]
placed\b2fa61f66204c6a5=@Variant(\0\0\0\b\0\0\0\x2\0\0\0\n\0m\0o\0\x64\0\x65\0l\0\0\0\x1a@r\xc0\0\0\0\0\0@\x1d\0\0\0\0\0\0\0\0\0\x4\0n\0\x31\0\0\0\x1a@)\0\0\0\0\0\0\xc0\x44\0\0\0\0\0\0)

[misc]
at=@@home
list=a, "b, c", @@d
localTime=@DateTime(\0\0\0\x10\0\0\0\0\0\0%\x8d\xb0\x1\x18\x42\x18\x2\0\0\xe\x10)
point=@Point(-5 6)
rect=@Rect(1 2 30 40)
size=@Size(3 4)

[sidebar]
folded="@Variant(\0\0\0\b\0\0\0\x2\0\0\0\x1a\0/\0\x66\0i\0l\0m\0s\0/\0\x41\0,\0 \0\x42\0=\0\x63\0\0\0\x1\0\0\0\0\x18\0/\0\x66\0i\0l\0m\0s\0/\0\x61\0t\0l\0\x61\0s\0\0\0\x1\x1)"

[timelineTrackHeights]
3d3edcc06986072ff18d1c590cbf1d3eff00a609\all=48
3d3edcc06986072ff18d1c590cbf1d3eff00a609\path=/films/atlas
3d3edcc06986072ff18d1c590cbf1d3eff00a609\tracks\%25C3%25BC=160
3d3edcc06986072ff18d1c590cbf1d3eff00a609\tracks\V%25201=64
3d3edcc06986072ff18d1c590cbf1d3eff00a609\tracks\a%252F2=24

[updates]
check=true
checkedAt=@DateTime(\0\0\0\x10\0\0\0\0\0\0%\x8d\xb0\x1\x18\x45\x92\x3\0\0\0\x1a\0O\0\x66\0\x66\0s\0\x65\0t\0\x46\0r\0o\0m\0U\0t\0\x63\0\0\0\x6\0U\0T\0\x43\0\0\0\0\0\0\0\x6\0U\0T\0\x43\0\0\0\x6\0U\0T\0\x43\0\0\0\0\0\0\0\x6\0U\0T\0\x43)
"#;

/// And what it wrote for `QDateTime::currentDateTimeUtc()` (what the Qt
/// editor's update check kept), a time in a named zone and a local one.
const QT_INI_TIMES: &str = r#"[misc]
local=@DateTime(\0\0\0\x10\0\0\0\0\0\0%\x8e'\x2\x93.\0\0)
zoned=@DateTime(\0\0\0\x10\0\0\0\0\0\0%\x8e'\x2\x93.\0\x3\0\0\0\x1a\0\x45\0u\0r\0o\0p\0\x65\0/\0\x42\0\x65\0r\0l\0i\0n)

[updates]
checkedAt=@DateTime(\0\0\0\x10\0\0\0\0\0\0%\x8e\xb5\x4{\x1a\x83\x1)
"#;

#[test]
fn the_qt_editors_qvariants_are_read_as_qt_wrote_them() {
    let settings = qt_settings_ini(QT_INI);
    assert_eq!(
        settings["graph/placed/b2fa61f66204c6a5"],
        json!({"n1": [12.5, -40.0], "model": [300.0, 7.25]})
    );
    // Quoted (its folder has a comma and an equals sign), and still a map.
    assert_eq!(
        settings["sidebar/folded"],
        json!({"/films/atlas": true, "/films/A, B=c": false})
    );
    // 2026-03-04 05:06:07.890 in UTC (a UTC zone), and 05:06:07 at +01:00.
    assert_eq!(settings["updates/checkedAt"], json!(1_772_600_767_890_i64));
    assert_eq!(settings["misc/localTime"], json!(1_772_597_167_000_i64));
    assert_eq!(settings["misc/point"], json!([-5, 6]));
    assert_eq!(settings["misc/rect"], json!([1, 2, 30, 40]));
    assert_eq!(settings["misc/size"], json!([3, 4]));
    assert_eq!(settings["misc/at"], "@home");
    assert_eq!(settings["misc/list"], json!(["a", "b, c", "@d"]));
    assert_eq!(settings["updates/check"], "true");

    let times = qt_settings_ini(QT_INI_TIMES);
    assert_eq!(times["updates/checkedAt"], json!(1_795_207_976_579_i64));
    // A named zone's, or the local, clock is taken as UTC: 2026-07-01 12:00.
    assert_eq!(times["misc/zoned"], json!(1_782_907_200_000_i64));
    assert_eq!(times["misc/local"], json!(1_782_907_200_000_i64));
}

#[test]
fn the_qt_editors_qvariants_are_taken_up() {
    let mut prefs = Prefs::default();
    let taken = import_qt(&qt_settings_ini(QT_INI), &mut prefs);
    for key in [
        "updates/checkedAt",
        "sidebar/folded",
        "graph/placed/b2fa61f66204c6a5",
        "timelineTrackHeights",
    ] {
        assert!(taken.contains(&key.to_owned()), "{key} in {taken:?}");
    }
    assert_eq!(prefs.update_checked_ms, 1_772_600_767_890);
    assert_eq!(
        prefs.sidebar_folded,
        BTreeMap::from([
            ("/films/A, B=c".to_owned(), false),
            ("/films/atlas".to_owned(), true),
        ])
    );
    // A shot's cards, found again by the key the graph keeps them under.
    let key = "/films/atlas\nmedia shot-1";
    assert_eq!(qt_graph_key(key), "qt:b2fa61f66204c6a5");
    assert_eq!(
        prefs.graph_places_of(key),
        HashMap::from([
            ("n1".to_owned(), (12.5, -40.0)),
            ("model".to_owned(), (300.0, 7.25)),
        ])
    );
    // Moved here once they are kept: the Qt editor's go.
    let mut moved = prefs.graph_places_of(key);
    moved.insert("n1".to_owned(), (1.0, 2.0));
    prefs.keep_graph_places(key, &moved);
    assert!(!prefs.graph_places.contains_key("qt:b2fa61f66204c6a5"));
    assert_eq!(prefs.graph_places_of(key)["n1"], (1.0, 2.0));
    // The project's track heights, each track by its id.
    let heights = prefs.track_heights_of(Path::new("/films/atlas"));
    assert_eq!(heights.all, Some(48.0));
    assert_eq!(
        heights.tracks,
        BTreeMap::from([
            ("V 1".to_owned(), 64.0),
            ("a/2".to_owned(), 24.0),
            ("ü".to_owned(), 160.0),
        ])
    );
    assert_eq!(
        prefs.track_heights_of(Path::new("/elsewhere")),
        Default::default()
    );
}

#[test]
fn a_macos_plists_data_and_dates_are_read_as_qt_reads_them() {
    // QSettings on macOS keeps a QVariant it cannot store natively as the
    // UTF-8 of its `@Variant(…)` text, in a <data>; a local time as a <date>.
    let folded = qt_variant_text(
        "@Variant(",
        &qmap(&[("/Users/jun/Films/atlas", qbool(true))]),
    );
    let xml = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<plist version="1.0">
<dict>
	<key>sidebar.folded</key>
	<data>
	{}
	</data>
	<key>updates.checkedAt</key>
	<date>2026-03-04T05:06:07Z</date>
</dict>
</plist>"#,
        base64(folded.as_bytes())
    );
    let settings = qt_settings_plist(&xml);
    assert_eq!(
        settings["sidebar/folded"],
        json!({"/Users/jun/Films/atlas": true})
    );
    assert_eq!(settings["updates/checkedAt"], json!(1_772_600_767_000_i64));
    let mut prefs = Prefs::default();
    import_qt(&settings, &mut prefs);
    assert_eq!(prefs.update_checked_ms, 1_772_600_767_000);
    assert!(prefs.sidebar_folded["/Users/jun/Films/atlas"]);
}

#[test]
fn a_qvariant_it_does_not_know_is_not_read() {
    // A QColor (Qt 6's 0x1003): not known, not guessed at.
    assert_eq!(
        qt_variant(&[0, 0, 0x10, 0x03, 0, 1, 2, 3], QDATASTREAM_5_6),
        None
    );
    // Too few bytes for what it says.
    assert_eq!(qt_variant(&[0, 0, 0, 2, 0, 1], QDATASTREAM_4_0), None);
    let settings = qt_settings_ini("[a]\nkey=@Variant(\\0\\0\\x10\\x3)\n");
    assert!(!settings.contains_key("a/key"));
}

#[test]
fn a_date_further_than_ms_since_1970_count_is_not_set() {
    // A Julian day past what milliseconds since 1970 can count, as a crafted
    // or broken settings file may hold: no date, read without harm, and
    // not taken (the editor started on it every time, and stopped).
    let ini = "[updates]\ncheckedAt=@DateTime(\\0\\0\\0\\x10\\0\\x7f\\xff\\xff\\xff\\xff\\xff\\0\\0\\0\\0\\0\\x1\\x1)\n";
    let settings = qt_settings_ini(ini);
    assert_eq!(settings["updates/checkedAt"], Value::Null);
    let mut prefs = Prefs::default();
    let taken = import_qt(&settings, &mut prefs);
    assert!(!taken.contains(&"updates/checkedAt".to_owned()));
    assert_eq!(prefs.update_checked_ms, 0);
    // The other way, and a date alone (a QDate, 14).
    for day in [i64::MIN + 1, i64::MAX, 1 << 60] {
        let mut bytes = datetime_bytes(day, 0, 1);
        assert_eq!(qt_variant(&bytes, QDATASTREAM_5_6), Some(Value::Null));
        bytes.truncate(13);
        bytes[3] = 14;
        assert_eq!(qt_variant(&bytes, QDATASTREAM_5_6), Some(Value::Null));
    }
    // A plist's <date> of a year no milliseconds count stays its text.
    let xml = "<plist><dict><key>updates.checkedAt</key><date>9223372036854775807-12-31T23:59:59Z</date></dict></plist>";
    assert_eq!(
        qt_settings_plist(xml)["updates/checkedAt"],
        "9223372036854775807-12-31T23:59:59Z"
    );
    let xml = "<plist><dict><key>a</key><date>1900-01-01T00:00:00Z</date><key>b</key><date>2000-03-01T00:00:00Z</date><key>c</key><date>2000-03-01T25:00:00Z</date></dict></plist>";
    let settings = qt_settings_plist(xml);
    assert_eq!(settings["a"], json!(-2_208_988_800_000_i64));
    assert_eq!(settings["b"], json!(951_868_800_000_i64));
    assert_eq!(settings["c"], "2000-03-01T25:00:00Z");
}

/// A QDateTime at QDataStream 5.6: the type, not null, its Julian day,
/// the ms into it, and its spec (UTC 1, OffsetFromUTC 2 with its seconds).
fn datetime_bytes(day: i64, ms: u32, spec: u8) -> Vec<u8> {
    let mut bytes = 16u32.to_be_bytes().to_vec();
    bytes.push(0);
    bytes.extend_from_slice(&day.to_be_bytes());
    bytes.extend_from_slice(&ms.to_be_bytes());
    bytes.push(spec);
    bytes
}

proptest! {
    #[test]
    fn a_datetime_of_any_day_time_and_offset_reads_as_qt_takes_it(
        day in prop_oneof![any::<i64>(), 2_000_000i64..3_000_000],
        ms in prop_oneof![any::<u32>(), 0u32..86_400_000],
        offset in prop_oneof![Just(None), any::<i32>().prop_map(Some)],
    ) {
        let mut bytes = datetime_bytes(day, ms, if offset.is_some() { 2 } else { 1 });
        if let Some(offset) = offset {
            bytes.extend_from_slice(&offset.to_be_bytes());
        }
        // Qt's: a time past the day's end is its midnight; a null day none.
        let time = if ms < 86_400_000 { i128::from(ms) } else { 0 };
        let at = (i128::from(day) - 2_440_588) * 86_400_000 + time
            - i128::from(offset.unwrap_or(0)) * 1000;
        let expected = match i64::try_from(at) {
            Ok(at) if day != i64::MIN => json!(at),
            _ => Value::Null,
        };
        prop_assert_eq!(qt_variant(&bytes, QDATASTREAM_5_6), Some(expected));
    }

    #[test]
    fn a_plists_date_of_any_year_reads_without_harm(
        year in any::<i64>(),
        month in 1i64..=12,
        day in 1i64..=31,
        second in 0f64..61.0,
    ) {
        let xml = format!(
            "<plist><dict><key>a</key><date>{year:04}-{month:02}-{day:02}T12:30:{second}Z</date></dict></plist>"
        );
        let _ = qt_settings_plist(&xml);
    }
}

#[test]
fn a_qurl_in_a_variant_reads_as_its_encoded_form() {
    // A map holding a QUrl (17, its toEncoded as a QByteArray), then a
    // bool after it: both read, the stream where it should be.
    let mut bytes = 8u32.to_be_bytes().to_vec();
    bytes.extend_from_slice(&2u32.to_be_bytes());
    let key = |text: &str, bytes: &mut Vec<u8>| {
        let units: Vec<u16> = text.encode_utf16().collect();
        bytes.extend_from_slice(&(units.len() as u32 * 2).to_be_bytes());
        for unit in units {
            bytes.extend_from_slice(&unit.to_be_bytes());
        }
    };
    key("link", &mut bytes);
    bytes.extend_from_slice(&17u32.to_be_bytes());
    let url = "file:///films/at%20las";
    bytes.extend_from_slice(&(url.len() as u32).to_be_bytes());
    bytes.extend_from_slice(url.as_bytes());
    key("on", &mut bytes);
    bytes.extend_from_slice(&1u32.to_be_bytes());
    bytes.push(1);
    assert_eq!(
        qt_variant(&bytes, QDATASTREAM_4_0),
        Some(json!({"link": "file:///films/at%20las", "on": true}))
    );
    // An empty QUrl: a null QByteArray.
    let mut empty = 17u32.to_be_bytes().to_vec();
    empty.extend_from_slice(&u32::MAX.to_be_bytes());
    assert_eq!(qt_variant(&empty, QDATASTREAM_4_0), Some(json!("")));
}

/// `text` as `reg query` shows a REG_BINARY QSettings wrote it in: its
/// UTF-16, little-endian, in hex.
fn reg_binary(text: &str) -> String {
    text.encode_utf16()
        .flat_map(u16::to_le_bytes)
        .map(|byte| format!("{byte:02X}"))
        .collect()
}

const REG_KEY: &str = r"HKEY_CURRENT_USER\Software\Industrial Kerf\Industrial Kerf";

#[test]
fn the_registrys_values_are_read_as_qsettings_reads_them() {
    // QSettings on Windows keeps text holding a \0 (each @Variant and
    // @DateTime) as REG_BINARY, other text as REG_SZ, read back alike.
    let folded = qt_variant_text("@Variant(", &qmap(&[("C:/Films/atlas", qbool(false))]));
    let mut checked = "@DateTime(".to_owned();
    checked.extend(
        datetime_bytes(2_461_104, 18_367_890, 1)
            .iter()
            .map(|byte| char::from(*byte)),
    );
    checked.push(')');
    let places = qt_variant_text("@Variant(", &qmap(&[("n1", Q::PointF(12.5, -40.0))]));
    let text = format!(
        "\r\n{REG_KEY}\\updates\r\n    checkedAt    REG_BINARY    {}\r\n    check    REG_SZ    false\r\n\r\n\
         {REG_KEY}\\sidebar\r\n    folded    REG_BINARY    {}\r\n\r\n\
         {REG_KEY}\\graph\\placed\r\n    b2fa61f66204c6a5    REG_BINARY    {}\r\n\r\n\
         {REG_KEY}\\misc\r\n    at    REG_SZ    @@home\r\n    none    REG_SZ    @Invalid()\r\n    empty    REG_SZ    \r\n    \
         list    REG_MULTI_SZ    a\\0@@b\\0c\r\n    width    REG_DWORD    0x12c\r\n",
        reg_binary(&checked),
        reg_binary(&folded),
        reg_binary(&places),
    );
    let settings = qt_settings_reg(&text);
    // 2026-03-04 05:06:07.890 in UTC.
    assert_eq!(settings["updates/checkedAt"], json!(1_772_600_767_890_i64));
    assert_eq!(settings["updates/check"], "false");
    assert_eq!(settings["sidebar/folded"], json!({"C:/Films/atlas": false}));
    assert_eq!(
        settings["graph/placed/b2fa61f66204c6a5"],
        json!({"n1": [12.5, -40.0]})
    );
    assert_eq!(settings["misc/at"], "@home");
    assert_eq!(settings["misc/none"], Value::Null);
    assert_eq!(settings["misc/empty"], "");
    assert_eq!(settings["misc/list"], json!(["a", "@b", "c"]));
    assert_eq!(settings["misc/width"], json!(300));

    let mut prefs = Prefs::default();
    let taken = import_qt(&settings, &mut prefs);
    for key in [
        "updates/checkedAt",
        "updates/check",
        "graph/placed/b2fa61f66204c6a5",
    ] {
        assert!(taken.contains(&key.to_owned()), "{key} in {taken:?}");
    }
    assert_eq!(prefs.update_checked_ms, 1_772_600_767_890);
    assert!(!prefs.check_updates);
}

proptest! {
    #[test]
    fn a_qvariant_in_the_registry_reads_back(value in any_q()) {
        let text = format!(
            "{REG_KEY}\\a\r\n    key    REG_BINARY    {}\r\n",
            reg_binary(&qt_variant_text("@Variant(", &value))
        );
        prop_assert_eq!(&qt_settings_reg(&text)["a/key"], &expected(&value));
    }
}

#[test]
fn a_folder_folded_by_hand_is_kept_by_its_folder() {
    let mut prefs = Prefs::default();
    assert_eq!(prefs.folder_folded(Path::new("/films/atlas")), None);
    prefs.keep_folder_folded(Path::new("/films/atlas/"), false);
    prefs.keep_folder_folded(Path::new("/films/sea"), true);
    // By its folder, however it is written.
    assert_eq!(prefs.folder_folded(Path::new("/films/atlas")), Some(false));
    assert_eq!(
        prefs.folder_folded(Path::new("/films/x/../sea")),
        Some(true)
    );
    assert_eq!(prefs.folder_folded(Path::new("/films/else")), None);
    // Kept in the file, and read back.
    let folder = std::env::temp_dir().join(format!("kerf-folds-{}", std::process::id()));
    std::fs::create_dir_all(&folder).unwrap();
    let path = folder.join("editor.json");
    prefs.save_to(&path).unwrap();
    let read = Prefs::load_from(&path);
    assert_eq!(read.folder_folded(Path::new("/films/atlas")), Some(false));
    assert_eq!(read.folder_folded(Path::new("/films/sea")), Some(true));
    std::fs::remove_dir_all(&folder).unwrap();
    // The Qt editor's folds, taken up, are found the same way.
    let mut prefs = Prefs::default();
    import_qt(&qt_settings_ini(QT_INI), &mut prefs);
    assert_eq!(prefs.folder_folded(Path::new("/films/atlas")), Some(true));
    assert_eq!(prefs.folder_folded(Path::new("/films/A, B=c")), Some(false));
}

// ---- a QDataStream and QSettings' INI escaping, as Qt writes them, for the round trips

#[derive(Clone, Debug)]
enum Q {
    Bool(bool),
    Int(i32),
    Double(f64),
    Text(String),
    PointF(f64, f64),
    Map(BTreeMap<String, Q>),
    List(Vec<Q>),
    /// A QDateTime in UTC, ms since 1970.
    Utc(i64),
}

fn qbool(on: bool) -> Q {
    Q::Bool(on)
}

fn qmap(entries: &[(&str, Q)]) -> Q {
    Q::Map(
        entries
            .iter()
            .map(|(key, value)| ((*key).to_owned(), value.clone()))
            .collect(),
    )
}

fn qstring(text: &str, out: &mut Vec<u8>) {
    let units: Vec<u16> = text.encode_utf16().collect();
    out.extend_from_slice(&(units.len() as u32 * 2).to_be_bytes());
    for unit in units {
        out.extend_from_slice(&unit.to_be_bytes());
    }
}

/// `value` as a QVariant at QDataStream 4.0 (QSettings' `@Variant`).
fn qvariant(value: &Q, out: &mut Vec<u8>) {
    match value {
        Q::Bool(on) => {
            out.extend_from_slice(&1u32.to_be_bytes());
            out.push(u8::from(*on));
        }
        Q::Int(number) => {
            out.extend_from_slice(&2u32.to_be_bytes());
            out.extend_from_slice(&number.to_be_bytes());
        }
        Q::Double(number) => {
            out.extend_from_slice(&6u32.to_be_bytes());
            out.extend_from_slice(&number.to_be_bytes());
        }
        Q::Text(text) => {
            out.extend_from_slice(&10u32.to_be_bytes());
            qstring(text, out);
        }
        Q::PointF(x, y) => {
            out.extend_from_slice(&26u32.to_be_bytes());
            out.extend_from_slice(&x.to_be_bytes());
            out.extend_from_slice(&y.to_be_bytes());
        }
        Q::Map(map) => {
            out.extend_from_slice(&8u32.to_be_bytes());
            out.extend_from_slice(&(map.len() as u32).to_be_bytes());
            for (key, value) in map {
                qstring(key, out);
                qvariant(value, out);
            }
        }
        Q::List(items) => {
            out.extend_from_slice(&9u32.to_be_bytes());
            out.extend_from_slice(&(items.len() as u32).to_be_bytes());
            for item in items {
                qvariant(item, out);
            }
        }
        // Qt 4.0's QDateTime: a quint32 Julian day, the ms into it, and
        // QDateTimePrivate's UTC (2).
        Q::Utc(ms) => {
            out.extend_from_slice(&16u32.to_be_bytes());
            let day = ms.div_euclid(86_400_000) + 2_440_588;
            out.extend_from_slice(&(day as u32).to_be_bytes());
            out.extend_from_slice(&(ms.rem_euclid(86_400_000) as u32).to_be_bytes());
            out.push(2);
        }
    }
}

/// What a value reads back as.
fn expected(value: &Q) -> Value {
    match value {
        Q::Bool(on) => json!(on),
        Q::Int(number) => json!(number),
        Q::Double(number) => json!(number),
        Q::Text(text) => json!(text),
        Q::PointF(x, y) => json!([x, y]),
        Q::Map(map) => Value::Object(
            map.iter()
                .map(|(key, value)| (key.clone(), expected(value)))
                .collect::<Map<_, _>>(),
        ),
        Q::List(items) => Value::Array(items.iter().map(expected).collect()),
        Q::Utc(ms) => json!(ms),
    }
}

/// `prefix` + the bytes of `value`, one a character, + `)`: QSettings'
/// variantToString.
fn qt_variant_text(prefix: &str, value: &Q) -> String {
    let mut bytes = Vec::new();
    qvariant(value, &mut bytes);
    let mut text = prefix.to_owned();
    text.extend(bytes.iter().map(|byte| char::from(*byte)));
    text.push(')');
    text
}

/// QSettings' iniEscapedString.
fn ini_escaped(text: &str) -> String {
    let codec = !["@ByteArray(", "@Variant(", "@DateTime("]
        .iter()
        .any(|prefix| text.starts_with(prefix));
    let mut out = String::new();
    let mut quotes = false;
    let mut escape_digit = false;
    for ch in text.chars() {
        let code = ch as u32;
        if matches!(ch, ';' | ',' | '=') {
            quotes = true;
        }
        if escape_digit && ch.is_ascii_hexdigit() {
            out.push_str(&format!("\\x{code:x}"));
            continue;
        }
        escape_digit = false;
        match ch {
            '\0' => {
                out.push_str("\\0");
                escape_digit = true;
            }
            '\u{7}' => out.push_str("\\a"),
            '\u{8}' => out.push_str("\\b"),
            '\u{c}' => out.push_str("\\f"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\u{b}' => out.push_str("\\v"),
            '"' | '\\' => {
                out.push('\\');
                out.push(ch);
            }
            _ if code <= 0x1f || (code >= 0x7f && !codec) => {
                out.push_str(&format!("\\x{code:x}"));
                escape_digit = true;
            }
            _ => out.push(ch),
        }
    }
    if quotes || out.starts_with(' ') || out.ends_with(' ') {
        format!("\"{out}\"")
    } else {
        out
    }
}

fn base64(bytes: &[u8]) -> String {
    const ALPHABET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::new();
    for chunk in bytes.chunks(3) {
        let triple = chunk.iter().enumerate().fold(0u32, |triple, (at, byte)| {
            triple | u32::from(*byte) << (16 - 8 * at)
        });
        for at in 0..4 {
            if at <= chunk.len() {
                out.push(char::from(
                    ALPHABET[(triple >> (18 - 6 * at) & 63) as usize],
                ));
            } else {
                out.push('=');
            }
        }
    }
    out
}

fn any_q() -> impl Strategy<Value = Q> {
    let leaf = prop_oneof![
        any::<bool>().prop_map(Q::Bool),
        any::<i32>().prop_map(Q::Int),
        prop::num::f64::NORMAL.prop_map(Q::Double),
        ".{0,12}".prop_map(Q::Text),
        (-1e6f64..1e6, -1e6f64..1e6).prop_map(|(x, y)| Q::PointF(x, y)),
        (0i64..4_102_444_800_000).prop_map(Q::Utc),
    ];
    leaf.prop_recursive(3, 24, 4, |inner| {
        prop_oneof![
            prop::collection::btree_map(".{0,8}", inner.clone(), 0..4).prop_map(Q::Map),
            prop::collection::vec(inner, 0..4).prop_map(Q::List),
        ]
    })
}

proptest! {
    #[test]
    fn a_qvariant_reads_back_as_qsettings_wrote_it(value in any_q()) {
        let line = ini_escaped(&qt_variant_text("@Variant(", &value));
        let settings = qt_settings_ini(&format!("[a]\nkey={line}\n"));
        prop_assert_eq!(&settings["a/key"], &expected(&value));
    }

    #[test]
    fn a_utc_datetime_reads_back_as_qsettings_wrote_it(ms in 0i64..4_102_444_800_000) {
        // QDataStream 5.6: the type, not null, a qint64 Julian day, the ms
        // into it, Qt::UTC.
        let mut bytes = 16u32.to_be_bytes().to_vec();
        bytes.push(0);
        bytes.extend_from_slice(&(ms.div_euclid(86_400_000) + 2_440_588).to_be_bytes());
        bytes.extend_from_slice(&(ms.rem_euclid(86_400_000) as u32).to_be_bytes());
        bytes.push(1);
        prop_assert_eq!(qt_variant(&bytes, QDATASTREAM_5_6), Some(json!(ms)));
        let mut text = "@DateTime(".to_owned();
        text.extend(bytes.iter().map(|byte| char::from(*byte)));
        text.push(')');
        let settings = qt_settings_ini(&format!("[a]\nkey={}\n", ini_escaped(&text)));
        prop_assert_eq!(&settings["a/key"], &json!(ms));
    }

    #[test]
    fn a_qvariant_in_a_plists_data_reads_back(value in any_q()) {
        let text = qt_variant_text("@Variant(", &value);
        let xml = format!(
            "<plist><dict><key>a.key</key><data>{}</data></dict></plist>",
            base64(text.as_bytes())
        );
        prop_assert_eq!(&qt_settings_plist(&xml)["a/key"], &expected(&value));
    }

    #[test]
    fn any_bytes_read_as_a_qvariant_without_harm(
        bytes in prop::collection::vec(any::<u8>(), 0..64),
        version in prop_oneof![Just(QDATASTREAM_4_0), Just(QDATASTREAM_5_6)],
    ) {
        let _ = qt_variant(&bytes, version);
    }
}

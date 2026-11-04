//! The public QC check: its findings, where they are on the picture and
//! the timeline, and the summary the editor shows as rows.

use kerf_lang::{apply_script, parse_document};
use kerf_mcp::{check, CheckReport, Finding, Severity};
use kerf_model::{Catalog, Project};
use kerf_text::{layout, Align, ParagraphStyle, TextStyle};
use tempfile::TempDir;

/// A 1080p spot: a title over the whole of it, and captions with room to
/// read them, two seconds apart.
const CLEAN: &str = "kerf 1
project name=\"QC\"
sequence s1 rate=24 size=1920x1080
  track V1 video
    clip bg gen=title@1 at=00:00:00:00 dur=00:00:24:00
      set text=\"Atlas\" size=0.1
  track V2 video name=\"Lower thirds\"
  track S1 subtitle
    clip cap1 caption=\"The city is still asleep\" at=00:00:01:00 dur=00:00:04:15
    clip cap2 caption=\"First light on the valley road\" at=00:00:06:00 dur=00:00:04:12
    clip cap3 caption=\"We open before the city does\" at=00:00:16:00 dur=00:00:03:19
";

fn project(script: &str) -> Project {
    let catalog = Catalog::builtin();
    let project = parse_document(CLEAN, catalog)
        .unwrap_or_else(|error| panic!("{error:#?}"))
        .project;
    if script.is_empty() {
        return project;
    }
    apply_script(&project, catalog, script)
        .unwrap_or_else(|error| panic!("{error:#?}"))
        .project
}

fn run(project: &Project) -> CheckReport {
    let directory = TempDir::new().unwrap();
    check(project, directory.path())
}

fn with<'a>(report: &'a CheckReport, code: &str) -> Vec<&'a Finding> {
    report
        .findings
        .iter()
        .filter(|finding| finding.code == code)
        .collect()
}

fn row<'a>(report: &'a CheckReport, code: &str) -> &'a kerf_mcp::Row {
    report
        .summary
        .rows
        .iter()
        .find(|row| row.code == code)
        .unwrap_or_else(|| panic!("no row {code}: {:#?}", report.summary))
}

/// The line boxes of Inter text centred at `x` with its bottom at `bottom`.
fn inter(text: &str, size: f64, x: f64, place: f64, y: f64) -> [f64; 4] {
    let style = TextStyle {
        family: "Inter".into(),
        size: size as f32,
        ..TextStyle::default()
    };
    let paragraph = ParagraphStyle {
        align: Align::Center,
        ..ParagraphStyle::default()
    };
    let [l, t, r, b] = layout(text, &style, &paragraph).bounds.map(f64::from);
    let height = b - t;
    let top = y - height * place;
    [x + l, top, x + r, top + height]
}

#[test]
fn a_clean_project_passes_every_row() {
    let report = run(&project(""));
    assert!(report.valid, "{report:#?}");
    assert!(report.findings.is_empty(), "{:#?}", report.findings);
    let summary = &report.summary;
    assert_eq!(summary.total, kerf_mcp::qc::ROWS.len());
    assert_eq!(summary.passed, summary.total, "{summary:#?}");
    assert_eq!(summary.blocking, 0);
    for row in &summary.rows {
        assert!(row.passed && row.count == 0 && row.at.is_none(), "{row:?}");
        assert!(row.value.starts_with("0 "), "{row:?}");
    }
    for code in [
        "text.safe_area",
        "text.overlap",
        "caption.reading_speed",
        "caption.short",
        "caption.gap",
    ] {
        row(&report, code);
    }
    assert_eq!(row(&report, "text.safe_area").value, "0 outside");
    assert_eq!(row(&report, "text.safe_area").name, "Title safe 90%");
}

#[test]
fn a_caption_over_a_lower_third_overlaps_it_by_its_depth() {
    // The caption as the compositor draws it: Inter at 4.5% of the frame
    // height, its bottom at 95% of the frame.
    let caption = inter("We open before the city does", 48.6, 960.0, 1.0, 1026.0);
    // A lower third left of centre whose bottom is 8 px below the
    // caption's top. The title is centred on the frame, then moved.
    let title = inter("Atlas Coffee", 0.04 * 1080.0, 960.0, 0.5, 540.0);
    let (x, y) = (-0.18, (caption[1] + 8.0 - title[3]) / 1080.0);
    let script = format!(
        "overwrite gen=title at=00:00:15:00 dur=00:00:05:00 on=V2 as=lt1\n\
         set lt1 text=\"Atlas Coffee\" size=0.04 x={x} y={y:.7} name=\"Lower third\""
    );
    let report = run(&project(&script));
    let overlaps = with(&report, "text.overlap");
    assert_eq!(overlaps.len(), 1, "{:#?}", report.findings);
    let overlap = overlaps[0];
    assert_eq!(overlap.severity, Severity::Warning);
    assert_eq!(overlap.entity.as_deref(), Some("cap3"));
    assert_eq!(overlap.value, Some(8.0), "{overlap:?}");
    assert_eq!(overlap.unit.as_deref(), Some("px"));
    assert!(
        overlap
            .message
            .contains("caption `cap3` overlaps title \"Lower third\" (`lt1`) by 8 px"),
        "{}",
        overlap.message
    );
    // While both show: the caption's 16:00 to its end at 19:19.
    assert_eq!(overlap.sequence.as_deref(), Some("s1"));
    assert_eq!(overlap.at, Some(16.0));
    assert!((overlap.duration.unwrap() - (3.0 + 19.0 / 24.0)).abs() < 1e-9);
    assert_eq!(overlap.timecode.as_deref(), Some("00:00:16:00"));
    // The intersection, in fractions of the frame.
    let shifted = [title[0] + x * 1920.0, title[2] + x * 1920.0];
    let rect = overlap.rect.unwrap();
    let close = |a: f64, b: f64| (a - b).abs() < 0.01 / 1080.0;
    assert!(
        close(rect.x, shifted[0].max(caption[0]) / 1920.0),
        "{rect:?}"
    );
    assert!(
        close(
            rect.w,
            (shifted[1].min(caption[2]) - shifted[0].max(caption[0])) / 1920.0
        ),
        "{rect:?}"
    );
    assert!(close(rect.y, caption[1] / 1080.0), "{rect:?}");
    assert!(close(rect.h, 8.0 / 1080.0), "{rect:?}");

    let overlap_row = row(&report, "text.overlap");
    assert!(!overlap_row.passed);
    assert_eq!(overlap_row.value, "1 at 00:00:16:00");
    assert_eq!(overlap_row.at, Some(16.0));
    assert_eq!(overlap_row.count, 1);
    assert_eq!(overlap_row.blocking, 0);
    assert_eq!(report.summary.passed, report.summary.total - 1);
    // A warning, not a block: the cut still delivers.
    assert!(report.valid);

    // Raised clear of the caption, nothing overlaps.
    let lifted = format!("{script}\nset lt1 y={:.7}", y - 12.0 / 1080.0);
    let report = run(&project(&lifted));
    assert!(
        with(&report, "text.overlap").is_empty(),
        "{:#?}",
        report.findings
    );
    assert!(row(&report, "text.overlap").passed);
}

#[test]
fn fast_short_and_crowded_captions_are_flagged() {
    // 43 characters in a second and a half; a caption up for half a
    // second (which also reads too fast); one that starts a frame after
    // the one before ends.
    let script = "set cap1 caption=\"Every cup by hand, since nineteen sixty-two\"\n\
                  trim cap1 out to=00:00:02:12\n\
                  trim cap2 out to=00:00:06:12\n\
                  overwrite caption=\"Roasted\" at=00:00:06:13 dur=00:00:02:00 on=S1 as=cap4";
    let report = run(&project(script));
    let fast = with(&report, "caption.reading_speed");
    assert_eq!(fast.len(), 2, "{:#?}", report.findings);
    assert_eq!(fast[0].entity.as_deref(), Some("cap1"));
    assert_eq!(fast[1].entity.as_deref(), Some("cap2"));
    assert_eq!(fast[1].value, Some(60.0));
    assert_eq!(fast[0].value, Some(28.7), "{:?}", fast[0]);
    assert_eq!(fast[0].limit, Some(17.0));
    assert_eq!(fast[0].unit.as_deref(), Some("cps"));
    assert_eq!(fast[0].at, Some(1.0));
    assert_eq!(fast[0].duration, Some(1.5));

    let short = with(&report, "caption.short");
    assert_eq!(short.len(), 1, "{:#?}", report.findings);
    assert_eq!(short[0].entity.as_deref(), Some("cap2"));
    assert_eq!(short[0].value, Some(0.5));
    assert_eq!(short[0].limit, Some(0.833));

    let gaps = with(&report, "caption.gap");
    assert_eq!(gaps.len(), 1, "{:#?}", report.findings);
    assert_eq!(gaps[0].entity.as_deref(), Some("cap4"));
    assert_eq!(gaps[0].value, Some(1.0));
    assert_eq!(gaps[0].timecode.as_deref(), Some("00:00:06:12"));
    assert!((gaps[0].duration.unwrap() - 1.0 / 24.0).abs() < 1e-9);

    for code in ["caption.reading_speed", "caption.short", "caption.gap"] {
        assert!(!row(&report, code).passed, "{code}");
    }
    assert_eq!(
        row(&report, "caption.reading_speed").value,
        "2 · first at 00:00:01:00"
    );
    assert_eq!(
        row(&report, "caption.reading_speed").limit.as_deref(),
        Some("≤ 17 cps")
    );
    // Back to back is fine.
    let chained = run(&project(
        "trim cap2 out to=00:00:06:12\noverwrite caption=\"Roasted\" at=00:00:10:12 dur=00:00:02:00 on=S1 as=cap4",
    ));
    assert!(
        with(&chained, "caption.gap").is_empty(),
        "{:#?}",
        chained.findings
    );
}

#[test]
fn text_outside_title_safe_is_flagged() {
    // A title pushed toward the top edge, and one far too wide.
    let script = "overwrite gen=title at=00:00:02:00 dur=00:00:02:00 on=V2 as=t2\n\
                  set t2 text=\"Top\" size=0.1 y=-0.47";
    let report = run(&project(script));
    let outside = with(&report, "text.safe_area");
    assert_eq!(outside.len(), 1, "{:#?}", report.findings);
    let finding = outside[0];
    assert_eq!(finding.entity.as_deref(), Some("t2"));
    assert!(finding.value.unwrap() > 0.0);
    assert_eq!(finding.at, Some(2.0));
    assert_eq!(finding.rect.unwrap().y, 0.0, "clipped to the frame");
    assert!(!row(&report, "text.safe_area").passed);
    assert_eq!(row(&report, "text.safe_area").value, "1 at 00:00:02:00");

    let wide = run(&project(
        "set cap1 caption=\"A caption that runs on and on across the whole of the frame and past its edges\"",
    ));
    let outside = with(&wide, "text.safe_area");
    assert!(
        outside
            .iter()
            .any(|finding| finding.entity.as_deref() == Some("cap1")),
        "{:#?}",
        wide.findings
    );
}

#[test]
fn the_check_is_deterministic_and_keeps_the_mcp_shape() {
    let script = "set cap1 caption=\"Every cup by hand, since nineteen sixty-two\"\n\
                  trim cap1 out to=00:00:02:12\n\
                  overwrite gen=title at=00:00:16:00 dur=00:00:02:00 on=V2 as=lt1\n\
                  set lt1 text=\"Atlas Coffee\" size=0.08 y=0.4";
    let project = project(script);
    let first = run(&project);
    let second = run(&project);
    assert_eq!(first, second);
    assert_eq!(
        serde_json::to_string(&first).unwrap(),
        serde_json::to_string(&second).unwrap()
    );
    assert!(!first.findings.is_empty());

    // MCP clients keep reading valid and findings, each with severity,
    // code, entity, and message.
    let json = first.to_json();
    assert_eq!(json["valid"], true);
    assert!(json.get("summary").is_none());
    for finding in json["findings"].as_array().unwrap() {
        for key in ["severity", "code", "entity", "message"] {
            assert!(finding.get(key).is_some(), "{key}: {finding}");
        }
        assert_eq!(finding["severity"], "warning");
    }
    // The full report carries the summary for the editor.
    let full = serde_json::to_value(&first).unwrap();
    assert_eq!(full["summary"]["total"], kerf_mcp::qc::ROWS.len());
    assert!(full["summary"]["rows"][0].get("name").is_some());
}

#[test]
fn missing_media_blocks_and_is_counted() {
    let script =
        "media add m1 path=\"/nowhere/shot.mov\" rate=24 dur=00:00:10:00 video=1920x1080\n\
                  track add V0 video before=V1\n\
                  overwrite media=m1 at=00:00:00:00 dur=00:00:02:00 on=V0 as=v0";
    let report = run(&project(script));
    assert!(!report.valid);
    let missing = row(&report, "media");
    assert!(!missing.passed);
    assert_eq!(missing.blocking, 1);
    assert_eq!(
        report.summary.blocking,
        report
            .summary
            .rows
            .iter()
            .map(|row| row.blocking)
            .sum::<usize>()
    );
    assert!(report.summary.blocking >= 1);
}

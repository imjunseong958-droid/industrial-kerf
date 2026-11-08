//! Who draws what the catalog offers: every effect and generator at its defaults, every look and
//! grade, and every transition, each on a clip of footage, asked of `why_on_cpu`; and what of it the
//! looks, grades, and transitions reach. Looks, grades, and transitions all stay on the GPU but Morph
//! Cut, which the test holds; the rest it lists.
//!
//! `KERF_GPU=software cargo test -p kerf-compose --test catalog_cost -- --nocapture` prints the list
//! with no GPU but Mesa's llvmpipe, as CI runs it; with no GPU at all it says so and passes.

use kerf_compose::{Accel, ComposeOptions, Compositor};
use kerf_lang::parse_document;
use kerf_model::look::{Line, On};
use kerf_model::{Catalog, Domain, FxKind, Rate};
use std::collections::BTreeMap;
use std::path::Path;
use std::process::Command;

fn footage(dir: &Path) -> String {
    let path = dir.join("footage.mp4");
    let output = Command::new(std::env::var("KERF_FFMPEG").unwrap_or_else(|_| "ffmpeg".to_owned()))
        .args(["-hide_banner", "-loglevel", "error", "-y", "-f", "lavfi"])
        .args(["-i", "testsrc2=s=320x180:r=24:d=1"])
        .args(["-c:v", "libx264", "-crf", "12", "-pix_fmt", "yuv420p"])
        .arg(&path)
        .output()
        .expect("ffmpeg runs");
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    path.display().to_string()
}

/// Why the CPU draws the middle of `body`, after `media`; none where the GPU does.
fn why(media: &str, body: &str) -> Result<Vec<String>, String> {
    let document = format!("{media}{body}");
    let project = parse_document(&document, Catalog::builtin()).map_err(|errors| format!("{errors:?}"))?.project;
    let compositor = Compositor::new(
        &project,
        Catalog::builtin(),
        &ComposeOptions { accel: Accel::Always, ..ComposeOptions::default() },
    )
    .map_err(|error| error.to_string())?;
    Ok(compositor.why_on_cpu(Rate::FPS_24.time_of(11), 1.0))
}

const CLIP: &str = "  track V1 video\n    clip c1 media=m src=0f at=0f dur=24f\n";
const CUT: &str = "  track V1 video\n    clip c1 media=m src=0f at=0f dur=12f\n    clip c2 media=m src=6f at=12f dur=12f\n";

#[test]
fn what_the_catalog_sends_to_the_cpu() {
    let dir = tempfile::tempdir().expect("a folder");
    let media = format!(
        "kerf 1\nproject name=\"t\"\nmedia m path=\"{}\" rate=24 dur=24f video=320x180\nsequence s1 rate=24 size=640x360 audio=48000\n",
        footage(dir.path())
    );
    let probe = why(&media, &format!("{CLIP}      fx color.basic exposure=0.3\n")).expect("a frame");
    if probe.iter().any(|line| line == "no GPU on this machine") {
        // Asked for llvmpipe, as CI is, a missing driver is a failure, not a pass.
        let software = std::env::var_os("KERF_GPU").is_some_and(|value| value == "software");
        assert!(!software, "KERF_GPU=software, but no GPU or software driver was found");
        eprintln!("no GPU on this machine");
        return;
    }
    let catalog = Catalog::builtin();
    // What sends a frame to the CPU, by id, and why.
    let mut on_cpu: BTreeMap<String, String> = BTreeMap::new();
    let mut unread = Vec::new();
    let title_looks: Vec<String> = catalog
        .looks()
        .filter(|look| look.on == On::Title)
        .map(|look| look.id.to_string())
        .collect();
    for def in catalog.all().filter(|def| def.domain == Domain::Video) {
        let id = format!("{}@{}", def.id, def.version);
        let body = match def.kind {
            // Titles' own looks go on a title, and an outline on a shape.
            FxKind::Effect if title_looks.contains(&def.id.to_string()) => format!(
                "  track V1 video\n    clip g1 gen=title@1 at=0f dur=24f\n      set text=\"Kerf\"\n      fx {id}\n"
            ),
            FxKind::Effect if def.id.as_str() == "shape.outline" => format!(
                "  track V1 video\n    clip g1 gen=shape@1 at=0f dur=24f\n      set kind=ellipse width=0.6 height=0.9 fill=#ff8000\n      fx {id}\n"
            ),
            FxKind::Effect => format!("{CLIP}      fx {id}\n"),
            FxKind::Transition => format!("{CUT}    transition t1 from=c1 to=c2 fx={id} dur=12f\n"),
            FxKind::Generator => format!("  track V1 video\n    clip g1 gen={id} at=0f dur=24f\n"),
        };
        match why(&media, &body) {
            Ok(why) if why.is_empty() => {}
            Ok(why) => {
                on_cpu.insert(def.id.to_string(), why.join("; "));
            }
            Err(error) => unread.push(format!("{id}: {error}")),
        }
    }
    // Every look and grade, as it is applied, and what each reaches of the CPU's.
    let mut looks_on_cpu = Vec::new();
    let mut reached: BTreeMap<String, (Vec<String>, Vec<String>)> = BTreeMap::new();
    for look in catalog.looks() {
        let grade = look.id.as_str().starts_with("look.grade.");
        let body = match look.on {
            On::Clip => format!("{CLIP}      fx {}\n", look.id),
            On::Title => format!("  track V1 video\n    clip g1 gen=title@1 at=0f dur=24f\n      set text=\"Kerf\"\n      fx {}\n", look.id),
            On::Transition => format!("{CUT}    transition t1 from=c1 to=c2 fx={} dur=12f\n", look.id),
        };
        match why(&media, &body) {
            Ok(why) if why.is_empty() => {}
            Ok(why) => looks_on_cpu.push(format!("{}: {}", look.id, why.join("; "))),
            Err(error) => unread.push(format!("{}: {error}", look.id)),
        }
        for line in &look.lines {
            if let Line::Fx { fx, .. } = line {
                if on_cpu.contains_key(fx.as_str()) {
                    let entry = reached.entry(fx.to_string()).or_default();
                    if grade { &mut entry.1 } else { &mut entry.0 }.push(look.id.to_string());
                }
            }
        }
    }
    let video = catalog.all().filter(|def| def.domain == Domain::Video).count();
    println!("{} of {video} video effects, generators, and transitions send a frame to the CPU at their defaults:", on_cpu.len());
    for (id, why) in &on_cpu {
        let (looks, grades) = reached.get(id).cloned().unwrap_or_default();
        println!("  {id}: reached by {} looks {looks:?}, {} grades {grades:?}; {why}", looks.len(), grades.len());
    }
    println!("{} of {} looks and grades go to the CPU: {looks_on_cpu:?}", looks_on_cpu.len(), catalog.looks().count());
    assert!(unread.is_empty(), "{}", unread.join("\n"));
    assert!(looks_on_cpu.is_empty(), "{}", looks_on_cpu.join("\n"));
    let transitions: Vec<&String> = on_cpu
        .keys()
        .filter(|id| catalog.all().any(|def| def.kind == FxKind::Transition && def.id.as_str() == id.as_str()))
        .collect();
    assert_eq!(transitions, ["morph-cut"], "transitions on the CPU");
    assert!(reached.is_empty(), "looks reach the CPU: {reached:?}");
}

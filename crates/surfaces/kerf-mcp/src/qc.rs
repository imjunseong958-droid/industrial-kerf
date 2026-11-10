//! Quality control: what stands between a project and its delivery, as a
//! report the editor shows as a table of checks ("6 of 8 pass") and an MCP
//! agent reads as a list of findings.
//!
//! [`check`] runs every check that needs no rendering: media that cannot be
//! read, features the renderer cannot lower, the model's warnings, and what
//! an editor would catch in the cut (see `lint`). Each check is one row of
//! the [`Summary`], whether it passes or not.

use crate::lint;
use kerf_lang::Clock;
use kerf_model::{Catalog, Content, Media, Project, Sequence, Time};
use kerf_runtime::CachedFacts;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::path::Path;

/// How serious a finding is. Errors block delivery.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    Error,
    Warning,
    Info,
}

impl Severity {
    pub fn is_blocking(self) -> bool {
        self == Self::Error
    }

    /// Whether a finding of this severity fails its row.
    pub fn fails(self) -> bool {
        self != Self::Info
    }
}

/// A rectangle on the picture, in fractions of the frame (0,0 top left).
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Rect {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

/// One thing a check found.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Finding {
    pub severity: Severity,
    /// Stable machine-readable code, such as `text.overlap`.
    pub code: String,
    /// The clip, media, or sequence the finding is about.
    pub entity: Option<String>,
    pub message: String,
    /// The sequence the finding is on, when it is on one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sequence: Option<String>,
    /// Where it starts on its sequence, in seconds from the sequence's
    /// first frame.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub at: Option<f64>,
    /// How long it lasts, in seconds.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duration: Option<f64>,
    /// `at` as its sequence's timecode.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timecode: Option<String>,
    /// Where on the picture, in fractions of the frame.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rect: Option<Rect>,
    /// What was measured, in `unit`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value: Option<f64>,
    /// The most (or least) the check allows, in `unit`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub unit: Option<String>,
}

impl Finding {
    pub fn new(
        severity: Severity,
        code: impl Into<String>,
        entity: Option<&str>,
        message: impl Into<String>,
    ) -> Self {
        Self {
            severity,
            code: code.into(),
            entity: entity.map(str::to_owned),
            message: message.into(),
            sequence: None,
            at: None,
            duration: None,
            timecode: None,
            rect: None,
            value: None,
            limit: None,
            unit: None,
        }
    }

    pub fn warning(code: &str, entity: &str, message: impl Into<String>) -> Self {
        Self::new(Severity::Warning, code, Some(entity), message)
    }

    /// The finding placed on `sequence` from `from` to `to`.
    pub fn during(mut self, sequence: &Sequence, from: Time, to: Time) -> Self {
        let clock = Clock::of_sequence(sequence);
        self.sequence = Some(sequence.id.to_string());
        self.at = Some(from.seconds());
        self.duration = Some((to - from).seconds().max(0.0));
        self.timecode = Some(clock.print_position(clock.rate.round(from)));
        self
    }

    /// The finding with a measurement against a limit.
    pub fn measured(mut self, value: f64, limit: f64, unit: &str) -> Self {
        self.value = Some(value);
        self.limit = Some(limit);
        self.unit = Some(unit.to_owned());
        self
    }

    pub fn on(mut self, rect: Rect) -> Self {
        self.rect = Some(rect);
        self
    }
}

/// One row of the summary: a check, and whether it passed.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Row {
    pub code: String,
    /// What the editor calls the check, such as `Title safe 90%`.
    pub name: String,
    pub passed: bool,
    /// What it found, for the table: `0 outside`, `1 at 00:00:17:08`.
    pub value: String,
    /// What it allows, for the table: `≤ 17 cps`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<String>,
    /// Where its first failing finding starts, in seconds.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub at: Option<f64>,
    /// How many findings fail it.
    pub count: usize,
    /// How many of those block delivery.
    pub blocking: usize,
}

/// The checks at a glance.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Summary {
    /// Rows that pass.
    pub passed: usize,
    /// Rows.
    pub total: usize,
    /// Findings that block delivery.
    pub blocking: usize,
    pub rows: Vec<Row>,
}

impl Summary {
    /// Count the rows again after rows were added or changed.
    pub fn recount(&mut self) {
        self.total = self.rows.len();
        self.passed = self.rows.iter().filter(|row| row.passed).count();
        self.blocking = self.rows.iter().map(|row| row.blocking).sum();
    }
}

/// Everything [`check`] found.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CheckReport {
    /// Nothing blocks delivery.
    pub valid: bool,
    pub findings: Vec<Finding>,
    pub summary: Summary,
}

impl CheckReport {
    /// A report of `findings`, summarised in `rows`. A finding goes to the
    /// first row that lists its code, or else to the row that lists none.
    pub fn new(findings: Vec<Finding>, rows: &[RowDef]) -> Self {
        let mut summary = Summary {
            rows: rows.iter().map(|def| def.row(&findings, rows)).collect(),
            ..Summary::default()
        };
        summary.recount();
        let valid = !findings
            .iter()
            .any(|finding| finding.severity.is_blocking());
        Self {
            valid,
            findings,
            summary,
        }
    }

    /// The report as MCP tools have always returned it: `valid` and the
    /// findings, each with its severity, code, entity, and message (and,
    /// when known, where it is and what was measured).
    pub fn to_json(&self) -> Value {
        json!({"valid": self.valid, "findings": self.findings})
    }
}

/// A row of the summary: the codes its findings carry, and how it reads.
#[derive(Clone, Copy, Debug)]
pub struct RowDef {
    pub code: &'static str,
    pub name: &'static str,
    /// Finding codes that fail this row. Empty takes every code no other
    /// row lists.
    pub codes: &'static [&'static str],
    /// What one finding is, after a count: `0 outside`.
    pub noun: &'static str,
    pub limit: Option<&'static str>,
}

impl RowDef {
    fn takes(&self, code: &str, rows: &[RowDef]) -> bool {
        if self.codes.is_empty() {
            !rows.iter().any(|row| row.codes.contains(&code))
        } else {
            self.codes.contains(&code)
        }
    }

    fn row(&self, findings: &[Finding], rows: &[RowDef]) -> Row {
        let failing: Vec<&Finding> = findings
            .iter()
            .filter(|finding| finding.severity.fails() && self.takes(&finding.code, rows))
            .collect();
        let first = failing.first();
        let value = match (failing.len(), first.and_then(|f| f.timecode.as_deref())) {
            (0, _) => format!("0 {}", self.noun),
            (1, Some(timecode)) => format!("1 at {timecode}"),
            (count, Some(timecode)) => format!("{count} · first at {timecode}"),
            (count, None) => format!("{count} {}", self.noun),
        };
        Row {
            code: self.code.to_owned(),
            name: self.name.to_owned(),
            passed: failing.is_empty(),
            value,
            limit: self.limit.map(str::to_owned),
            at: first.and_then(|finding| finding.at),
            count: failing.len(),
            blocking: failing
                .iter()
                .filter(|finding| finding.severity.is_blocking())
                .count(),
        }
    }
}

/// The rows [`check`] reports, in the order the editor lists them. Checks
/// that need the program rendered or measured (loudness, true peak, gamut,
/// luma levels, flashing) are in [`crate::FULL_ROWS`], which
/// [`crate::check_full`] reports.
pub const ROWS: &[RowDef] = &[
    MEDIA,
    RENDER,
    DOCUMENT,
    BLACK_FRAMES,
    FRAME_EDGES,
    FLASH_FRAMES,
    CUTS_IN_WORDS,
    TITLE_SAFE,
    CAPTION_OVERLAP,
    READING_SPEED_ROW,
    CAPTION_DURATION,
    CAPTION_GAPS,
];

pub(crate) const MEDIA: RowDef = RowDef {
    code: "media",
    name: "Media online",
    codes: &["missing_media"],
    noun: "missing",
    limit: None,
};

pub(crate) const RENDER: RowDef = RowDef {
    code: "render",
    name: "Renders",
    codes: &["unrenderable"],
    noun: "unrenderable",
    limit: None,
};

pub(crate) const DOCUMENT: RowDef = RowDef {
    code: "document",
    name: "Document",
    codes: &[],
    noun: "problems",
    limit: None,
};

pub(crate) const BLACK_FRAMES: RowDef = RowDef {
    code: "picture.black",
    name: "Black frames",
    codes: &["picture.black"],
    noun: "black",
    limit: None,
};

pub(crate) const FRAME_EDGES: RowDef = RowDef {
    code: "picture.edges",
    name: "Frame edges",
    codes: &["picture.edges"],
    noun: "uncovered",
    limit: None,
};

pub(crate) const FLASH_FRAMES: RowDef = RowDef {
    code: "clip.flash",
    name: "Flash frames",
    codes: &["clip.flash", "cut.near_scene_cut"],
    noun: "flash frames",
    limit: Some("≥ 3 frames"),
};

pub(crate) const CUTS_IN_WORDS: RowDef = RowDef {
    code: "cut.in_word",
    name: "Cuts in words",
    codes: &["cut.in_word"],
    noun: "in words",
    limit: None,
};

pub(crate) const TITLE_SAFE: RowDef = RowDef {
    code: "text.safe_area",
    name: "Title safe 90%",
    codes: &["text.safe_area"],
    noun: "outside",
    limit: Some("90%"),
};

pub(crate) const CAPTION_OVERLAP: RowDef = RowDef {
    code: "text.overlap",
    name: "Caption overlap",
    codes: &["text.overlap"],
    noun: "overlapping",
    limit: None,
};

pub(crate) const READING_SPEED_ROW: RowDef = RowDef {
    code: "caption.reading_speed",
    name: "Caption reading speed",
    codes: &["caption.reading_speed"],
    noun: "too fast",
    limit: Some("≤ 17 cps"),
};

pub(crate) const CAPTION_DURATION: RowDef = RowDef {
    code: "caption.short",
    name: "Caption duration",
    codes: &["caption.short"],
    noun: "too short",
    limit: Some("≥ 5/6 s"),
};

pub(crate) const CAPTION_GAPS: RowDef = RowDef {
    code: "caption.gap",
    name: "Caption gaps",
    codes: &["caption.gap"],
    noun: "too close",
    limit: Some("≥ 2 frames or none"),
};

/// What stands between a project and its render: media that cannot be
/// read, features the renderer cannot lower, and the model's warnings;
/// and what an editor would catch in the cut. `project` has its media paths
/// resolved; `dir` is the project, whose transcripts show where words are.
pub fn check(project: &Project, dir: &Path) -> CheckReport {
    let mut findings = Vec::new();
    for media in used_media(project) {
        if !Path::new(&media.path).is_file() {
            findings.push(Finding::new(
                Severity::Error,
                "missing_media",
                Some(media.id.as_str()),
                format!("media `{}` is missing: {}", media.id, media.path),
            ));
        }
    }
    for sequence in &project.sequences {
        let id = sequence.id.as_str();
        if let Err(error) = kerf_engine::render::renderable(project, Some(id)) {
            let mut finding = Finding::new(
                Severity::Error,
                "unrenderable",
                Some(error.entity().unwrap_or(id)),
                error.to_string(),
            );
            finding.sequence = Some(id.to_owned());
            findings.push(finding);
        }
    }
    for warning in kerf_model::validate(project, Catalog::builtin()) {
        findings.push(Finding::new(
            if warning.is_error() {
                Severity::Error
            } else {
                Severity::Warning
            },
            warning.code,
            warning.entity.as_ref().map(|id| id.as_str()),
            warning.message,
        ));
    }
    findings.extend(lint::findings(project, &CachedFacts::new(dir)));
    for finding in &mut findings {
        place(project, finding);
    }
    CheckReport::new(findings, ROWS)
}

/// Put a finding about a clip where the clip is, when nothing placed it.
fn place(project: &Project, finding: &mut Finding) {
    if finding.at.is_some() {
        return;
    }
    let Some(entity) = finding.entity.as_deref() else {
        return;
    };
    let found = project.sequences.iter().find_map(|sequence| {
        sequence
            .tracks
            .iter()
            .find_map(|track| track.clip(entity))
            .map(|clip| (sequence, clip))
    });
    if let Some((sequence, clip)) = found {
        let placed = finding.clone().during(sequence, clip.start, clip.end());
        *finding = placed;
    }
}

/// Media that some clip uses.
pub(crate) fn used_media(project: &Project) -> impl Iterator<Item = &Media> {
    project.media.iter().filter(|media| {
        project.sequences.iter().any(|sequence| {
            sequence.tracks.iter().any(|track| {
                track.clips.iter().any(|clip| {
                    matches!(&clip.content, Content::Media { media: id, .. } if *id == media.id)
                })
            })
        })
    })
}

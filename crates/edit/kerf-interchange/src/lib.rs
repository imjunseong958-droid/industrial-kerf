//! Project interchange: the timeline files another editor opens.
//!
//! Two are written here, in Rust, with nothing else to install: an
//! OpenTimelineIO `.otio` file (`otio_json`, which also reads one) and Final
//! Cut Pro XML (`fcpxml`). [`export`] writes either and returns, with the
//! file, the list of what it does not carry; `otio` converts between a
//! project and the timeline an `.otio` file holds.

pub mod fcpxml;
pub mod otio;
pub mod otio_json;

use kerf_model::Project;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::fmt;
use std::io;
use std::path::{Path, PathBuf};

/// The adapter name of an `.otio` file, which is written and read here.
pub const NATIVE_OTIO: &str = "otio_json";

/// A timeline file written here, in Rust.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Format {
    /// OpenTimelineIO JSON, an `.otio` file: DaVinci Resolve and Premiere
    /// Pro import it.
    Otio,
    /// Final Cut Pro XML, an `.fcpxml` file: Final Cut Pro and DaVinci
    /// Resolve import it.
    Fcpxml,
}

impl Format {
    pub const ALL: [Self; 2] = [Self::Otio, Self::Fcpxml];

    /// Its name, which is also its file extension.
    pub const fn name(self) -> &'static str {
        match self {
            Self::Otio => "otio",
            Self::Fcpxml => "fcpxml",
        }
    }

    /// The name [`ExportResult::adapter`] reports it under.
    pub const fn adapter(self) -> &'static str {
        match self {
            Self::Otio => NATIVE_OTIO,
            Self::Fcpxml => "fcpxml",
        }
    }

    /// The format `name` asks for: `otio` (or `json`, `otio_json`), `fcpxml`.
    pub fn parse(name: &str) -> Option<Self> {
        match name.to_ascii_lowercase().as_str() {
            "otio" | "json" | "otio_json" | "otio-json" => Some(Self::Otio),
            "fcpxml" => Some(Self::Fcpxml),
            _ => None,
        }
    }

    /// The format a file's extension names.
    pub fn of_path(path: &Path) -> Option<Self> {
        let extension = path.extension()?.to_str()?;
        Self::ALL
            .into_iter()
            .find(|format| format.name().eq_ignore_ascii_case(extension))
    }
}

impl fmt::Display for Format {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.name())
    }
}

/// `sequence` of `project` as the text of a `format` file, and what the file
/// leaves out.
pub fn render(
    project: &Project,
    sequence: &kerf_model::Sequence,
    format: Format,
) -> Result<(String, Vec<InterchangeIssue>), InterchangeError> {
    match format {
        Format::Otio => {
            let (timeline, issues) = otio::sequence_timeline(project, sequence);
            Ok((otio_json::write(&timeline)?, issues))
        }
        Format::Fcpxml => Ok(fcpxml::to_fcpxml(project, sequence)),
    }
}

/// Write a sequence of `project`, its first unless `sequence` names one, to
/// `output_path` as `format`. The report lists what the file does not carry.
pub fn export(
    project: &Project,
    sequence: Option<&str>,
    output_path: &Path,
    format: Format,
) -> Result<ExportResult, InterchangeError> {
    validate_path(output_path, "outputPath")?;
    let chosen = match sequence {
        Some(id) => project.sequence(id).ok_or_else(|| {
            InterchangeError::new(
                InterchangeCode::ProjectInvalid,
                format!("the project has no sequence `{id}`"),
            )
        })?,
        None => project.sequences.first().ok_or_else(|| {
            InterchangeError::new(
                InterchangeCode::ProjectInvalid,
                "the project has no sequence",
            )
        })?,
    };
    let (text, issues) = render(project, chosen, format)?;
    let failed = |error: io::Error| {
        InterchangeError::new(
            InterchangeCode::IoFailed,
            format!("could not write {}: {error}", output_path.display()),
        )
    };
    if let Some(folder) = output_path
        .parent()
        .filter(|folder| !folder.as_os_str().is_empty())
    {
        std::fs::create_dir_all(folder).map_err(failed)?;
    }
    std::fs::write(output_path, text).map_err(failed)?;
    Ok(ExportResult {
        output_path: output_path.to_path_buf(),
        adapter: format.adapter().to_owned(),
        report: create_report("export", Some(format.adapter()), issues),
    })
}

/// Read an `.otio` file as a one-sequence project. The report lists what of
/// the file was not imported and what was assumed.
pub fn import_otio(input_path: &Path) -> Result<ImportResult, InterchangeError> {
    validate_path(input_path, "inputPath")?;
    let text = std::fs::read_to_string(input_path).map_err(|error| {
        InterchangeError::new(
            InterchangeCode::IoFailed,
            format!("could not read {}: {error}", input_path.display()),
        )
    })?;
    let (timeline, read) = otio_json::read(&text)?;
    let (project, converted) = otio::from_timeline(&timeline)?;
    Ok(ImportResult {
        project,
        input_path: input_path.to_path_buf(),
        adapter: Some(NATIVE_OTIO.to_owned()),
        report: create_report(
            "import",
            Some(NATIVE_OTIO),
            read.into_iter().chain(converted),
        ),
    })
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InterchangeIssue {
    pub code: String,
    pub message: String,
    pub severity: IssueSeverity,
    pub path: Option<String>,
    pub details: Option<Value>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum IssueSeverity {
    Warning,
    Loss,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InterchangeReport {
    pub direction: String,
    pub adapter: Option<String>,
    pub issues: Vec<InterchangeIssue>,
    pub warnings: Vec<InterchangeIssue>,
    pub losses: Vec<InterchangeIssue>,
    pub omissions: Vec<InterchangeIssue>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ImportResult {
    pub project: Project,
    pub input_path: PathBuf,
    pub adapter: Option<String>,
    pub report: InterchangeReport,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ExportResult {
    pub output_path: PathBuf,
    pub adapter: String,
    pub report: InterchangeReport,
}

#[derive(Debug)]
pub struct InterchangeError {
    pub code: InterchangeCode,
    pub message: String,
}

impl InterchangeError {
    pub(crate) fn new(code: InterchangeCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }
}

/// Which failure an [`InterchangeError`] is. Its name ([`InterchangeCode::as_str`]) is what
/// the error prints before its message.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InterchangeCode {
    InputInvalid,
    IoFailed,
    OutputInvalid,
    PathInvalid,
    ProjectInvalid,
}

impl InterchangeCode {
    // Two codes with one name would be two arms the same.
    #[deny(clippy::match_same_arms)]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::InputInvalid => "INTERCHANGE_INPUT_INVALID",
            Self::IoFailed => "INTERCHANGE_IO_FAILED",
            Self::OutputInvalid => "INTERCHANGE_OUTPUT_INVALID",
            Self::PathInvalid => "INTERCHANGE_PATH_INVALID",
            Self::ProjectInvalid => "INTERCHANGE_PROJECT_INVALID",
        }
    }
}

impl fmt::Display for InterchangeCode {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl fmt::Display for InterchangeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}: {}", self.code, self.message)
    }
}

impl std::error::Error for InterchangeError {}

pub fn validate_path(path: &Path, field: &str) -> Result<(), InterchangeError> {
    if path.as_os_str().is_empty() || path.as_os_str().as_encoded_bytes().contains(&0) {
        return Err(InterchangeError::new(
            InterchangeCode::PathInvalid,
            format!("{field} must be a non-empty path without NUL bytes"),
        ));
    }
    Ok(())
}

pub fn issue(
    code: impl Into<String>,
    message: impl Into<String>,
    severity: IssueSeverity,
    path: Option<&str>,
) -> InterchangeIssue {
    InterchangeIssue {
        code: code.into(),
        message: message.into(),
        severity,
        path: path.map(str::to_owned),
        details: None,
    }
}

pub fn create_report(
    direction: &str,
    adapter: Option<&str>,
    issues: impl IntoIterator<Item = InterchangeIssue>,
) -> InterchangeReport {
    let mut deduped = Vec::new();
    for candidate in issues {
        if deduped.iter().any(|existing: &InterchangeIssue| {
            existing.code == candidate.code
                && existing.message == candidate.message
                && existing.path == candidate.path
                && existing.severity == candidate.severity
        }) {
            continue;
        }
        deduped.push(candidate);
    }
    let warnings = deduped
        .iter()
        .filter(|issue| issue.severity == IssueSeverity::Warning)
        .cloned()
        .collect::<Vec<_>>();
    let losses = deduped
        .iter()
        .filter(|issue| issue.severity == IssueSeverity::Loss)
        .cloned()
        .collect::<Vec<_>>();
    InterchangeReport {
        direction: direction.into(),
        adapter: adapter.map(str::to_owned),
        omissions: losses.clone(),
        issues: deduped,
        warnings,
        losses,
    }
}

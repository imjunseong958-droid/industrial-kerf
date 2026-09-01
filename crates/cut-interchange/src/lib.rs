//! OpenTimelineIO interchange through the bundled Python bridge.
//!
//! OTIO remains an optional process dependency. Rust owns validation,
//! invocation, timeout handling, and structured conversion reports; the
//! bridge owns the Python OTIO object model.

use cut_domain::Project;
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use std::fmt;
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(120);
pub const NATIVE_ADAPTERS: &[&str] = &["otio_json", "otioz"];

#[derive(Clone, Debug)]
pub struct InterchangeConfig {
    pub python_executable: PathBuf,
    pub bridge_script: Option<PathBuf>,
    pub cwd: Option<PathBuf>,
    pub timeout: Duration,
}

impl Default for InterchangeConfig {
    fn default() -> Self {
        Self {
            python_executable: PathBuf::from("python3"),
            bridge_script: None,
            cwd: None,
            timeout: DEFAULT_TIMEOUT,
        }
    }
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

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProcessInvocation {
    pub command: PathBuf,
    pub args: Vec<String>,
    pub cwd: Option<PathBuf>,
    pub stdin: Option<String>,
}

#[derive(Debug)]
pub struct InterchangeError {
    pub code: String,
    pub message: String,
}

impl InterchangeError {
    fn new(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
        }
    }
}

impl fmt::Display for InterchangeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}: {}", self.code, self.message)
    }
}

impl std::error::Error for InterchangeError {}

impl From<io::Error> for InterchangeError {
    fn from(error: io::Error) -> Self {
        Self::new("INTERCHANGE_PROCESS_FAILED", error.to_string())
    }
}

/// A validated OTIO adapter client.
#[derive(Clone, Debug)]
pub struct OpenTimelineIoAdapter {
    config: InterchangeConfig,
    bridge_script: PathBuf,
}

impl OpenTimelineIoAdapter {
    pub fn new(config: InterchangeConfig) -> Result<Self, InterchangeError> {
        if config.python_executable.as_os_str().is_empty() {
            return Err(InterchangeError::new(
                "INTERCHANGE_CONFIG_INVALID",
                "python executable must be non-empty",
            ));
        }
        if config.timeout.is_zero() {
            return Err(InterchangeError::new(
                "INTERCHANGE_CONFIG_INVALID",
                "timeout must be positive",
            ));
        }
        let bridge_script = config.bridge_script.clone().unwrap_or_else(|| {
            PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("python")
                .join("otio_bridge.py")
        });
        Ok(Self {
            config,
            bridge_script,
        })
    }

    pub fn with_defaults() -> Result<Self, InterchangeError> {
        Self::new(InterchangeConfig::default())
    }

    pub fn config(&self) -> &InterchangeConfig {
        &self.config
    }

    pub fn bridge_script(&self) -> &Path {
        &self.bridge_script
    }

    pub fn build_import_invocation(
        &self,
        input_path: &Path,
        adapter: Option<&str>,
    ) -> Result<ProcessInvocation, InterchangeError> {
        validate_path(input_path, "inputPath")?;
        let mut args = vec![
            self.bridge_script.to_string_lossy().into_owned(),
            "import".into(),
            "--input".into(),
            input_path.to_string_lossy().into_owned(),
        ];
        if let Some(adapter) = adapter {
            args.extend(["--adapter".into(), validate_adapter_name(adapter)?]);
        }
        Ok(ProcessInvocation {
            command: self.config.python_executable.clone(),
            args,
            cwd: self.config.cwd.clone(),
            stdin: None,
        })
    }

    pub fn build_export_invocation(
        &self,
        project: &Project,
        output_path: &Path,
        adapter: Option<&str>,
    ) -> Result<ProcessInvocation, InterchangeError> {
        validate_path(output_path, "outputPath")?;
        let project = cut_domain::serialize_project(project).map_err(|error| {
            InterchangeError::new("INTERCHANGE_CONFIG_INVALID", error.to_string())
        })?;
        let adapter = normalize_adapter(adapter, Some(output_path))?;
        let args = vec![
            self.bridge_script.to_string_lossy().into_owned(),
            "export".into(),
            "--output".into(),
            output_path.to_string_lossy().into_owned(),
            "--adapter".into(),
            adapter,
        ];
        let stdin = serde_json::to_string(&json!({ "project": project })).map_err(|error| {
            InterchangeError::new("INTERCHANGE_CONFIG_INVALID", error.to_string())
        })?;
        Ok(ProcessInvocation {
            command: self.config.python_executable.clone(),
            args,
            cwd: self.config.cwd.clone(),
            stdin: Some(stdin),
        })
    }

    pub fn import(
        &self,
        input_path: &Path,
        adapter: Option<&str>,
    ) -> Result<ImportResult, InterchangeError> {
        let invocation = self.build_import_invocation(input_path, adapter)?;
        let args = invocation.args.clone();
        let output = run_process(
            &invocation.command,
            &args,
            invocation.cwd.as_deref(),
            None,
            self.config.timeout,
        )?;
        if output.timed_out {
            return Err(InterchangeError::new(
                "INTERCHANGE_TIMEOUT",
                format!(
                    "OpenTimelineIO conversion timed out after {:?}",
                    self.config.timeout
                ),
            ));
        }
        if output.code != Some(0) {
            return Err(InterchangeError::new(
                "INTERCHANGE_PROCESS_FAILED",
                process_error_message(output.code, &output.stderr),
            ));
        }
        let parsed = parse_bridge_output(&output.stdout)?;
        let project_value = parsed
            .get("project")
            .cloned()
            .or_else(|| {
                parsed
                    .get("result")
                    .and_then(|value| value.get("project"))
                    .cloned()
            })
            .ok_or_else(|| {
                InterchangeError::new(
                    "INTERCHANGE_OUTPUT_INVALID",
                    "OTIO bridge response does not contain a project",
                )
            })?;
        let project = cut_domain::parse_project(&project_value).map_err(|error| {
            InterchangeError::new(
                "INTERCHANGE_OUTPUT_INVALID",
                format!("bridge returned an invalid cut v2 project: {error}"),
            )
        })?;
        let adapter = adapter.map(str::to_owned);
        let bridge_report = report_from_value("import", adapter.as_deref(), parsed.get("report"));
        Ok(ImportResult {
            project,
            input_path: input_path.to_path_buf(),
            adapter,
            report: bridge_report,
        })
    }

    pub fn export(
        &self,
        project: &Project,
        output_path: &Path,
        adapter: Option<&str>,
    ) -> Result<ExportResult, InterchangeError> {
        let adapter = normalize_adapter(adapter, Some(output_path))?;
        let project_value = cut_domain::serialize_project(project).map_err(|error| {
            InterchangeError::new("INTERCHANGE_CONFIG_INVALID", error.to_string())
        })?;
        let stdin =
            serde_json::to_string(&json!({ "project": project_value })).map_err(|error| {
                InterchangeError::new("INTERCHANGE_CONFIG_INVALID", error.to_string())
            })?;
        let args = vec![
            self.bridge_script.to_string_lossy().into_owned(),
            "export".into(),
            "--output".into(),
            output_path.to_string_lossy().into_owned(),
            "--adapter".into(),
            adapter.clone(),
        ];
        let output = run_process(
            &self.config.python_executable,
            &args,
            self.config.cwd.as_deref(),
            Some(stdin),
            self.config.timeout,
        )?;
        if output.timed_out {
            return Err(InterchangeError::new(
                "INTERCHANGE_TIMEOUT",
                format!(
                    "OpenTimelineIO conversion timed out after {:?}",
                    self.config.timeout
                ),
            ));
        }
        if output.code != Some(0) {
            return Err(InterchangeError::new(
                "INTERCHANGE_PROCESS_FAILED",
                process_error_message(output.code, &output.stderr),
            ));
        }
        let parsed = parse_bridge_output(&output.stdout)?;
        Ok(ExportResult {
            output_path: output_path.to_path_buf(),
            adapter: adapter.clone(),
            report: report_from_value("export", Some(&adapter), parsed.get("report")),
        })
    }
}

pub type OtioAdapter = OpenTimelineIoAdapter;

pub fn normalize_adapter(
    value: Option<&str>,
    output_path: Option<&Path>,
) -> Result<String, InterchangeError> {
    if let Some(value) = value {
        let value = validate_adapter_name(value)?;
        return Ok(match value.as_str() {
            "otio" | "json" | "otio-json" => "otio_json".into(),
            "otio-z" => "otioz".into(),
            _ => value,
        });
    }
    if let Some(extension) = output_path
        .and_then(|path| path.extension())
        .and_then(|value| value.to_str())
    {
        return Ok(if extension.eq_ignore_ascii_case("otioz") {
            "otioz".into()
        } else {
            "otio_json".into()
        });
    }
    Ok("otio_json".into())
}

pub fn validate_adapter_name(value: &str) -> Result<String, InterchangeError> {
    if value.is_empty()
        || value.len() > 128
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
        || !value.as_bytes()[0].is_ascii_alphanumeric()
    {
        return Err(InterchangeError::new(
            "INTERCHANGE_FORMAT_INVALID",
            "adapter must contain 1-128 letters, numbers, dots, underscores, or hyphens and start with a letter or number",
        ));
    }
    Ok(value.to_owned())
}

pub fn validate_path(path: &Path, field: &str) -> Result<(), InterchangeError> {
    if path.as_os_str().is_empty() || path.as_os_str().as_encoded_bytes().contains(&0) {
        return Err(InterchangeError::new(
            "INTERCHANGE_PATH_INVALID",
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

fn report_from_value(
    direction: &str,
    adapter: Option<&str>,
    value: Option<&Value>,
) -> InterchangeReport {
    let mut issues = Vec::new();
    let Some(value) = value.and_then(Value::as_object) else {
        return create_report(direction, adapter, issues);
    };
    for (field, fallback) in [
        ("issues", IssueSeverity::Warning),
        ("warnings", IssueSeverity::Warning),
        ("losses", IssueSeverity::Loss),
        ("omissions", IssueSeverity::Loss),
    ] {
        let Some(entries) = value.get(field).and_then(Value::as_array) else {
            continue;
        };
        for entry in entries {
            if let Some(message) = entry.as_str() {
                issues.push(issue("adapter_limitation", message, fallback, None));
                continue;
            }
            let Some(object) = entry.as_object() else {
                continue;
            };
            let Some(message) = object.get("message").and_then(Value::as_str) else {
                continue;
            };
            let severity = match object.get("severity").and_then(Value::as_str) {
                Some("loss") => IssueSeverity::Loss,
                Some("warning") => IssueSeverity::Warning,
                _ => fallback,
            };
            let mut item = issue(
                object
                    .get("code")
                    .and_then(Value::as_str)
                    .unwrap_or("adapter_limitation"),
                message,
                severity,
                object.get("path").and_then(Value::as_str),
            );
            item.details = object.get("details").cloned();
            issues.push(item);
        }
    }
    create_report(direction, adapter, issues)
}

fn parse_bridge_output(stdout: &str) -> Result<Map<String, Value>, InterchangeError> {
    let value: Value = serde_json::from_str(stdout.trim()).map_err(|error| {
        InterchangeError::new(
            "INTERCHANGE_OUTPUT_INVALID",
            format!("OTIO bridge returned invalid JSON: {error}"),
        )
    })?;
    let object = value.as_object().cloned().ok_or_else(|| {
        InterchangeError::new(
            "INTERCHANGE_OUTPUT_INVALID",
            "OTIO bridge response must be an object",
        )
    })?;
    if object.get("ok").and_then(Value::as_bool) == Some(false) {
        return Err(InterchangeError::new(
            "INTERCHANGE_OUTPUT_INVALID",
            object
                .get("error")
                .and_then(Value::as_str)
                .unwrap_or("OTIO bridge reported a conversion failure"),
        ));
    }
    Ok(object)
}

struct ProcessResult {
    stdout: String,
    stderr: String,
    code: Option<i32>,
    timed_out: bool,
}

fn run_process(
    command: &Path,
    args: &[String],
    cwd: Option<&Path>,
    stdin: Option<String>,
    timeout: Duration,
) -> Result<ProcessResult, InterchangeError> {
    let mut child = Command::new(command)
        .args(args)
        .current_dir(cwd.unwrap_or_else(|| Path::new(".")))
        .stdin(if stdin.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| {
            InterchangeError::new(
                "INTERCHANGE_PROCESS_FAILED",
                format!("could not start {}: {error}", command.display()),
            )
        })?;
    if let Some(stdin) = stdin {
        child
            .stdin
            .take()
            .expect("stdin was piped")
            .write_all(stdin.as_bytes())?;
    }
    let stdout = child.stdout.take().expect("stdout was piped");
    let stderr = child.stderr.take().expect("stderr was piped");
    let stdout_thread = thread::spawn(move || read_all(stdout));
    let stderr_thread = thread::spawn(move || read_all(stderr));
    let deadline = Instant::now() + timeout;
    let mut timed_out = false;
    let code = loop {
        if let Some(status) = child.try_wait()? {
            break status.code();
        }
        if Instant::now() >= deadline {
            timed_out = true;
            child.kill()?;
            break child.wait()?.code();
        }
        thread::sleep(Duration::from_millis(10));
    };
    Ok(ProcessResult {
        stdout: stdout_thread.join().map_err(|_| {
            InterchangeError::new("INTERCHANGE_PROCESS_FAILED", "stdout reader panicked")
        })??,
        stderr: stderr_thread.join().map_err(|_| {
            InterchangeError::new("INTERCHANGE_PROCESS_FAILED", "stderr reader panicked")
        })??,
        code,
        timed_out,
    })
}

fn read_all<R: Read>(mut reader: R) -> io::Result<String> {
    let mut bytes = Vec::new();
    reader.read_to_end(&mut bytes)?;
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}

fn process_error_message(code: Option<i32>, stderr: &str) -> String {
    let detail = stderr.trim();
    if detail.is_empty() {
        format!("OTIO bridge exited with code {code:?}")
    } else {
        format!("OTIO bridge exited with code {code:?}: {detail}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn normalizes_native_formats_and_rejects_shell_fragments() {
        assert_eq!(normalize_adapter(Some("otio"), None).unwrap(), "otio_json");
        assert_eq!(
            normalize_adapter(None, Some(Path::new("timeline.otioz"))).unwrap(),
            "otioz"
        );
        assert!(validate_adapter_name("adapter/name").is_err());
        assert!(validate_adapter_name("").is_err());
    }

    #[test]
    fn creates_deduplicated_loss_report() {
        let item = issue(
            "crop_not_supported",
            "crop is not portable",
            IssueSeverity::Loss,
            Some("tracks[0]"),
        );
        let report = create_report("export", Some("otio_json"), [item.clone(), item]);
        assert_eq!(report.issues.len(), 1);
        assert_eq!(report.losses.len(), 1);
        assert_eq!(report.omissions, report.losses);
    }

    #[test]
    fn validates_projects_before_export() {
        let project = cut_domain::empty_project();
        let adapter = OpenTimelineIoAdapter::with_defaults().unwrap();
        let invocation = adapter
            .build_export_invocation(&project, Path::new("out.otio"), None)
            .unwrap();
        assert!(invocation.args.iter().any(|arg| arg == "export"));
        assert!(invocation
            .stdin
            .as_deref()
            .is_some_and(|value| value.contains("\"project\"")));
        let _ = json!({});
    }
}

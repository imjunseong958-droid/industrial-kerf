//! Versioned composition manifest and instance-override validation.
//!
//! The manifest is deliberately a sidecar contract. It describes the inputs
//! accepted by an HTML composition, but it is not part of the canonical Cut
//! project and this module never writes or mutates project state.

use crate::composition::{open_local, relative_path};
use crate::McpError;
use serde_json::{json, Map, Value};
use std::collections::{BTreeMap, BTreeSet};
use std::fs::File;
use std::io::Read;
use std::path::Path;

pub(super) const MAX_MANIFEST_BYTES: usize = 128 * 1024;
pub(super) const MAX_INSTANCE_BYTES: usize = 64 * 1024;

const MANIFEST_SCHEMA_VERSION: u64 = 1;
const MAX_SLOTS: usize = 64;
const MAX_OPTIONS: usize = 32;
const MAX_ID_BYTES: usize = 128;
const MAX_VERSION_BYTES: usize = 64;
const MAX_STRING_BYTES: usize = 4096;
const MAX_REPORT_STRING_BYTES: usize = 128;
const MAX_REPORT_OPTION_BYTES: usize = 64;
const MAX_RETURNED_SLOTS: usize = 16;
const MAX_RETURNED_OPTIONS: usize = 8;
const MAX_RETURNED_OVERRIDES: usize = 16;
const MAX_RETURNED_VARIABLES: usize = 32;

const SLOT_TYPES: [&str; 8] = [
    "text", "color", "number", "boolean", "enum", "image", "video", "audio",
];

#[derive(Debug)]
pub(super) struct Report {
    pub manifest: Value,
    pub findings: Vec<Value>,
    pub errors: usize,
    pub warnings: usize,
    pub deterministic: bool,
    // Full, bounded sources for the render boundary; never part of the public
    // compact check report. Rendering must not use truncated display values.
    pub sources: BTreeMap<String, Vec<u8>>,
    pub source: Value,
    pub instance_source: Option<Value>,
}

impl Report {
    fn new(path: &str) -> Self {
        Self {
            manifest: json!({"path":path,"valid":false}),
            findings: Vec::new(),
            errors: 0,
            warnings: 0,
            deterministic: true,
            sources: BTreeMap::new(),
            source: Value::Null,
            instance_source: None,
        }
    }

    fn finding(
        &mut self,
        code: &str,
        message: impl Into<String>,
        path: &str,
        error: bool,
        affects_determinism: bool,
    ) {
        if error {
            self.errors += 1;
        } else {
            self.warnings += 1;
        }
        self.deterministic &= !affects_determinism;
        self.findings.push(json!({
            "code": code,
            "severity": if error { "error" } else { "warning" },
            "message": message.into(),
            "path": path,
            "offset": 0,
            "affectsDeterminism": affects_determinism,
        }));
    }
}

#[derive(Clone, Debug)]
struct Slot {
    id: String,
    kind: String,
    required: bool,
    default: Option<Value>,
    options: Vec<String>,
    description: Option<String>,
}

#[derive(Clone, Debug)]
struct ManifestData {
    template_id: String,
    template_version: String,
    slots: Vec<Slot>,
}

pub(super) fn inspect(
    root: &File,
    entry: &str,
    manifest_ref: &str,
    instance_ref: Option<&str>,
    root_metadata: &Value,
    variables: &[Value],
) -> Result<Report, McpError> {
    let manifest_path = relative_path(Path::new(""), manifest_ref, false)
        .map_err(|message| McpError::new("INVALID_ARGUMENTS", message))?;
    let manifest_bytes = read_bounded(root, &manifest_path, MAX_MANIFEST_BYTES, "manifest")?;
    let mut checker = Checker {
        root,
        entry,
        manifest_dir: manifest_path.parent().unwrap_or(Path::new("")),
        root_metadata,
        variables,
        report: Report::new(manifest_ref),
    };
    checker.report.sources.insert(
        manifest_path.to_string_lossy().into_owned(),
        manifest_bytes.clone(),
    );

    let manifest_value = match serde_json::from_slice::<Value>(&manifest_bytes) {
        Ok(value) => value,
        Err(error) => {
            checker.report.finding(
                "MANIFEST_JSON_INVALID",
                format!("Manifest is not valid JSON: {error}"),
                "/",
                true,
                false,
            );
            return Ok(checker.finish());
        }
    };
    checker.report.source = manifest_value.clone();
    let manifest_data = checker.validate_manifest(&manifest_value);
    checker.report.manifest["valid"] = json!(checker.report.errors == 0);
    checker.validate_instance(manifest_data.as_ref(), instance_ref)?;
    Ok(checker.finish())
}

struct Checker<'a> {
    root: &'a File,
    entry: &'a str,
    manifest_dir: &'a Path,
    root_metadata: &'a Value,
    variables: &'a [Value],
    report: Report,
}

impl<'a> Checker<'a> {
    fn finish(self) -> Report {
        self.report
    }

    fn validate_manifest(&mut self, value: &Value) -> Option<ManifestData> {
        let Some(object) = value.as_object() else {
            self.report.finding(
                "MANIFEST_NOT_OBJECT",
                "Manifest root must be a JSON object",
                "/",
                true,
                false,
            );
            return None;
        };
        self.unknown_fields(
            object,
            &[
                "schemaVersion",
                "templateId",
                "templateVersion",
                "entry",
                "output",
                "slots",
                "description",
            ],
            "/",
        );
        if let Some(description) = object.get("description") {
            if !description
                .as_str()
                .is_some_and(|description| description.len() <= MAX_STRING_BYTES)
            {
                self.report.finding(
                    "MANIFEST_FIELD_INVALID",
                    format!("description must be a string of at most {MAX_STRING_BYTES} bytes"),
                    "/description",
                    true,
                    false,
                );
            }
        }

        let schema_version = self.u64_field(object, "schemaVersion", "/schemaVersion");
        if schema_version != Some(MANIFEST_SCHEMA_VERSION) {
            self.report.finding(
                "MANIFEST_SCHEMA_UNSUPPORTED",
                format!("schemaVersion must be {MANIFEST_SCHEMA_VERSION}"),
                "/schemaVersion",
                true,
                false,
            );
        }
        self.report.manifest["schemaVersion"] =
            schema_version.map(Value::from).unwrap_or(Value::Null);

        let template_id = self.identifier_field(object, "templateId", "/templateId");
        let template_version = self.version_field(object, "templateVersion", "/templateVersion");
        let manifest_entry = self.entry_field(object, "/entry");
        let output = self.output_field(object, "/output");
        let slots = self.slots_field(object, "/slots");

        self.report.manifest["templateId"] =
            template_id.clone().map(Value::from).unwrap_or(Value::Null);
        self.report.manifest["templateVersion"] = template_version
            .clone()
            .map(Value::from)
            .unwrap_or(Value::Null);
        self.report.manifest["entry"] = manifest_entry
            .clone()
            .map(Value::from)
            .unwrap_or(Value::Null);
        self.report.manifest["output"] = output
            .as_ref()
            .map(|values| {
                let mut object = Map::new();
                for (key, value) in values {
                    object.insert(key.clone(), json!(value));
                }
                Value::Object(object)
            })
            .unwrap_or(Value::Null);
        self.report.manifest["slots"] = Value::Array(
            slots
                .as_ref()
                .map(|slots| {
                    slots
                        .iter()
                        .take(MAX_RETURNED_SLOTS)
                        .map(slot_value)
                        .collect()
                })
                .unwrap_or_default(),
        );
        if slots
            .as_ref()
            .is_some_and(|slots| slots.len() > MAX_RETURNED_SLOTS)
        {
            self.report.manifest["slotsTruncated"] = json!(true);
        }
        self.report.manifest["slotCount"] = json!(slots.as_ref().map_or(0, Vec::len));

        if let Some(manifest_entry) = &manifest_entry {
            let sidecar_entry = relative_path(self.manifest_dir, manifest_entry, false)
                .ok()
                .map(|path| path.to_string_lossy() == self.entry)
                .unwrap_or(false);
            if manifest_entry != self.entry && !sidecar_entry {
                self.report.finding(
                    "MANIFEST_ENTRY_MISMATCH",
                    format!(
                        "Manifest entry {manifest_entry:?} does not match checked entry {:?}",
                        self.entry
                    ),
                    "/entry",
                    true,
                    false,
                );
            }
        }
        if let Some(output) = &output {
            for field in ["width", "height", "fps", "duration"] {
                let Some(expected) = output.get(field) else {
                    continue;
                };
                let Some(actual) = self.root_metadata.get(field).and_then(Value::as_f64) else {
                    continue;
                };
                if (actual - expected).abs() > 1e-9 {
                    self.report.finding(
                        "MANIFEST_OUTPUT_MISMATCH",
                        format!(
                            "Manifest output {field}={expected} does not match HTML root value {actual}"
                        ),
                        &format!("/output/{field}"),
                        true,
                        false,
                    );
                }
            }
        }

        let template_id = template_id?;
        let template_version = template_version?;
        manifest_entry.as_ref()?;
        let slots = slots?;
        self.validate_variable_bindings(&slots);
        Some(ManifestData {
            template_id,
            template_version,
            slots,
        })
    }

    fn validate_instance(
        &mut self,
        manifest: Option<&ManifestData>,
        instance_ref: Option<&str>,
    ) -> Result<(), McpError> {
        let Some(instance_ref) = instance_ref else {
            return Ok(());
        };
        let path = relative_path(Path::new(""), instance_ref, false)
            .map_err(|message| McpError::new("INVALID_ARGUMENTS", message))?;
        let bytes = read_bounded(self.root, &path, MAX_INSTANCE_BYTES, "instance")?;
        self.report
            .sources
            .insert(path.to_string_lossy().into_owned(), bytes.clone());
        let mut instance_report = json!({
            "path": instance_ref,
            "valid": false,
            "overrides": {},
            "overrideCount": 0,
        });
        let start_errors = self.report.errors;
        let value = match serde_json::from_slice::<Value>(&bytes) {
            Ok(value) => value,
            Err(error) => {
                self.report.finding(
                    "INSTANCE_JSON_INVALID",
                    format!("Instance is not valid JSON: {error}"),
                    "/instance",
                    true,
                    false,
                );
                self.report.manifest["instance"] = instance_report;
                return Ok(());
            }
        };
        self.report.instance_source = Some(value.clone());
        let Some(object) = value.as_object() else {
            self.report.finding(
                "INSTANCE_NOT_OBJECT",
                "Instance root must be a JSON object",
                "/instance",
                true,
                false,
            );
            self.report.manifest["instance"] = instance_report;
            return Ok(());
        };
        self.unknown_fields(
            object,
            &[
                "schemaVersion",
                "templateId",
                "templateVersion",
                "overrides",
            ],
            "/instance/",
        );
        let schema_version = self.u64_field(object, "schemaVersion", "/instance/schemaVersion");
        if schema_version != Some(MANIFEST_SCHEMA_VERSION) {
            self.report.finding(
                "INSTANCE_SCHEMA_UNSUPPORTED",
                format!("Instance schemaVersion must be {MANIFEST_SCHEMA_VERSION}"),
                "/instance/schemaVersion",
                true,
                false,
            );
        }
        let template_id = self.identifier_field(object, "templateId", "/instance/templateId");
        let template_version =
            self.version_field(object, "templateVersion", "/instance/templateVersion");
        if let (Some(manifest), Some(template_id)) = (manifest, template_id.as_deref()) {
            if template_id != manifest.template_id {
                self.report.finding(
                    "INSTANCE_TEMPLATE_MISMATCH",
                    format!(
                        "Instance templateId {template_id:?} does not match manifest {:?}",
                        manifest.template_id
                    ),
                    "/instance/templateId",
                    true,
                    false,
                );
            }
        }
        if let (Some(manifest), Some(template_version)) = (manifest, template_version.as_deref()) {
            if template_version != manifest.template_version {
                self.report.finding(
                    "INSTANCE_VERSION_MISMATCH",
                    format!(
                        "Instance templateVersion {template_version:?} does not match manifest {:?}",
                        manifest.template_version
                    ),
                    "/instance/templateVersion",
                    true,
                    false,
                );
            }
        }
        let overrides = match object.get("overrides") {
            Some(Value::Object(overrides)) => overrides,
            Some(_) => {
                self.report.finding(
                    "INSTANCE_OVERRIDES_INVALID",
                    "Instance overrides must be an object",
                    "/instance/overrides",
                    true,
                    false,
                );
                self.report.manifest["instance"] = instance_report;
                return Ok(());
            }
            None => {
                self.report.finding(
                    "INSTANCE_OVERRIDES_MISSING",
                    "Instance overrides are required",
                    "/instance/overrides",
                    true,
                    false,
                );
                self.report.manifest["instance"] = instance_report;
                return Ok(());
            }
        };
        if overrides.len() > MAX_SLOTS {
            self.report.finding(
                "INSTANCE_LIMIT",
                format!("Instance has more than {MAX_SLOTS} overrides"),
                "/instance/overrides",
                true,
                false,
            );
        }
        let slot_map = manifest.map(|manifest| {
            manifest
                .slots
                .iter()
                .map(|slot| (slot.id.as_str(), slot))
                .collect::<BTreeMap<_, _>>()
        });
        let mut returned_overrides = Map::new();
        for (index, (key, value)) in overrides.iter().enumerate() {
            let display_key = compact_string(key);
            let path = format!("/instance/overrides/{display_key}");
            let Some(slot) = slot_map.as_ref().and_then(|slots| slots.get(key.as_str())) else {
                self.report.finding(
                    "UNKNOWN_OVERRIDE_SLOT",
                    format!("Instance override {display_key:?} is not declared by the manifest"),
                    &path,
                    true,
                    false,
                );
                continue;
            };
            if !value_matches_slot(slot, value) {
                self.report.finding(
                    "OVERRIDE_TYPE_MISMATCH",
                    format!(
                        "Override {key:?} does not match declared slot type {:?}",
                        slot.kind
                    ),
                    &path,
                    true,
                    false,
                );
            } else if index < MAX_RETURNED_OVERRIDES {
                returned_overrides.insert(key.clone(), compact_value(value));
            }
        }
        if overrides.len() > MAX_RETURNED_OVERRIDES {
            instance_report["overridesTruncated"] = json!(true);
        }
        instance_report["overrides"] = Value::Object(returned_overrides);
        instance_report["overrideCount"] = json!(overrides.len());
        if let Some(manifest) = manifest {
            for slot in &manifest.slots {
                if slot.required && !overrides.contains_key(&slot.id) {
                    self.report.finding(
                        "MISSING_REQUIRED_OVERRIDE",
                        format!("Required slot {:?} has no instance override", slot.id),
                        &format!("/instance/overrides/{}", slot.id),
                        true,
                        false,
                    );
                }
            }
        } else {
            self.report.finding(
                "INSTANCE_MANIFEST_INVALID",
                "Instance overrides cannot be validated because the manifest is invalid",
                "/instance",
                true,
                false,
            );
        }
        instance_report["valid"] = json!(self.report.errors == start_errors);
        self.report.manifest["instance"] = instance_report;
        Ok(())
    }

    fn validate_variable_bindings(&mut self, slots: &[Slot]) {
        let declared = slots
            .iter()
            .map(|slot| (slot.id.to_ascii_lowercase(), slot))
            .collect::<BTreeMap<_, _>>();
        let mut used = Vec::new();
        for (index, variable) in self.variables.iter().enumerate() {
            let Some(id) = variable.get("id").and_then(Value::as_str) else {
                continue;
            };
            let normalized = id.to_ascii_lowercase();
            if !declared.contains_key(&normalized) {
                self.report.finding(
                    "UNKNOWN_VARIABLE_SLOT",
                    format!("HTML variable binding {id:?} is not declared by the manifest"),
                    "/slots",
                    true,
                    false,
                );
            }
            if index < MAX_RETURNED_VARIABLES {
                used.push(compact_string(id));
            }
        }
        self.report.manifest["usedSlots"] = json!(used);
        if self.variables.len() > MAX_RETURNED_VARIABLES {
            self.report.manifest["usedSlotsTruncated"] = json!(true);
        }
    }

    fn unknown_fields(&mut self, object: &Map<String, Value>, allowed: &[&str], path: &str) {
        for key in object.keys() {
            if !allowed.iter().any(|allowed| *allowed == key) {
                self.report.finding(
                    "MANIFEST_UNKNOWN_FIELD",
                    format!("Unknown field {:?} was ignored", compact_string(key)),
                    &format!("{path}{}", compact_string(key)),
                    true,
                    false,
                );
            }
        }
    }

    fn u64_field(&mut self, object: &Map<String, Value>, key: &str, path: &str) -> Option<u64> {
        match object.get(key) {
            Some(Value::Number(value)) => value.as_u64().or_else(|| {
                self.report.finding(
                    "MANIFEST_FIELD_INVALID",
                    format!("{key} must be a non-negative integer"),
                    path,
                    true,
                    false,
                );
                None
            }),
            Some(_) => {
                self.report.finding(
                    "MANIFEST_FIELD_INVALID",
                    format!("{key} must be a non-negative integer"),
                    path,
                    true,
                    false,
                );
                None
            }
            None => {
                self.report.finding(
                    "MANIFEST_FIELD_MISSING",
                    format!("{key} is required"),
                    path,
                    true,
                    false,
                );
                None
            }
        }
    }

    fn identifier_field(
        &mut self,
        object: &Map<String, Value>,
        key: &str,
        path: &str,
    ) -> Option<String> {
        let value = self.string_field(object, key, path, MAX_ID_BYTES)?;
        if !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
        {
            self.report.finding(
                "MANIFEST_IDENTIFIER_INVALID",
                format!("{key} must contain only ASCII letters, digits, '-' or '_'"),
                path,
                true,
                false,
            );
            return None;
        }
        Some(value)
    }

    fn version_field(
        &mut self,
        object: &Map<String, Value>,
        key: &str,
        path: &str,
    ) -> Option<String> {
        let value = self.string_field(object, key, path, MAX_VERSION_BYTES)?;
        if value.bytes().any(|byte| byte.is_ascii_control()) {
            self.report.finding(
                "MANIFEST_VERSION_INVALID",
                format!("{key} must not contain control characters"),
                path,
                true,
                false,
            );
            return None;
        }
        Some(value)
    }

    fn entry_field(&mut self, object: &Map<String, Value>, path: &str) -> Option<String> {
        let value =
            self.string_field(object, "entry", path, crate::composition::MAX_ENTRY_BYTES)?;
        let normalized = match relative_path(Path::new(""), &value, false) {
            Ok(path) => path.to_string_lossy().into_owned(),
            Err(message) => {
                self.report
                    .finding("MANIFEST_ENTRY_INVALID", message, path, true, false);
                return None;
            }
        };
        Some(normalized)
    }

    fn string_field(
        &mut self,
        object: &Map<String, Value>,
        key: &str,
        path: &str,
        max_bytes: usize,
    ) -> Option<String> {
        match object.get(key) {
            Some(Value::String(value)) if !value.is_empty() && value.len() <= max_bytes => {
                Some(value.clone())
            }
            Some(Value::String(_)) => {
                self.report.finding(
                    "MANIFEST_FIELD_INVALID",
                    format!("{key} must be non-empty and at most {max_bytes} bytes"),
                    path,
                    true,
                    false,
                );
                None
            }
            Some(_) => {
                self.report.finding(
                    "MANIFEST_FIELD_INVALID",
                    format!("{key} must be a string"),
                    path,
                    true,
                    false,
                );
                None
            }
            None => {
                self.report.finding(
                    "MANIFEST_FIELD_MISSING",
                    format!("{key} is required"),
                    path,
                    true,
                    false,
                );
                None
            }
        }
    }

    fn output_field(
        &mut self,
        object: &Map<String, Value>,
        path: &str,
    ) -> Option<BTreeMap<String, f64>> {
        let Some(value) = object.get("output") else {
            self.report.finding(
                "MANIFEST_FIELD_MISSING",
                "output is required",
                path,
                true,
                false,
            );
            return None;
        };
        let Some(output) = value.as_object() else {
            self.report.finding(
                "MANIFEST_FIELD_INVALID",
                "output must be an object",
                path,
                true,
                false,
            );
            return None;
        };
        self.unknown_fields(output, &["width", "height", "fps", "duration"], "/output/");
        let mut values = BTreeMap::new();
        for field in ["width", "height", "fps", "duration"] {
            let Some(value) = output.get(field) else {
                self.report.finding(
                    "MANIFEST_FIELD_MISSING",
                    format!("output.{field} is required"),
                    &format!("/output/{field}"),
                    true,
                    false,
                );
                continue;
            };
            let Some(number) = value
                .as_f64()
                .filter(|value| value.is_finite() && *value > 0.0)
            else {
                self.report.finding(
                    "MANIFEST_OUTPUT_INVALID",
                    format!("output.{field} must be a finite positive number"),
                    &format!("/output/{field}"),
                    true,
                    false,
                );
                continue;
            };
            if matches!(field, "width" | "height")
                && (number.fract() != 0.0 || number > u32::MAX as f64)
            {
                self.report.finding(
                    "MANIFEST_OUTPUT_INVALID",
                    format!("output.{field} must be a positive 32-bit integer"),
                    &format!("/output/{field}"),
                    true,
                    false,
                );
                continue;
            }
            values.insert(field.to_owned(), number);
        }
        if values.len() == 4 {
            Some(values)
        } else {
            None
        }
    }

    fn slots_field(&mut self, object: &Map<String, Value>, path: &str) -> Option<Vec<Slot>> {
        let Some(value) = object.get("slots") else {
            self.report.finding(
                "MANIFEST_FIELD_MISSING",
                "slots is required",
                path,
                true,
                false,
            );
            return None;
        };
        let Some(slots) = value.as_array() else {
            self.report.finding(
                "MANIFEST_FIELD_INVALID",
                "slots must be an array",
                path,
                true,
                false,
            );
            return None;
        };
        if slots.len() > MAX_SLOTS {
            self.report.finding(
                "MANIFEST_LIMIT",
                format!("Manifest has more than {MAX_SLOTS} slots"),
                path,
                true,
                false,
            );
        }
        let mut parsed = Vec::new();
        let mut ids = BTreeSet::new();
        for (index, value) in slots.iter().take(MAX_SLOTS).enumerate() {
            let path = format!("/slots/{index}");
            let Some(object) = value.as_object() else {
                self.report.finding(
                    "MANIFEST_SLOT_INVALID",
                    "Each slot must be an object",
                    &path,
                    true,
                    false,
                );
                continue;
            };
            self.unknown_fields(
                object,
                &[
                    "id",
                    "type",
                    "required",
                    "default",
                    "options",
                    "description",
                ],
                &format!("{path}/"),
            );
            let id = self.identifier_field(object, "id", &format!("{path}/id"));
            let kind = self.string_field(object, "type", &format!("{path}/type"), 16);
            let Some(id) = id else {
                continue;
            };
            let Some(kind) = kind else {
                continue;
            };
            if !SLOT_TYPES.contains(&kind.as_str()) {
                self.report.finding(
                    "MANIFEST_SLOT_TYPE_UNSUPPORTED",
                    format!("Slot type {kind:?} is not supported"),
                    &format!("{path}/type"),
                    true,
                    false,
                );
                continue;
            }
            // HTML attribute names are case-insensitive, so distinct-looking
            // manifest IDs must not alias the same data-var binding.
            if !ids.insert(id.to_ascii_lowercase()) {
                self.report.finding(
                    "MANIFEST_SLOT_DUPLICATE",
                    format!("Slot id {id:?} is declared more than once"),
                    &format!("{path}/id"),
                    true,
                    false,
                );
                continue;
            }
            let required = match object.get("required") {
                None => false,
                Some(Value::Bool(value)) => *value,
                Some(_) => {
                    self.report.finding(
                        "MANIFEST_SLOT_INVALID",
                        "Slot required must be a boolean",
                        &format!("{path}/required"),
                        true,
                        false,
                    );
                    false
                }
            };
            let options = self.options_field(object, &kind, &path);
            let default = object.get("default").cloned();
            if let Some(default) = &default {
                if !value_matches_kind(&kind, default, &options) {
                    self.report.finding(
                        "MANIFEST_DEFAULT_TYPE_MISMATCH",
                        format!("Default for slot {id:?} does not match type {kind:?}"),
                        &format!("{path}/default"),
                        true,
                        false,
                    );
                }
            }
            let description = match object.get("description") {
                None => None,
                Some(Value::String(value)) if value.len() <= MAX_STRING_BYTES => {
                    Some(value.clone())
                }
                Some(_) => {
                    self.report.finding(
                        "MANIFEST_SLOT_INVALID",
                        format!("Description must be at most {MAX_STRING_BYTES} bytes"),
                        &format!("{path}/description"),
                        true,
                        false,
                    );
                    None
                }
            };
            parsed.push(Slot {
                id,
                kind,
                required,
                default,
                options,
                description,
            });
        }
        Some(parsed)
    }

    fn options_field(
        &mut self,
        object: &Map<String, Value>,
        kind: &str,
        path: &str,
    ) -> Vec<String> {
        let Some(value) = object.get("options") else {
            if kind == "enum" {
                self.report.finding(
                    "MANIFEST_ENUM_OPTIONS_MISSING",
                    "Enum slots require a non-empty options array",
                    &format!("{path}/options"),
                    true,
                    false,
                );
            }
            return Vec::new();
        };
        if kind != "enum" {
            self.report.finding(
                "MANIFEST_SLOT_INVALID",
                "Only enum slots may declare options",
                &format!("{path}/options"),
                true,
                false,
            );
            return Vec::new();
        }
        let Some(values) = value.as_array() else {
            self.report.finding(
                "MANIFEST_ENUM_OPTIONS_INVALID",
                "Enum options must be an array of strings",
                &format!("{path}/options"),
                true,
                false,
            );
            return Vec::new();
        };
        if values.is_empty() || values.len() > MAX_OPTIONS {
            self.report.finding(
                "MANIFEST_ENUM_OPTIONS_INVALID",
                format!("Enum options must contain 1..={MAX_OPTIONS} items"),
                &format!("{path}/options"),
                true,
                false,
            );
        }
        let mut options = Vec::new();
        let mut seen = BTreeSet::new();
        for (index, value) in values.iter().take(MAX_OPTIONS).enumerate() {
            let Some(value) = value.as_str() else {
                self.report.finding(
                    "MANIFEST_ENUM_OPTIONS_INVALID",
                    "Enum options must be strings",
                    &format!("{path}/options/{index}"),
                    true,
                    false,
                );
                continue;
            };
            if value.is_empty() || value.len() > MAX_STRING_BYTES || !seen.insert(value) {
                self.report.finding(
                    "MANIFEST_ENUM_OPTIONS_INVALID",
                    "Enum options must be unique, non-empty, bounded strings",
                    &format!("{path}/options/{index}"),
                    true,
                    false,
                );
                continue;
            }
            options.push(value.to_owned());
        }
        options
    }
}

pub(super) fn read_bounded(
    root: &File,
    path: &Path,
    limit: usize,
    label: &str,
) -> Result<Vec<u8>, McpError> {
    let file = open_local(root, path).map_err(|error| {
        McpError::new(
            "COMPOSITION_UNAVAILABLE",
            format!("{label} regular file unavailable: {error}"),
        )
    })?;
    if file
        .metadata()
        .map_err(|error| McpError::new("COMPOSITION_UNAVAILABLE", error.to_string()))?
        .len()
        > limit as u64
    {
        return Err(McpError::new(
            "COMPOSITION_LIMIT",
            format!("{label} exceeds {limit} bytes"),
        ));
    }
    let mut bytes = Vec::new();
    file.take(limit as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| McpError::new("COMPOSITION_UNAVAILABLE", error.to_string()))?;
    if bytes.len() > limit {
        return Err(McpError::new(
            "COMPOSITION_LIMIT",
            format!("{label} exceeds {limit} bytes"),
        ));
    }
    Ok(bytes)
}

fn slot_value(slot: &Slot) -> Value {
    let mut value = json!({
        "id": slot.id,
        "type": slot.kind,
        "required": slot.required,
    });
    if let Some(default) = &slot.default {
        value["default"] = compact_value(default);
    }
    if !slot.options.is_empty() {
        value["options"] = json!(slot
            .options
            .iter()
            .take(MAX_RETURNED_OPTIONS)
            .map(|value| compact_string_with_limit(value, MAX_REPORT_OPTION_BYTES))
            .collect::<Vec<_>>());
        if slot.options.len() > MAX_RETURNED_OPTIONS {
            value["optionsTruncated"] = json!(true);
        }
    }
    if let Some(description) = &slot.description {
        value["description"] = json!(compact_string(description));
    }
    value
}

fn value_matches_slot(slot: &Slot, value: &Value) -> bool {
    value_matches_kind(&slot.kind, value, &slot.options)
}

fn value_matches_kind(kind: &str, value: &Value, options: &[String]) -> bool {
    match kind {
        "text" => value
            .as_str()
            .is_some_and(|value| value.len() <= MAX_STRING_BYTES),
        "color" => value.as_str().is_some_and(valid_color),
        "number" => value.as_f64().is_some_and(f64::is_finite),
        "boolean" => value.is_boolean(),
        "enum" => value
            .as_str()
            .is_some_and(|value| options.iter().any(|option| option == value)),
        "image" | "video" | "audio" => value.as_str().is_some_and(valid_asset_id),
        _ => false,
    }
}

fn valid_color(value: &str) -> bool {
    (value.len() == 4 || value.len() == 5 || value.len() == 7 || value.len() == 9)
        && value.as_bytes().first() == Some(&b'#')
        && value.as_bytes()[1..].iter().all(u8::is_ascii_hexdigit)
}

fn valid_asset_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= MAX_ID_BYTES
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
}

fn compact_value(value: &Value) -> Value {
    match value {
        Value::String(value) => Value::String(compact_string(value)),
        Value::Array(_) | Value::Object(_) => json!("<structured value>"),
        _ => value.clone(),
    }
}

fn compact_string(value: &str) -> String {
    compact_string_with_limit(value, MAX_REPORT_STRING_BYTES)
}

fn compact_string_with_limit(value: &str, limit: usize) -> String {
    if value.len() <= limit {
        return value.to_owned();
    }
    let mut end = limit;
    while !value.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}…", &value[..end])
}

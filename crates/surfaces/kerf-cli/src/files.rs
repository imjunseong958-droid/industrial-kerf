//! `kerf preset` and `kerf export`: looks and programs kept as files, and a sequence written as a
//! timeline file another editor opens.

use crate::{in_step, open_router, runtime_error, runtime_for, PresetAction};
use anyhow::{anyhow, bail, Context, Result};
use serde_json::{json, Value};
use std::path::Path;

pub(crate) fn run_preset(
    action: &PresetAction,
    directory: &Path,
    kerfd: Option<&Path>,
) -> Result<()> {
    let (tool, arguments) = preset_request(action)?;
    let router = open_router(directory, kerfd)?;
    let response = router.call(tool, arguments);
    if response["ok"] != true {
        bail!(
            "{}: {}",
            response["error"]["code"].as_str().unwrap_or("PRESET_ERROR"),
            response["error"]["message"]
                .as_str()
                .unwrap_or("preset operation failed")
        );
    }
    let mut result = response["result"].clone();
    // Images remain available in the editor and MCP response; the terminal
    // prints their paths, without megabytes of inline image data.
    fn omit_images(value: &mut Value) {
        match value {
            Value::Object(fields) => {
                fields.remove("imageBase64");
                for value in fields.values_mut() {
                    omit_images(value);
                }
            }
            Value::Array(values) => {
                for value in values {
                    omit_images(value);
                }
            }
            _ => {}
        }
    }
    omit_images(&mut result);
    println!("{}", serde_json::to_string_pretty(&result)?);
    Ok(())
}

fn preset_request(action: &PresetAction) -> Result<(&'static str, Value)> {
    Ok(match action {
        PresetAction::List => ("look", json!({"presets": true})),
        PresetAction::Show { file } => ("look", json!({"preset": file})),
        PresetAction::Save {
            file,
            name,
            sequence,
        } => (
            "edit",
            json!({"savePreset": file, "name": name, "sequence": sequence}),
        ),
        PresetAction::Apply {
            file,
            bindings,
            texts,
            beats,
            sequence,
        } => {
            let mut media = serde_json::Map::new();
            for binding in bindings {
                let (slot, source) = binding
                    .split_once('=')
                    .context("--bind needs SLOT=MEDIA or SLOT=MEDIA@FROM")?;
                let (id, from) = source.split_once('@').unwrap_or((source, "0f"));
                if slot.is_empty() || id.is_empty() || from.is_empty() {
                    bail!("empty slot, media or source start in --bind");
                }
                if media
                    .insert(slot.into(), json!({"media": id, "from": from}))
                    .is_some()
                {
                    bail!("duplicate binding for `{slot}`");
                }
            }
            for slot in beats {
                let binding = media
                    .get_mut(slot)
                    .with_context(|| format!("--beats {slot}: bind `{slot}` with --bind"))?;
                binding["beats"] = json!(true);
            }
            let mut overrides = serde_json::Map::new();
            for text in texts {
                let (clip, value) = text
                    .split_once('=')
                    .context("--text needs TEMPLATE_CLIP=TEXT")?;
                if clip.is_empty() {
                    bail!("empty text slot");
                }
                if overrides.insert(clip.into(), json!(value)).is_some() {
                    bail!("duplicate text for `{clip}`");
                }
            }
            (
                "edit",
                json!({"preset": file, "bindings": media, "texts": overrides, "sequence": sequence}),
            )
        }
    })
}

/// The format `kerf export` writes `file` as: the one `--format` names, else the one its
/// extension does.
pub(crate) fn export_format(file: &Path, format: Option<&str>) -> Result<kerf_interchange::Format> {
    use kerf_interchange::Format;
    match format {
        Some(name) => Format::parse(name)
            .ok_or_else(|| anyhow!("`{name}` is not a format kerf export writes: otio or fcpxml")),
        None => Format::of_path(file).ok_or_else(|| {
            anyhow!(
                "{} does not say its format: name it .otio or .fcpxml, or pass --format",
                file.display()
            )
        }),
    }
}

/// Write a sequence of the project as a timeline file another editor opens, then say what
/// the file does not carry. The project is not changed.
pub(crate) fn run_export(
    directory: &Path,
    kerfd: Option<&Path>,
    file: &Path,
    format: kerf_interchange::Format,
    sequence: Option<&str>,
) -> Result<()> {
    let runtime = runtime_for(directory, kerfd)?;
    let state = runtime.get_state().map_err(runtime_error)?;
    in_step(&state)?;
    let mut project = kerf_runtime::parse_document(&state.document).map_err(runtime_error)?;
    // Another editor finds a file by where it is, not by where the project is.
    kerf_runtime::resolve_media_paths(&mut project, runtime.directory());
    let output =
        std::path::absolute(file).with_context(|| format!("resolve {}", file.display()))?;
    let written = kerf_interchange::export(&project, sequence, &output, format)
        .map_err(|error| anyhow!(error))?;
    print!("{}", export_report(&written));
    Ok(())
}

/// What `kerf export` says of the file it wrote: where it is, then everything it does not
/// carry and everything that arrives changed, a line each.
fn export_report(written: &kerf_interchange::ExportResult) -> String {
    let report = &written.report;
    let mut text = format!(
        "wrote {} ({})\n",
        written.output_path.display(),
        written.adapter
    );
    let mut list = |heading: &str, issues: &[kerf_interchange::InterchangeIssue]| {
        if issues.is_empty() {
            return;
        }
        text.push_str(&format!("{heading} ({}):\n", issues.len()));
        for issue in issues {
            let at = issue.path.as_deref().unwrap_or("-");
            text.push_str(&format!("  {at}\t{}\n", issue.message));
        }
    };
    list("not carried", &report.losses);
    list("check", &report.warnings);
    if report.issues.is_empty() {
        text.push_str("everything in the sequence is in the file\n");
    }
    text
}

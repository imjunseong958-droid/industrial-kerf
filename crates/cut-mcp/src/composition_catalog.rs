//! Bounded, sorted, non-executing discovery of local composition sidecars.

use crate::composition;
use crate::composition_files::{self as files, Directory};
use crate::composition_manifest::read_bounded;
use crate::McpError;
use serde::Deserialize;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};

const MAX_ENTRIES: usize = 2048;
const MAX_COMPOSITIONS: usize = 128;
const MAX_BYTES: usize = 16 * 1024 * 1024;

pub(super) fn input_schema() -> Value {
    json!({"type":"object","properties":{
        "path":{"type":"string","default":".","maxLength":1024,"description":"Project-relative directory or HTML file. Hidden entries, target and node_modules are excluded."},
        "maxDepth":{"type":"integer","minimum":0,"maximum":8,"default":4},
        "query":{"type":"string","maxLength":256},
        "tag":{"type":"string","maxLength":128},
        "offset":{"type":"integer","minimum":0},
        "limit":{"type":"integer","minimum":1,"maximum":20,"default":8}
    },"additionalProperties":false})
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Options {
    path: Option<String>,
    max_depth: Option<usize>,
    query: Option<String>,
    tag: Option<String>,
    offset: Option<usize>,
    limit: Option<usize>,
}

pub(super) fn catalog(directory: &Path, arguments: &Value) -> Result<Value, McpError> {
    let options: Options = serde_json::from_value(arguments.clone())
        .map_err(|error| McpError::new("INVALID_ARGUMENTS", error.to_string()))?;
    let depth = options.max_depth.unwrap_or(4);
    let limit = options.limit.unwrap_or(8);
    if depth > 8
        || !(1..=20).contains(&limit)
        || options
            .query
            .as_ref()
            .is_some_and(|value| value.len() > 256)
        || options.tag.as_ref().is_some_and(|value| value.len() > 128)
    {
        return Err(McpError::new(
            "INVALID_ARGUMENTS",
            "catalog options exceed limits",
        ));
    }
    let root = Directory::project(directory)?;
    let reference = options.path.as_deref().unwrap_or(".");
    let relative = if reference == "." {
        PathBuf::new()
    } else {
        files::path(reference)?
    };
    if relative
        .components()
        .any(|part| excluded(&part.as_os_str().to_string_lossy()))
    {
        return Err(McpError::new(
            "INVALID_ARGUMENTS",
            "catalog path is excluded",
        ));
    }
    let mut entries = Vec::new();
    let mut visited = 0;
    if let Ok(folder) = root.directory(&relative) {
        discover(&folder, &relative, depth, &mut visited, &mut entries)?;
    } else {
        root.open(&relative)?;
        if relative
            .extension()
            .and_then(|extension| extension.to_str())
            != Some("html")
        {
            return Err(McpError::new(
                "INVALID_ARGUMENTS",
                "catalog file must be .html",
            ));
        }
        entries.push(relative);
    }
    entries.sort();
    let discovered = entries.len();
    let query = options.query.as_deref().unwrap_or("").to_lowercase();
    let mut matched = Vec::new();
    let mut bytes = 0;
    for entry in entries {
        let item = describe(&root, &entry, &mut bytes)?;
        let searchable = json!([
            item["templateId"],
            item["name"],
            item["description"],
            item["entry"],
            item["tags"]
        ])
        .to_string()
        .to_lowercase();
        if !searchable.contains(&query) {
            continue;
        }
        if let Some(tag) = &options.tag {
            if !item["tags"].as_array().is_some_and(|tags| {
                tags.iter()
                    .any(|value| value.as_str() == Some(tag.as_str()))
            }) {
                continue;
            }
        }
        matched.push(item);
    }
    let total = matched.len();
    let offset = options.offset.unwrap_or(0).min(total);
    let mut items = Vec::new();
    let mut returned_bytes = 0;
    for item in matched.into_iter().skip(offset).take(limit) {
        let size = item.to_string().len();
        if returned_bytes + size > crate::results::MAX_INLINE_BYTES - 1024 {
            if items.is_empty() {
                return Err(McpError::new(
                    "COMPOSITION_LIMIT",
                    "catalog item exceeds inline budget",
                ));
            }
            break;
        }
        returned_bytes += size;
        items.push(item);
    }
    let next = offset + items.len();
    Ok(
        json!({"items":items,"total":total,"discovered":discovered,"offset":offset,
        "nextOffset":if next < total { Some(next) } else { None },
        "maxDepth":depth,"limits":{"entries":MAX_ENTRIES,"compositions":MAX_COMPOSITIONS,"bytes":MAX_BYTES},
        "scope":"local-static","canonicalStateChanged":false}),
    )
}

fn excluded(name: &str) -> bool {
    name.starts_with('.') || matches!(name, "node_modules" | "target")
}

fn discover(
    directory: &Directory,
    relative: &Path,
    depth: usize,
    visited: &mut usize,
    entries: &mut Vec<PathBuf>,
) -> Result<(), McpError> {
    let names = files::names(&directory.file, MAX_ENTRIES - *visited)?;
    *visited += names.len();
    for name in names {
        if excluded(&name) {
            continue;
        }
        let local = files::path(&name)?;
        let path = relative.join(&local);
        files::path(&path.to_string_lossy())?;
        if let Ok(child) = directory.directory(&local) {
            if depth > 0 {
                discover(&child, &path, depth - 1, visited, entries)?;
            }
        } else {
            // Even an unrecognized visible entry must be regular; never follow
            // a symlink or wait on a FIFO while deciding what to discover.
            directory.open(&local)?;
            if path.extension().and_then(|value| value.to_str()) == Some("html") {
                if entries.len() == MAX_COMPOSITIONS {
                    return Err(McpError::new(
                        "COMPOSITION_LIMIT",
                        "catalog exceeds 128 compositions",
                    ));
                }
                entries.push(path);
            }
        }
    }
    Ok(())
}

fn describe(root: &Directory, entry: &Path, budget: &mut usize) -> Result<Value, McpError> {
    let parent = entry.parent().unwrap_or(Path::new(""));
    let folder = root.directory(parent)?;
    let names = files::names(&folder.file, MAX_ENTRIES)?;
    let sidecar = |name: &str| {
        names
            .iter()
            .any(|item| item == name)
            .then(|| parent.join(name))
    };
    let manifest = sidecar("composition.manifest.json").or_else(|| sidecar("manifest.json"));
    let meta_path = sidecar("meta.json");
    let storyboard = sidecar("STORYBOARD.md");
    let mut meta = json!({});
    if let Some(path) = &meta_path {
        let bytes = read_bounded(&root.file, path, 32 * 1024, "catalog meta")?;
        *budget += bytes.len();
        meta = serde_json::from_slice(&bytes).map_err(|error| {
            McpError::new(
                "COMPOSITION_METADATA_INVALID",
                format!("{}: {error}", path.display()),
            )
        })?;
        if !meta.is_object() {
            return Err(McpError::new(
                "COMPOSITION_METADATA_INVALID",
                "meta.json must be an object",
            ));
        }
    }
    if let Some(path) = &storyboard {
        *budget += read_bounded(&root.file, path, 128 * 1024, "storyboard")?.len();
    }
    let thumbnail = if let Some(reference) = meta.get("thumbnail") {
        let reference = reference.as_str().ok_or_else(|| {
            McpError::new(
                "COMPOSITION_METADATA_INVALID",
                "thumbnail must be a local path",
            )
        })?;
        let path = parent.join(files::path(reference)?);
        root.open(&path)?;
        Some(path)
    } else {
        None
    };
    let mut arguments = json!({"entry":entry});
    if let Some(manifest) = &manifest {
        arguments["manifest"] = json!(manifest);
    }
    let checked = composition::inspect_open(&root.file, &arguments)?;
    *budget += checked.sources.values().map(Vec::len).sum::<usize>();
    if *budget > MAX_BYTES {
        return Err(McpError::new(
            "COMPOSITION_LIMIT",
            "catalog exceeds 16 MiB source budget",
        ));
    }
    let source = &checked.manifest;
    let report = &checked.report;
    let id = source
        .get("templateId")
        .and_then(Value::as_str)
        .or_else(|| report["metadata"]["id"].as_str())
        .unwrap_or("unknown");
    let slots = source["slots"].as_array().cloned().unwrap_or_default();
    let mut tags = Vec::new();
    if let Some(values) = meta.get("tags") {
        let values = values
            .as_array()
            .filter(|values| values.len() <= 32)
            .ok_or_else(|| {
                McpError::new(
                    "COMPOSITION_METADATA_INVALID",
                    "tags must contain at most 32 strings",
                )
            })?;
        for value in values {
            let tag = value
                .as_str()
                .filter(|value| !value.is_empty() && value.len() <= 128)
                .ok_or_else(|| {
                    McpError::new(
                        "COMPOSITION_METADATA_INVALID",
                        "tags must be bounded strings",
                    )
                })?;
            tags.push(tag.to_owned());
        }
        tags.sort();
        tags.dedup();
    }
    Ok(json!({
        "templateId":text(id,128),"templateVersion":text(source["templateVersion"].as_str().unwrap_or("unversioned"),64),
        "name":text(meta["name"].as_str().unwrap_or(id),256),
        "description":text(meta["description"].as_str().or_else(|| source["description"].as_str()).unwrap_or(""),512),
        "entry":entry,"manifest":manifest,"meta":meta_path,"storyboard":storyboard,"thumbnail":thumbnail,
        "output":source.get("output").unwrap_or(&report["metadata"]),
        "slots":slots.iter().take(16).map(|slot| json!({"id":slot["id"].as_str().map(|id| text(id,128)),"type":slot["type"].as_str().map(|kind| text(kind,16)),"required":slot["required"].as_bool().unwrap_or(false)})).collect::<Vec<_>>(),
        "slotCount":slots.len(),"slotsTruncated":slots.len()>16,"tags":tags,
        "valid":report["valid"],"deterministic":report["deterministic"],
        "validation":{"scanComplete":report["scanComplete"],"errors":report["counts"]["errors"],"warnings":report["counts"]["warnings"]}
    }))
}

fn text(value: &str, limit: usize) -> String {
    let mut end = value.len().min(limit);
    while !value.is_char_boundary(end) {
        end -= 1;
    }
    value[..end].to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::fs;
    use tempfile::TempDir;

    fn write_composition(root: &std::path::Path, name: &str, template_id: &str) {
        let directory = root.join(name);
        fs::create_dir_all(&directory).unwrap();
        fs::write(
            directory.join("index.html"),
            r#"<main data-composition-id="catalog" data-width="320" data-height="180" data-fps="30" data-duration="1"></main>"#,
        )
        .unwrap();
        fs::write(
            directory.join("composition.manifest.json"),
            serde_json::to_vec(&json!({
                "schemaVersion": 1,
                "templateId": template_id,
                "templateVersion": "1.0.0",
                "entry": "index.html",
                "output": {"width": 320, "height": 180, "fps": 30, "duration": 1},
                "slots": []
            }))
            .unwrap(),
        )
        .unwrap();
        fs::write(
            directory.join("meta.json"),
            serde_json::to_vec(&json!({"name": name, "tags": ["demo", name]})).unwrap(),
        )
        .unwrap();
    }

    #[test]
    fn catalog_discovers_sorted_metadata_and_supports_tag_filtering() {
        let root = TempDir::new().unwrap();
        write_composition(root.path(), "zeta", "zeta-template");
        write_composition(root.path(), "alpha", "alpha-template");

        let all = catalog(root.path(), &json!({"limit": 20})).unwrap();
        let items = all["items"].as_array().unwrap();
        assert_eq!(items.len(), 2);
        assert_eq!(items[0]["entry"], "alpha/index.html");
        assert_eq!(items[0]["templateId"], "alpha-template");
        assert_eq!(items[1]["entry"], "zeta/index.html");
        assert_eq!(items[0]["valid"], true);
        assert_eq!(items[0]["deterministic"], true);

        let filtered = catalog(root.path(), &json!({"tag":"zeta"})).unwrap();
        assert_eq!(filtered["total"], 1);
        assert_eq!(filtered["items"][0]["templateId"], "zeta-template");
    }

    #[cfg(unix)]
    #[test]
    fn catalog_does_not_follow_symlinked_directories() {
        use std::os::unix::fs::symlink;

        let root = TempDir::new().unwrap();
        let external = TempDir::new().unwrap();
        write_composition(external.path(), "outside", "outside-template");
        symlink(external.path().join("outside"), root.path().join("linked")).unwrap();
        let error = catalog(root.path(), &json!({})).unwrap_err();
        assert_eq!(error.code, "COMPOSITION_IO");
    }
}

//! One resolver, snapshot and cache contract for composition preview and render.
//! Only regenerable cache artifacts and generated provenance are written here.

use crate::composition;
use crate::composition_bridge::{self as bridge, Renderer};
use crate::composition_files::{self as files, Directory};
use crate::composition_manifest::read_bounded;
use crate::{GeneratedAssetManifest, GeneratedAssetRecord, McpError};
use cut_engine::derivatives::cache_key_for_values;
use serde::Deserialize;
use serde_json::{json, Map, Value};
use std::collections::{BTreeMap, BTreeSet};
use std::fs::File;
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::Path;
use std::sync::atomic::AtomicBool;
use std::time::Instant;

const MAX_ASSET_MANIFEST_BYTES: usize = 4 * 1024 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Mode {
    Preview,
    Render,
}

impl Mode {
    fn name(self) -> &'static str {
        if self == Self::Preview {
            "preview"
        } else {
            "render"
        }
    }
    fn format(self) -> &'static str {
        if self == Self::Preview {
            "png"
        } else {
            "mp4"
        }
    }
    fn media_type(self) -> &'static str {
        if self == Self::Preview {
            "image/png"
        } else {
            "video/mp4"
        }
    }
    fn kind(self) -> &'static str {
        if self == Self::Preview {
            "image"
        } else {
            "video"
        }
    }
}

pub(super) fn input_schema(mode: Mode) -> Value {
    let mut schema = composition::input_schema();
    schema["properties"]["format"] =
        json!({"type":"string","enum":[mode.format()],"default":mode.format()});
    if mode == Mode::Preview {
        schema["properties"]["frame"] = json!({"type":"integer","minimum":0,"default":0,"description":"Zero-based frame in the composition's explicit fps/duration."});
    } else {
        schema["properties"]["startFrame"] = json!({"type":"integer","minimum":0,"default":0});
        schema["properties"]["endFrame"] = json!({"type":"integer","minimum":1,"description":"Exclusive end; defaults to ceil(duration * fps)."});
    }
    schema
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Options {
    entry: Option<String>,
    manifest: Option<String>,
    instance: Option<String>,
    frame: Option<u64>,
    start_frame: Option<u64>,
    end_frame: Option<u64>,
    format: Option<String>,
}

struct InputFile {
    path: String,
    fingerprint: String,
    size: u64,
    // Source bytes are the exact bytes inspected. Media/dependencies are copied
    // from these open descriptors and rehashed before the worker can see them.
    bytes: Option<Vec<u8>>,
    file: Option<File>,
}

struct Resolved {
    inputs: Value,
    parameters: Value,
    files: BTreeMap<String, InputFile>,
}

fn normalized_path(value: &str) -> Result<String, McpError> {
    Ok(files::path(value)?.to_string_lossy().into_owned())
}

fn resolve(root: &Directory, arguments: &Value, mode: Mode) -> Result<Resolved, McpError> {
    let options: Options = serde_json::from_value(arguments.clone())
        .map_err(|error| McpError::new("INVALID_ARGUMENTS", error.to_string()))?;
    if options
        .format
        .as_deref()
        .is_some_and(|format| format != mode.format())
        || (mode == Mode::Preview && (options.start_frame.is_some() || options.end_frame.is_some()))
        || (mode == Mode::Render && options.frame.is_some())
    {
        return Err(McpError::new(
            "INVALID_ARGUMENTS",
            "preview accepts PNG/frame; render accepts MP4/startFrame/endFrame",
        ));
    }
    let entry = normalized_path(options.entry.as_deref().unwrap_or("index.html"))?;
    let manifest_path = options
        .manifest
        .as_deref()
        .map(normalized_path)
        .transpose()?;
    let instance_path = options
        .instance
        .as_deref()
        .map(normalized_path)
        .transpose()?;
    let mut check_arguments = json!({"entry":entry});
    if let Some(path) = &manifest_path {
        check_arguments["manifest"] = json!(path);
    }
    if let Some(path) = &instance_path {
        check_arguments["instance"] = json!(path);
    }
    let checked = composition::inspect_open(&root.file, &check_arguments)?;
    if checked.report["valid"] != true || checked.report["deterministic"] != true {
        return Err(McpError::new(
            "COMPOSITION_INVALID",
            "composition must pass static validation with deterministic=true",
        )
        .with_details(checked.report));
    }
    if !checked.inputs_complete || (manifest_path.is_none() && checked.variable_count > 0) {
        return Err(McpError::new(
            "COMPOSITION_INPUTS_INCOMPLETE",
            "all dependencies and variable slots must be available; variables require a manifest",
        ));
    }
    let metadata = &checked.report["metadata"];
    let output = json!({"width":metadata["width"].as_f64().unwrap_or(0.0) as u64,
        "height":metadata["height"].as_f64().unwrap_or(0.0) as u64,
        "fps":metadata["fps"],"duration":metadata["duration"]});
    let fps = output["fps"].as_f64().unwrap_or(0.0);
    let duration = output["duration"].as_f64().unwrap_or(0.0);
    if output["width"].as_u64().unwrap_or(0) > 8192
        || output["height"].as_u64().unwrap_or(0) > 8192
        || fps > 240.0
        || duration > 600.0
    {
        return Err(McpError::new(
            "COMPOSITION_LIMIT",
            "render output is limited to 8192 pixels per axis, 240 fps, and 600 seconds",
        ));
    }
    let total_frames = (fps * duration).ceil() as u64;
    let start = if mode == Mode::Preview {
        options.frame.unwrap_or(0)
    } else {
        options.start_frame.unwrap_or(0)
    };
    let end = if mode == Mode::Preview {
        start.checked_add(1)
    } else {
        Some(options.end_frame.unwrap_or(total_frames))
    }
    .ok_or_else(|| McpError::new("INVALID_ARGUMENTS", "frame overflow"))?;
    if start >= end || end > total_frames {
        return Err(McpError::new(
            "INVALID_ARGUMENTS",
            "frame range must be nonempty and within ceil(duration * fps)",
        ));
    }
    let parameters = json!({"mode":mode.name(),"format":mode.format(),"mediaType":mode.media_type(),
        "startFrame":start,"endFrame":end,"frameCount":end-start,"totalFrames":total_frames});

    let mut input_files = BTreeMap::new();
    let mut source_identity = Map::new();
    for (path, bytes) in &checked.sources {
        // JSON object order/whitespace is immaterial. HTML remains byte-exact.
        let identity = if path == &entry {
            json!({"contentFingerprint":files::hash(bytes)})
        } else {
            let value: Value = serde_json::from_slice(bytes)
                .map_err(|error| McpError::new("COMPOSITION_INVALID", error.to_string()))?;
            json!({"contentFingerprint":cache_key_for_values("composition-json", &value, &Value::Null)})
        };
        source_identity.insert(path.clone(), identity);
        input_files.insert(
            path.clone(),
            InputFile {
                path: path.clone(),
                fingerprint: files::hash(bytes),
                size: bytes.len() as u64,
                bytes: Some(bytes.clone()),
                file: None,
            },
        );
    }
    let mut bindings = Map::new();
    let mut used_ids = BTreeSet::new();
    let asset_manifest = if checked.manifest["slots"].as_array().is_some_and(|slots| {
        slots
            .iter()
            .any(|slot| matches!(slot["type"].as_str(), Some("image" | "video" | "audio")))
    }) {
        Some(read_assets(root, false)?)
    } else {
        None
    };
    if let Some(slots) = checked.manifest["slots"].as_array() {
        let overrides = checked
            .instance
            .as_ref()
            .and_then(|value| value["overrides"].as_object());
        for slot in slots {
            let id = slot["id"].as_str().expect("validated slot id");
            // HTML attribute names are ASCII-insensitive. Reject ambiguous slot
            // identities instead of letting worker implementations pick one.
            if !used_ids.insert(id.to_ascii_lowercase()) {
                return Err(McpError::new(
                    "COMPOSITION_BINDING_INVALID",
                    "slot IDs collide under HTML case folding",
                ));
            }
            let kind = slot["type"].as_str().expect("validated slot type");
            let override_value = overrides.and_then(|values| values.get(id));
            if slot["required"] == true && override_value.is_none() {
                return Err(McpError::new(
                    "COMPOSITION_BINDING_MISSING",
                    format!("required slot {id:?} requires an explicit instance override"),
                ));
            }
            let value = override_value
                .or_else(|| slot.get("default"))
                .ok_or_else(|| {
                    McpError::new(
                        "COMPOSITION_BINDING_MISSING",
                        format!("slot {id:?} needs an override or default for rendering"),
                    )
                })?;
            if matches!(kind, "image" | "video" | "audio") {
                let asset_id = value.as_str().expect("validated media ID");
                let records = &asset_manifest.as_ref().expect("media manifest").assets;
                let candidates = records
                    .iter()
                    .filter(|record| record.asset_id == asset_id)
                    .collect::<Vec<_>>();
                let record = candidates.first().copied().ok_or_else(|| {
                    McpError::new(
                        "COMPOSITION_ASSET_UNAVAILABLE",
                        format!("media slot {id:?}: asset ID {asset_id:?} is not registered"),
                    )
                })?;
                if candidates.iter().any(|other| {
                    other.path != record.path
                        || other.kind != record.kind
                        || other.content_fingerprint != record.content_fingerprint
                }) {
                    return Err(McpError::new(
                        "COMPOSITION_ASSET_AMBIGUOUS",
                        format!("asset ID {asset_id:?} has conflicting records"),
                    ));
                }
                if record.kind != kind
                    || record.content_fingerprint.len() != 64
                    || record.request_fingerprint.is_empty()
                {
                    return Err(McpError::new(
                        "COMPOSITION_ASSET_INVALID",
                        format!(
                            "asset {asset_id:?} must have matching kind and complete provenance"
                        ),
                    ));
                }
                let recorded_path = Path::new(&record.path);
                let relative = if recorded_path.is_absolute() {
                    recorded_path.strip_prefix(&root.path).map_err(|_| {
                        McpError::new(
                            "COMPOSITION_ASSET_UNSAFE",
                            "media asset must be inside the project",
                        )
                    })?
                } else {
                    recorded_path
                };
                let path = normalized_path(relative.to_str().ok_or_else(|| {
                    McpError::new("COMPOSITION_ASSET_UNSAFE", "asset path must be UTF-8")
                })?)?;
                let input = add_file(root, &mut input_files, &path)?;
                if input.fingerprint != record.content_fingerprint {
                    return Err(McpError::new(
                        "COMPOSITION_ASSET_CHANGED",
                        format!("asset {asset_id:?} differs from its registered fingerprint"),
                    ));
                }
                bindings.insert(
                    id.to_owned(),
                    json!({"type":kind,"assetId":asset_id,"path":path,
                    "contentFingerprint":input.fingerprint,"size":input.size}),
                );
            } else {
                let effective = if kind == "number" {
                    json!(value.as_f64().expect("validated number"))
                } else {
                    value.clone()
                };
                bindings.insert(id.to_owned(), json!({"type":kind,"value":effective}));
            }
        }
    }
    let mut dependencies = Map::new();
    for path in &checked.dependencies {
        let path = normalized_path(path)?;
        let input = add_file(root, &mut input_files, &path)?;
        dependencies.insert(
            path,
            json!({"contentFingerprint":input.fingerprint,"size":input.size}),
        );
    }
    if input_files.len() > 256
        || input_files.values().map(|input| input.size).sum::<u64>() > files::MAX_INPUT_BYTES
    {
        return Err(McpError::new(
            "COMPOSITION_LIMIT",
            "render inputs exceed 256 files or 1 GiB",
        ));
    }
    let inputs = json!({
        "entry":entry,"manifest":manifest_path,"instance":instance_path,
        "templateId":checked.manifest.get("templateId").unwrap_or(&metadata["id"]),
        "templateVersion":checked.manifest.get("templateVersion").cloned().unwrap_or(json!("unversioned")),
        "sources":source_identity,"bindings":bindings,"dependencies":dependencies,"output":output
    });
    Ok(Resolved {
        inputs,
        parameters,
        files: input_files,
    })
}

fn add_file<'a>(
    root: &Directory,
    inputs: &'a mut BTreeMap<String, InputFile>,
    path: &str,
) -> Result<&'a InputFile, McpError> {
    if !inputs.contains_key(path) {
        let mut file = root
            .open(&files::path(path)?)
            .map_err(|error| McpError::new("COMPOSITION_ASSET_UNSAFE", error.to_string()))?;
        let (fingerprint, size) = files::fingerprint(&mut file, None)?;
        file.seek(SeekFrom::Start(0)).map_err(files::io)?;
        inputs.insert(
            path.to_owned(),
            InputFile {
                path: path.to_owned(),
                fingerprint,
                size,
                bytes: None,
                file: Some(file),
            },
        );
    }
    Ok(inputs.get(path).expect("inserted input"))
}

pub(super) fn execute(
    directory: &Path,
    arguments: &Value,
    mode: Mode,
    cancelled: Option<&AtomicBool>,
) -> Result<Value, McpError> {
    let renderer = Renderer::from_env()?;
    execute_with(directory, arguments, mode, &renderer, cancelled)
}

pub(super) fn execute_with(
    directory: &Path,
    arguments: &Value,
    mode: Mode,
    renderer: &Renderer,
    cancelled: Option<&AtomicBool>,
) -> Result<Value, McpError> {
    let root = Directory::project(&directory.canonicalize().map_err(files::io)?)?;
    let now = Instant::now();
    // A pathological administrator-supplied timeout must fail closed rather
    // than panic when Instant arithmetic would overflow.
    let deadline = now.checked_add(renderer.timeout).unwrap_or(now);
    bridge::interrupted(deadline, cancelled)?;
    let mut resolved = resolve(&root, arguments, mode)?;
    bridge::interrupted(deadline, cancelled)?;
    let runtime = json!({"bridgeVersion":bridge::BRIDGE_VERSION,"cutVersion":env!("CARGO_PKG_VERSION"),
        "rendererVersion":renderer.version,"executableFingerprint":renderer.fingerprint,
        "policy":{"network":"deny","clock":"frame","randomSeed":0,"unresolvedMedia":"error","authoredMediaFallback":false}});
    let input_fingerprint = cache_key_for_values("composition-inputs", &resolved.inputs, &runtime);
    let cache_key = cache_key_for_values(
        "composition",
        &json!({"inputs":resolved.inputs,"runtime":runtime}),
        &resolved.parameters,
    );
    let cache = root.child(".cut")?.child("cache")?;
    let derived = cache.child("derived")?;
    let entry = derived.child(&format!("composition-{cache_key}"))?;
    let _lock = entry.lock("render.lock", deadline)?;
    bridge::interrupted(deadline, cancelled)?;
    let artifact_name = format!("artifact.{}", mode.format());
    let artifact = json!({"path":artifact_name,"mediaType":mode.media_type()});
    let request = json!({
        "bridgeVersion":bridge::BRIDGE_VERSION,"rendererVersion":renderer.version,
        "cacheKey":cache_key,"inputFingerprint":input_fingerprint,"inputRoot":"inputs",
        "entry":resolved.inputs["entry"],"manifest":resolved.inputs["manifest"],"instance":resolved.inputs["instance"],
        "inputs":resolved.inputs,"mode":mode.name(),"parameters":resolved.parameters,
        "runtime":runtime,"artifact":artifact
    });
    let provenance = json!({"templateId":resolved.inputs["templateId"],"templateVersion":resolved.inputs["templateVersion"],
        "mode":mode.name(),"cacheKey":cache_key,"inputFingerprint":input_fingerprint,
        "rendererVersion":renderer.version,"rendererFingerprint":renderer.fingerprint,
        "bridgeVersion":bridge::BRIDGE_VERSION,"cutVersion":env!("CARGO_PKG_VERSION"),
        "output":resolved.inputs["output"],"parameters":resolved.parameters});
    if let Some((fingerprint, size)) = cached_artifact(&entry, &artifact_name, &request, mode)? {
        let record = record(
            &entry,
            &artifact_name,
            mode,
            &cache_key,
            &fingerprint,
            &provenance,
        );
        let asset = register(&cache, &record, deadline, || Ok(()))?;
        return Ok(result(
            &entry,
            mode,
            &request,
            &provenance,
            asset,
            size,
            true,
        ));
    }
    // Keep the renderer snapshot outside the project cache. TempDir removes it
    // on every exit path, including bridge failures, so repeated cache misses
    // cannot accumulate unbounded input trees under .cut.
    let work_dir = tempfile::Builder::new()
        .prefix("cut-composition-")
        .tempdir()
        .map_err(files::io)?;
    let work = Directory::project(work_dir.path())?;
    let inputs = work.child("inputs")?;
    for input in resolved.files.values_mut() {
        bridge::interrupted(deadline, cancelled)?;
        let mut output = inputs.create_file(&files::path(&input.path)?)?;
        if let Some(bytes) = &input.bytes {
            output.write_all(bytes).map_err(files::io)?;
        } else {
            let file = input.file.as_mut().expect("open media input");
            let (fingerprint, size) = files::fingerprint(file, Some(&mut output))?;
            if fingerprint != input.fingerprint || size != input.size {
                return Err(McpError::new(
                    "COMPOSITION_INPUT_CHANGED",
                    "input changed while snapshotting; retry resolution",
                ));
            }
        }
        output.sync_all().map_err(files::io)?;
    }
    bridge::invoke(renderer, &work, &request, deadline, cancelled)?;
    bridge::interrupted(deadline, cancelled)?;
    let mut output = work.open(Path::new(&artifact_name)).map_err(|error| {
        McpError::new(
            "COMPOSITION_ARTIFACT_INVALID",
            format!("renderer did not write a regular requested artifact: {error}"),
        )
    })?;
    let temporary = files::temporary_name("artifact");
    let mut publication = entry.create_file(Path::new(&temporary))?;
    let (fingerprint, size) = files::fingerprint(&mut output, Some(&mut publication))?;
    validate_artifact(&mut publication, size, mode)?;
    publication.sync_all().map_err(files::io)?;
    let record = record(
        &entry,
        &artifact_name,
        mode,
        &cache_key,
        &fingerprint,
        &provenance,
    );
    // Check immutable request/content linkage before replacing any published
    // artifact; the asset-manifest lock serializes against media.asset too.
    let asset = register(&cache, &record, deadline, || {
        entry.rename(&temporary, &artifact_name)
    })?;
    let completion =
        json!({"schemaVersion":1,"request":request,"contentFingerprint":fingerprint,"size":size});
    entry.atomic_write("complete.json", &json_bytes(&completion)?)?;
    Ok(result(
        &entry,
        mode,
        &request,
        &provenance,
        asset,
        size,
        false,
    ))
}

fn json_bytes(value: &Value) -> Result<Vec<u8>, McpError> {
    serde_json::to_vec(value).map_err(|error| McpError::new("COMPOSITION_IO", error.to_string()))
}

fn cached_artifact(
    entry: &Directory,
    name: &str,
    request: &Value,
    mode: Mode,
) -> Result<Option<(String, u64)>, McpError> {
    let bytes = match read_bounded(
        &entry.file,
        Path::new("complete.json"),
        bridge::MAX_REQUEST_BYTES + 4096,
        "completion",
    ) {
        Ok(bytes) => bytes,
        Err(_) => return Ok(None),
    };
    let Ok(completion) = serde_json::from_slice::<Value>(&bytes) else {
        return Ok(None);
    };
    if completion["schemaVersion"] != 1 || completion.get("request") != Some(request) {
        return Ok(None);
    }
    let Ok(mut file) = entry.open(Path::new(name)) else {
        return Ok(None);
    };
    let Ok((fingerprint, size)) = files::fingerprint(&mut file, None) else {
        return Ok(None);
    };
    if completion["contentFingerprint"] != fingerprint
        || completion["size"] != size
        || validate_artifact(&mut file, size, mode).is_err()
    {
        return Ok(None);
    }
    Ok(Some((fingerprint, size)))
}

fn validate_artifact(file: &mut File, size: u64, mode: Mode) -> Result<(), McpError> {
    file.seek(SeekFrom::Start(0)).map_err(files::io)?;
    let mut header = [0_u8; 12];
    let valid = size >= header.len() as u64
        && file.read_exact(&mut header).is_ok()
        && if mode == Mode::Preview {
            &header[..8] == b"\x89PNG\r\n\x1a\n"
        } else {
            &header[4..8] == b"ftyp"
        };
    if !valid {
        return Err(McpError::new(
            "COMPOSITION_ARTIFACT_INVALID",
            "artifact is empty, truncated or has an unexpected PNG/MP4 signature",
        ));
    }
    Ok(())
}

fn record(
    entry: &Directory,
    name: &str,
    mode: Mode,
    cache_key: &str,
    fingerprint: &str,
    provenance: &Value,
) -> GeneratedAssetRecord {
    GeneratedAssetRecord {
        // The request is part of the ID: identical bytes from different modes
        // or templates must retain distinct provenance rather than aliasing.
        asset_id: format!("asset-composition-{cache_key}"),
        path: entry.path.join(name).to_string_lossy().into_owned(),
        kind: mode.kind().into(),
        request_fingerprint: format!("composition-{cache_key}"),
        content_fingerprint: fingerprint.into(),
        backend: "composition-bridge".into(),
        provider: None,
        model: None,
        provenance: Some(provenance.clone()),
    }
}

fn read_assets(root: &Directory, allow_missing: bool) -> Result<GeneratedAssetManifest, McpError> {
    let path = Path::new(".cut/cache/assets.json");
    match composition::open_local(&root.file, path) {
        Err(error) if allow_missing && error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(GeneratedAssetManifest {
                schema_version: 1,
                assets: Vec::new(),
            });
        }
        Err(error) => {
            return Err(McpError::new(
                "COMPOSITION_ASSET_UNAVAILABLE",
                format!("asset provenance manifest unavailable: {error}"),
            ))
        }
        Ok(_) => (),
    }
    let bytes = read_bounded(
        &root.file,
        path,
        MAX_ASSET_MANIFEST_BYTES,
        "generated assets",
    )?;
    parse_assets(&bytes)
}

fn parse_assets(bytes: &[u8]) -> Result<GeneratedAssetManifest, McpError> {
    let manifest: GeneratedAssetManifest = serde_json::from_slice(bytes)
        .map_err(|error| McpError::new("COMPOSITION_ASSET_INVALID", error.to_string()))?;
    if manifest.schema_version != 1 || manifest.assets.len() > 4096 {
        return Err(McpError::new(
            "COMPOSITION_ASSET_INVALID",
            "asset manifest requires schema version 1 and at most 4096 records",
        ));
    }
    Ok(manifest)
}

fn register(
    cache: &Directory,
    record: &GeneratedAssetRecord,
    deadline: Instant,
    publish: impl FnOnce() -> Result<(), McpError>,
) -> Result<GeneratedAssetRecord, McpError> {
    let _lock = cache.lock("assets.json.lock", deadline)?;
    let mut manifest = match composition::open_local(&cache.file, Path::new("assets.json")) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => GeneratedAssetManifest {
            schema_version: 1,
            assets: Vec::new(),
        },
        Err(error) => return Err(files::io(error)),
        Ok(_) => parse_assets(&read_bounded(
            &cache.file,
            Path::new("assets.json"),
            MAX_ASSET_MANIFEST_BYTES,
            "generated assets",
        )?)?,
    };
    if let Some(existing) = manifest
        .assets
        .iter()
        .find(|asset| asset.request_fingerprint == record.request_fingerprint)
    {
        if existing.content_fingerprint != record.content_fingerprint
            || existing.asset_id != record.asset_id
            || existing.path != record.path
            || existing.provenance != record.provenance
            || existing.backend != record.backend
            || existing.kind != record.kind
        {
            return Err(McpError::new(
                "ASSET_REQUEST_CONFLICT",
                "composition request is already bound to different content or provenance",
            ));
        }
        publish()?;
        return Ok(existing.clone());
    }
    if manifest
        .assets
        .iter()
        .any(|asset| asset.asset_id == record.asset_id)
    {
        return Err(McpError::new(
            "ASSET_REQUEST_CONFLICT",
            "composition asset ID already has different provenance",
        ));
    }
    manifest.assets.push(record.clone());
    manifest
        .assets
        .sort_by(|left, right| left.asset_id.cmp(&right.asset_id));
    let bytes = serde_json::to_vec(&manifest)
        .map_err(|error| McpError::new("COMPOSITION_IO", error.to_string()))?;
    if bytes.len() > MAX_ASSET_MANIFEST_BYTES || manifest.assets.len() > 4096 {
        return Err(McpError::new(
            "COMPOSITION_LIMIT",
            "generated asset manifest is full",
        ));
    }
    publish()?;
    cache.atomic_write("assets.json", &bytes)?;
    Ok(record.clone())
}

fn result(
    entry: &Directory,
    mode: Mode,
    request: &Value,
    provenance: &Value,
    asset: GeneratedAssetRecord,
    size: u64,
    cached: bool,
) -> Value {
    json!({"mode":mode.name(),"artifact":{"path":entry.path.join(format!("artifact.{}",mode.format())),"mediaType":mode.media_type(),"size":size},
        "metadata":request["inputs"]["output"],"parameters":request["parameters"],
        "rendererVersion":request["rendererVersion"],"cacheKey":request["cacheKey"],"inputFingerprint":request["inputFingerprint"],
        "cached":cached,"asset":asset,"provenance":provenance,"canonicalStateChanged":false,"inserted":false})
}

#[cfg(test)]
#[path = "composition_render_tests.rs"]
mod tests;

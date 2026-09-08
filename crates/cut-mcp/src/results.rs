//! Bounded, project-local evidence for external MCP clients. Handles refer to
//! immutable content, never to arbitrary filesystem paths or executable calls.

use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::fs;
use std::io::{Read, Write};
use std::path::Path;

pub const MAX_INLINE_BYTES: usize = 16 * 1024;
const MAX_RESULT_BYTES: u64 = 64 * 1024 * 1024;
const MAX_CACHE_BYTES: u64 = 128 * 1024 * 1024;

fn valid_id(id: &str) -> bool {
    id.len() == 64 && id.bytes().all(|b| b.is_ascii_hexdigit())
}

pub(super) fn bound(directory: &Path, result: Value) -> Value {
    let bytes = match serde_json::to_vec(&result) {
        Ok(bytes) => bytes,
        Err(_) => return result,
    };
    if bytes.len() <= MAX_INLINE_BYTES {
        return result;
    }
    let summary = json!({"revision":result.get("revision"),"status":result.get("status"),
        "staged":result.get("staged"),"valid":result.get("valid")});
    match store(directory, &bytes) {
        Ok(id) => json!({"archived":true,"resultId":id,"totalBytes":bytes.len(),"summary":summary,
            "readWith":{"tool":"results.read","resultId":id},
            "note":"The operation already completed. Retrieve its historical result with results.read; do not repeat a mutation to retrieve output. Recheck current project state before editing."}),
        Err(error) => json!({"archived":false,"outputUnavailable":true,"summary":summary,
            "note":"The operation already completed, but its oversized output could not be cached. Recheck project state; do not repeat a mutation to retrieve output.","error":error}),
    }
}

fn store(directory: &Path, bytes: &[u8]) -> Result<String, String> {
    if bytes.len() as u64 > MAX_RESULT_BYTES {
        return Err("Result exceeds the 64 MiB evidence cache entry limit".into());
    }
    let root = directory.join(".cut/cache/tool-results");
    fs::create_dir_all(&root).map_err(|e| e.to_string())?;
    if let Ok(mut ignore) = fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(root.join(".gitignore"))
    {
        let _ = ignore.write_all(b"*\n");
    }
    let id = format!("{:x}", Sha256::digest(bytes));
    let path = root.join(format!("{id}.json"));
    // Lock publication and eviction across adapters for the same project.
    let lock = fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(root.join(".lock"))
        .map_err(|e| e.to_string())?;
    fs2::FileExt::lock_exclusive(&lock).map_err(|e| e.to_string())?;
    if !path.exists() {
        let temporary = root.join(format!("{}.tmp", super::operation_key("result")));
        let mut file = fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&temporary)
            .map_err(|e| e.to_string())?;
        file.write_all(bytes).map_err(|e| e.to_string())?;
        fs::rename(&temporary, &path).map_err(|e| e.to_string())?;
    }
    let mut entries = fs::read_dir(&root)
        .map_err(|e| e.to_string())?
        .filter_map(Result::ok)
        .filter(|entry| {
            entry.path().extension().is_some_and(|ext| ext == "json")
                && entry
                    .path()
                    .file_stem()
                    .and_then(|s| s.to_str())
                    .is_some_and(valid_id)
        })
        .filter_map(|entry| {
            entry
                .metadata()
                .ok()
                .map(|meta| (entry.path(), meta.len(), meta.modified().ok()))
        })
        .collect::<Vec<_>>();
    let mut total = entries.iter().map(|(_, size, _)| *size).sum::<u64>();
    entries.sort_by_key(|(_, _, modified)| *modified);
    for (candidate, size, _) in entries {
        if total <= MAX_CACHE_BYTES {
            break;
        }
        if candidate != path && fs::remove_file(candidate).is_ok() {
            total = total.saturating_sub(size);
        }
    }
    Ok(id)
}

pub(super) fn read(directory: &Path, input: &Value) -> Result<Value, String> {
    let id = input
        .get("resultId")
        .and_then(Value::as_str)
        .filter(|id| valid_id(id))
        .ok_or("resultId must be a returned evidence handle")?;
    let path = directory
        .join(".cut/cache/tool-results")
        .join(format!("{id}.json"));
    let metadata = fs::symlink_metadata(&path).map_err(|_|"Evidence is unavailable or evicted; query current state, and do not repeat a mutation to recover its result")?;
    if !metadata.file_type().is_file() || metadata.len() > MAX_RESULT_BYTES {
        return Err("Invalid cached evidence".into());
    }
    let mut bytes = Vec::new();
    fs::File::open(&path)
        .map_err(|e| e.to_string())?
        .take(MAX_RESULT_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if format!("{:x}", Sha256::digest(&bytes)) != id {
        return Err("Cached evidence failed its integrity check".into());
    }
    let value: Value = serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
    let pointer = input
        .get("pointer")
        .map(|v| v.as_str().ok_or("pointer must be a JSON pointer string"))
        .transpose()?
        .unwrap_or("");
    let selected = value.pointer(pointer).ok_or("JSON pointer not found")?;
    let serialized;
    let text = if let Some(text) = selected.as_str() {
        text
    } else {
        serialized = selected.to_string();
        &serialized
    };
    let offset = number(input, "offset", 0)?;
    let limit = number(input, "limit", 4096)?;
    if limit == 0 || limit > 4096 {
        return Err("limit must be between 1 and 4096 bytes".into());
    }
    if offset > text.len() || !text.is_char_boundary(offset) {
        return Err("offset is not a UTF-8 boundary in the result".into());
    }
    let mut end = offset.saturating_add(limit).min(text.len());
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    if end == offset && offset < text.len() {
        return Err("limit is too small for the next UTF-8 character".into());
    }
    loop {
        let page = json!({"resultId":id,"pointer":pointer,"offset":offset,"text":&text[offset..end],"totalBytes":text.len(),"nextOffset":(end<text.len()).then_some(end)});
        if page.to_string().len() <= MAX_INLINE_BYTES {
            return Ok(page);
        }
        let next_end = offset + (end - offset) / 2;
        end = next_end;
        while !text.is_char_boundary(end) {
            end -= 1;
        }
        if end == offset {
            return Err("JSON pointer or result page exceeds the response budget".into());
        }
    }
}

fn number(input: &Value, key: &str, default: usize) -> Result<usize, String> {
    input
        .get(key)
        .map(|v| {
            v.as_u64()
                .and_then(|v| usize::try_from(v).ok())
                .ok_or_else(|| format!("{key} must be a non-negative integer"))
        })
        .unwrap_or(Ok(default))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn large_result_is_retrievable_by_pointer_and_utf8_pages() {
        let dir = tempfile::tempdir().unwrap();
        let original = json!({"revision":42,"text":"한글".repeat(10000)});
        let reference = bound(dir.path(), original.clone());
        assert_eq!(reference["archived"], true);
        assert_eq!(reference["summary"]["revision"], 42);
        assert!(reference.to_string().len() < MAX_INLINE_BYTES);
        let mut offset = 0;
        let mut text = String::new();
        loop {
            let page = read(dir.path(),&json!({"resultId":reference["resultId"],"pointer":"/text","offset":offset,"limit":4096})).unwrap();
            text.push_str(page["text"].as_str().unwrap());
            match page["nextOffset"].as_u64() {
                Some(next) => offset = next,
                None => break,
            }
        }
        assert_eq!(text, original["text"].as_str().unwrap());
        assert!(read(dir.path(), &json!({"resultId":"../../secret"})).is_err());
    }
    #[test]
    fn result_handle_rejects_modified_evidence() {
        let dir = tempfile::tempdir().unwrap();
        let reference = bound(dir.path(), json!({"text":"x".repeat(20000)}));
        let id = reference["resultId"].as_str().unwrap();
        fs::write(
            dir.path()
                .join(".cut/cache/tool-results")
                .join(format!("{id}.json")),
            b"{}",
        )
        .unwrap();
        assert!(read(dir.path(), &json!({"resultId":id}))
            .unwrap_err()
            .contains("integrity"));
    }
}

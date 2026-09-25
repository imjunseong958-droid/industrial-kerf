use super::*;
use serde_json::json;
use std::fs;
use std::path::PathBuf;
use std::time::Duration;
use tempfile::TempDir;

#[cfg(unix)]
fn fixture_renderer(root: &TempDir) -> Renderer {
    use std::os::unix::fs::PermissionsExt;

    let path = root.path().join("renderer.sh");
    fs::write(
        &path,
        r#"#!/bin/sh
request=$(cat)
cache=$(printf '%s' "$request" | sed -n 's/.*"cacheKey":"\([^"]*\)".*/\1/p')
input=$(printf '%s' "$request" | sed -n 's/.*"inputFingerprint":"\([^"]*\)".*/\1/p')
case "$request" in
  *'"mediaType":"image/png"'*)
    printf '\211PNG\r\n\032\n0000' > artifact.png
    artifact='{"path":"artifact.png","mediaType":"image/png"}'
    ;;
  *)
    printf '0000ftypisom0000' > artifact.mp4
    artifact='{"path":"artifact.mp4","mediaType":"video/mp4"}'
    ;;
esac
printf '{"bridgeVersion":1,"rendererVersion":"fixture-v1","cacheKey":"%s","inputFingerprint":"%s","artifact":%s,"ok":true}' "$cache" "$input" "$artifact"
"#,
    )
    .unwrap();
    let mut permissions = fs::metadata(&path).unwrap().permissions();
    permissions.set_mode(0o700);
    fs::set_permissions(&path, permissions).unwrap();
    Renderer::new(path, "fixture-v1".into(), Duration::from_secs(5)).unwrap()
}

fn composition_files(root: &TempDir) {
    fs::write(
        root.path().join("index.html"),
        r#"<main data-composition-id="fixture" data-width="320" data-height="180" data-fps="30" data-duration="1"></main>"#,
    )
    .unwrap();
    fs::write(
        root.path().join("composition.manifest.json"),
        serde_json::to_vec(&json!({
            "schemaVersion": 1,
            "templateId": "fixture-template",
            "templateVersion": "1.0.0",
            "entry": "index.html",
            "output": {"width": 320, "height": 180, "fps": 30, "duration": 1},
            "slots": []
        }))
        .unwrap(),
    )
    .unwrap();
}

#[test]
fn composition_render_rejects_unregistered_media_without_renderer_fallback() {
    let root = TempDir::new().unwrap();
    fs::write(
        root.path().join("index.html"),
        r#"<main data-composition-id="fixture" data-width="320" data-height="180" data-fps="30" data-duration="1"><video data-var-clip></video></main>"#,
    )
    .unwrap();
    fs::write(
        root.path().join("composition.manifest.json"),
        serde_json::to_vec(&json!({
            "schemaVersion": 1,
            "templateId": "fixture-template",
            "templateVersion": "1.0.0",
            "entry": "index.html",
            "output": {"width": 320, "height": 180, "fps": 30, "duration": 1},
            "slots": [{"id":"clip","type":"video","required":true}]
        }))
        .unwrap(),
    )
    .unwrap();
    fs::write(
        root.path().join("instance.json"),
        serde_json::to_vec(&json!({
            "schemaVersion": 1,
            "templateId": "fixture-template",
            "templateVersion": "1.0.0",
            "overrides": {"clip":"asset-missing"}
        }))
        .unwrap(),
    )
    .unwrap();
    #[cfg(unix)]
    {
        let renderer = Renderer::new(
            std::env::current_exe().unwrap_or_else(|_| PathBuf::from("/usr/bin/true")),
            "fixture-v1".into(),
            Duration::from_secs(1),
        )
        .unwrap();
        let error = execute_with(
            root.path(),
            &json!({"manifest":"composition.manifest.json","instance":"instance.json"}),
            Mode::Preview,
            &renderer,
            None,
        )
        .unwrap_err();
        assert_eq!(error.code, "COMPOSITION_ASSET_UNAVAILABLE");
        assert!(!root.path().join("project.cut.json").exists());
    }
}

#[cfg(unix)]
#[test]
fn composition_preview_uses_verified_cache_and_preserves_canonical_files() {
    let root = TempDir::new().unwrap();
    composition_files(&root);
    let renderer_root = TempDir::new().unwrap();
    let renderer = fixture_renderer(&renderer_root);
    let arguments = json!({"manifest":"composition.manifest.json","frame":0});

    let first = execute_with(root.path(), &arguments, Mode::Preview, &renderer, None).unwrap();
    assert_eq!(first["cached"], false);
    assert_eq!(first["artifact"]["mediaType"], "image/png");
    assert!(first["cacheKey"].as_str().is_some());
    assert!(root.path().join(".cut/cache/assets.json").is_file());

    let second = execute_with(root.path(), &arguments, Mode::Preview, &renderer, None).unwrap();
    assert_eq!(second["cached"], true);
    assert_eq!(second["cacheKey"], first["cacheKey"]);
    assert_eq!(second["asset"]["assetId"], first["asset"]["assetId"]);

    let render = execute_with(
        root.path(),
        &json!({"manifest":"composition.manifest.json"}),
        Mode::Render,
        &renderer,
        None,
    )
    .unwrap();
    assert_eq!(render["cached"], false);
    assert_eq!(render["artifact"]["mediaType"], "video/mp4");
    assert_ne!(render["cacheKey"], first["cacheKey"]);
    assert!(!root.path().join("project.cut.json").exists());

    let derived = root.path().join(".cut/cache/derived");
    for cache_entry in fs::read_dir(derived).unwrap() {
        let cache_entry = cache_entry.unwrap();
        for child in fs::read_dir(cache_entry.path()).unwrap() {
            let child = child.unwrap();
            assert!(
                !child.file_name().to_string_lossy().starts_with(".work-"),
                "transient renderer snapshot leaked into cache"
            );
        }
    }
}

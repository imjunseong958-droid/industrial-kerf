use cut_domain::{empty_project, serialize_project};
use cut_mcp::{CutMcpRouter, McpProfile};
use serde_json::{json, Value};
use tempfile::TempDir;

fn router_with_clips(count: usize) -> (TempDir, CutMcpRouter) {
    let dir = TempDir::new().unwrap();
    let source = dir.path().join("source.mov");
    std::fs::write(&source, b"test").unwrap();
    let mut project = serialize_project(&empty_project()).unwrap();
    project["sources"] = json!([{"id":"source-1","path":source,"duration":300.0}]);
    project["tracks"] = json!([{"id":"video-1","type":"video","clips":[]}]);
    project["tracks"][0]["clips"] = Value::Array(
        (0..count)
            .map(|i| {
                json!({
                    "id": format!("clip-{i}"), "sourceId":"source-1", "in":0.0, "out":1.0,
                    "timelineStart": i as f64
                })
            })
            .collect(),
    );
    std::fs::write(
        dir.path().join("project.cut.json"),
        serde_json::to_vec(&project).unwrap(),
    )
    .unwrap();
    let cutd = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/debug/cutd");
    let router = CutMcpRouter::new(dir.path(), McpProfile::Core, Some(&cutd)).unwrap();
    (dir, router)
}

#[test]
fn outline_node_path_pages_retrieve_all_children() {
    let (_dir, router) = router_with_clips(237);
    let mut offset = 0;
    let mut seen = Vec::new();
    loop {
        let page = router.call(
            "project.outline",
            json!({"nodePath":"/tracks/0", "childOffset":offset, "childLimit":50}),
        );
        assert!(page["ok"].as_bool().unwrap_or(false), "{page}");
        let result = &page["result"]["root"]["children"][0]["children"];
        for child in result.as_array().unwrap() {
            seen.push(child["entityId"].as_str().unwrap().to_owned());
        }
        match page["result"]["childNextOffset"].as_u64() {
            Some(next) => offset = next as usize,
            None => break,
        }
    }
    assert_eq!(seen.len(), 237);
    assert_eq!(seen[0], "clip-0");
    assert_eq!(seen.last().unwrap(), "clip-236");
}

#[test]
fn describe_exposes_each_canonical_command_variant() {
    let (_dir, router) = router_with_clips(0);
    let types = router.call("tools.describe", json!({"name":"timeline.apply"}))["result"]
        ["commandTypes"]
        .as_array()
        .unwrap()
        .clone();
    for kind in types {
        let kind = kind.as_str().unwrap();
        let schema = router.call(
            "tools.describe",
            json!({"name":"timeline.apply", "commandType":kind}),
        );
        assert_eq!(
            schema["result"]["tool"]["inputSchema"]["properties"]["command"]["properties"]["type"]
                ["const"],
            kind
        );
        let example = &schema["result"]["example"]["command"];
        assert_eq!(example["type"], kind);
        cut_domain::parse_command(example).unwrap();
    }
    let search = router.call("tools.search", json!({"query":"crop"}));
    assert!(search["result"]["items"]
        .as_array()
        .unwrap()
        .iter()
        .any(|item| item["commandType"] == "update_clip"));
}

#[test]
fn queries_respect_half_open_ranges_and_batch_limits_without_mutation() {
    let (_dir, router) = router_with_clips(3);
    let page = router.call("timeline.query", json!({"start":1.0,"end":2.0}));
    let clips = page["result"]["tracks"][0]["clips"].as_array().unwrap();
    assert_eq!(clips.len(), 1);
    assert_eq!(clips[0]["id"], "clip-1");
    let invalid_range = router.call("timeline.query", json!({"start":2.0,"end":1.0}));
    assert_eq!(invalid_range["ok"], false);
    let before = router.runtime().get_state().unwrap();
    let batch = router.call(
        "timeline.apply",
        json!({"commands":vec![json!({"type":"remove_clip","clipId":"clip-0"});257]}),
    );
    assert_eq!(batch["ok"], false);
    assert_eq!(
        router.runtime().get_state().unwrap().revision,
        before.revision
    );
}

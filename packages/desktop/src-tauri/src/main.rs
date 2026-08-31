use cut_runtime::{
    probe, run_render, validate_actor, PendingProposal, RenderJob, RuntimeError, RuntimeHandle,
    StateSnapshot,
};
use serde::Deserialize;
use serde_json::Value;
use std::env;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::thread;
use tauri::{AppHandle, Manager, State};

type SharedRuntime = RuntimeHandle;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CommandRequest {
    command: Value,
    actor: String,
    base_revision: u64,
    idempotency_key: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RevisionRequest {
    actor: String,
    base_revision: u64,
    idempotency_key: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct AgentRequest {
    prompt: String,
    base_revision: u64,
    idempotency_key: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ProposalRequest {
    proposal_id: String,
    actor: String,
    base_revision: u64,
    idempotency_key: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RejectProposalRequest {
    proposal_id: String,
    #[serde(default)]
    actor: Option<String>,
    #[serde(default)]
    base_revision: Option<u64>,
    #[serde(default)]
    idempotency_key: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ImportRequest {
    path: String,
    actor: String,
    base_revision: u64,
    idempotency_key: String,
}

fn repository_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(3)
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."))
}

fn project_directory(app: &AppHandle) -> Result<PathBuf, RuntimeError> {
    if let Some(value) = env::var_os("CUT_PROJECT_DIR") {
        let path = PathBuf::from(value);
        std::fs::create_dir_all(&path).map_err(|error| RuntimeError {
            code: "IO_ERROR".into(),
            message: error.to_string(),
            expected_revision: None,
            actual_revision: None,
        })?;
        return Ok(path);
    }
    let path = if cfg!(debug_assertions) {
        repository_root().join(".cut").join("desktop-project")
    } else {
        app.path()
            .app_data_dir()
            .map_err(|error| RuntimeError {
                code: "IO_ERROR".into(),
                message: error.to_string(),
                expected_revision: None,
                actual_revision: None,
            })?
            .join("project")
    };
    std::fs::create_dir_all(&path).map_err(|error| RuntimeError {
        code: "IO_ERROR".into(),
        message: error.to_string(),
        expected_revision: None,
        actual_revision: None,
    })?;
    Ok(path)
}

#[tauri::command]
fn desktop_get_state(state: State<'_, SharedRuntime>) -> Result<StateSnapshot, RuntimeError> {
    state.inner().get_state()
}

#[tauri::command]
fn desktop_apply_command(
    request: CommandRequest,
    state: State<'_, SharedRuntime>,
) -> Result<StateSnapshot, RuntimeError> {
    if request.actor.trim().is_empty() || request.idempotency_key.trim().is_empty() {
        return Err(RuntimeError {
            code: "INVALID_REQUEST".into(),
            message: "actor and idempotencyKey are required".into(),
            expected_revision: None,
            actual_revision: None,
        });
    }
    validate_actor(&request.actor)?;
    state.inner().apply_request(
        &request.actor,
        &request.idempotency_key,
        request.command,
        request.base_revision,
    )
}

#[tauri::command]
fn desktop_undo(
    request: RevisionRequest,
    state: State<'_, SharedRuntime>,
) -> Result<StateSnapshot, RuntimeError> {
    if request.actor.trim().is_empty() || request.idempotency_key.trim().is_empty() {
        return Err(RuntimeError {
            code: "INVALID_REQUEST".into(),
            message: "actor and idempotencyKey are required".into(),
            expected_revision: None,
            actual_revision: None,
        });
    }
    validate_actor(&request.actor)?;
    state.inner().undo_request(
        &request.actor,
        &request.idempotency_key,
        request.base_revision,
    )
}

#[tauri::command]
fn desktop_redo(
    request: RevisionRequest,
    state: State<'_, SharedRuntime>,
) -> Result<StateSnapshot, RuntimeError> {
    if request.actor.trim().is_empty() || request.idempotency_key.trim().is_empty() {
        return Err(RuntimeError {
            code: "INVALID_REQUEST".into(),
            message: "actor and idempotencyKey are required".into(),
            expected_revision: None,
            actual_revision: None,
        });
    }
    validate_actor(&request.actor)?;
    state.inner().redo_request(
        &request.actor,
        &request.idempotency_key,
        request.base_revision,
    )
}

#[tauri::command]
fn desktop_start_render(
    quality: String,
    state: State<'_, SharedRuntime>,
) -> Result<RenderJob, RuntimeError> {
    let shared = state.inner().clone();
    let (job, project, output) = state.inner().start_render(&quality)?;
    let job_id = job.job_id.clone();
    let draft = quality == "preview";
    thread::spawn(move || {
        let _ = shared.update_job(&job_id, "running", None, None);
        let result = run_render(job_id.clone(), project, output, draft);
        match result {
            Ok(()) => {
                let _ = shared.update_job(&job_id, "completed", None, None);
            }
            Err(error) => {
                let _ = shared.update_job(&job_id, "failed", Some(error.message), None);
            }
        }
    });
    Ok(job)
}

#[tauri::command]
fn desktop_get_render(
    job_id: String,
    state: State<'_, SharedRuntime>,
) -> Result<RenderJob, RuntimeError> {
    state.inner().render_job(&job_id)
}

#[tauri::command]
fn desktop_probe(path: String) -> Result<Value, RuntimeError> {
    probe(&path)
}

#[tauri::command]
fn desktop_import_media(
    request: ImportRequest,
    state: State<'_, SharedRuntime>,
) -> Result<Value, RuntimeError> {
    if request.actor.trim().is_empty() || request.idempotency_key.trim().is_empty() {
        return Err(RuntimeError {
            code: "INVALID_REQUEST".into(),
            message: "actor and idempotencyKey are required".into(),
            expected_revision: None,
            actual_revision: None,
        });
    }
    validate_actor(&request.actor)?;
    let (snapshot, source_id, clip_id) = state.inner().import_media_request(
        &request.actor,
        &request.idempotency_key,
        &request.path,
        request.base_revision,
        None,
        None,
    )?;
    Ok(serde_json::json!({ "sourceId": source_id, "clipId": clip_id, "state": snapshot }))
}

/// Run an explicitly supplied agent adapter. The adapter is intentionally a
/// process boundary: it receives a snapshot and returns semantic commands;
/// only this runtime applies them and advances canonical history.
#[tauri::command]
fn desktop_run_agent(
    request: AgentRequest,
    state: State<'_, SharedRuntime>,
) -> Result<Value, RuntimeError> {
    let binary = env::var_os("CUT_AGENT_BIN").ok_or_else(|| RuntimeError {
        code: "AGENT_UNAVAILABLE".into(),
        message: "set CUT_AGENT_BIN to an agent adapter executable".into(),
        expected_revision: None,
        actual_revision: None,
    })?;
    let snapshot = state.inner().get_state()?;
    if request.idempotency_key.trim().is_empty() {
        return Err(RuntimeError {
            code: "INVALID_REQUEST".into(),
            message: "idempotencyKey is required".into(),
            expected_revision: None,
            actual_revision: None,
        });
    }
    if request.base_revision != snapshot.revision {
        return Err(RuntimeError::conflict(
            request.base_revision,
            snapshot.revision,
        ));
    }
    if matches!(
        snapshot.pending_proposal,
        Some(PendingProposal::Pending { .. })
    ) {
        return Err(RuntimeError::new(
            "PROPOSAL_PENDING",
            "another agent proposal is awaiting approval",
        ));
    }
    let input =
        serde_json::json!({ "prompt": request.prompt, "state": snapshot, "actor": "agent" });
    let mut child = Command::new(binary)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| RuntimeError {
            code: "AGENT_ERROR".into(),
            message: error.to_string(),
            expected_revision: None,
            actual_revision: None,
        })?;
    if let Some(mut stdin) = child.stdin.take() {
        serde_json::to_writer(&mut stdin, &input).map_err(|error| RuntimeError {
            code: "AGENT_PROTOCOL_ERROR".into(),
            message: error.to_string(),
            expected_revision: None,
            actual_revision: None,
        })?;
    }
    let output = child.wait_with_output().map_err(|error| RuntimeError {
        code: "AGENT_ERROR".into(),
        message: error.to_string(),
        expected_revision: None,
        actual_revision: None,
    })?;
    if !output.status.success() {
        return Err(RuntimeError {
            code: "AGENT_ERROR".into(),
            message: String::from_utf8_lossy(&output.stderr).trim().to_string(),
            expected_revision: None,
            actual_revision: None,
        });
    }
    let response: Value = serde_json::from_slice(&output.stdout).map_err(|error| RuntimeError {
        code: "AGENT_PROTOCOL_ERROR".into(),
        message: error.to_string(),
        expected_revision: None,
        actual_revision: None,
    })?;
    let protocol_version = response.get("protocolVersion").and_then(Value::as_u64);
    if protocol_version != Some(1) {
        return Err(RuntimeError {
            code: "AGENT_PROTOCOL_ERROR".into(),
            message: "agent response must declare protocolVersion 1".into(),
            expected_revision: None,
            actual_revision: None,
        });
    }
    let transactions = response
        .get("transactions")
        .and_then(Value::as_array)
        .ok_or_else(|| RuntimeError {
            code: "AGENT_PROTOCOL_ERROR".into(),
            message: "agent response must contain a transactions array".into(),
            expected_revision: None,
            actual_revision: None,
        })?;
    let next_state = state.inner().stage_agent_proposal(
        &request.idempotency_key,
        request.base_revision,
        transactions.clone(),
    )?;
    let mut result = response;
    if let Some(object) = result.as_object_mut() {
        object.insert(
            "state".into(),
            serde_json::to_value(&next_state).unwrap_or(Value::Null),
        );
        object.insert(
            "proposal".into(),
            serde_json::to_value(next_state.pending_proposal.clone()).unwrap_or(Value::Null),
        );
    }
    Ok(result)
}

#[tauri::command]
fn desktop_accept_proposal(
    request: ProposalRequest,
    state: State<'_, SharedRuntime>,
) -> Result<StateSnapshot, RuntimeError> {
    if request.proposal_id.trim().is_empty()
        || request.actor.trim().is_empty()
        || request.idempotency_key.trim().is_empty()
    {
        return Err(RuntimeError {
            code: "INVALID_REQUEST".into(),
            message: "proposalId, actor, and idempotencyKey are required".into(),
            expected_revision: None,
            actual_revision: None,
        });
    }
    validate_actor(&request.actor)?;
    state.inner().accept_proposal(
        &request.proposal_id,
        &request.actor,
        &request.idempotency_key,
        request.base_revision,
    )
}

#[tauri::command]
fn desktop_reject_proposal(
    request: RejectProposalRequest,
    state: State<'_, SharedRuntime>,
) -> Result<StateSnapshot, RuntimeError> {
    if request.proposal_id.trim().is_empty() {
        return Err(RuntimeError {
            code: "INVALID_REQUEST".into(),
            message: "proposalId is required".into(),
            expected_revision: None,
            actual_revision: None,
        });
    }
    match (
        request.actor,
        request.idempotency_key,
        request.base_revision,
    ) {
        (None, None, None) => state.inner().reject_proposal(&request.proposal_id),
        (Some(actor), Some(idempotency_key), Some(base_revision)) => {
            if actor.trim().is_empty() || idempotency_key.trim().is_empty() {
                return Err(RuntimeError {
                    code: "INVALID_REQUEST".into(),
                    message: "actor and idempotencyKey are required".into(),
                    expected_revision: None,
                    actual_revision: None,
                });
            }
            validate_actor(&actor)?;
            state.inner().reject_proposal_request(
                &request.proposal_id,
                &actor,
                &idempotency_key,
                base_revision,
            )
        }
        _ => Err(RuntimeError {
            code: "INVALID_REQUEST".into(),
            message: "actor, idempotencyKey, and baseRevision must be supplied together".into(),
            expected_revision: None,
            actual_revision: None,
        }),
    }
}

fn main() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            let directory = project_directory(app.handle())
                .map_err(|error| std::io::Error::other(error.message))?;
            let runtime = RuntimeHandle::open(&directory)
                .map_err(|error| std::io::Error::other(error.message))?;
            app.manage(runtime as SharedRuntime);
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            desktop_get_state,
            desktop_apply_command,
            desktop_undo,
            desktop_redo,
            desktop_start_render,
            desktop_get_render,
            desktop_probe,
            desktop_import_media,
            desktop_run_agent,
            desktop_accept_proposal,
            desktop_reject_proposal,
        ])
        .run(tauri::generate_context!())
        .expect("error while running Cut desktop application");
}

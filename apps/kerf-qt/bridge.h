#pragma once
#include <stdint.h>
#include <stddef.h>

// Calls are nonblocking and confined to the Qt GUI thread. Runtime, media and
// agent work run on Rust workers. All strings are UTF-8.
extern "C" {
void *kerf_bridge_new(const char *project_directory, const char *kerfd_binary);
void kerf_bridge_free(void *bridge);
// JSON commands: refresh, reconnect, preview, import {path}, edit {ops,revision},
// undo/redo {revision}, accept/reject {proposalId,revision}, agent {text,
// provider,apiMode,apiKey,baseUrl,model,mcpEnabled,mcpConfigVersion}, abort,
// play, pause, seek {position,scrub}, volume {value}, export {quality},
// effect_previews {clip,at,effects,width}. Agent
// settings are request-scoped so switching providers never mutates the
// project or leaks credentials into its state.
// Projection: projection_import loads a project.kerf changed outside Kerf
// (a git pull in a terminal, a text editor) as one revision;
// projection_repair writes the editor's project over it.
// Collaboration over Git, all asynchronous and answered by collab events:
// collab_status; collab_sync {message?} commits local edits, pulls and
// merges the upstream branch, and pushes; collab_live {enabled, interval?}
// syncs every interval seconds (default 10) while enabled; collab_commit
// {message}; collab_branch {name, create} commits local edits, then switches
// (or creates and switches); collab_pull_request {title, body?};
// collab_publish {remote?, github?: {name, private}} adds a remote or
// creates the GitHub repository with gh, then pushes; collab_consolidate
// copies media from outside the project into media/ and relinks it.
// false means the bounded queue is full; show an error, never silently drop edits.
bool kerf_bridge_send(void *bridge, const char *json);
// One JSON event, owned until kerf_bridge_string_free; nullptr when idle.
// Events: state {snapshot}, connection {status,message}, error {message},
// agent {kind,text}, agent_busy {busy}, agent_state {state,message},
// clock {position,duration}, playback {playing}, caption {text},
// viewer_reset, preview {status,progress,message}, export {status,path,progress},
// effect_preview {clip,fx,width,height,rgba (base64)},
// collab {status: {repository, branch, upstream, remote, ahead, behind,
// changed, conflicted, branches, lfs, gh, live, externalMedia}},
// collab_sync {stage: fetching|merging|pushing|done|failed, message?,
// report?: {committed?, incoming: [{commit, author, subject}], pushed,
// conflicts: [{path, what, kept}], renamed: [[from, to]], summary: [line]}},
// collab_pull_request {url, created}.
char *kerf_bridge_poll(void *bridge);
void kerf_bridge_string_free(char *value);
struct KerfFrame { const uint8_t *pixels; size_t length; uint32_t width; uint32_t height; double time; };
// Latest RGBA frame only. Bytes live until the next frame poll or bridge free.
bool kerf_bridge_frame(void *bridge, KerfFrame *frame);
// The command registry (kerf-commands). Synchronous and pure, so it can be
// called before a bridge exists. Results are owned JSON strings, freed with
// kerf_bridge_string_free.
// The registry with shortcuts in a keymap id: {keymap, keymaps, menus, commands}.
char *kerf_commands_catalog(const char *keymap);
// {commandId: reason or null} for a JSON editor context.
char *kerf_commands_states(const char *context);
// {command, params, context} -> {request} (a kerf_bridge_send command) or {error}.
char *kerf_commands_run(const char *invocation);
// The picture effects and transitions offered, as a JSON array of
// {id, kind, category, name, doc, engine, params}. Free with kerf_bridge_string_free.
char *kerf_fx_catalog(void);
// The project launcher, before any bridge exists. {path} -> {path, exists,
// project, name, repository, branch, remote, modified (ISO 8601)}.
char *kerf_project_inspect(const char *json);
// Project tasks that may take a while, polled from the GUI thread:
// {type: "create", name, location, template: standard|vertical|square,
// rate: "23.976"|"24"|"25"|"29.97"|"30"|"60", git, remote?, github?:
// {private}}; {type: "clone", url, location}; {type: "tools"} -> {git,
// gitLfs, gh, ghUser}. Poll returns {status: "running", message?},
// {status: "done", path?, remote?, warnings?, ...} or {status: "failed",
// error}; each call returns an owned string.
void *kerf_task_start(const char *json);
char *kerf_task_poll(void *task);
void kerf_task_free(void *task);
}

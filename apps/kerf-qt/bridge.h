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
// play, pause, seek {position,scrub}, volume {value}, export {quality}. Agent
// settings are request-scoped so switching providers never mutates the
// project or leaks credentials into its state.
// false means the bounded queue is full; show an error, never silently drop edits.
bool kerf_bridge_send(void *bridge, const char *json);
// One JSON event, owned until kerf_bridge_string_free; nullptr when idle.
// Events: state {snapshot}, connection {status,message}, error {message},
// agent {kind,text}, agent_busy {busy}, agent_state {state,message},
// clock {position,duration}, playback {playing}, caption {text},
// viewer_reset, preview {status,progress,message}, export {status,path,progress}.
char *kerf_bridge_poll(void *bridge);
void kerf_bridge_string_free(char *value);
struct CutFrame { const uint8_t *pixels; size_t length; uint32_t width; uint32_t height; double time; };
// Latest RGBA frame only. Bytes live until the next frame poll or bridge free.
bool kerf_bridge_frame(void *bridge, CutFrame *frame);
// The command registry (kerf-commands). Synchronous and pure, so it can be
// called before a bridge exists. Results are owned JSON strings, freed with
// kerf_bridge_string_free.
// The registry with shortcuts in a keymap id: {keymap, keymaps, menus, commands}.
char *kerf_commands_catalog(const char *keymap);
// {commandId: reason or null} for a JSON editor context.
char *kerf_commands_states(const char *context);
// {command, params, context} -> {request} (a kerf_bridge_send command) or {error}.
char *kerf_commands_run(const char *invocation);
}

#pragma once
#include <stdint.h>
#include <stddef.h>

// Calls are nonblocking and confined to the Qt GUI thread. Runtime, media and
// agent work run on Rust workers. All strings are UTF-8.
extern "C" {
void *cut_bridge_new(const char *project_directory, const char *cutd_binary);
void cut_bridge_free(void *bridge);
// JSON commands: refresh, import {path}, apply {command,revision}, undo/redo
// {revision}, accept/reject {proposalId,revision}, agent {text}, abort,
// play, pause, seek {position}, volume {value}, export {quality}.
// false means the bounded queue is full; show an error, never silently drop edits.
bool cut_bridge_send(void *bridge, const char *json);
// One JSON event, owned until cut_bridge_string_free; nullptr when idle.
// Events: state {snapshot}, error {message}, agent {kind,text},
// agent_busy {busy}, clock {position,duration}, playback {playing},
// export {status,path,progress}.
char *cut_bridge_poll(void *bridge);
void cut_bridge_string_free(char *value);
struct CutFrame { const uint8_t *pixels; size_t length; uint32_t width; uint32_t height; double time; };
// Latest RGBA frame only. Bytes live until the next frame poll or bridge free.
bool cut_bridge_frame(void *bridge, CutFrame *frame);
}

#!/usr/bin/env python3
"""Exercise the real Qt C ABI, daemon, decoder, and agent proposal loop locally.
Requires built cutd/libcut_qt_bridge and ffmpeg. No external provider is contacted.
"""
import ctypes as C
import http.server
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import threading
import time

ROOT = Path(__file__).resolve().parents[1]
requests = []

class Provider(http.server.BaseHTTPRequestHandler):
    def log_message(self, *_):
        pass

    def do_POST(self):
        request = json.loads(self.rfile.read(int(self.headers['Content-Length'])))
        requests.append(request)
        if len(requests) == 1:
            message = {'role': 'assistant', 'content': None, 'tool_calls': [{
                'id': 'edit-1', 'type': 'function', 'function': {
                    'name': 'timeline.apply',
                    'arguments': json.dumps({'command': {'type': 'set_metadata', 'patch': {'name': 'Agent reviewed'}}})
                }}]}
            finish = 'tool_calls'
        else:
            message = {'role': 'assistant', 'content': 'Changes are ready for your review.'}
            finish = 'stop'
        data = json.dumps({'choices': [{'message': message, 'finish_reason': finish}]}).encode()
        self.send_response(200)
        self.send_header('Content-Type', 'application/json')
        self.send_header('Content-Length', str(len(data)))
        self.end_headers()
        self.wfile.write(data)

server = http.server.ThreadingHTTPServer(('127.0.0.1', 0), Provider)
threading.Thread(target=server.serve_forever, daemon=True).start()
os.environ.update(CUT_PROVIDER='openai', OPENAI_API_MODE='chat', OPENAI_API_KEY='local-test-only', OPENAI_BASE_URL=f'http://127.0.0.1:{server.server_port}', CUT_MODEL='local-fixture')
os.environ.pop('OPENAI_REASONING_EFFORT', None)
lib = C.CDLL(str(ROOT / 'target/debug' / ('libcut_qt_bridge.dylib' if sys.platform == 'darwin' else 'libcut_qt_bridge.so')))
lib.cut_bridge_new.argtypes = [C.c_char_p, C.c_char_p]
lib.cut_bridge_new.restype = C.c_void_p
lib.cut_bridge_free.argtypes = [C.c_void_p]
lib.cut_bridge_send.argtypes = [C.c_void_p, C.c_char_p]
lib.cut_bridge_send.restype = C.c_bool
lib.cut_bridge_poll.argtypes = [C.c_void_p]
lib.cut_bridge_poll.restype = C.c_void_p
lib.cut_bridge_string_free.argtypes = [C.c_void_p]

class Frame(C.Structure):
    _fields_ = [('pixels', C.POINTER(C.c_uint8)), ('length', C.c_size_t), ('width', C.c_uint32), ('height', C.c_uint32), ('time', C.c_double)]

lib.cut_bridge_frame.argtypes = [C.c_void_p, C.POINTER(Frame)]
lib.cut_bridge_frame.restype = C.c_bool

with tempfile.TemporaryDirectory(prefix='cut-qt-', dir=ROOT / 'target') as temporary:
    directory = Path(temporary)
    media = directory / 'fixture.mp4'
    subprocess.run(['ffmpeg', '-hide_banner', '-loglevel', 'error', '-f', 'lavfi', '-i', 'testsrc2=size=640x360:rate=30', '-t', '4', '-c:v', 'libx264', '-preset', 'ultrafast', str(media)], check=True)
    daemon = subprocess.Popen([str(ROOT / 'target/debug/cutd'), '--dir', str(directory)], stdout=subprocess.DEVNULL, stderr=subprocess.PIPE)
    bridge = lib.cut_bridge_new(os.fsencode(directory), os.fsencode(ROOT / 'target/debug/cutd'))
    assert bridge
    observed = []
    timings = []

    def send(command):
        start = time.perf_counter()
        assert lib.cut_bridge_send(bridge, json.dumps(command).encode()), command
        timings.append((time.perf_counter() - start) * 1000)

    def poll():
        pointer = lib.cut_bridge_poll(bridge)
        if not pointer:
            return None
        value = json.loads(C.string_at(pointer))
        lib.cut_bridge_string_free(pointer)
        observed.append(value)
        if value['type'] == 'error':
            raise AssertionError(value)
        return value

    def wait(predicate, timeout=20):
        deadline = time.monotonic() + timeout
        while time.monotonic() < deadline:
            event = poll()
            if event and predicate(event):
                return event
            time.sleep(.005)
        raise AssertionError(f'Timeout, recent events: {observed[-8:]}')

    def state():
        return wait(lambda event: event['type'] == 'state')['snapshot']

    try:
        initial = state()
        send({'type': 'import', 'path': str(media)})
        imported = state()
        assert imported['revision'] > initial['revision']
        clip = imported['project']['tracks'][0]['clips'][0]
        send({'type': 'apply', 'revision': imported['revision'], 'command': {'type': 'trim_clip', 'clipId': clip['id'], 'edge': 'trailing', 'timelineFrame': 90}})
        trimmed = state()
        assert abs(trimmed['duration'] - 3) < .05
        send({'type': 'undo', 'revision': trimmed['revision']})
        undone = state()
        assert abs(undone['duration'] - 4) < .05
        send({'type': 'redo', 'revision': undone['revision']})
        redone = state()
        for i in range(200):
            send({'type': 'seek', 'position': 2.0 * i / 199})
        frame = Frame()
        deadline = time.monotonic() + 15
        while time.monotonic() < deadline:
            poll()
            if lib.cut_bridge_frame(bridge, C.byref(frame)) and frame.time >= 1.95:
                assert frame.length == frame.width * frame.height * 4
                assert max(frame.width, frame.height) <= 1280
                assert C.string_at(frame.pixels, 16)
                break
            time.sleep(.005)
        else:
            raise AssertionError('Latest seek frame did not arrive')
        send({'type': 'agent', 'text': 'Rename this project and show me the proposal.'})
        proposal_state = state()
        proposal = proposal_state['pendingProposal']
        assert proposal, observed[-10:]
        assert proposal_state['revision'] == redone['revision']
        proposal_id = proposal.get('id', proposal.get('proposalId'))
        assert proposal_id, proposal
        send({'type': 'accept', 'proposalId': proposal_id, 'revision': proposal_state['revision']})
        accepted = state()
        assert accepted['project']['metadata']['name'] == 'Agent reviewed'
        send({'type': 'agent', 'text': 'Keep the previous change.'})
        wait(lambda event: event['type'] == 'agent_busy' and event['busy'] is False)
        assert len(requests) == 3
        assert any('Rename this project' in json.dumps(message) for message in requests[-1]['messages'])
        refreshed = state()  # Agent refresh completes before the export.
        send({'type': 'export', 'quality': 'preview'})
        exported = wait(lambda event: event['type'] == 'export' and event['status'] == 'completed', 60)
        assert Path(exported['path']).is_file()
        send({'type': 'apply', 'revision': refreshed['revision'], 'command': {'type': 'update_clip', 'clipId': clip['id'], 'patch': {'opacity': 0.5}}})
        state()
        wait(lambda event: event['type'] == 'preview' and event['status'] == 'required')
        send({'type': 'preview'})
        wait(lambda event: event['type'] == 'preview' and event['status'] == 'ready', 60)
        assert any(event.get('kind') == 'tool' for event in observed)
        print(json.dumps({'result': 'passed', 'checks': ['import', 'trim', 'undo', 'redo', '200 seeks/latest frame', 'local provider/tool activity', 'proposal/accept', 'conversation history', 'real export', 'effect rendered fallback'], 'max_enqueue_ms': round(max(timings), 3), 'frame': [frame.width, frame.height]}, indent=2))
    finally:
        lib.cut_bridge_free(bridge)
        daemon.terminate()
        try:
            daemon.wait(timeout=5)
        except subprocess.TimeoutExpired:
            daemon.kill()
        server.shutdown()

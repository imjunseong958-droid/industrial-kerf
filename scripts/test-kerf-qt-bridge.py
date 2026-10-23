#!/usr/bin/env python3
"""Exercise the real Qt C ABI, daemon, decoder, and agent proposal loop locally.
Requires built kerfd/libkerf_qt_bridge and ffmpeg. No external provider is contacted.
"""
import base64
import ctypes as C
import http.server
import json
import os
from pathlib import Path
import subprocess
import socket
import sys
import tempfile
import threading
import time

ROOT = Path(__file__).resolve().parents[1]
requests = []

class Provider(http.server.BaseHTTPRequestHandler):
    def log_message(self, *_):
        pass

    def reply(self, status, value):
        data = json.dumps(value).encode()
        self.send_response(status)
        self.send_header('Content-Type', 'application/json')
        self.send_header('Content-Length', str(len(data)))
        self.end_headers()
        self.wfile.write(data)

    # A stand-in for Kerf Cloud: the account, and the hosted model's messages.
    def do_GET(self):
        if self.path == '/v1/account' and self.headers.get('Authorization') == 'Bearer tok-cloud':
            self.reply(200, {'email': 'editor@example.com', 'plan': 'Pro', 'ai': True, 'credits': 42})
        else:
            self.reply(401, {})

    def do_POST(self):
        request = json.loads(self.rfile.read(int(self.headers['Content-Length'])))
        if self.path == '/v1/messages':
            cloud_requests.append((self.headers.get('x-api-key'), request))
            self.reply(200, {'type': 'message', 'role': 'assistant', 'stop_reason': 'end_turn',
                             'content': [{'type': 'text', 'text': 'Cloud reply.'}]})
            return
        requests.append(request)
        if len(requests) == 1:
            system = request['messages'][0]['content']
            assert request['messages'][0]['role'] == 'system', request['messages'][0]
            assert '# Kerf edit script' in system, system[:400]
            # The project document arrives as a message after the system prompt.
            assert any('```kerf' in str(m.get('content')) for m in request['messages'][1:]), request['messages'][1:]
            assert [tool['function']['name'] for tool in request['tools']] == ['edit', 'look', 'render', 'suggest', 'session.read'], request['tools']
            assert '`suggest`' in system, system[-600:]
            message = {'role': 'assistant', 'content': None, 'tool_calls': [{
                'id': 'edit-1', 'type': 'function', 'function': {
                    'name': 'edit',
                    'arguments': json.dumps({'script': 'marker add at=0f name="Agent reviewed"'})
                }}]}
            finish = 'tool_calls'
        elif len(requests) == 2:
            # The editor's agent ends its turn with next steps as buttons.
            message = {'role': 'assistant', 'content': None, 'tool_calls': [{
                'id': 'suggest-1', 'type': 'function', 'function': {
                    'name': 'suggest',
                    'arguments': json.dumps({'steps': [
                        {'label': 'Add a glow', 'request': 'Add a soft glow to the opening clip.'},
                        {'label': 'Wipe to the next shot', 'request': 'Use a linear wipe into the next shot.'}]})
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

cloud_requests = []
server = http.server.ThreadingHTTPServer(('127.0.0.1', 0), Provider)
threading.Thread(target=server.serve_forever, daemon=True).start()
os.environ.update(KERF_PROVIDER='openai', OPENAI_API_MODE='chat', OPENAI_API_KEY='local-test-only', OPENAI_BASE_URL=f'http://127.0.0.1:{server.server_port}', KERF_MODEL='local-fixture')
os.environ.pop('OPENAI_REASONING_EFFORT', None)
lib = C.CDLL(str(ROOT / 'target/debug' / ('libkerf_qt_bridge.dylib' if sys.platform == 'darwin' else 'libkerf_qt_bridge.so')))
lib.kerf_bridge_new.argtypes = [C.c_char_p, C.c_char_p]
lib.kerf_bridge_new.restype = C.c_void_p
lib.kerf_bridge_free.argtypes = [C.c_void_p]
lib.kerf_bridge_send.argtypes = [C.c_void_p, C.c_char_p]
lib.kerf_bridge_send.restype = C.c_bool
lib.kerf_bridge_poll.argtypes = [C.c_void_p]
lib.kerf_bridge_poll.restype = C.c_void_p
lib.kerf_bridge_string_free.argtypes = [C.c_void_p]

class Frame(C.Structure):
    _fields_ = [('pixels', C.POINTER(C.c_uint8)), ('length', C.c_size_t), ('width', C.c_uint32), ('height', C.c_uint32), ('time', C.c_double)]

lib.kerf_commands_run.argtypes = [C.c_char_p]
lib.kerf_commands_run.restype = C.c_void_p
lib.kerf_fx_catalog.argtypes = []
lib.kerf_fx_catalog.restype = C.c_void_p

def run_command(invocation):
    pointer = lib.kerf_commands_run(json.dumps(invocation).encode())
    value = json.loads(C.string_at(pointer))
    lib.kerf_bridge_string_free(pointer)
    assert 'request' in value, value
    return value['request']

lib.kerf_bridge_frame.argtypes = [C.c_void_p, C.POINTER(Frame)]
lib.kerf_bridge_frame.restype = C.c_bool

def runtime_endpoint(directory):
    # Ask kerfd: a project path too long for a socket moves it to a runtime directory.
    return subprocess.run([str(ROOT / 'target/debug/kerfd'), '--dir', str(directory), '--endpoint'],
                          check=True, capture_output=True, text=True).stdout.strip()

def runtime_request(endpoint, request):
    payload = {'protocolVersion': 1, 'id': f'qt-test-{time.time_ns()}'}
    payload.update(request)
    with socket.socket(socket.AF_UNIX, socket.SOCK_STREAM) as connection:
        connection.settimeout(5)
        connection.connect(os.fsencode(endpoint))
        connection.sendall(json.dumps(payload).encode() + b'\n')
        response = b''
        while not response.endswith(b'\n'):
            chunk = connection.recv(65536)
            if not chunk:
                raise RuntimeError('kerfd closed the socket before replying')
            response += chunk
    return json.loads(response)

with tempfile.TemporaryDirectory(prefix='kerf-qt-', dir=ROOT / 'target') as temporary:
    directory = Path(temporary)
    # The Kerf Cloud session lives in ~/.kerf; keep it inside the test.
    os.environ.update(HOME=str(directory / 'home'), KERF_CLOUD_URL=f'http://127.0.0.1:{server.server_port}')
    media = directory / 'fixture.mp4'
    subprocess.run(['ffmpeg', '-hide_banner', '-loglevel', 'error', '-f', 'lavfi', '-i', 'testsrc2=size=640x360:rate=30', '-t', '4', '-c:v', 'libx264', '-preset', 'ultrafast', str(media)], check=True)
    daemon = subprocess.Popen([str(ROOT / 'target/debug/kerfd'), '--dir', str(directory)], stdout=subprocess.DEVNULL, stderr=subprocess.PIPE)
    endpoint = runtime_endpoint(directory)
    deadline = time.monotonic() + 5
    while time.monotonic() < deadline:
        if daemon.poll() is not None:
            raise RuntimeError('kerfd exited before opening its socket')
        try:
            with socket.socket(socket.AF_UNIX, socket.SOCK_STREAM) as connection:
                connection.settimeout(.2)
                connection.connect(os.fsencode(endpoint))
            break
        except OSError:
            time.sleep(.02)
    else:
        daemon.terminate()
        raise RuntimeError('kerfd did not open its socket')
    bridge = lib.kerf_bridge_new(os.fsencode(directory), os.fsencode(ROOT / 'target/debug/kerfd'))
    assert bridge
    observed = []
    connection_statuses = []
    timings = []

    def send(command):
        start = time.perf_counter()
        assert lib.kerf_bridge_send(bridge, json.dumps(command).encode()), command
        timings.append((time.perf_counter() - start) * 1000)

    def poll():
        pointer = lib.kerf_bridge_poll(bridge)
        if not pointer:
            return None
        value = json.loads(C.string_at(pointer))
        lib.kerf_bridge_string_free(pointer)
        observed.append(value)
        if value['type'] == 'connection':
            connection_statuses.append(value['status'])
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
        wait(lambda event: event['type'] == 'connection' and event['status'] == 'ready')
        assert 'connecting' in connection_statuses
        send({'type': 'import', 'path': str(media)})
        imported = state()
        assert imported['revision'] > initial['revision']
        clip = imported['timeline']['tracks'][0]['clips'][0]
        end_frame = clip['startFrame'] + clip['durationFrames']
        target_frame = round(3 * imported['timeline']['fps'])
        send({'type': 'edit', 'revision': imported['revision'], 'ops': [{'op': 'trim', 'clip': clip['id'], 'edge': 'out', 'frames': target_frame - end_frame}]})
        trimmed = state()
        assert abs(trimmed['duration'] - 3) < .05
        send({'type': 'undo', 'revision': trimmed['revision']})
        undone = state()
        assert abs(undone['duration'] - 4) < .05
        send({'type': 'redo', 'revision': undone['revision']})
        redone = state()
        # The effects offered, how one looks on the clip, and adding it.
        pointer = lib.kerf_fx_catalog()
        catalog = json.loads(C.string_at(pointer))
        lib.kerf_bridge_string_free(pointer)
        assert any(fx['id'] == 'stylize.glow' and fx['params'] for fx in catalog), catalog[:2]
        send({'type': 'effect_previews', 'clip': clip['id'], 'at': 1.0,
              'effects': ['channel.invert', 'color.basic'], 'width': 64})
        preview = wait(lambda event: event['type'] == 'effect_preview')
        assert preview['fx'] == 'channel.invert' and preview['width'] == 64, preview['fx']
        assert len(base64.b64decode(preview['rgba'])) == 64 * preview['height'] * 4
        send({'type': 'edit', 'revision': redone['revision'],
              'ops': [{'op': 'fx-add', 'clip': clip['id'], 'fx': 'channel.invert'}]})
        with_effect = state()
        effects = with_effect['timeline']['tracks'][0]['clips'][0].get('effects', [])
        assert [effect['fx'] for effect in effects] == ['channel.invert'], effects
        send({'type': 'undo', 'revision': with_effect['revision']})
        redone = state()
        # A native preset is exported directly from the accepted edit, without
        # asking a model. It carries empty media slots and reusable .kerf code.
        preset_path = directory / 'native.kerfpreset'
        send(run_command({'command': 'file.exportPreset', 'params': {'path': str(preset_path)},
                          'context': {'connected': True, 'revision': redone['revision']}}))
        preset_event = wait(lambda event: event['type'] == 'preset_export')
        assert 'slot:slot1' in preset_event['document'] and str(media) not in preset_event['document']
        preset_path.write_text(preset_event['document'])  # Qt writes this with QSaveFile.
        send({'type': 'preset_inspect', 'path': str(preset_path)})
        inspected = wait(lambda event: event['type'] == 'preset_inspect')['preset']
        assert len(inspected['slots']) == 1 and inspected['slots'][0]['seconds'] == 3
        assert [fit['media'] for fit in inspected['slots'][0]['fits']] == [redone['timeline']['sources'][0]['id']], inspected
        send(run_command({'command': 'file.applyPreset', 'params': {'path': str(preset_path),
                          'bindings': {'slot1': {'media': redone['timeline']['sources'][0]['id'], 'from': '1s'}}},
                          'context': {'connected': True, 'revision': redone['revision']}}))
        applied_preset = wait(lambda event: event['type'] == 'preset_applied')
        pending_preset = state()
        assert pending_preset['revision'] == redone['revision'] and pending_preset.get('proposalTimeline', {}).get('tracks')
        assert pending_preset['document'] == redone['document']
        send({'type': 'reject', 'proposalId': applied_preset['proposal'], 'revision': pending_preset['revision']})
        redone = state()
        external = runtime_request(endpoint, {
            'operation': 'apply_script',
            'actor': 'mcp',
            'idempotencyKey': 'qt-external-edit',
            'baseRevision': redone['revision'],
            'script': 'marker add at=1s name="external"',
        })
        assert external['ok'], external
        external_state = wait(lambda event: event['type'] == 'state' and event['snapshot']['revision'] == external['state']['revision'])['snapshot']
        assert [marker.get('label') for marker in external_state['timeline']['markers']] == ['external'], external_state['timeline']['markers']

        daemon.terminate()
        daemon.wait(timeout=5)
        wait(lambda event: event['type'] == 'connection' and event['status'] == 'disconnected')
        daemon = subprocess.Popen([str(ROOT / 'target/debug/kerfd'), '--dir', str(directory)], stdout=subprocess.DEVNULL, stderr=subprocess.PIPE)
        send({'type': 'reconnect'})
        wait(lambda event: event['type'] == 'connection' and event['status'] == 'ready')
        assert connection_statuses[-3:] == ['disconnected', 'connecting', 'ready'], connection_statuses
        for i in range(200):
            send({'type': 'seek', 'position': 2.0 * i / 199})
        frame = Frame()
        deadline = time.monotonic() + 15
        while time.monotonic() < deadline:
            poll()
            if lib.kerf_bridge_frame(bridge, C.byref(frame)) and frame.time >= 1.95:
                assert frame.length == frame.width * frame.height * 4
                assert max(frame.width, frame.height) <= 1920, (frame.width, frame.height)
                assert C.string_at(frame.pixels, 16)
                break
            time.sleep(.005)
        else:
            raise AssertionError('Latest seek frame did not arrive')
        send({'type': 'agent', 'text': 'Rename this project and show me the proposal.',
              'focus': {'scope': 'clip', 'clip': clip['id'], 'fromFrame': 0, 'toFrame': 30}})
        proposal_state = state()
        proposal = proposal_state['pendingProposal']
        assert proposal, observed[-10:]
        # The request reached the model with its focus, and the state carries
        # the timeline the proposal would make.
        assert f"Focus: clip {clip['id']} (0f–30f)" in json.dumps(requests[0]['messages'], ensure_ascii=False)
        assert proposal_state.get('proposalTimeline', {}).get('tracks'), proposal_state.keys()
        assert proposal_state['revision'] == external_state['revision']
        proposal_id = proposal.get('id', proposal.get('proposalId'))
        assert proposal_id, proposal
        send({'type': 'accept', 'proposalId': proposal_id, 'revision': proposal_state['revision']})
        accepted = state()
        assert sorted(marker.get('label') for marker in accepted['timeline']['markers']) == ['Agent reviewed', 'external'], accepted['timeline']['markers']
        send({'type': 'agent', 'text': 'Keep the previous change.'})
        wait(lambda event: event['type'] == 'agent_busy' and event['busy'] is False)
        assert len(requests) == 4
        steps = [event['steps'] for event in observed if event.get('type') == 'agent_next']
        assert steps and steps[0][1]['label'] == 'Wipe to the next shot', steps
        assert any('Rename this project' in json.dumps(message) for message in requests[-1]['messages'])
        refreshed = state()  # Agent refresh completes before the export.
        send({'type': 'export', 'quality': 'preview'})
        exported = wait(lambda event: event['type'] == 'export' and event['status'] == 'completed', 60)
        assert Path(exported['path']).is_file()
        send({'type': 'edit', 'revision': refreshed['revision'], 'ops': [{'op': 'set', 'clip': clip['id'], 'props': {'opacity': 0.5}}]})
        state()
        # Cuts cannot show the half-transparent clip; the compositor draws it
        # live, and a seek shows its frame.
        wait(lambda event: event['type'] == 'preview' and event['status'] == 'live')
        send({'type': 'seek', 'position': 1.0})
        deadline = time.monotonic() + 15
        while time.monotonic() < deadline:
            poll()
            if lib.kerf_bridge_frame(bridge, C.byref(frame)) and abs(frame.time - 1.0) < 0.02:
                assert frame.length == frame.width * frame.height * 4
                break
            time.sleep(.005)
        else:
            raise AssertionError('The composited frame did not arrive')
        # A rendered preview still plays it exactly.
        send({'type': 'preview'})
        wait(lambda event: event['type'] == 'preview' and event['status'] == 'ready', 60)
        assert any(event.get('kind') == 'tool' for event in observed)
        # A source edit from the command registry: half a second of the media
        # inserted at the start of the picture track ripples the cut.
        send({'type': 'refresh'})
        current = state()
        video = next(track for track in current['timeline']['tracks'] if track['type'] == 'video')
        before = len(video['clips'])
        send(run_command({'command': 'source.insert', 'context': {
            'connected': True, 'revision': current['revision'], 'playheadFrame': 0,
            'source': {'media': current['timeline']['sources'][0]['id'], 'inFrame': 0, 'durationFrames': 15},
            'sourceTargets': [video['id']]}}))
        inserted = state()
        placed = next(track for track in inserted['timeline']['tracks'] if track['id'] == video['id'])['clips']
        assert len(placed) == before + 1 and placed[0]['startFrame'] == 0 and placed[0]['durationFrames'] == 15, placed
        assert abs(inserted['duration'] - current['duration'] - .5) < .05
        # Kerf Cloud: sign in, read the account, and send a request with the token.
        send({'type': 'cloud_sign_in', 'token': 'tok-cloud'})
        signed_in = wait(lambda event: event['type'] == 'cloud' and event.get('account'))
        assert signed_in['access'] == 'ready' and signed_in['account']['plan'] == 'Pro', signed_in
        send({'type': 'agent', 'text': 'Hello from the cloud.', 'cloud': True})
        wait(lambda event: event['type'] == 'agent_busy' and event['busy'] is False)
        assert cloud_requests and cloud_requests[-1][0] == 'tok-cloud', cloud_requests
        assert 'Hello from the cloud.' in json.dumps(cloud_requests[-1][1]['messages'])
        assert any(event.get('type') == 'agent' and 'Cloud reply.' in event.get('text', '') for event in observed)
        send({'type': 'cloud_sign_out'})
        wait(lambda event: event['type'] == 'cloud' and event['access'] == 'signedOut')
        assert 'token' not in json.loads((directory / 'home/.kerf/cloud.json').read_text())
        print(json.dumps({'result': 'passed', 'checks': ['native preset export/inspect/apply/reject', 'kerf cloud sign-in/account/request', 'focused agent request', 'proposal timeline', 'source insert', 'connection lifecycle/reconnect', 'import', 'trim', 'undo', 'redo', 'external CLI edit sync', '200 seeks/latest frame', 'local provider/tool activity', 'proposal/accept', 'conversation history', 'real export', 'effect rendered fallback', 'effect catalog/previews/add', 'next-step buttons'], 'max_enqueue_ms': round(max(timings), 3), 'frame': [frame.width, frame.height]}, indent=2))
    finally:
        lib.kerf_bridge_free(bridge)
        if daemon.poll() is None:
            daemon.terminate()
            try:
                daemon.wait(timeout=5)
            except subprocess.TimeoutExpired:
                daemon.kill()
        server.shutdown()

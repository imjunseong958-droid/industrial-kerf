#!/usr/bin/env python3
"""Exercise the real Qt C ABI, daemon, decoder, and agent proposal loop locally.
Requires built kerfd/libkerf_qt_bridge and ffmpeg. No external provider is contacted.

Three passes, each on its own project:
- editing: import, edits, masks, presets, the agent, Kerf Cloud, reconnect;
- titles: an empty sequence's frame rate, then generator inserts;
- review: comments, proposals and previews, keyframes, QC, loudness, the
  meter while a tone plays, solo across a kerfd restart, effect previews and
  looks, source edits by target, the render queue with a ProRes master, the
  paperwork, and media info.
`--review-only` skips the editing pass, `--titles-only` the review pass.
It binds a localhost port and Unix sockets, which a sandbox may refuse.
"""
import base64
import ctypes as C
import http.server
import json
import shutil
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
        if self.path.startswith('/review/'):
            # The review pass's agent: one turn that stages a marker, then
            # says what it did.
            review_requests.append(request)
            if len(review_requests) % 2 == 1:
                message = {'role': 'assistant', 'content': None, 'tool_calls': [{
                    'id': f'edit-review-{len(review_requests)}', 'type': 'function', 'function': {
                        'name': 'edit',
                        'arguments': json.dumps({'script': 'marker add at=12f name="Caption starts"'})
                    }}]}
                finish = 'tool_calls'
            else:
                message = {'role': 'assistant', 'content': 'Marked where the caption starts.'}
                finish = 'stop'
            self.reply(200, {'choices': [{'message': message, 'finish_reason': finish}]})
            return
        requests.append(request)
        if len(requests) == 1:
            system = request['messages'][0]['content']
            assert request['messages'][0]['role'] == 'system', request['messages'][0]
            assert '# Kerf edit script' in system, system[:400]
            # The project document arrives as a message after the system prompt.
            assert any('```kerf' in str(m.get('content')) for m in request['messages'][1:]), request['messages'][1:]
            assert [tool['function']['name'] for tool in request['tools']] == ['edit', 'look', 'note', 'render', 'suggest', 'session.read'], request['tools']
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
review_requests = []
server =http.server.ThreadingHTTPServer(('127.0.0.1', 0), Provider)
threading.Thread(target=server.serve_forever, daemon=True).start()
os.environ.update(KERF_PROVIDER='openai', OPENAI_API_MODE='chat', OPENAI_API_KEY='local-test-only', OPENAI_BASE_URL=f'http://127.0.0.1:{server.server_port}', KERF_MODEL='local-fixture')
os.environ.pop('OPENAI_REASONING_EFFORT', None)
# Never the machine's own keychain: the Kerf Cloud token then stays in its file.
os.environ['KERF_SECRETS'] = 'off'
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

def first_pass():
    """The editing loop: import, edits, previews, presets, the agent, Kerf Cloud."""
    with tempfile.TemporaryDirectory(prefix='kerf-qt-', dir=ROOT / 'target') as temporary:
        directory = Path(temporary)
        # The Kerf Cloud session lives in ~/.kerf; keep it inside the test.
        os.environ.update(HOME=str(HOME / 'editing'), KERF_CLOUD_URL=f'http://127.0.0.1:{server.server_port}')
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

        def wait(predicate, timeout=20, since=None):
            # Events from two workers arrive in either order: one already read
            # since `since` (an index into `observed`) counts too.
            if since is not None:
                for event in observed[since:]:
                    if predicate(event):
                        return event
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
            assert any(fx['id'] == 'light.edgeglow' and fx['params'] for fx in catalog), catalog[:2]
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
            # A mask drawn on the viewer is a typed edit: the document gains its
            # line, native playback shows the picture cut to it, and undo takes
            # it away.
            mark = len(observed)
            send({'type': 'edit', 'revision': redone['revision'],
                  'ops': [{'op': 'mask-add', 'clip': clip['id'], 'name': 'k1', 'rect': [0.25, 0.25, 0.75, 0.75]}]})
            masked = state()
            assert 'mask k1 rect=0.25,0.25,0.75,0.75' in masked['document'], masked['document']
            masks = masked['timeline']['tracks'][0]['clips'][0].get('masks', [])
            assert [(mask['name'], mask['rect']) for mask in masks] == [('k1', [0.25, 0.25, 0.75, 0.75])], masks
            assert any(prop['name'] == 'feather' for prop in masked['timeline']['maskProperties'])
            wait(lambda event: event['type'] == 'preview' and event['status'] == 'ready', since=mark)
            send({'type': 'seek', 'position': 1.0})
            frame = Frame()
            deadline = time.monotonic() + 20
            while time.monotonic() < deadline:
                poll()
                if lib.kerf_bridge_frame(bridge, C.byref(frame)) and abs(frame.time - 1.0) < .05:
                    break
                time.sleep(.005)
            else:
                raise AssertionError('The masked frame did not arrive')
            pixels = C.string_at(frame.pixels, frame.length)

            def brightness(x0, y0, x1, y1):
                total = count = 0
                for y in range(int(y0 * frame.height), int(y1 * frame.height), 4):
                    for x in range(int(x0 * frame.width), int(x1 * frame.width), 4):
                        at = (y * frame.width + x) * 4
                        total += pixels[at] + pixels[at + 1] + pixels[at + 2]
                        count += 3
                return total / count

            outside, inside = brightness(0.02, 0.02, 0.2, 0.2), brightness(0.35, 0.35, 0.65, 0.65)
            assert outside < 2 and inside > 30, (outside, inside)
            send({'type': 'undo', 'revision': masked['revision']})
            redone = state()
            assert 'mask ' not in redone['document'], redone['document']
            assert 'masks' not in redone['timeline']['tracks'][0]['clips'][0]
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
            asked = len(observed)
            send({'type': 'agent', 'text': 'Rename this project and show me the proposal.',
                  'focus': {'scope': 'clip', 'clip': clip['id'], 'fromFrame': 0, 'toFrame': 30}})
            # The proposal shows as soon as it is staged, with the turn still running.
            proposal_state = wait(lambda event: event['type'] == 'state' and event['snapshot'].get('pendingProposal'))['snapshot']
            proposal = proposal_state['pendingProposal']
            assert proposal, observed[-10:]
            wait(lambda event: event['type'] == 'agent_busy' and event['busy'] is False, since=asked)
            # The request reached the model with its focus, and the state carries
            # the timeline the proposal would make.
            assert f"Focus: clip {clip['id']} (0f–30f)" in json.dumps(requests[0]['messages'], ensure_ascii=False)
            assert proposal_state.get('proposalTimeline', {}).get('tracks'), proposal_state.keys()
            assert proposal_state['revision'] == external_state['revision']
            proposal_id = proposal.get('id', proposal.get('proposalId'))
            assert proposal_id, proposal
            send({'type': 'accept', 'proposalId': proposal_id, 'revision': proposal_state['revision']})
            accepted = wait(lambda event: event['type'] == 'state' and event['snapshot']['revision'] > proposal_state['revision'])['snapshot']
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
            mark = len(observed)
            send({'type': 'edit', 'revision': refreshed['revision'], 'ops': [{'op': 'set', 'clip': clip['id'], 'props': {'opacity': 0.5}}]})
            state()
            # Cuts cannot show the half-transparent clip; the compositor draws it
            # live, and a seek shows its frame.
            wait(lambda event: event['type'] == 'preview' and event['status'] == 'live', since=mark)
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
            # With no keychain to keep it, the token is in the file, for its owner alone.
            cloud_file = HOME / 'editing/.kerf/cloud.json'
            assert json.loads(cloud_file.read_text()).get('token') == 'tok-cloud'
            assert cloud_file.stat().st_mode & 0o777 == 0o600, oct(cloud_file.stat().st_mode)
            send({'type': 'agent', 'text': 'Hello from the cloud.', 'cloud': True})
            wait(lambda event: event['type'] == 'agent_busy' and event['busy'] is False)
            assert cloud_requests and cloud_requests[-1][0] == 'tok-cloud', cloud_requests
            assert 'Hello from the cloud.' in json.dumps(cloud_requests[-1][1]['messages'])
            assert any(event.get('type') == 'agent' and 'Cloud reply.' in event.get('text', '') for event in observed)
            send({'type': 'cloud_sign_out'})
            wait(lambda event: event['type'] == 'cloud' and event['access'] == 'signedOut')
            assert 'token' not in json.loads((HOME / 'editing/.kerf/cloud.json').read_text())
            return {'checks': ['native preset export/inspect/apply/reject', 'kerf cloud sign-in/account/request', 'focused agent request', 'proposal timeline', 'source insert', 'connection lifecycle/reconnect', 'import', 'trim', 'undo', 'redo', 'external CLI edit sync', '200 seeks/latest frame', 'local provider/tool activity', 'proposal/accept', 'conversation history', 'real export', 'effect rendered fallback', 'effect catalog/previews/add', 'mask add/composite/undo', 'next-step buttons'], 'max_enqueue_ms': round(max(timings), 3), 'frame': [frame.width, frame.height]}
        finally:
            lib.kerf_bridge_free(bridge)
            if daemon.poll() is None:
                daemon.terminate()
                try:
                    daemon.wait(timeout=5)
                except subprocess.TimeoutExpired:
                    daemon.kill()

lib.kerf_fx_catalog_all.argtypes = []
lib.kerf_fx_catalog_all.restype = C.c_void_p
KERFD = ROOT / 'target/debug/kerfd'


def start_kerfd(directory):
    """kerfd for `directory`, once its socket answers, and its endpoint."""
    daemon = subprocess.Popen([str(KERFD), '--dir', str(directory)], stdout=subprocess.DEVNULL, stderr=subprocess.PIPE)
    endpoint = runtime_endpoint(directory)
    deadline = time.monotonic() + 5
    while time.monotonic() < deadline:
        if daemon.poll() is not None:
            raise RuntimeError('kerfd exited before opening its socket')
        try:
            with socket.socket(socket.AF_UNIX, socket.SOCK_STREAM) as connection:
                connection.settimeout(.2)
                connection.connect(os.fsencode(endpoint))
            return daemon, endpoint
        except OSError:
            time.sleep(.02)
    daemon.terminate()
    raise RuntimeError('kerfd did not open its socket')


def stop_kerfd(daemon):
    if daemon.poll() is None:
        daemon.terminate()
        try:
            daemon.wait(timeout=5)
        except subprocess.TimeoutExpired:
            daemon.kill()


class Editor:
    """The C ABI as the Qt editor drives it: one bridge, polled from one
    thread. Events not yet asked for wait in `pending`, so two workers'
    events may arrive in either order."""

    def __init__(self, directory):
        self.bridge = lib.kerf_bridge_new(os.fsencode(directory), os.fsencode(KERFD))
        assert self.bridge
        self.pending = []
        self.recent = []
        self.refusals = []  # predicates of error events a step expects

    def close(self):
        if self.bridge:
            lib.kerf_bridge_free(self.bridge)
            self.bridge = None

    def send(self, command):
        assert lib.kerf_bridge_send(self.bridge, json.dumps(command).encode()), command

    def poll(self):
        pointer = lib.kerf_bridge_poll(self.bridge)
        if not pointer:
            return None
        value = json.loads(C.string_at(pointer))
        lib.kerf_bridge_string_free(pointer)
        self.recent = (self.recent + [value])[-16:]
        if value['type'] == 'error' and not any(refusal(value) for refusal in self.refusals):
            raise AssertionError(value)
        return value

    def expect(self, what, predicate, timeout=60):
        for index, event in enumerate(self.pending):
            if predicate(event):
                return self.pending.pop(index)
        deadline = time.monotonic() + timeout
        while time.monotonic() < deadline:
            event = self.poll()
            if event is None:
                time.sleep(.005)
            elif predicate(event):
                return event
            else:
                self.pending.append(event)
                del self.pending[:-4000]
        raise AssertionError(f'no event for {what}; recent events: {json.dumps(self.recent)[-3000:]}')

    def gather(self, seconds, predicate):
        """Every event that matches over the next `seconds`."""
        taken = [event for event in self.pending if predicate(event)]
        self.pending = [event for event in self.pending if not predicate(event)]
        deadline = time.monotonic() + seconds
        while time.monotonic() < deadline:
            event = self.poll()
            if event is None:
                time.sleep(.005)
            elif predicate(event):
                taken.append(event)
            else:
                self.pending.append(event)
        return taken

    def forget(self, kind):
        self.pending = [event for event in self.pending if event['type'] != kind]

    def state(self, what, predicate=lambda snapshot: True, timeout=60):
        return self.expect(what, lambda event: event['type'] == 'state' and predicate(event['snapshot']), timeout)['snapshot']

    def comments(self, what, predicate, timeout=60):
        return self.expect(what, lambda event: event['type'] == 'comments' and predicate(event['items']), timeout)['items']

    def edit(self, snapshot, ops, what):
        self.send({'type': 'edit', 'revision': snapshot['revision'], 'ops': ops})
        return self.state(what, lambda next: next['revision'] > snapshot['revision'])


def clip_of(snapshot, clip):
    for track in snapshot['timeline']['tracks']:
        for item in track.get('clips', []):
            if item['id'] == clip:
                return item
    raise AssertionError(f'no clip {clip} in {snapshot["document"]}')


def track_of(snapshot, track):
    return next(item for item in snapshot['timeline']['tracks'] if item['id'] == track)


def ffprobe(path):
    return json.loads(subprocess.run(['ffprobe', '-v', 'error', '-show_streams', '-of', 'json', str(path)],
                                     check=True, capture_output=True, text=True).stdout)['streams']


def lavfi(path, picture, sound, seconds):
    subprocess.run(['ffmpeg', '-hide_banner', '-loglevel', 'error', '-y',
                    '-f', 'lavfi', '-i', f'{picture}=size=320x180:rate=24:duration={seconds}',
                    '-f', 'lavfi', '-i', f'{sound}:duration={seconds}',
                    '-filter_complex', '[1:a]aformat=sample_rates=48000:channel_layouts=stereo[a]',
                    '-map', '0:v', '-map', '[a]', '-c:v', 'libx264', '-preset', 'ultrafast', '-pix_fmt', 'yuv420p',
                    '-c:a', 'aac', '-b:a', '256k', '-shortest', str(path)], check=True)


# The review pass works on a project written by hand: a 1 kHz tone at
# -23 dBFS (so -23 LUFS) on A1 over a silent A2, a lower third on V2 that
# a caption runs into, and two picture clips to put effects on.
REVIEW_PROJECT = '''kerf 1
project name="Review"
media m1 path="media/tone.mp4" rate=24 dur=96f video=320x180 audio=2ch@48000
media m2 path="media/quiet.mp4" rate=24 dur=96f video=320x180 audio=2ch@48000
sequence s1 rate=24 size=640x360 audio=48000
  track V1 video
    clip c1 media=m1 src=0f at=0f dur=48f
    clip c2 media=m1 src=48f at=48f dur=48f
  track V2 video name="Lower thirds"
    clip lt1 gen=title@1 at=0f dur=96f
      set text="Atlas Coffee" size=0.08 y=0.4
  track A1 audio
    clip a1 media=m1 src=0f at=0f dur=96f
  track A2 audio
    clip a2 media=m2 src=0f at=0f dur=96f
  track S1 subtitle
    clip cap1 caption="We open before the city does" at=24f dur=48f
'''


def review_pass():
    """The review and delivery protocol of the mockup, end to end."""
    checks = []
    notes = {}
    with tempfile.TemporaryDirectory(prefix='kerf-qt-review-', dir=ROOT / 'target') as temporary:
        directory = Path(temporary)
        os.environ.update(HOME=str(HOME / 'review'))
        (directory / 'media').mkdir()
        lavfi(directory / 'media/tone.mp4', 'testsrc2', 'aevalsrc=0.0707946*sin(2*PI*1000*t)|0.0707946*sin(2*PI*1000*t):c=stereo:s=48000', 4)
        lavfi(directory / 'media/quiet.mp4', 'smptebars', 'anullsrc=r=48000:cl=stereo', 4)
        lavfi(directory / 'third.mp4', 'rgbtestsrc', 'sine=frequency=440:sample_rate=48000', 2)
        (directory / 'project.kerf').write_text(REVIEW_PROJECT)
        daemon, endpoint = start_kerfd(directory)
        editor = Editor(directory)
        try:
            editor.expect('kerfd to be ready', lambda event: event['type'] == 'connection' and event['status'] == 'ready')
            snapshot = editor.state('the project', lambda snapshot: 'cap1' in snapshot['document'])
            # Keep the speakers quiet: volume applies after the meter.
            editor.send({'type': 'volume', 'value': 0.0})
            review_media(editor, directory, checks)
            snapshot = review_comments(editor, endpoint, snapshot, checks)
            snapshot = review_proposals(editor, endpoint, snapshot, checks)
            snapshot = review_keyframes(editor, snapshot, checks)
            snapshot = review_check(editor, snapshot, checks)
            review_loudness(editor, checks, notes)
            review_meter(editor, checks, notes)
            notes['daemon'] = daemon
            snapshot = review_solo_and_reconnect(editor, directory, snapshot, checks, notes)
            daemon, endpoint = notes.pop('daemon'), notes.pop('endpoint')
            snapshot = review_effects(editor, snapshot, checks)
            snapshot = review_source_edits(editor, snapshot, checks)
            review_delivery(editor, endpoint, directory, checks, notes)
            review_import(editor, directory, snapshot, checks)
        finally:
            editor.close()
            stop_kerfd(notes.pop('daemon', daemon))
    return {'checks': checks, **notes}


def review_media(editor, directory, checks):
    # media_info comes on its own for media new to the editor; one asked
    # for answers alone (the editor merges it into what it has).
    info = editor.expect('media info on opening', lambda event: event['type'] == 'media_info')
    assert set(info['media']) == {'m1', 'm2'}, info
    tone = info['media']['m1']
    assert tone['codec'] == 'h264' and tone['pixFmt'] == 'yuv420p' and tone['bitDepth'] == 8, tone
    assert tone['audioCodec'] == 'aac' and tone['audioChannels'] == 2 and tone['audioSampleRate'] == 48000, tone
    editor.send({'type': 'media_info', 'media': 'm2'})
    one = editor.expect('media info for m2', lambda event: event['type'] == 'media_info')
    assert set(one['media']) == {'m2'}, one
    editor.send({'type': 'media_info'})
    every = editor.expect('media info for all', lambda event: event['type'] == 'media_info')
    assert set(every['media']) == {'m1', 'm2'}, every
    editor.refusals.append(lambda event: event.get('command') == 'media_info')
    editor.send({'type': 'media_info', 'media': 'nope'})
    refused = editor.expect('media info refused', lambda event: event['type'] == 'error' and event.get('command') == 'media_info')
    assert refused['media'] == 'nope', refused
    # The index: the tone never pauses; the quiet file is one silence.
    editor.send({'type': 'media_index', 'media': 'm1'})
    index = editor.expect('the tone index', lambda event: event['type'] == 'media_index' and event['media'] == 'm1')
    assert abs(index['duration'] - 4) < .1 and index['ranges'] and index['silences'] == [], index
    editor.send({'type': 'media_index', 'media': 'm2'})
    quiet = editor.expect('the quiet index', lambda event: event['type'] == 'media_index' and event['media'] == 'm2')
    assert len(quiet['silences']) == 1 and quiet['silences'][0][1] - quiet['silences'][0][0] > 3.5, quiet
    checks += ['media_info on opening/asked/all/refused', 'media_index cuts and silences']


def review_comments(editor, endpoint, snapshot, checks):
    editor.comments('the review log on connecting', lambda items: items == [])
    note = {'type': 'comment_add', 'id': 'c-e2e-1', 'body': 'Hold on the title', 'at': 1.0, 'duration': 1.5,
            'tags': ['titles'], 'drawing': [{'tool': 'ellipse', 'points': [[.6, .2], [.8, .4]], 'color': '#ededed'}]}
    editor.send(note)
    added = editor.expect('the note', lambda event: event['type'] == 'comments' and event.get('comment', {}).get('id') == 'c-e2e-1')
    comment = added['comment']
    assert comment['author']['kind'] == 'person' and comment['frame'] == 24 and comment['duration'] == 1.5, comment
    assert comment['drawing'][0]['tool'] == 'ellipse' and comment['tags'] == ['titles'], comment
    assert comment['revision'] == snapshot['revision'] and comment['created'], comment
    # A retried add with the same client id is harmless.
    editor.send(note)
    again = editor.expect('the retried note', lambda event: event['type'] == 'comments' and event.get('comment', {}).get('id') == 'c-e2e-1')
    assert [item['id'] for item in again['items']] == ['c-e2e-1'], again
    editor.forget('comments')
    editor.send({'type': 'comment_list'})
    listed = editor.comments('the list asked for, though unchanged', lambda items: [item['id'] for item in items] == ['c-e2e-1'])
    assert listed[0]['body'] == 'Hold on the title', listed
    editor.send({'type': 'comment_set', 'id': 'c-e2e-1', 'body': 'Hold on the lower third', 'tags': ['titles', 'graphics']})
    edited = editor.comments('the edit', lambda items: items and items[0]['body'] == 'Hold on the lower third')
    assert edited[0]['edited'] and edited[0]['tags'] == ['titles', 'graphics'], edited
    editor.refusals.append(lambda event: str(event.get('command', '')).startswith('comment_'))
    editor.send({'type': 'comment_set', 'id': 'missing', 'resolved': True})
    refused = editor.expect('a refused change', lambda event: event['type'] == 'error' and event.get('command') == 'comment_set')
    assert refused['code'] == 'COMMENT_NOT_FOUND' and refused['id'] == 'missing', refused
    editor.send({'type': 'comment_set', 'id': 'c-e2e-1', 'resolved': True})
    resolved = editor.comments('the resolve', lambda items: items and items[0]['resolved'])
    assert resolved[0]['resolvedBy']['kind'] == 'person', resolved
    editor.send({'type': 'comment_set', 'id': 'c-e2e-1', 'resolved': False})
    editor.comments('the reopen', lambda items: items and not items[0]['resolved'])
    editor.send({'type': 'comment_add', 'body': 'Agreed', 'parent': 'c-e2e-1'})
    replied = editor.comments('the reply', lambda items: any(item['parent'] == 'c-e2e-1' for item in items))
    reply = next(item for item in replied if item['parent'] == 'c-e2e-1')
    editor.send({'type': 'comment_set', 'id': reply['id'], 'deleted': True})
    editor.comments('the reply deleted', lambda items: all(item['id'] != reply['id'] for item in items))
    # A note another kerfd client (an MCP agent) writes shows up without a refresh.
    written = runtime_request(endpoint, {'operation': 'add_comment', 'actor': 'mcp', 'comment': {
        'author': {'kind': 'mcp', 'name': 'Codex', 'id': 'codex'}, 'body': 'The caption runs into the lower third',
        'at': 1.5, 'tags': ['captions']}})
    assert written['ok'], written
    live = editor.comments('the agent note', lambda items: any(item['author']['kind'] == 'mcp' for item in items))
    agent_note = next(item for item in live if item['author']['kind'] == 'mcp')
    assert agent_note['author']['name'] == 'Codex' and agent_note['at'] == 1.5, agent_note
    # A turn kept in the review log: the person's words, then Kerf's reply
    # with its steps and the proposal it staged.
    editor.send({'type': 'agent', 'text': 'Mark where the caption starts', 'provider': 'openai', 'apiMode': 'chat',
                 'apiKey': 'local-test-only', 'baseUrl': f'http://127.0.0.1:{server.server_port}/review/v1',
                 'model': 'local-fixture', 'comment': {'at': 1.0, 'duration': 2.0}})
    asked = editor.comments('the request', lambda items: any(item['body'] == 'Mark where the caption starts' for item in items))
    request = next(item for item in asked if item['body'] == 'Mark where the caption starts')
    assert request['author']['kind'] == 'person' and request['at'] == 1.0 and request['parent'] is None, request
    answered = editor.comments('Kerf\'s reply', lambda items: any(item['author']['kind'] == 'kerf' for item in items))
    kerf = next(item for item in answered if item['author']['kind'] == 'kerf')
    assert kerf['parent'] == request['id'] and kerf['body'] == 'Marked where the caption starts.', kerf
    assert any(step.startswith('edit · done') for step in kerf['steps']), kerf['steps']
    assert kerf['proposal'] and not kerf['error'], kerf
    editor.expect('the turn to end', lambda event: event['type'] == 'agent_busy' and event['busy'] is False)
    assert len(review_requests) == 2, len(review_requests)
    assert 'Mark where the caption starts' in json.dumps(review_requests[0]['messages'])
    checks += ['comment_add/list/set/resolve/reopen/reply/delete/refusal', 'retried client id', 'MCP note live',
               'agent turn as person comment + Kerf reply with steps and proposal']
    snapshot = editor.state('the staged proposal', lambda next: any(
        entry['proposalId'] == kerf['proposal'] and entry['status'] == 'pending' for entry in next.get('proposals', [])))
    snapshot['_kerf_proposal'] = kerf['proposal']
    snapshot['_thread'] = request['id']
    return snapshot


def proposal(snapshot, proposal_id):
    return next((entry for entry in snapshot.get('proposals', []) if entry['proposalId'] == proposal_id), {})


def review_proposals(editor, endpoint, snapshot, checks):
    kerf_id, thread = snapshot['_kerf_proposal'], snapshot['_thread']
    # A second proposal, from an MCP agent, waits beside Kerf's.
    staged = runtime_request(endpoint, {'operation': 'stage_script_proposal', 'requestKey': 'e2e-mcp-1',
                                        'baseRevision': snapshot['revision'], 'script': 'marker add at=2s name="Agent marker"',
                                        'author': {'kind': 'mcp', 'name': 'Codex', 'id': 'codex'}})
    assert staged['ok'], staged
    both = editor.state('two pending proposals', lambda next: sum(
        entry['status'] == 'pending' for entry in next.get('proposals', [])) == 2)
    mcp = next(entry for entry in both['proposals'] if entry['author']['kind'] == 'mcp')
    assert proposal(both, kerf_id)['author']['kind'] == 'kerf' and mcp['version'] == 1, both['proposals']
    assert set(both['proposalTimelines']) >= {kerf_id, mcp['proposalId']}, list(both['proposalTimelines'])
    # Accepting one makes the other stale and resolves the thread it came from.
    editor.send({'type': 'accept', 'proposalId': kerf_id, 'revision': both['revision']})
    accepted = editor.state('the accept', lambda next: next['revision'] > both['revision'])
    assert proposal(accepted, kerf_id)['status'] == 'accepted', accepted['proposals']
    stale = proposal(accepted, mcp['proposalId'])
    assert stale['status'] == 'pending' and stale['freshness'] == 'stale', stale
    assert any(marker.get('label') == 'Caption starts' for marker in accepted['timeline']['markers'])
    settled = editor.comments('the thread settled', lambda items: any(
        item['proposal'] == kerf_id and item['resolved'] for item in items))
    linked = [item for item in settled if item['proposal'] == kerf_id]
    assert all(item['resolved'] and item['resolvedRevision'] == accepted['revision'] for item in linked), linked
    assert any(item['id'] == thread for item in linked), linked
    # Revising the stale one makes its version 2 on the new revision.
    revised = runtime_request(endpoint, {'operation': 'stage_script_proposal', 'requestKey': 'e2e-mcp-2',
                                         'baseRevision': accepted['revision'], 'script': 'marker add at=2s name="Agent marker, v2"',
                                         'author': {'kind': 'mcp', 'name': 'Codex', 'id': 'codex'},
                                         'revises': mcp['proposalId']})
    assert revised['ok'], revised
    snapshot = editor.state('version 2', lambda next: any(
        entry.get('supersedes') == mcp['proposalId'] for entry in next.get('proposals', [])))
    v2 = next(entry for entry in snapshot['proposals'] if entry.get('supersedes') == mcp['proposalId'])
    assert v2['version'] == 2 and v2['freshness'] == 'current' and v2['status'] == 'pending', v2
    assert [version['version'] for version in v2['versions']] == [1, 2], v2['versions']
    assert v2['versions'][0]['proposalId'] == mcp['proposalId'] and v2['versions'][1]['proposalId'] == v2['proposalId']
    assert v2['proposalId'] in snapshot['proposalTimelines']
    # The proposal rendered and played in place of the program, then the
    # program again.
    editor.forget('preview')
    editor.send({'type': 'preview', 'proposalId': v2['proposalId']})
    editor.expect('the proposal rendering', lambda event: event['type'] == 'preview' and event['status'] == 'running')
    shown = editor.expect('the proposal shown', lambda event: event['type'] == 'preview' and (
        event['status'] == 'ready' and event.get('proposalId') == v2['proposalId'] or event['status'] in ('failed', 'required')), 120)
    assert shown['status'] == 'ready' and shown.get('proposalId') == v2['proposalId'], shown
    editor.forget('viewer_reset')
    editor.send({'type': 'preview_exit'})
    editor.expect('the preview left', lambda event: event['type'] == 'preview' and event['status'] == 'exited')
    editor.expect('the program again', lambda event: event['type'] == 'viewer_reset')
    editor.expect('the program\'s own status', lambda event: event['type'] == 'preview' and event['status'] in ('ready', 'live') and not event.get('proposalId'))
    # A range preview stopped while it renders; and a stop with nothing running.
    editor.send({'type': 'preview', 'start': 0, 'duration': 4})
    editor.expect('the range rendering', lambda event: event['type'] == 'preview' and event['status'] == 'running')
    editor.send({'type': 'preview_stop'})
    editor.expect('the render stopped', lambda event: event['type'] == 'preview' and event['status'] == 'stopped', 120)
    editor.send({'type': 'preview_stop'})
    editor.expect('a stop with nothing running', lambda event: event['type'] == 'preview' and event['status'] == 'stopped')
    editor.send({'type': 'reject', 'proposalId': v2['proposalId'], 'revision': snapshot['revision']})
    snapshot = editor.state('the reject', lambda next: proposal(next, v2['proposalId']).get('status') == 'rejected')
    checks += ['two pending proposals, accept one -> other stale', 'accept resolves the linked thread',
               'revise -> v2 with versions/supersedes', 'preview{proposalId}/preview_exit/viewer_reset',
               'preview{start,duration}/preview_stop']
    return snapshot


OPACITY_KEYS = [{'at': 0.0, 'frame': 0, 'interp': 'linear', 'value': 1.0},
                {'at': 1.0, 'frame': 24, 'interp': 'ease', 'value': 0.25}]


def review_keyframes(editor, snapshot, checks):
    # Keys go in as Qt writes them (frames from the clip's first frame),
    # land in the document, and come back with both frame and seconds.
    keys = {'keys': [{'at': 0, 'value': 1.0, 'interp': 'linear'}, {'at': 24, 'value': 0.25, 'interp': 'ease'}]}
    keyed = editor.edit(snapshot, [{'op': 'set', 'clip': 'c1', 'props': {'opacity': keys}}], 'the opacity keys')
    assert clip_of(keyed, 'c1')['props']['opacity'] == OPACITY_KEYS, clip_of(keyed, 'c1')
    assert '      set opacity={0f:1 24f:0.25:ease}' in keyed['document'].splitlines(), keyed['document']
    # Sent back from what the timeline reports, the document is unchanged.
    echoed = {'keys': [{'at': key['frame'], 'value': key['value'], 'interp': key['interp']} for key in OPACITY_KEYS]}
    again = editor.edit(keyed, [{'op': 'set', 'clip': 'c1', 'props': {'opacity': echoed}}], 'the keys sent back')
    assert again['document'] == keyed['document'], again['document']
    # Keys on pan are refused; the project stays as it was.
    editor.refusals.append(lambda event: 'gain' in event.get('message', '') or 'pan' in event.get('message', ''))
    editor.send({'type': 'edit', 'revision': again['revision'], 'ops': [{'op': 'set', 'clip': 'a1', 'props': {
        'pan': {'keys': [{'at': 0, 'value': -1}, {'at': 24, 'value': 1}]}}}]})
    editor.expect('pan keys refused', lambda event: event['type'] == 'error')
    checks += ['keyframes {keys:[{at: frame}]} -> document and timeline frame/at', 'keys echoed back unchanged', 'pan keys refused']
    return editor.state('the project after the refusal', lambda next: next['revision'] == again['revision'] and next['document'] == again['document'])


def review_check(editor, snapshot, checks):
    editor.send({'type': 'check'})
    first = editor.expect('the check started', lambda event: event['type'] == 'check' and event['status'] == 'running')
    assert first['progress'] == 0, first
    done = editor.expect('the check', lambda event: event['type'] == 'check' and event['status'] in ('done', 'failed'), 300)
    assert done['status'] == 'done', done
    overlaps = [finding for finding in done['findings'] if finding['code'] == 'text.overlap']
    assert len(overlaps) == 1 and overlaps[0]['entity'] == 'cap1' and overlaps[0]['severity'] == 'warning', done['findings']
    overlap = overlaps[0]
    assert overlap['at'] == 1.0 and overlap['rect']['y'] > .7 and overlap['commentId'], overlap
    row = next(row for row in done['summary']['rows'] if row['code'] == 'text.overlap')
    assert not row['passed'] and row['commentId'] == overlap['commentId'], row
    assert len(done['summary']['rows']) >= 8, done['summary']
    kept = editor.comments('the qc comment', lambda items: any(item['id'] == overlap['commentId'] for item in items))
    qc = next(item for item in kept if item['id'] == overlap['commentId'])
    assert qc['author']['kind'] == 'qc' and qc['qcKey'] == 'qc:text.overlap:cap1' and not qc['resolved'], qc
    assert qc['author'].get('role') == 'captions' and qc['at'] == 1.0 and not qc['blocking'], qc
    # Raising the lower third clear of the caption resolves it on the next check.
    lifted = editor.edit(snapshot, [{'op': 'set', 'clip': 'lt1', 'props': {'y': -0.3}}], 'the lower third raised')
    editor.send({'type': 'check'})
    done = editor.expect('the second check', lambda event: event['type'] == 'check' and event['status'] in ('done', 'failed'), 300)
    assert done['status'] == 'done' and not any(finding['code'] == 'text.overlap' for finding in done['findings']), done['findings']
    assert next(row for row in done['summary']['rows'] if row['code'] == 'text.overlap')['passed']
    settled = editor.comments('the qc comment resolved', lambda items: any(
        item['id'] == qc['id'] and item['resolved'] for item in items))
    qc = next(item for item in settled if item['id'] == qc['id'])
    assert qc['resolvedBy']['kind'] == 'qc', qc
    checks += ['check: caption over a lower third -> text.overlap + qc comment', 'fixing it auto-resolves the qc comment']
    return lifted


def review_loudness(editor, checks, notes):
    editor.send({'type': 'loudness'})
    editor.expect('loudness running', lambda event: event['type'] == 'loudness' and event['status'] == 'running')
    measured = editor.expect('loudness', lambda event: event['type'] == 'loudness' and event['status'] != 'running', 300)
    assert measured['status'] == 'ready', measured
    assert abs(measured['integrated'] + 23) < 1.0, measured['integrated']
    assert measured['target'] == -23 and measured['limit'] == -1 and measured['lra'] < 1.0, measured
    assert all(peak < -21 for peak in measured['truePeak']), measured['truePeak']
    assert measured['series'] and abs(measured['duration'] - 4) < .1, measured['duration']
    notes['integrated'] = measured['integrated']
    checks.append('loudness on a -23 LUFS tone')


def meter_while_playing(editor, seconds=2.0):
    editor.forget('meter')
    editor.forget('clock')
    editor.send({'type': 'seek', 'position': 0.5})
    editor.send({'type': 'play'})
    # A busy machine may take a moment to start the sound.
    events = [editor.expect('the first reading', lambda event: event['type'] == 'meter' and event['peak'] is not None, 30)]
    events += editor.gather(seconds, lambda event: event['type'] in ('meter', 'clock'))
    editor.send({'type': 'pause'})
    editor.expect('the meter cleared on pause', lambda event: event['type'] == 'meter' and event['peak'] is None, 10)
    readings = [event for event in events if event['type'] == 'meter' and event['peak'] is not None]
    clocks = [event for event in events if event['type'] == 'clock']
    assert len(readings) >= 10, events[-10:]
    return readings, clocks


def review_meter(editor, checks, notes):
    readings, clocks = meter_while_playing(editor)
    settled = readings[len(readings) // 2:]
    peaks = [max(reading['peak']) for reading in settled]
    rms = [max(reading['rms']) for reading in settled]
    assert all(-24.5 < peak < -21.5 for peak in peaks), peaks
    assert all(-27.5 < value < -24.5 for value in rms), rms
    momentary = [reading['momentary'] for reading in settled if reading['momentary'] is not None]
    assert momentary and all(abs(value + 23) < 1.5 for value in momentary), momentary
    assert all(set(reading) >= {'peak', 'rms', 'momentary', 'shortTerm', 'at'} for reading in readings)
    assert readings[-1]['at'] > readings[0]['at'], (readings[0], readings[-1])
    assert clocks and all(isinstance(clock['dropped'], int) and clock['dropped'] >= 0 for clock in clocks), clocks[-3:]
    # Played to the end, the meter clears without a pause.
    editor.forget('meter')
    editor.send({'type': 'seek', 'position': 3.4})
    editor.send({'type': 'play'})
    editor.expect('a reading near the end', lambda event: event['type'] == 'meter' and event['peak'] is not None, 30)
    editor.expect('the meter cleared at the end', lambda event: event['type'] == 'meter' and event['peak'] is None, 30)
    editor.send({'type': 'pause'})
    notes['meter'] = {'readings': len(readings), 'peak': round(sum(peaks) / len(peaks), 2), 'momentary': momentary[-1],
                      'dropped': clocks[-1]['dropped']}
    checks += ['meter while playing a -23 dBFS sine (peak, rms, momentary)', 'meter cleared at the end of the program', 'clock.dropped']


def silent(readings):
    return all(max(reading['peak']) < -90 for reading in readings[2:])


def review_solo_and_reconnect(editor, directory, snapshot, checks, notes):
    # Solo the silent track: nothing is heard.
    editor.send({'type': 'solo', 'tracks': ['A2']})
    readings, _ = meter_while_playing(editor, 1.5)
    assert silent(readings), [reading['peak'] for reading in readings]
    # kerfd goes away and comes back: the bridge keeps the solo, and the
    # keys come back from kerfd's project file.
    stop_kerfd(notes['daemon'])
    editor.expect('kerfd gone', lambda event: event['type'] == 'connection' and event['status'] == 'disconnected')
    notes['daemon'], notes['endpoint'] = start_kerfd(directory)
    editor.forget('state')
    editor.send({'type': 'reconnect'})
    editor.expect('kerfd again', lambda event: event['type'] == 'connection' and event['status'] == 'ready')
    reopened = editor.state('the project reopened', lambda next: 'cap1' in next['document'])
    assert clip_of(reopened, 'c1')['props']['opacity'] == OPACITY_KEYS, clip_of(reopened, 'c1')
    assert 'set opacity={0f:1 24f:0.25:ease}' in (directory / 'project.kerf').read_text()
    readings, _ = meter_while_playing(editor, 1.5)
    assert silent(readings), [reading['peak'] for reading in readings]
    # The editor sends the solo again once the engine is ready; same mix.
    editor.send({'type': 'solo', 'tracks': ['A2']})
    readings, _ = meter_while_playing(editor, 1.5)
    assert silent(readings), [reading['peak'] for reading in readings]
    editor.send({'type': 'solo', 'tracks': []})
    readings, _ = meter_while_playing(editor, 1.5)
    assert all(max(reading['peak']) > -25 for reading in readings[len(readings) // 2:]), [reading['peak'] for reading in readings]
    checks += ['solo hears only the soloed track', 'solo kept across a kerfd restart/reconnect and resent',
               '.kerf keyframe round trip through kerfd restart']
    return reopened


def pixels(preview):
    data = base64.b64decode(preview['rgba'])
    assert len(data) == preview['width'] * preview['height'] * 4, preview['fx']
    return [tuple(data[at:at + 3]) for at in range(0, len(data), 4)]


def chroma(preview):
    values = pixels(preview)
    return sum(max(pixel) - min(pixel) for pixel in values) / len(values)


def review_effects(editor, snapshot, checks):
    # An effect card dropped on a clip (the timeline's own edit), then set.
    dropped = editor.edit(snapshot, [{'op': 'fx-add', 'clip': 'c2', 'fx': 'color.basic'}], 'the dropped effect')
    assert [effect['fx'] for effect in clip_of(dropped, 'c2').get('effects', [])] == ['color.basic'], clip_of(dropped, 'c2')
    graded = editor.edit(dropped, [{'op': 'fx-set', 'clip': 'c2', 'index': 0, 'params': {'saturation': 0.2}}], 'the effect set')
    assert 'saturation=0.2' in graded['document'], graded['document']
    # Previews with parameters, before and after, keyed as asked.
    editor.forget('effect_preview')
    editor.send({'type': 'effect_previews', 'clip': 'c2', 'at': 3.0, 'width': 64, 'effects': [
        {'fx': 'color.basic', 'params': {'saturation': 0}, 'key': 'after'},
        {'fx': 'color.basic', 'params': {'saturation': 1}, 'key': 'before'}]})
    after = editor.expect('the after preview', lambda event: event['type'] == 'effect_preview' and event.get('key') == 'after')
    before = editor.expect('the before preview', lambda event: event['type'] == 'effect_preview' and event.get('key') == 'before')
    assert after['clip'] == 'c2' and after['fx'] == 'color.basic' and after['width'] == 64 and after['height'] == 36, after['height']
    assert chroma(after) < 4 < 20 < chroma(before), (chroma(after), chroma(before))
    # Generators alone on black, on a canvas shaped like the sequence.
    editor.send({'type': 'effect_previews', 'clip': '', 'at': 0, 'width': 64, 'effects': [
        {'fx': 'solid', 'params': {'color': '#ff0000'}, 'key': 'solid'},
        {'fx': 'title', 'params': {'text': 'Hello'}, 'key': 'title'}]})
    solid = editor.expect('the solid', lambda event: event['type'] == 'effect_preview' and event.get('key') == 'solid')
    assert all(r > 200 and g < 40 and b < 40 for r, g, b in pixels(solid)), pixels(solid)[:4]
    title = editor.expect('the title', lambda event: event['type'] == 'effect_preview' and event.get('key') == 'title')
    lit = [pixel for pixel in pixels(title) if min(pixel) > 128]
    assert title['height'] == 36 and 0 < len(lit) < len(pixels(title)) // 3, len(lit)
    # An effects preset kept from c2 and put on c1.
    editor.send({'type': 'look_save', 'clip': 'c2', 'name': 'Muted'})
    saved = editor.expect('the saved look', lambda event: event['type'] == 'look_saved')
    assert saved['path'] == 'presets/looks/Muted.kerflook' and saved['id'] == 'look.muted' and saved['effects'] == 1, saved
    editor.send({'type': 'look_apply', 'clip': 'c1', 'path': saved['path']})
    applied = editor.expect('the applied look', lambda event: event['type'] == 'look_applied')
    assert applied['fx'] == 'look.muted' and applied['clip'] == 'c1', applied
    looked = editor.state('c1 with the look', lambda next: any(
        effect['fx'] == 'look.muted' for effect in clip_of(next, 'c1').get('effects', [])))
    checks += ['effect drop (fx-add) and fx-set', 'effect_previews with params before/after',
               'generator previews (solid, title) on clip ""', 'look_save/look_apply']
    return looked


def review_source_edits(editor, snapshot, checks):
    a1 = track_of(snapshot, 'A1')['clips']
    context = {'connected': True, 'playheadFrame': 0, 'source': {'media': 'm2', 'inFrame': 0, 'durationFrames': 12}}
    # The audio target switched off: Overwrite edits the picture only.
    editor.send(run_command({'command': 'source.overwrite', 'context': {
        **context, 'revision': snapshot['revision'], 'sourceTargets': ['V1']}}))
    over = editor.state('the overwrite', lambda next: next['revision'] > snapshot['revision'])
    first = track_of(over, 'V1')['clips'][0]
    assert first['sourceId'] == 'm2' and first['startFrame'] == 0 and first['durationFrames'] == 12, first
    assert track_of(over, 'A1')['clips'] == a1 and abs(over['duration'] - snapshot['duration']) < 1e-6
    # Both targets: Insert ripples picture and sound alike.
    editor.send(run_command({'command': 'source.insert', 'context': {
        **context, 'revision': over['revision'], 'sourceTargets': ['V1', 'A1']}}))
    inserted = editor.state('the insert', lambda next: next['revision'] > over['revision'])
    sound = track_of(inserted, 'A1')['clips']
    assert sound[0]['sourceId'] == 'm2' and sound[0]['durationFrames'] == 12 and sound[1]['id'] == 'a1' and sound[1]['startFrame'] == 12, sound
    assert track_of(inserted, 'V1')['clips'][0]['sourceId'] == 'm2' and track_of(inserted, 'V1')['clips'][1]['startFrame'] == 12
    assert abs(inserted['duration'] - over['duration'] - .5) < .05
    editor.send({'type': 'undo', 'revision': inserted['revision']})
    undone = editor.state('the insert undone', lambda next: next['revision'] > inserted['revision'])
    editor.send({'type': 'undo', 'revision': undone['revision']})
    restored = editor.state('the overwrite undone', lambda next: next['revision'] > undone['revision'])
    assert track_of(restored, 'A1')['clips'] == a1 and track_of(restored, 'V1')['clips'][0]['id'] == 'c1'
    checks += ['target-driven Overwrite (audio target off)', 'target-driven Insert ripples both targets']
    return restored


FINISHED = ('completed', 'failed', 'cancelled')


def review_delivery(editor, endpoint, directory, checks, notes):
    editor.send({'type': 'export', 'quality': 'master', 'name': 'Master', 'output': 'deliveries/'})
    master = editor.expect('the master queued', lambda event: event['type'] == 'export' and event['status'] == 'queued' and event['name'] == 'Master')
    assert master['codec'] == 'prores_hq' and master['quality'] == 'master', master
    assert Path(master['path']) == (directory / 'deliveries/Master.mov').resolve() or master['path'].endswith('deliveries/Master.mov'), master
    editor.send({'type': 'export', 'quality': 'review', 'name': 'Review copy'})
    review = editor.expect('the review copy queued', lambda event: event['type'] == 'export' and event['status'] == 'queued' and event['name'] == 'Review copy')
    assert review['codec'] == 'h264', review
    editor.send({'type': 'export_cancel', 'jobId': review['jobId']})
    editor.expect('the review copy cancelled', lambda event: event['type'] == 'export' and event['status'] == 'cancelled' and event['jobId'] == review['jobId'])
    editor.refusals.append(lambda event: event.get('command') == 'export')
    editor.send({'type': 'export', 'quality': 'master', 'output': 'deliveries/master.mp4'})
    refused = editor.expect('a master into an .mp4 refused', lambda event: event['type'] == 'error' and event.get('command') == 'export')
    assert refused.get('code') == 'INVALID_RENDER_OUTPUT', refused
    done = editor.expect('the master', lambda event: event['type'] == 'export' and event.get('jobId') == master['jobId'] and event['status'] in FINISHED, 600)
    assert done['status'] == 'completed' and done['progress'] == 1, done
    streams = ffprobe(done['path'])
    video = next(stream for stream in streams if stream['codec_type'] == 'video')
    sound = next(stream for stream in streams if stream['codec_type'] == 'audio')
    assert video['codec_name'] == 'prores' and video['profile'] == 'HQ' and video['pix_fmt'].startswith('yuv422p10'), video
    assert sound['codec_name'] == 'pcm_s24le' and sound['sample_rate'] == '48000', sound
    notes['master'] = {'vendor': video.get('tags', {}).get('vendor_id'), 'encoder': video.get('tags', {}).get('encoder'),
                       'size': [video['width'], video['height']]}
    editor.send({'type': 'render_queue'})
    queue = editor.expect('the render queue', lambda event: event['type'] == 'render_queue' and any(
        job['jobId'] == master['jobId'] and job['status'] == 'completed' for job in event['jobs']))
    jobs = {job['jobId']: job for job in queue['jobs']}
    assert jobs[review['jobId']]['status'] == 'cancelled' and jobs[review['jobId']]['name'] == 'Review copy', queue
    assert jobs[master['jobId']]['outputPath'] == done['path'] and jobs[master['jobId']]['codec'] == 'prores_hq', queue
    assert all(job['quality'] != 'preview' for job in queue['jobs']), queue  # viewer previews are no output
    # The paperwork goes beside the master, each answer naming its format.
    folder = Path(done['path']).parent
    editor.send({'type': 'export_comments', 'format': 'csv'})
    csv = editor.expect('the sheet', lambda event: event['type'] == 'export' and event.get('kind') == 'paperwork' and event.get('format') == 'csv')
    assert csv['status'] == 'completed' and Path(csv['path']) == folder / 'Master comments.csv', csv
    sheet = (folder / 'Master comments.csv').read_text()
    assert 'Hold on the lower third' in sheet and 'Codex' in sheet and 'overlaps' not in sheet, sheet
    editor.send({'type': 'export_comments', 'format': 'csv', 'includeResolved': True, 'path': 'deliveries/everything.csv'})
    everything = editor.expect('the whole sheet', lambda event: event['type'] == 'export' and event.get('format') == 'csv')
    assert 'overlaps' in (directory / 'deliveries/everything.csv').read_text(), everything
    editor.send({'type': 'export_comments', 'format': 'edl'})
    edl = editor.expect('the EDL', lambda event: event['type'] == 'export' and event.get('format') == 'edl')
    text = (folder / 'Master.edl').read_text()
    assert edl['status'] == 'completed' and 'TITLE:' in text and '* LOC: 00:00:01:00' in text, text
    editor.send({'type': 'export_comments', 'format': 'markers'})
    marked = editor.expect('the markers', lambda event: event['type'] == 'export' and event.get('format') == 'markers')
    assert marked['status'] == 'completed' and marked['count'] >= 2 and 'path' not in marked, marked
    editor.send({'type': 'export_captions', 'format': 'srt'})
    srt = editor.expect('the SRT', lambda event: event['type'] == 'export' and event.get('kind') == 'captions' and event.get('format') == 'srt')
    assert srt['status'] == 'completed' and 'We open before the city does' in (folder / 'Master.srt').read_text(), srt
    editor.send({'type': 'export_captions', 'format': 'vtt'})
    vtt = editor.expect('the VTT', lambda event: event['type'] == 'export' and event.get('kind') == 'captions' and event.get('format') == 'vtt')
    assert vtt['status'] == 'completed' and (folder / 'Master.vtt').read_text().startswith('WEBVTT'), vtt
    editor.refusals.append(lambda event: event.get('command') == 'export_captions')
    editor.send({'type': 'export_captions', 'format': 'srt', 'track': 'V1'})
    failed = editor.expect('captions of a picture track', lambda event: event['type'] == 'export' and event.get('kind') == 'captions' and event['status'] == 'failed')
    assert failed['format'] == 'srt' and failed['message'], failed
    # kerfd keeps the newest 50 finished jobs, from any client, trimming
    # them when it queues one; the queue shows what it keeps, oldest first.
    def start(number):
        started = runtime_request(endpoint, {'operation': 'start_render', 'quality': 'full', 'name': f'History {number}'})
        assert started['ok'], started
        return started['job']['jobId']
    for number in range(52):
        assert runtime_request(endpoint, {'operation': 'cancel_render', 'jobId': start(number)})['ok']
    waiting = start(52)
    editor.send({'type': 'render_queue'})
    bounded = editor.expect('the bounded queue', lambda event: event['type'] == 'render_queue' and any(
        job['jobId'] == waiting for job in event['jobs']))
    finished = [job for job in bounded['jobs'] if job['status'] in FINISHED]
    names = [job['name'] for job in finished]
    assert names == [f'History {number}' for number in range(2, 52)], names
    assert bounded['jobs'][-1]['jobId'] == waiting and bounded['jobs'][-1]['status'] == 'queued', bounded['jobs'][-1]
    assert runtime_request(endpoint, {'operation': 'cancel_render', 'jobId': waiting})['ok']
    notes['queue'] = {'finished': len(finished), 'kept': names[0]}
    checks += ['export master prores_hq -> .mov ProRes 422 HQ + PCM 24-bit', 'export review + export_cancel',
               'export refusal code', 'render_queue event', 'export_comments csv/edl/markers beside the master',
               'export_captions srt/vtt and a refusal', 'render_queue history bound (50 finished, any client)']


def review_import(editor, directory, snapshot, checks):
    # Media new to the editor is probed on its own, and the answer names
    # only it: the editor merges it into what it has.
    editor.forget('media_info')
    editor.send({'type': 'import', 'path': str(directory / 'third.mp4')})
    imported = editor.state('the import', lambda next: len(next['timeline']['sources']) == 3)
    new = {source['id'] for source in imported['timeline']['sources']} - {'m1', 'm2'}
    assert len(new) == 1, imported['timeline']['sources']
    info = editor.expect('media info for the import', lambda event: event['type'] == 'media_info')
    assert set(info['media']) == new and info['media'][next(iter(new))]['codec'] == 'h264', info
    editor.send({'type': 'media_info'})
    every = editor.expect('media info for all', lambda event: event['type'] == 'media_info')
    assert set(every['media']) == {'m1', 'm2'} | new, every
    checks.append('media_info after an import names only the new media; asked, all')
    return imported


def frame_at(editor, position, timeout=20):
    """The viewer frame at `position`, as RGB tuples, and its size."""
    editor.send({'type': 'seek', 'position': position})
    frame = Frame()
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        event = editor.poll()
        if event is not None:
            editor.pending.append(event)
        if lib.kerf_bridge_frame(editor.bridge, C.byref(frame)) and abs(frame.time - position) < .05:
            data = C.string_at(frame.pixels, frame.length)
            return [tuple(data[at:at + 3]) for at in range(0, len(data), 4)], (frame.width, frame.height)
        time.sleep(.005)
    raise AssertionError(f'no frame at {position}')


def titles_pass():
    """A new, empty project: its frame rate set while it is empty, then
    titles placed by the generator insert op (as the Titles browser sends
    it), after which the rate is refused."""
    checks = []
    with tempfile.TemporaryDirectory(prefix='kerf-qt-titles-', dir=ROOT / 'target') as temporary:
        directory = Path(temporary)
        editor = Editor(directory)
        try:
            editor.expect('kerfd to be ready', lambda event: event['type'] == 'connection' and event['status'] == 'ready')
            empty = editor.state('the empty project')
            assert empty['timeline']['fps'] == 30 and not empty['timeline']['tracks'], empty['timeline']['fps']
            spot = editor.edit(empty, [{'op': 'set-sequence', 'rate': {'num': 25, 'den': 1}, 'width': 640, 'height': 360,
                                        'name': 'Spot'}], 'the empty sequence at 25 fps')
            sequence = spot['timeline']['sequences'][0]
            assert spot['timeline']['fps'] == 25 and sequence['name'] == 'Spot', sequence
            assert (sequence['width'], sequence['height']) == (640, 360), sequence
            assert 'sequence s1 name="Spot" rate=25 size=640x360' in spot['document'], spot['document']
            # Text the language reads works too, and NTSC rates keep their fraction.
            ntsc = editor.edit(spot, [{'op': 'set-sequence', 'rate': '29.97'}], 'the empty sequence at 29.97')
            assert abs(ntsc['timeline']['fps'] - 30000 / 1001) < 1e-6, ntsc['timeline']['fps']
            spot = editor.edit(ntsc, [{'op': 'set-sequence', 'rate': 25}], 'back to 25')
            assert spot['timeline']['fps'] == 25
            # The Sequence Settings dialog's own edit: the program's id and what
            # changed, its rate as {num,den} and the start as timecode.
            applied = editor.edit(spot, [{'op': 'set-sequence', 'sequence': sequence['id'], 'name': 'Spot v2',
                                          'rate': {'num': 24, 'den': 1}, 'start': '01:00:00:00'}], 'the dialog applied')
            assert applied['timeline']['fps'] == 24 and applied['timeline']['sequences'][0]['name'] == 'Spot v2', applied['timeline']['sequences']
            assert 'name="Spot v2"' in applied['document'] and 'start=01:00:00:00' in applied['document'], applied['document']
            spot = editor.edit(applied, [{'op': 'set-sequence', 'sequence': sequence['id'], 'name': 'Spot',
                                          'rate': {'num': 25, 'den': 1}, 'start': '00:00:00:00'}], 'the dialog undone')
            assert spot['timeline']['fps'] == 25
            # A title as the Titles browser adds it: a new video track and the
            # generator over what is there, in one edit.
            titled = editor.edit(spot, [
                {'op': 'add-track', 'kind': 'video', 'id': 'V2'},
                {'op': 'insert', 'generator': {'fx': 'title', 'params': {'text': 'Atlas Coffee', 'size': 0.2}},
                 'at': 0, 'frames': 50, 'tracks': ['V2'], 'mode': 'overwrite'}], 'the title added')
            title = track_of(titled, 'V2')['clips']
            assert len(title) == 1 and title[0]['startFrame'] == 0 and title[0]['durationFrames'] == 50, title
            assert 'gen=title@1' in titled['document'] and 'text="Atlas Coffee"' in titled['document'], titled['document']
            assert abs(titled['duration'] - 2.0) < 1e-6, titled['duration']
            # A solid inserted at the start ripples the title along.
            solid = editor.edit(titled, [{'op': 'insert', 'generator': {'fx': 'solid', 'params': {'color': '#ff0000'}},
                                          'at': 0, 'frames': 25, 'tracks': ['V2']}], 'the solid inserted')
            clips = track_of(solid, 'V2')['clips']
            assert [clip['startFrame'] for clip in clips] == [0, 25] and 'gen=solid@1' in solid['document'], clips
            assert abs(solid['duration'] - 3.0) < 1e-6, solid['duration']
            # Refusals: an effect is not a generator; the rate no longer changes.
            editor.refusals.append(lambda event: 'not a generator' in event.get('message', '') or
                                   'Frame rate can change only while the sequence is empty' in event.get('message', ''))
            editor.send({'type': 'edit', 'revision': solid['revision'], 'ops': [
                {'op': 'insert', 'generator': {'fx': 'color.basic'}, 'at': 0, 'frames': 25, 'tracks': ['V2']}]})
            editor.expect('an effect refused as a generator', lambda event: event['type'] == 'error' and 'not a generator' in event['message'])
            editor.send({'type': 'edit', 'revision': solid['revision'], 'ops': [{'op': 'set-sequence', 'rate': 30}]})
            editor.expect('the rate refused', lambda event: event['type'] == 'error' and
                          event['message'].endswith('Frame rate can change only while the sequence is empty'))
            kept = editor.state('the project unchanged', lambda next: next['revision'] == solid['revision'])
            assert kept['timeline']['fps'] == 25 and kept['document'] == solid['document']
            # Playback draws them: red first, then white text on black.
            red, size = frame_at(editor, 0.5)
            assert size == (640, 360) and all(r > 200 and g < 40 and b < 40 for r, g, b in red[::97]), red[:3]
            text, _ = frame_at(editor, 2.0)
            lit = sum(1 for pixel in text if min(pixel) > 200)
            assert 0 < lit < len(text) // 4, lit
            checks += ['set-sequence rate/size/name while empty (25, "29.97", 25)', 'set-sequence as the Sequence Settings dialog sends it', 'insert {generator} as the Titles browser sends it',
                       'generator insert ripples', 'generator refusals (effect, rate with content)', 'generators drawn in playback']
        finally:
            editor.close()
    return {'checks': checks}


# --- main ---
# The passes' home folders (Kerf Cloud's session, Git's and gh's state) sit
# outside the projects: gh may still write there once a project is gone.
HOME = Path(tempfile.mkdtemp(prefix='kerf-qt-home-', dir=ROOT / 'target'))
try:
    results = {}
    if '--review-only' not in sys.argv:
        results['editing'] = first_pass()
    results['titles'] = titles_pass()
    if '--titles-only' not in sys.argv:
        results['review'] = review_pass()
    print(json.dumps({'result': 'passed', **results}, indent=2))
finally:
    server.shutdown()
    shutil.rmtree(HOME, ignore_errors=True)

"""The Kerf generation API, version 1, over HTTP: the standard library's server, every request
checked for a key, the files a request names kept by their SHA-256, and one worker that runs the
requests in turn.
"""
import hashlib
import http.server
import json
import os
import re
import shutil
import sys
import threading
import traceback

from . import jobs as jobs_module
from .keys import Keys
from .models import Stopped

REQUEST_VERSION = 1
SHA256 = re.compile(r"^[0-9a-f]{64}$")
FILE_PATH = re.compile(r"^/v1/files/([^/]+)$")
GENERATION_PATH = re.compile(r"^/v1/generations/([0-9a-f]+)$")
CANCEL_PATH = re.compile(r"^/v1/generations/([0-9a-f]+)/cancel$")
# What a request's `out` may be called: a name, whose extension says what file is wanted.
OUT_NAME = re.compile(r"^[A-Za-z0-9._-]+$")


class Store:
    """The files sent and made, each kept by the SHA-256 of its bytes."""

    def __init__(self, data):
        self.folder = os.path.join(data, "files")
        self.work = os.path.join(data, "work")
        os.makedirs(self.folder, exist_ok=True)
        os.makedirs(self.work, exist_ok=True)

    def path(self, sha256):
        return os.path.join(self.folder, sha256)

    def has(self, sha256):
        return os.path.isfile(self.path(sha256))

    def keep(self, path):
        """Keep the file at `path`, moving it in, and give back its SHA-256."""
        digest = hashlib.sha256()
        with open(path, "rb") as file:
            for block in iter(lambda: file.read(1 << 16), b""):
                digest.update(block)
        sha256 = digest.hexdigest()
        os.replace(path, self.path(sha256))
        return sha256


def files_of(request):
    """Every file `request` names, as the API's GenerateRequest names them."""
    named = []
    if request.get("guide") is not None:
        named.append(request["guide"])
    named.extend((request.get("guides") or {}).values())
    for key in ("first", "last"):
        if request.get(key) is not None:
            named.append(request[key])
    named.extend(request.get("references") or [])
    named.extend(region["mask"] for region in request.get("regions") or [])
    return named


def with_paths(request, store):
    """`request` with each file it names, a SHA-256, as where that file is here."""
    here = dict(request)
    path = store.path
    if here.get("guide") is not None:
        here["guide"] = path(here["guide"])
    here["guides"] = {name: path(sha) for name, sha in (here.get("guides") or {}).items()}
    for key in ("first", "last"):
        if here.get(key) is not None:
            here[key] = path(here[key])
    here["references"] = [path(sha) for sha in here.get("references") or []]
    here["regions"] = [
        {"prompt": region["prompt"], "mask": path(region["mask"])}
        for region in here.get("regions") or []
    ]
    return here


def refuse_request(request, models, store):
    """Why `request` cannot be asked, or None where it can."""
    if not isinstance(request, dict):
        return "a request is a JSON object"
    if request.get("kerf-request") != REQUEST_VERSION:
        return f"this server reads requests of version {REQUEST_VERSION}"
    if request.get("model") not in models:
        return f"no model `{request.get('model')}` here: the models are {', '.join(sorted(models))}"
    out = request.get("out")
    if not isinstance(out, str) or not OUT_NAME.match(out):
        return "`out` is a file's name, such as out.mp4"
    try:
        named = files_of(request)
    except (AttributeError, KeyError, TypeError):
        return "the files a request names are SHA-256s, where the API says"
    for sha256 in named:
        if not isinstance(sha256, str) or not SHA256.match(sha256):
            return f"`{sha256}` is not a SHA-256 in lowercase hex"
        if not store.has(sha256):
            return f"no file {sha256} has been sent: PUT /v1/files/{sha256} first"
    return None


class Worker(threading.Thread):
    """Runs the requests one at a time, as they came."""

    def __init__(self, jobs, models, store):
        super().__init__(daemon=True, name="kerf-gen worker")
        self.jobs = jobs
        self.models = models
        self.store = store
        self.stopping = threading.Event()

    def run(self):
        while not self.stopping.is_set():
            job = self.jobs.take(timeout=0.2)
            if job is not None:
                self.run_job(job)

    def run_job(self, job):
        request = job.request
        model = self.models[request["model"]]
        folder = os.path.join(self.store.work, job.id)
        os.makedirs(folder, exist_ok=True)
        out = os.path.join(folder, request["out"])
        try:
            model.generate(
                with_paths(request, self.store),
                out,
                lambda progress=None, line=None: self.jobs.progress(job.id, progress, line),
                job.stop.is_set,
            )
            if job.stop.is_set():
                raise Stopped()
            if not os.path.isfile(out) or os.path.getsize(out) == 0:
                raise RuntimeError("the model ended and wrote nothing")
            self.jobs.done(job.id, self.store.keep(out))
        except Stopped:
            self.jobs.cancelled(job.id)
        except Exception as error:  # A model's failure is the request's, not the server's.
            traceback.print_exc()
            self.jobs.failed(job.id, error)
        finally:
            shutil.rmtree(folder, ignore_errors=True)


class Handler(http.server.BaseHTTPRequestHandler):
    protocol_version = "HTTP/1.1"
    server_version = "kerf-gen/1"

    # Set on the class the server makes for itself.
    keys = None
    jobs = None
    models = None
    store = None

    def log_message(self, form, *args):
        # To stderr, as the standard library's server logs: stdout says only where it listens.
        # The key is in a header, never in a line logged.
        print(f"{self.address_string()} {form % args}", file=sys.stderr, flush=True)

    def answer(self, status, body=None):
        data = b"" if body is None else json.dumps(body).encode("utf-8")
        self.send_response(status)
        if body is not None:
            self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(data)))
        self.end_headers()
        if self.command != "HEAD":
            self.wfile.write(data)

    def refuse(self, status, why):
        self.answer(status, {"error": why})

    def allowed(self):
        said = self.headers.get("Authorization", "")
        if said.startswith("Bearer ") and self.keys.takes(said[len("Bearer "):].strip()):
            return True
        # What a request sends is read, so the connection stays in step for the next.
        self.drain()
        self.refuse(401, "this server takes an API key: Authorization: Bearer <key>")
        return False

    def drain(self):
        left = int(self.headers.get("Content-Length") or 0)
        while left > 0:
            block = self.rfile.read(min(left, 1 << 16))
            if not block:
                break
            left -= len(block)

    def body_json(self):
        length = int(self.headers.get("Content-Length") or 0)
        return json.loads(self.rfile.read(length) or b"null")

    def do_GET(self):
        if not self.allowed():
            return
        if self.path == "/v1/models":
            listed = [dict(name=name, **model.SPEC) for name, model in sorted(self.models.items())]
            return self.answer(200, {"models": listed})
        found = FILE_PATH.match(self.path)
        if found:
            return self.send_file(found.group(1))
        found = GENERATION_PATH.match(self.path)
        if found:
            state = self.jobs.state(found.group(1))
            if state is None:
                return self.refuse(404, "no such request")
            return self.answer(200, state)
        self.refuse(404, "no such path in the Kerf generation API, version 1")

    def do_HEAD(self):
        if not self.allowed():
            return
        found = FILE_PATH.match(self.path)
        if not found or not SHA256.match(found.group(1)):
            return self.answer(404)
        self.answer(200 if self.store.has(found.group(1)) else 404)

    def do_PUT(self):
        if not self.allowed():
            return
        found = FILE_PATH.match(self.path)
        if not found or not SHA256.match(found.group(1)):
            self.drain()
            return self.refuse(404, "a file is sent to /v1/files/<its SHA-256>")
        sha256 = found.group(1)
        length = int(self.headers.get("Content-Length") or -1)
        if length < 0:
            return self.refuse(411, "a file is sent with its Content-Length")
        receiving = os.path.join(self.store.work, f"receiving-{threading.get_ident()}")
        digest = hashlib.sha256()
        with open(receiving, "wb") as file:
            left = length
            while left > 0:
                block = self.rfile.read(min(left, 1 << 16))
                if not block:
                    break
                digest.update(block)
                file.write(block)
                left -= len(block)
        if digest.hexdigest() != sha256:
            os.remove(receiving)
            return self.refuse(400, "the file sent is not the one its SHA-256 names")
        os.replace(receiving, self.store.path(sha256))
        self.answer(201, {"sha256": sha256})

    def do_POST(self):
        if not self.allowed():
            return
        if self.path == "/v1/generations":
            try:
                request = self.body_json()
            except ValueError:
                return self.refuse(400, "a request is JSON")
            why = refuse_request(request, self.models, self.store)
            if why:
                return self.refuse(400, why)
            return self.answer(201, {"id": self.jobs.submit(request)})
        found = CANCEL_PATH.match(self.path)
        if found:
            self.drain()
            state = self.jobs.cancel(found.group(1))
            if state is None:
                return self.refuse(404, "no such request")
            return self.answer(200, state)
        self.drain()
        self.refuse(404, "no such path in the Kerf generation API, version 1")

    def send_file(self, sha256):
        if not SHA256.match(sha256) or not self.store.has(sha256):
            return self.refuse(404, "no such file")
        path = self.store.path(sha256)
        self.send_response(200)
        self.send_header("Content-Type", "application/octet-stream")
        self.send_header("Content-Length", str(os.path.getsize(path)))
        self.end_headers()
        with open(path, "rb") as file:
            shutil.copyfileobj(file, self.wfile, 1 << 16)


class Server(http.server.ThreadingHTTPServer):
    daemon_threads = True

    def handle_error(self, request, client_address):
        # A client that closes a connection it kept open is no error of the server's.
        if isinstance(sys.exc_info()[1], (ConnectionResetError, BrokenPipeError)):
            return
        super().handle_error(request, client_address)


def make_server(data, listen, models):
    """A server for the data folder `data`, at `listen` (`host:port`), running `models` (by
    name), its worker started. A port of 0 is one the system picks."""
    host, _, port = listen.rpartition(":")
    keys = Keys(data)
    jobs = jobs_module.Jobs()
    store = Store(data)
    handler = type(
        "Bound", (Handler,), {"keys": keys, "jobs": jobs, "models": models, "store": store}
    )
    server = Server((host or "127.0.0.1", int(port)), handler)
    worker = Worker(jobs, models, store)
    worker.start()
    server.worker = worker
    return server

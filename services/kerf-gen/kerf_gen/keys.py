"""The API keys a server takes, kept in its data folder as their SHA-256 alone: a key is shown
once, when it is made, and is never written down here."""
import hashlib
import hmac
import json
import os
import secrets
import threading
import time

FILE = "keys.json"


def _digest(key):
    return hashlib.sha256(key.encode("utf-8")).hexdigest()


def _read(data):
    try:
        with open(os.path.join(data, FILE), encoding="utf-8") as file:
            return json.load(file)
    except FileNotFoundError:
        return []


def add(data, name):
    """Make a key named `name`, keep its digest, and give the key back."""
    key = "kg_" + secrets.token_urlsafe(32)
    kept = _read(data)
    kept.append({"name": name, "sha256": _digest(key), "added": int(time.time())})
    os.makedirs(data, exist_ok=True)
    path = os.path.join(data, FILE)
    written = path + ".writing"
    with open(written, "w", encoding="utf-8") as file:
        json.dump(kept, file, indent=2)
    os.chmod(written, 0o600)
    os.replace(written, path)
    return key


class Keys:
    """The keys a server takes: read again whenever the file changes, so a key made while it
    runs is taken at once."""

    def __init__(self, data):
        self._path = os.path.join(data, FILE)
        self._seen = None
        self._digests = []
        self._lock = threading.Lock()
        self._read()

    def _read(self):
        try:
            # Its size as well: two writes within one tick of the clock differ in it.
            found = os.stat(self._path)
            stamp = (found.st_mtime_ns, found.st_size)
        except FileNotFoundError:
            stamp = None
        with self._lock:
            if stamp != self._seen:
                self._seen = stamp
                self._digests = [entry["sha256"] for entry in _read(os.path.dirname(self._path))]
            return list(self._digests)

    def __len__(self):
        return len(self._read())

    def takes(self, key):
        digest = _digest(key)
        # Compared in constant time, and against every key, so how long it takes says nothing.
        found = False
        for kept in self._read():
            found |= hmac.compare_digest(kept, digest)
        return found

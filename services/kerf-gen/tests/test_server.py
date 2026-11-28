"""The server as a client speaks to it: what it refuses, and a request stopped while it runs."""
import hashlib
import http.client
import json
import tempfile
import threading
import time
import unittest

from kerf_gen import keys, models, server


class ServerTest(unittest.TestCase):
    def setUp(self):
        self.data = tempfile.mkdtemp()
        self.key = keys.add(self.data, "test")
        self.httpd = server.make_server(self.data, "127.0.0.1:0", {"fake": models.load("fake")})
        threading.Thread(target=self.httpd.serve_forever, daemon=True).start()
        self.port = self.httpd.server_address[1]

    def tearDown(self):
        self.httpd.shutdown()
        self.httpd.worker.stopping.set()
        self.httpd.server_close()

    def ask(self, method, path, body=None, key=None):
        connection = http.client.HTTPConnection("127.0.0.1", self.port, timeout=10)
        headers = {"Authorization": f"Bearer {self.key if key is None else key}"}
        if isinstance(body, (dict, list)):
            body = json.dumps(body).encode()
        connection.request(method, path, body=body, headers=headers)
        response = connection.getresponse()
        data = response.read()
        connection.close()
        return response.status, (json.loads(data) if data else None)

    def request(self, **more):
        return {
            "kerf-request": 1,
            "model": "fake",
            "prompt": "a test",
            "guide": None,
            "guides": {},
            "first": None,
            "last": None,
            "references": [],
            "seconds": None,
            "size": None,
            "seed": None,
            "signals": [],
            "moves": [],
            "more": {},
            "out": "out.mp4",
            **more,
        }

    def test_nothing_is_answered_without_a_key_it_took(self):
        for key in ["", "kg_not-a-key-of-its"]:
            status, said = self.ask("GET", "/v1/models", key=key)
            self.assertEqual(status, 401)
            self.assertIn("API key", said["error"])
        self.assertEqual(self.ask("GET", "/v1/models")[0], 200)

    def test_a_file_is_kept_only_as_the_sha256_it_was_sent_to(self):
        data = b"a guide"
        sha256 = hashlib.sha256(data).hexdigest()
        self.assertEqual(self.ask("HEAD", f"/v1/files/{sha256}")[0], 404)
        wrong = hashlib.sha256(b"another").hexdigest()
        self.assertEqual(self.ask("PUT", f"/v1/files/{wrong}", data)[0], 400)
        self.assertEqual(self.ask("HEAD", f"/v1/files/{wrong}")[0], 404)
        self.assertEqual(self.ask("PUT", f"/v1/files/{sha256}", data)[0], 201)
        self.assertEqual(self.ask("HEAD", f"/v1/files/{sha256}")[0], 200)
        # A request may name only a file it has been sent.
        unsent = hashlib.sha256(b"never sent").hexdigest()
        status, said = self.ask("POST", "/v1/generations", self.request(guide=unsent))
        self.assertEqual(status, 400)
        self.assertIn("has been sent", said["error"])
        status, _ = self.ask("POST", "/v1/generations", self.request(guide=sha256))
        self.assertEqual(status, 201)

    def test_a_request_stopped_while_it_runs_ends_cancelled_and_stays_so(self):
        status, created = self.ask("POST", "/v1/generations", self.request(more={"hold": "30"}))
        self.assertEqual(status, 201)
        path = f"/v1/generations/{created['id']}"
        ends = time.monotonic() + 10
        while self.ask("GET", path)[1]["state"] != "running":
            self.assertLess(time.monotonic(), ends)
            time.sleep(0.02)
        self.ask("POST", f"{path}/cancel")
        while self.ask("GET", path)[1]["state"] == "running":
            self.assertLess(time.monotonic(), ends)
            time.sleep(0.02)
        self.assertEqual(self.ask("GET", path)[1], {"state": "cancelled"})
        # Asked again, it is as it was.
        self.assertEqual(self.ask("POST", f"{path}/cancel")[1], {"state": "cancelled"})


if __name__ == "__main__":
    unittest.main()

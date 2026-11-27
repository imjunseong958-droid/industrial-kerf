"""The Kerf generation API as Kerf writes it (`contract.json`, which kerf-generate-api's
`contract` test writes from its types): this server reads what Kerf sends and answers as Kerf
reads it. A change to either side alone fails one test or the other."""
import json
import os
import tempfile
import unittest

from kerf_gen import jobs as J
from kerf_gen import models, server

with open(os.path.join(os.path.dirname(__file__), "contract.json"), encoding="utf-8") as file:
    CONTRACT = json.load(file)


class ContractTest(unittest.TestCase):
    def test_a_request_kerf_sends_is_one_the_server_takes(self):
        request = CONTRACT["request"]
        store = server.Store(tempfile.mkdtemp())
        named = server.files_of(request)
        self.assertEqual(len(named), 5)
        self.assertIn("no file", server.refuse_request(request, {"fake": models.load("fake")}, store))
        for sha256 in named:
            with open(store.path(sha256), "wb") as file:
                file.write(b"sent")
        self.assertIsNone(server.refuse_request(request, {"fake": models.load("fake")}, store))
        here = server.with_paths(request, store)
        self.assertEqual(sorted(server.files_of(here)), sorted(store.path(sha) for sha in named))

    def test_what_the_server_answers_is_what_kerf_reads(self):
        self.assertEqual(server.created(CONTRACT["created"]["id"]), CONTRACT["created"])
        self.assertEqual(server.refusal(CONTRACT["error"]["error"]), CONTRACT["error"])
        queued, running, done, failed, cancelled = CONTRACT["states"]
        jobs = J.Jobs()
        first = jobs.submit({})
        self.assertEqual(jobs.state(first), queued)
        jobs.take(timeout=0)
        jobs.progress(first, running["progress"], running["line"])
        self.assertEqual(jobs.state(first), running)
        jobs.done(first, done["out"])
        self.assertEqual(jobs.state(first), done)
        second = jobs.submit({})
        jobs.take(timeout=0)
        jobs.failed(second, failed["message"])
        self.assertEqual(jobs.state(second), failed)
        third = jobs.submit({})
        jobs.cancel(third)
        self.assertEqual(jobs.state(third), cancelled)

    def test_each_model_says_what_it_takes_in_the_words_kerf_reads(self):
        said = CONTRACT["models"]["models"][0]
        reads = {"guide", "first", "last", "ref", "signal", "moves", "regions"}
        for name in models.KNOWN:
            listed = server.listing({name: models.load(name)})["models"][0]
            self.assertLessEqual(set(listed), set(said), name)
            for key, value in listed.items():
                kind = (int, float) if isinstance(said[key], (int, float)) else type(said[key])
                self.assertIsInstance(value, kind, f"{name}: {key}")
            for read in listed["reads"]:
                self.assertTrue(read in reads or read.startswith("guide_"), f"{name}: {read}")

if __name__ == "__main__":
    unittest.main()

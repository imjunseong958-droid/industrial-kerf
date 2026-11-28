"""A request, once done, failed or cancelled, stays so whatever happens after: checked by running
random sequences of what a worker and a person do against a model of how a request goes."""
import random
import unittest

from kerf_gen import jobs as J


class Model:
    """How each request should be going, said as simply as it can be."""

    def __init__(self):
        self.states = {}
        self.queue = []

    def submit(self, job_id):
        self.states[job_id] = J.QUEUED
        self.queue.append(job_id)

    def take(self):
        if not self.queue:
            return None
        job_id = self.queue.pop(0)
        self.states[job_id] = J.RUNNING
        return job_id

    def cancel(self, job_id):
        if self.states[job_id] == J.QUEUED:
            self.queue.remove(job_id)
            self.states[job_id] = J.CANCELLED

    def end(self, job_id, state):
        if self.states[job_id] == J.RUNNING:
            self.states[job_id] = state


class JobsTest(unittest.TestCase):
    def test_a_request_that_has_ended_stays_as_it_ended(self):
        for seed in range(400):
            rng = random.Random(seed)
            jobs, model = J.Jobs(), Model()
            ended = {}
            for _ in range(rng.randint(1, 40)):
                ids = list(model.states)
                operation = rng.choice(
                    ["submit", "take", "cancel", "progress", "done", "failed", "cancelled"]
                    if ids
                    else ["submit"]
                )
                if operation == "submit":
                    model.submit(jobs.submit({"n": len(ids)}))
                elif operation == "take":
                    job = jobs.take(timeout=0)
                    expected = model.take()
                    self.assertEqual(None if job is None else job.id, expected, seed)
                else:
                    job_id = rng.choice(ids)
                    if operation == "cancel":
                        jobs.cancel(job_id)
                        model.cancel(job_id)
                    elif operation == "progress":
                        jobs.progress(job_id, rng.random(), "a line")
                    elif operation == "done":
                        jobs.done(job_id, "0" * 64)
                        model.end(job_id, J.DONE)
                    elif operation == "failed":
                        jobs.failed(job_id, "it broke")
                        model.end(job_id, J.FAILED)
                    else:
                        jobs.cancelled(job_id)
                        model.end(job_id, J.CANCELLED)
                for job_id, expected in model.states.items():
                    state = jobs.state(job_id)
                    self.assertEqual(state["state"], expected, seed)
                    if job_id in ended:
                        self.assertEqual(state, ended[job_id], seed)
                    elif expected in J.FINISHED:
                        ended[job_id] = state


if __name__ == "__main__":
    unittest.main()

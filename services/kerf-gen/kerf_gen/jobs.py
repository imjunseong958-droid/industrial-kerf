"""The requests a server is asked, and how each is going.

A request is queued, runs, and ends done, failed or cancelled. Once it has ended it stays as it
ended: nothing a worker or a person does after that changes it. One worker runs the requests one
at a time, in the order they came, as one GPU makes one thing at a time.
"""
import collections
import threading
import uuid

QUEUED = "queued"
RUNNING = "running"
DONE = "done"
FAILED = "failed"
CANCELLED = "cancelled"
FINISHED = (DONE, FAILED, CANCELLED)


class Job:
    def __init__(self, job_id, request):
        self.id = job_id
        self.request = request
        self.state = {"state": QUEUED}
        # Asked to stop while it runs: the model stops at the next point it can.
        self.stop = threading.Event()

    def finished(self):
        return self.state["state"] in FINISHED


class Jobs:
    def __init__(self):
        self._lock = threading.Condition()
        self._jobs = {}
        self._queue = collections.deque()

    def submit(self, request):
        """Queue `request` and give back its id."""
        with self._lock:
            job = Job(uuid.uuid4().hex, request)
            self._jobs[job.id] = job
            self._queue.append(job)
            self._lock.notify_all()
            return job.id

    def state(self, job_id):
        """How request `job_id` is going, or None for one there is not."""
        with self._lock:
            job = self._jobs.get(job_id)
            return None if job is None else dict(job.state)

    def cancel(self, job_id):
        """Stop request `job_id`: one that waits is cancelled now, and one that runs is asked to
        stop. One that has ended stays as it ended. Gives back how it is going."""
        with self._lock:
            job = self._jobs.get(job_id)
            if job is None:
                return None
            if job.state["state"] == QUEUED:
                self._queue.remove(job)
                job.state = {"state": CANCELLED}
            elif job.state["state"] == RUNNING:
                job.stop.set()
            return dict(job.state)

    def take(self, timeout=None):
        """The next request to run, now running; None where none came within `timeout`."""
        with self._lock:
            if not self._queue and not self._lock.wait_for(lambda: self._queue, timeout):
                return None
            job = self._queue.popleft()
            job.state = {"state": RUNNING}
            return job

    def progress(self, job_id, progress=None, line=None):
        """What a running request says of itself: how far, from 0 to 1, and a line."""
        with self._lock:
            job = self._jobs[job_id]
            if job.state["state"] != RUNNING:
                return
            state = {"state": RUNNING}
            if progress is not None:
                state["progress"] = max(0.0, min(1.0, float(progress)))
            if line:
                state["line"] = str(line)
            job.state = state

    def done(self, job_id, out):
        self._end(job_id, {"state": DONE, "out": out})

    def failed(self, job_id, message):
        self._end(job_id, {"state": FAILED, "message": str(message)})

    def cancelled(self, job_id):
        self._end(job_id, {"state": CANCELLED})

    def _end(self, job_id, state):
        # Only a request that runs is ended by what runs it: one that waits is cancelled, and
        # one that has ended stays as it ended.
        with self._lock:
            job = self._jobs[job_id]
            if job.state["state"] == RUNNING:
                job.state = state

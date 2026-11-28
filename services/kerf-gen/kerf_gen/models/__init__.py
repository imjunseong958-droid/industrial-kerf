"""The models a server can run, each a module of its own that says what it takes (`SPEC`, as
the Kerf generation API's ModelSpec) and makes a file (`generate`). What a model takes is said
in its module and nowhere else."""
import importlib


class Stopped(Exception):
    """A model stopped because it was asked to."""


# A model's name, as a `generate` line and `--model` name it, and its module.
KNOWN = {
    "fake": "fake",
}


def load(name):
    """The model `name`, its module imported only once it is asked for."""
    if name not in KNOWN:
        raise SystemExit(f"no model `{name}`: the models are {', '.join(sorted(KNOWN))}")
    module = importlib.import_module(f"kerf_gen.models.{KNOWN[name]}")
    return module

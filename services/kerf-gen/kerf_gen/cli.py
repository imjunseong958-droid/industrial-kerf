"""`kerf-gen key add` makes an API key; `kerf-gen serve` serves the Kerf generation API."""
import argparse
import os
import sys

from . import keys, models, server


def main(argv):
    parser = argparse.ArgumentParser(
        prog="kerf-gen",
        description="Kerf's generation server: the Kerf generation API, version 1, in front of models on this machine.",
    )
    commands = parser.add_subparsers(dest="command", required=True)

    key = commands.add_parser("key", help="the API keys the server takes")
    key_commands = key.add_subparsers(dest="key_command", required=True)
    add = key_commands.add_parser("add", help="make a key and print it, once")
    add.add_argument("--data", required=True, help="the server's data folder")
    add.add_argument("--name", default="key", help="what the key is for, to tell it from others")

    serve = commands.add_parser("serve", help="serve the API")
    serve.add_argument("--data", required=True, help="the server's data folder: its keys and files")
    serve.add_argument(
        "--listen", default="127.0.0.1:8190", help="host:port to listen at; port 0 is any free one"
    )
    serve.add_argument(
        "--model",
        action="append",
        required=True,
        help=f"a model to serve, one of {', '.join(sorted(models.KNOWN))}; given again for more",
    )

    said = parser.parse_args(argv)
    if said.command == "key":
        print(keys.add(said.data, said.name))
        return 0

    served = {name: models.load(name) for name in said.model}
    if not len(keys.Keys(said.data)):
        print(
            f"{said.data} has no API key yet: `kerf-gen key add --data {said.data}` makes one",
            file=sys.stderr,
        )
        return 2
    os.makedirs(said.data, exist_ok=True)
    # A model that cannot load says so now, not at its first request.
    for model in served.values():
        if hasattr(model, "prepare"):
            model.prepare()
    httpd = server.make_server(said.data, said.listen, served)
    host, port = httpd.server_address[:2]
    # The first line says where it listens, which a program that started it reads.
    print(f"listening on http://{host}:{port}", flush=True)
    try:
        httpd.serve_forever()
    except KeyboardInterrupt:
        pass
    finally:
        httpd.worker.stopping.set()
        httpd.server_close()
    return 0

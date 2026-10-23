#!/usr/bin/env python3
"""Minimal FluxBar plugin written in Python (no SDK required).

FluxBar speaks newline-delimited JSON-RPC 2.0 over stdin/stdout. This plugin
updates the `hello` item once per second and demonstrates that plugins can be
written in any language.

Install without a build step:

    fluxbar plugin install examples/plugins/hello-python --no-build

Then add this to your config (the installer writes it for you):

    [items.hello]
    kind = "text"
    position = "right"
    label = "python"
"""

import json
import sys
import time


def send(payload):
    sys.stdout.write(json.dumps(payload) + "\n")
    sys.stdout.flush()


def main():
    started = time.time()
    for line in sys.stdin:
        line = line.strip()
        if not line:
            continue

        try:
            request = json.loads(line)
        except json.JSONDecodeError:
            continue

        method = request.get("method")
        request_id = request.get("id")

        if method == "init":
            send({"jsonrpc": "2.0", "id": request_id, "result": None})
        elif method == "event":
            kind = request.get("params", {}).get("kind", "event")
            result = {
                "updates": {
                    "hello": {
                        "label": f"python: {kind}",
                        "progress": min(100.0, (time.time() - started) * 20 % 100),
                    }
                }
            }
            send({"jsonrpc": "2.0", "id": request_id, "result": result})
        elif method == "shutdown":
            send({"jsonrpc": "2.0", "id": request_id, "result": None})
            break
        else:
            send(
                {
                    "jsonrpc": "2.0",
                    "id": request_id,
                    "error": {"code": -32601, "message": f"unknown method {method}"},
                }
            )


if __name__ == "__main__":
    main()

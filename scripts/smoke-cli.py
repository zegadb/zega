"""Start the released CLI binary and exercise its authenticated HTTP API.

    python3 scripts/smoke-cli.py BINARY                  # run it here
    python3 scripts/smoke-cli.py --image ubuntu:22.04 BINARY
                                                         # run it in a plain image (no nix)

With --image the binary is copied into a fresh container of that image
(scripts/linux_binary.py run), so the run proves it starts on that
distribution, not only on the host that built it.
"""

import argparse
import json
from pathlib import Path
import socket
import subprocess
import sys
import tempfile
import time
from urllib.error import URLError
from urllib.request import Request, urlopen

sys.path.insert(0, str(Path(__file__).resolve().parent))
import linux_binary  # noqa: E402

parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
parser.add_argument("--image", help="run the binary inside a fresh container of this image")
parser.add_argument("binary")
options = parser.parse_args()
binary = Path(options.binary).resolve()

if options.image:
    _, code = linux_binary.run(options.image, binary, "--version")
else:
    code = subprocess.call([str(binary), "--version"])
if code:
    raise SystemExit(f"Built CLI: --version exited {code}" + (f" in {options.image}" if options.image else ""))

Path(".tmp").mkdir(exist_ok=True)
with tempfile.TemporaryDirectory(dir=".tmp") as directory:
    with socket.socket() as listener:
        listener.bind(("127.0.0.1", 0))
        port = listener.getsockname()[1]
    token = Path(directory).resolve() / "token"
    token.write_text("local-smoke-fixture\n")
    container = None
    if options.image:
        container, server = linux_binary.run(
            options.image, binary, "start", "--host", "0.0.0.0", "--data", "/var/tmp/zega-smoke",
            "--port", str(port), "--token-file", "/var/tmp/zega-smoke-token",
            ports=[port], files=[(token, "/var/tmp/zega-smoke-token")], detach=True)
    else:
        server = subprocess.Popen([str(binary), "start", "--data", str(Path(directory).resolve() / "data"),
                                   "--port", str(port), "--token-file", str(token)])
    try:
        request = Request(f"http://127.0.0.1:{port}/health", headers={"Authorization": "Bearer local-smoke-fixture"})
        # A container takes longer to come up than a local process.
        for attempt in range(300 if container else 100):
            if server.poll() is not None:
                raise RuntimeError("Built CLI exited before becoming healthy")
            try:
                with urlopen(request, timeout=1) as response:
                    assert response.status == 200
                    assert json.load(response) == {"ok": True}
                    print("Built CLI: authenticated /health HTTP 200" + (f" in {options.image}" if container else ""))
                    break
            except (URLError, ConnectionError):
                time.sleep(0.1)
        else:
            raise RuntimeError("Built CLI did not become healthy")
        request = Request(f"http://127.0.0.1:{port}/zql", data=json.dumps({"schema": "type Release { answer: Int }", "query": "mutation { Release(answer: 42) { answer } }"}).encode(),
                          headers={"Authorization": "Bearer local-smoke-fixture", "Content-Type": "application/json"})
        with urlopen(request, timeout=5) as response:
            result = json.load(response)
            assert result == {"ok": True, "result": {"answer": 42}}, result
            print("Built CLI: /zql returned answer=42" + (f" in {options.image}" if container else ""))
    finally:
        if container:
            linux_binary.remove(container)
        server.terminate()
        server.wait(timeout=10)

"""Real process ownership and reopen over fixture pipes, not Unix-socket evidence."""
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import time


example, cli = (Path(arg).resolve() for arg in sys.argv[1:3])
manifest = (Path(__file__).parent /
            "fixtures/build/shared_stdlib_local_dependency/project/wosy.toml").read_text()
manifest = manifest.replace("categories = []", 'categories = ["WHY"]') + """
[builders.llvm-native]
command = ["python", "scripts/build-native"]
args = []
toolchain_identity = "llvm-18.native"

[runners.native]
command = ["python", "scripts/run-native"]
args = []

[targets.app.artifacts.main.profiles.native]
backend = "llvm"
output = "native"
builder = "llvm-native"
run_runner = "native"

[targets.app.profiles.native]
main_artifact = "main"
"""


def exchange(project, store, root, profile):
    request_read, request_write = os.pipe()
    reply_read, reply_write = os.pipe()
    children = []
    start = time.perf_counter()
    with tempfile.TemporaryFile() as client_log, tempfile.TemporaryFile() as peer_log:
        try:
            try:
                peer = subprocess.Popen(
                    [example, "--fixture-diagnostic-peer", store, project, "app", root],
                    stdin=request_read, stdout=reply_write, stderr=peer_log,
                )
                children.append((peer, peer_log))
                client = subprocess.Popen(
                    [example, "--fixture-diagnostic-client", project, "app", profile],
                    stdin=reply_read, stdout=request_write, stderr=client_log,
                )
                children.append((client, client_log))
            finally:
                for fd in (request_read, request_write, reply_read, reply_write):
                    os.close(fd)
            reports = []
            for child, log in children:
                status = child.wait(timeout=15)
                log.seek(0)
                data = log.read()
                assert status == 0, (child.args, status, data)
                result = json.loads(data)
                assert result["pid"] == child.pid
                reports.append(result)
            assert client.pid != peer.pid
            return reports[0], reports[1], (time.perf_counter() - start) * 1_000
        finally:
            for child, _ in children:
                if child.poll() is None:
                    child.kill()
                child.wait()


with tempfile.TemporaryDirectory() as tmp:
    project, store = Path(tmp).resolve() / "project", Path(tmp).resolve() / "surrealkv"
    (project / "src").mkdir(parents=True)
    (project / "stdlib/src").mkdir(parents=True)
    (project / "wosy.toml").write_text(manifest)
    (project / "stdlib/wosy.toml").write_text('[package]\nname="std"\nsource_root="src"\n')
    (project / "src/main.w").write_text('%%start\nlocal = namespace std "item.w";\n%%end\n')
    dependency = project / "stdlib/src/item.w"
    authored = ("%%start\n# WHY: sessão selecionada ☕\nstruct Item { u8 value; }\n"
                "Item(Item) take = fn(input) { next = input; next; next };\n%%end\n")
    dependency.write_text(authored)

    def source(*args):
        return json.loads(subprocess.check_output(
            [example, project, store, "--persistent-source", "app", *args], timeout=30,
        ))

    def client_state():
        return tuple((project / name).read_bytes() for name in (".wosy/client.sqlite", ".wosy/cst/nodes.sqlite"))

    def ordinary(profile):
        proc = subprocess.run([cli, "build", "app", "--profile", profile], cwd=project,
                              capture_output=True, timeout=30)
        tail = b"wosy: build rejected by semantic diagnostics\n"
        assert proc.returncode == 1 and not proc.stdout and proc.stderr.endswith(tail), proc.stderr
        return proc.stderr[:-len(tail)].decode()

    pending = source("stage")
    root = source("submit-pending", "app", "src/main.w")["selected"]
    assert root == pending["pending"]
    expected = {profile: ordinary(profile) for profile in ("dev", "native")}
    archived = {}
    metrics = {}

    def report(profile, origin):
        peer, client, wall_ms = exchange(project, store, root, profile)
        assert peer["result"] == {"end": "Disconnected"}
        result = client["result"]
        assert result["kind"] == "report" and result["rendered"] == expected[profile]
        response = result["response"]
        assert bytes(response["source_root"]).hex() == root and response["profile"] == profile
        assert response["origin"] == origin and response["phase"] == "Analyzed"
        moved, = response["diagnostics"]
        for label in moved["labels"]:
            owner, span = label["span"]["source"], label["span"]["range"]
            assert owner["path"] == "src/item.w"
            assert authored.encode()[span["start"]:span["end"]] == b"next"
        return {key: value for key, value in response.items() if key != "origin"}, {
            "pair_wall_ms": round(wall_ms, 3), "client_ms": round(client["elapsed_ms"], 3),
            "peer_ms": round(peer["elapsed_ms"], 3),
        }

    initial = client_state()
    for profile in expected:
        archived[profile], cold = report(profile, "Derived")
        metrics[profile] = {"cold": cold}
    assert client_state() == initial

    dependency.write_text(authored.replace("sessão selecionada ☕", "pendente, sem confirmação 🚀"))
    staged = source("stage")
    assert staged["selected"] == root and staged["pending"] != root and staged["result"]["parsed"] == 1
    dependency.unlink()  # Only this fixture's own source; no ambient reparse can succeed.
    selected = client_state()
    for profile in expected:
        reopened, warm = report(profile, "Stored")
        assert reopened == archived[profile]
        metrics[profile]["fresh_process_warm"] = warm

    # Actual empty replacement store, but no replay payload is passed to this
    # fixture. The separate client observes the real terminal replay challenge.
    peer, client, _ = exchange(project, store.with_name("empty-catalog"), root, "dev")
    assert peer["result"] == {"end": "ReplayRequired"}
    assert client["result"] == {"kind": "replay-required"}
    assert client_state() == selected and not (project / ".wosy/artifacts").exists()
    print(json.dumps({"profile_timings": metrics, "separate_processes": True,
                      "selected_while_pending": True, "terminal_replay_challenge": True}))

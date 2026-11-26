"""Verify local named pins and independent reflection into a fresh real store."""
import json
import os
from pathlib import Path
import sqlite3
import subprocess
import sys
import tempfile
import time


example = Path(sys.argv[1]).resolve()
manifest = (Path(__file__).parent /
            "fixtures/build/shared_stdlib_local_dependency/project/wosy.toml").read_text()
manifest = manifest.replace("categories = []", 'categories = ["WHY"]')
source_key = ("std", "src/packet.w")


def invoke(*args, failure=None):
    start = time.perf_counter()
    with subprocess.Popen([example, *args], stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True) as child:
        stdout, stderr = child.stdout.read(), child.stderr.read()
        _, status, usage = os.wait4(child.pid, 0)
        child.returncode = os.waitstatus_to_exitcode(status)
    if failure is not None:
        assert child.returncode and not stdout and failure in stderr, (stdout, stderr, child.returncode)
        return
    assert child.returncode == 0 and not stderr, (stdout, stderr, child.returncode)
    response = json.loads(stdout)
    response["wall_ms"] = (time.perf_counter() - start) * 1000
    response["peak_kib"] = usage.ru_maxrss
    return response


def local(project, store, *args, target="app", **kwargs):
    return invoke(project, store, "--persistent-source", target, *args, **kwargs)


def packet(width, why):
    return f"%%start\n# WHY: {why}\nstruct copy Packet {{ u{width} count; }}\n%%end\n"


def project(path, initial):
    (path / "src").mkdir(parents=True)
    (path / "stdlib/src").mkdir(parents=True)
    (path / "wosy.toml").write_text(manifest)
    (path / "stdlib/wosy.toml").write_text('[package]\nname="std"\nsource_root="src"\n')
    (path / "src/main.w").write_text(
        '%%start\npayload = namespace std "packet.w";\npayload.Packet packet = { .count = 19; };\n%%end\n'
    )
    leaf = path / "stdlib/src/packet.w"
    leaf.write_text(initial)
    return leaf


with tempfile.TemporaryDirectory() as directory:
    base = Path(directory)
    a, b, original, replacement = (base / part for part in ("app-a", "app-b", "original", "replacement"))
    old_a = packet(32, "primeira versão do projeto A ☕")
    old_b = packet(16, "projeto B 🌱")
    a_leaf, b_leaf = project(a, old_a), project(b, old_b)
    name = "release/Δ"

    local(a, original, "pin-local", name, failure="without an acknowledged source root")
    a_stage = local(a, original, "stage")
    a_root = a_stage["pending"]
    local(a, original, "pin-local", name, failure="without an acknowledged source root")
    assert local(a, original, "local-pins")["result"] == []
    a_ack = local(a, original, "submit-pending", *source_key)
    assert a_ack["selected"] == a_root and a_ack["pending"] is None
    local(a, original, "pin-local", "", failure="pin name is empty")

    # Exact preceding metadata schema on this owned project; existing
    # acknowledged selection and canonical bytes must survive migration.
    a_db = a / ".wosy/client.sqlite"
    with sqlite3.connect(a_db) as connection:
        connection.executescript("ALTER TABLE source_targets DROP COLUMN selected_generation; DROP TABLE source_pins; PRAGMA user_version=2;")
    first = local(a, original, "pin-local", name)
    assert first["result"] == {"name": name, "target": "app", "root": a_root, "generation": 1}
    assert first["selected"] == a_root
    first_pin = first["result"]
    # The pin operation's binary exited before connection cleanup. This new
    # process sees the same durable local name without asking SurrealKV.
    assert local(a, original, "local-pins")["result"] == [first_pin]
    assert invoke(a, original, a_root, "--source-pins")["pins"] == []
    release = local(a, original, "unpin-local", name, a_root, "1")
    assert release["result"]["released"] and release["selected"] == a_root
    assert local(a, original, "local-pins")["result"] == []
    second = local(a, original, "pin-local", name)["result"]
    assert second == {**first_pin, "generation": 2}
    assert not local(a, original, "unpin-local", name, a_root, "1")["result"]["released"]
    assert not local(a, original, "unpin-local", name, a_root, "2", target="wrong")["result"]["released"]

    a_leaf.write_text(packet(64, "versão editada 🚀"))
    new_a = local(a, original, "stage")
    latest = new_a["pending"]
    assert latest != a_root and new_a["selected"] == a_root
    assert local(a, original, "local-pins")["result"] == [second]
    a_latest = local(a, original, "submit-pending", *source_key)
    assert a_latest["selected"] == latest
    assert a_latest["result"]["modules"][1]["structs"][0]["layout"]["size"] == 8
    b_stage = local(b, original, "stage")
    b_root = b_stage["pending"]
    assert b_root not in (a_root, latest)
    assert local(b, original, "submit-pending", *source_key)["selected"] == b_root
    other = local(b, original, "pin-local", name)["result"]
    assert other == {"name": name, "target": "app", "root": b_root, "generation": 1}
    assert local(a, original, "local-pins")["result"] == [second]

    # No workspace parse is needed to restore an older explicitly named
    # project generation when the store and the client process are both new.
    a_leaf.write_text("not valid Wosy")
    b_leaf.write_text("also not valid Wosy")
    local(a, replacement, "reflect-pin", "unknown", *source_key, failure="no named local source pin")
    assert not replacement.exists()
    refused = base / "not-a-store"
    refused.write_text("an ordinary file")
    local(a, refused, "reflect-pin", name, *source_key, failure="Error")
    assert local(a, original, "local-pins")["result"] == [second]
    restored = local(a, replacement, "reflect-pin", name, *source_key)
    ra = restored["result"]
    assert restored["selected"] == latest and ra["reflected_pin"] == second
    assert ra["root"] == a_root and not ra["known"] and ra["written"] > 0 and ra["parsed"] == 0
    assert ra["authored"] == old_a and ra["modules"][1]["structs"][0]["layout"]["size"] == 4
    mirror_a = [{"name": name, "root": a_root}]
    assert invoke(a, replacement, a_root, "--source-pins")["pins"] == mirror_a
    mirror_b = [{"name": name, "root": b_root}]
    restored_b = local(b, replacement, "reflect-pin", name, *source_key)["result"]
    assert restored_b["reflected_pin"] == other and restored_b["authored"] == old_b
    assert restored_b["modules"][1]["structs"][0]["layout"]["size"] == 2
    assert invoke(b, replacement, b_root, "--source-pins")["pins"] == mirror_b
    assert invoke(a, replacement, a_root, "--source-pins")["pins"] == mirror_a
    retry = local(a, replacement, "reflect-pin", name, *source_key)["result"]
    assert retry["known"] and retry["written"] == 0 and retry["reflected_pin"] == second

    # Releasing a local name removes neither the immutable source bytes nor
    # its separate graph projection. A distinct, explicit graph operation is
    # still required; do not imply cross-store atomicity.
    released = local(a, replacement, "unpin-local", name, a_root, "2")
    assert released["result"]["released"] and released["selected"] == latest
    assert local(a, replacement, "local-pins")["result"] == []
    local(a, replacement, "reflect-pin", name, *source_key, failure="no named local source pin")
    assert invoke(a, replacement, a_root, "--source-pins")["pins"] == mirror_a
    assert invoke(a, replacement, a_root, "--source-unpin", name)["removed"]
    assert invoke(a, replacement, a_root, "--source-pins")["pins"] == []
    assert invoke(b, replacement, b_root, "--source-pins")["pins"] == mirror_b
    old_cst = invoke(a, replacement, a_root, *source_key)
    assert old_cst["text"] == old_a
    assert a_leaf.read_text() == "not valid Wosy" and b_leaf.read_text() == "also not valid Wosy"
    assert not (a / ".wosy/cst/current.json").exists()
    print("local source pins survived child exit and reflected exact historical sources", {
        "source_layouts": [4, 8, 2], "local_pin_process_ms": round(first["wall_ms"], 3),
        "local_pin_process_peak_kib": first["peak_kib"], "restore_nodes": ra["nodes"],
        "restore_sqlite_ms": round(ra["replay_ms"], 3), "restore_store_ms": round(ra["publish_ms"], 3),
        "durable_pin_ms": round(ra["pin_ms"], 3), "restore_process_ms": round(restored["wall_ms"], 3),
        "restore_process_peak_kib": restored["peak_kib"],
    })

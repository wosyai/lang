"""Prepare a real owned v4 client, then reflect and recover named source under the current schema."""
import json
from pathlib import Path
import sqlite3
import subprocess
import sys


binary, mode, home = Path(sys.argv[1]).resolve(), sys.argv[2], Path(sys.argv[3]).resolve()
project, original_store, replacement, ordered = (
    home / name for name in ("project", "original-daemon", "replacement-daemon", "order-daemon")
)
main = project / "src/main.w"
db_path = project / ".wosy/client.sqlite"
name = "archive/☕"
already_released = "legacy-released"
first = "%%start\n# WHY: original histórico ☕\nu32 count = 11;\n%%end\n"
second = "%%start\n# WHY: current 🚀\nu32 count = 29;\n%%end\n"


def call(store, *args, refusal=None):
    result = subprocess.run([binary, project, store, *args], text=True, capture_output=True, timeout=30)
    if refusal:
        assert result.returncode and refusal in result.stderr and not result.stdout, result.stderr
        return
    result.check_returncode()
    return json.loads(result.stdout)


def state(store, *args, refusal=None):
    return call(store, "--persistent-source", "app", *args, refusal=refusal)


def version():
    with sqlite3.connect(db_path) as client, sqlite3.connect(project / ".wosy/cst/nodes.sqlite") as cst:
        assert cst.execute("PRAGMA user_version").fetchone() == (1,)
        return client.execute("PRAGMA user_version").fetchone()[0]


if mode == "seed":
    manifest = (Path(__file__).parent /
                "fixtures/build/shared_stdlib_local_dependency/project/wosy.toml").read_bytes()
    (project / "src").mkdir(parents=True)
    (project / "stdlib/src").mkdir(parents=True)
    (project / "wosy.toml").write_bytes(manifest.replace(b"categories = []", b'categories = ["WHY"]'))
    (project / "stdlib/wosy.toml").write_text('[package]\nname="std"\nsource_root="src"\n')
    main.write_text(first)
    a = state(original_store, "stage")["pending"]
    assert state(original_store, "submit-pending", "app", "src/main.w")["selected"] == a
    assert state(original_store, "pin-local", name)["result"] == {
        "name": name, "target": "app", "root": a, "generation": 1,
    }
    state(original_store, "pin-local", already_released)
    assert state(original_store, "unpin-local", already_released, a, "1")["result"]["released"]
    main.write_text(second)
    b = state(original_store, "stage")["pending"]
    assert b != a and state(original_store, "submit-pending", "app", "src/main.w")["selected"] == b
    assert version() == 4
elif mode == "verify":
    assert version() == 4 and not replacement.exists() and not ordered.exists()
    with sqlite3.connect(db_path) as client:
        a = client.execute("SELECT hex(root) FROM source_pins WHERE name=?", [name]).fetchone()[0].lower()
        b = client.execute("SELECT hex(selected) FROM source_targets WHERE target='app'").fetchone()[0].lower()
        assert a != b
    # The actual source worker writes the complete graph and pin in one
    # durable commit, then exits without closing SurrealKV or the client.
    cold = state(replacement, "reflect-durable-source-pin-exit", name)["result"]
    assert cold["root"] == a and cold["generation"] == 1 and cold["retained"]
    assert cold["replayed_nodes"] > 0
    assert version() == 6 and main.read_text() == second
    retained = call(replacement, a, "--source-pins")["pins"]
    assert retained == [{"name": name, "root": a}]
    restored = call(replacement, a, "app", "src/main.w")
    assert restored["text"] == first and restored["comments"][0]["raw_payload"].strip() == "original histórico ☕"
    hot = state(replacement, "reflect-durable-source-pin", name)["result"]
    assert hot["replayed_nodes"] is None and hot["root"] == a
    state(replacement, "reflect-durable-source-pin", already_released, refusal="fixture needs a local named source pin")
    ordered_result = state(ordered, "reflect-durable-source-pin-order", name)["result"]
    assert ordered_result == {
        "past": a, "latest": b, "release_before_source": True,
        "stale_receipts_and_intents_refused": True, "released_root_not_retained": True,
        "over_budget_rollback": True, "reopened_release_tombstone": True,
        "legacy_and_reflected_names_protect_both_owners": True,
    }
    assert call(ordered, b, "--source-pins")["pins"] == [{"name": name, "root": b}]
    print(json.dumps({"cold_atomic_reflection": cold, "hot_reflection": hot, "ordering": ordered_result}))
else:
    raise ValueError(mode)

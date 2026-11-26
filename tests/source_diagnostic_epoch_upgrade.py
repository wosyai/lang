"""Seed with the prior optimized example, then verify v4 using the same owned files."""
import json
from pathlib import Path
import sqlite3
import subprocess
import sys


binary, mode, home = Path(sys.argv[1]).resolve(), sys.argv[2], Path(sys.argv[3]).resolve()
project, store = home / "project", home / "daemon"
main = project / "src/main.w"
database = project / ".wosy/client.sqlite"
initial, edit = ("%%start\nu32 count = 11;\n%%end\n", "%%start\nu32 count = 29;\n%%end\n")


def request(*args):
    reply = subprocess.run([binary, project, store, "--persistent-source", "app", *args],
                           text=True, check=True, capture_output=True, timeout=30)
    return json.loads(reply.stdout)


def sqlite_state(version):
    with sqlite3.connect(database) as db:
        assert db.execute("PRAGMA user_version").fetchone() == (version,)
        fields = "hex(selected),hex(pending),generation"
        if version >= 4:
            fields += ",selected_generation"
        state = db.execute(f"SELECT {fields} FROM source_targets WHERE target = 'app'").fetchone()
    with sqlite3.connect(project / ".wosy/cst/nodes.sqlite") as nodes:
        assert nodes.execute("PRAGMA user_version").fetchone() == (1,)
    return state


if mode == "seed":
    manifest = (Path(__file__).parent /
                "fixtures/build/shared_stdlib_local_dependency/project/wosy.toml").read_bytes()
    (project / "src").mkdir(parents=True)
    (project / "stdlib/src").mkdir(parents=True)
    (project / "wosy.toml").write_bytes(manifest)
    (project / "stdlib/wosy.toml").write_text('[package]\nname="std"\nsource_root="src"\n')
    main.write_text(initial)
    first = request("stage")["pending"]
    assert request("submit-pending", "app", "src/main.w")["selected"] == first
    main.write_text(edit)
    second = request("stage")["pending"]
    state = sqlite_state(3)
    assert state == (first.upper(), second.upper(), 2) and first != second
    print("prior schema v3: selected A unchanged with pending B and generation 2")
elif mode == "verify":
    a, b, issued = sqlite_state(3)
    assert a and b and a != b and issued == 2
    observed = request("reconcile-framed-generation", "dev", edit)["result"]
    assert observed["root"].upper() == a
    assert observed["initial_generation"] == issued and observed["final_generation"] == issued + 1
    assert observed["pending_retained_selection"] and observed["delayed_reply_refused"]
    assert observed["old_terminal_replay_refused"]
    assert sqlite_state(5) == (a, "", issued + 1, issued + 1)
    assert main.read_text() == initial
    print("v3 migrated to current schema, exact B attempt kept, re-acknowledged identical A has a fresh epoch")
else:
    raise ValueError(mode)

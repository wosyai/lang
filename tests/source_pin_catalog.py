"""Seed real v5 pins, then page current v6 into two disposable catalogs."""
import json
from pathlib import Path
import sqlite3
import subprocess
import sys


binary, mode, home = Path(sys.argv[1]).resolve(), sys.argv[2], Path(sys.argv[3]).resolve()
project, original, cold, conflict = (home / name for name in ("project", "original", "cold", "conflict"))
main = project / "src/main.w"
db = project / ".wosy/client.sqlite"
names = ["10-released", "20-a/☕", "30-a/🚀", "40-direct-collision", "50-b", "60-b"]


def call(store, *args):
    run = subprocess.run([binary, project, store, *args], text=True, capture_output=True, timeout=30)
    run.check_returncode()
    return json.loads(run.stdout)


def state(store, *args):
    return call(store, "--persistent-source", "app", *args)


def schema():
    with sqlite3.connect(db) as connection:
        return connection.execute("PRAGMA user_version").fetchone()[0]


if mode in ("seed", "seed-legacy-prefix"):
    manifest = (Path(__file__).parent /
                "fixtures/build/shared_stdlib_local_dependency/project/wosy.toml").read_bytes()
    (project / "src").mkdir(parents=True)
    (project / "stdlib/src").mkdir(parents=True)
    (project / "wosy.toml").write_bytes(manifest)
    (project / "stdlib/wosy.toml").write_text('[package]\nname="std"\nsource_root="src"\n')
    main.write_text("%%start\nu32 count = 11;\n%%end\n")
    a = state(original, "stage")["pending"]
    assert state(original, "submit-pending", "app", "src/main.w")["selected"] == a
    for name in names[:4]:
        assert state(original, "pin-local", name)["result"]["root"] == a
    assert state(original, "unpin-local", names[0], a, "1")["result"]["released"]
    main.write_text("%%start\nu32 count = 29;\n%%end\n")
    b = state(original, "stage")["pending"]
    assert b != a and state(original, "submit-pending", "app", "src/main.w")["selected"] == b
    for name in names[4:]:
        assert state(original, "pin-local", name)["result"]["root"] == b
    assert schema() == 5
    if mode == "seed-legacy-prefix":
        # The exact inert row shape left when v5 upgraded v4 releases that
        # had no historical root witness. Only this disposable fixture adds
        # historical rows; ordinary v5 writers preserve release witnesses.
        with sqlite3.connect(db) as connection:
            connection.executemany(
                "INSERT INTO source_pins (name,target,root,generation,released_root) VALUES (?, 'app', NULL, 1, NULL)",
                [(f"00-legacy/{n:03}",) for n in range(80)],
            )
elif mode == "verify":
    assert schema() == 5 and not cold.exists() and not conflict.exists()
    with sqlite3.connect(db) as connection:
        a = connection.execute("SELECT hex(root) FROM source_pins WHERE name=?", [names[1]]).fetchone()[0].lower()
        b = connection.execute("SELECT hex(root) FROM source_pins WHERE name=?", [names[4]]).fetchone()[0].lower()
    result = state(cold, "reconcile-pin-catalog")["result"]
    assert result["pages"] == 3 and result["empty_pages"] == 0
    rows = result["rows"]
    assert [row["name"] for row in rows] == names
    assert [row["root"] for row in rows] == [a] * 4 + [b] * 2
    assert all(row["status"] == "durable" for row in rows)
    assert not rows[0]["retained"] and all(row["retained"] for row in rows[1:])
    assert [row["name"] for row in rows if row["replayed"]] == [names[1], names[4]]
    assert result["stale_cursor_rescanned"] and result["stopped_during_page"]
    after = result["after_changes"]
    changed_names = ["00-inserted-before-cursor", "01-inserted-during-page"]
    assert [row["name"] for row in after] == changed_names + names
    assert all(row["status"] == "durable" and not row["replayed"] for row in after)
    assert schema() == 6
    with sqlite3.connect(db) as connection:
        assert connection.execute("SELECT revision FROM source_pin_catalog WHERE singleton=0").fetchone() == (2,)
    listed = call(cold, b, "--source-pins")["pins"]
    assert [pin["name"] for pin in listed] == changed_names + names[1:]
    found = state(conflict, "reconcile-pin-catalog-collision")["result"]
    rows = found["rows"]
    assert found["pages"] == 4 and [row["name"] for row in rows] == changed_names + names
    assert [row["name"] for row in rows if row["status"] != "durable"] == [names[3]]
    assert next(row for row in rows if row["name"] == names[3])["status"] == "direct pin"
    assert rows[-1]["status"] == "durable"
    listed = call(conflict, b, "--source-pins")["pins"]
    assert [pin for pin in listed if pin["name"] == names[3]] == [{"name": names[3], "root": b}]
    print(json.dumps({"scan_pages": result["pages"], "cold_roots_replayed_once": 2, "scan_ms": result["elapsed_ms"],
                      "stale_cursor_and_midpage_stopped": True, "rescan_names": len(after),
                      "collision_reported_without_skipping_later_names": True}))
elif mode == "verify-legacy-prefix":
    assert schema() == 5 and not conflict.exists()
    observed = state(conflict, "reconcile-pin-catalog-collision")["result"]
    assert observed["empty_pages"] == 40 and observed["pages"] == 43
    assert [row["name"] for row in observed["rows"]] == names
    assert [row["name"] for row in observed["rows"] if row["status"] != "durable"] == [names[3]]
    assert schema() == 6
    print(json.dumps({"physical_windows": observed["pages"], "inert_only_windows": observed["empty_pages"],
                      "eligible_names_after_prefix": len(observed["rows"])}))
else:
    raise ValueError(mode)

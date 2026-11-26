"""The exact selected + pending SQLite generations produce one per-file CST delta."""
import json
from pathlib import Path
import sqlite3
import subprocess
import sys
import tempfile


example = Path(sys.argv[1]).resolve()
manifest = (Path(__file__).parent /
            "fixtures/build/shared_stdlib_local_dependency/project/wosy.toml").read_text()
manifest = manifest.replace("categories = []", 'categories = ["WHY"]')


def call(project, store, *args, target="app", refusal=None):
    child = subprocess.run([example, project, store, "--persistent-source", target, *map(str, args)],
                           text=True, capture_output=True)
    if refusal:
        assert child.returncode and refusal in child.stderr, child.stderr
        return
    child.check_returncode()
    return json.loads(child.stdout)


with tempfile.TemporaryDirectory() as temp:
    project, store = Path(temp) / "project", Path(temp) / "store"
    (project / "src").mkdir(parents=True)
    (project / "stdlib" / "src").mkdir(parents=True)
    (project / "wosy.toml").write_text(manifest)
    (project / "stdlib" / "wosy.toml").write_text('[package]\nname="std"\nsource_root="src"\n')
    main = project / "src" / "main.w"
    pkg = project / "stdlib" / "src"
    (pkg / "old.w").write_text("%%start\nu32 old_value = 7;\n%%end\n")
    (pkg / "stable.w").write_text("%%start\nu32 stable_value = 19;\n%%end\n")
    (pkg / "note.w").write_text("%%start\n# WHY: original ☕\nu32 note = 11;\n%%end\n")
    main.write_text('%%start\nold = namespace std "old.w";\n'
                    'stable = namespace std "stable.w";\nnote = namespace std "note.w";\n%%end\n')
    first = call(project, store, "stage")
    call(project, store, "proposal", first["pending"], first["attempt"],
         refusal="requires a selected predecessor")
    assert not store.exists()
    assert call(project, store, "state")["pending"] == first["pending"]
    selected = call(project, store, "submit-pending", "app", "src/main.w")
    assert selected["selected"] == first["pending"]

    shared = "%%start\nu32 deliberately_shared_new_binding = 913;\n%%end\n"
    (pkg / "a.w").write_text(shared)
    (pkg / "b.w").write_text(shared)
    (pkg / "note.w").write_text("%%start\n# WHY: nova anotação 🚀\nu32 note = 11;\n%%end\n")
    updated = ('%%start\na = namespace std "a.w";\nb = namespace std "b.w";\n'
               'stable = namespace std "stable.w";\nnote = namespace std "note.w";\n%%end\n')
    main.write_text(updated)
    candidate = call(project, store, "stage")
    assert candidate["result"]["parsed"] == 4
    assert candidate["result"]["reused"] == 1
    assert (candidate["selected"], candidate["pending"]) == (selected["selected"], candidate["result"]["root"])
    root, attempt = candidate["pending"], candidate["attempt"]

    # Reopen genuine prior state with smaller limits. Removing just the
    # disposable legacy usage summary leaves the persisted source unchanged
    # and exercises the bounded full traversal on its next reopen.
    metadata_db = project / ".wosy" / "client.sqlite"
    nodes_db = project / ".wosy" / "cst" / "nodes.sqlite"
    baseline_nodes = selected["result"]["nodes"]
    limit_args = ("proposal-limited", baseline_nodes, 16 << 20, root, attempt)
    before = (metadata_db.read_bytes(), nodes_db.read_bytes())
    call(project, store, "proposal-limited", 0, 16 << 20, root, attempt,
         refusal="LimitExceeded { field: Nodes, limit: 0 }")
    call(project, store, *limit_args, refusal=f"LimitExceeded {{ field: Nodes, limit: {baseline_nodes} }}")
    byte_limit = selected["result"]["encoded_bytes"] - 1
    call(project, store, "proposal-limited", 25000, byte_limit, root, attempt,
         refusal=f"LimitExceeded {{ field: EncodedBytes, limit: {byte_limit} }}")
    assert (metadata_db.read_bytes(), nodes_db.read_bytes()) == before
    with sqlite3.connect(metadata_db) as db:
        assert db.execute("DELETE FROM source_root_usage WHERE root = ?", (bytes.fromhex(root),)).rowcount == 1
    before = (metadata_db.read_bytes(), nodes_db.read_bytes())
    call(project, store, *limit_args, refusal=f"LimitExceeded {{ field: Nodes, limit: {baseline_nodes} }}")
    assert (metadata_db.read_bytes(), nodes_db.read_bytes()) == before
    after_budget = call(project, store, "state")
    assert (after_budget["selected"], after_budget["pending"], after_budget["attempt"]) == (selected["selected"], root, attempt)

    # Process-local observation is not needed to materialize the staged
    # proposal. A different later live source is not silently captured.
    main.write_text("%%start\nu32 outside_staged_generation = 23;\n%%end\n")
    local = call(project, store, "proposal", root, attempt)
    main.write_text(updated)
    assert (local["selected"], local["pending"], local["attempt"]) == (selected["selected"], root, attempt)
    assert local["result"]["oracle"] is False
    out = local["result"]
    assert 0 < out["wire_bytes"] < 1 << 20
    assert out["root"] == root and out["base"] == selected["selected"]
    by_name = {file["file"]["path"]: file for file in out["files"]}
    assert list(by_name) == ["src/main.w", "src/a.w", "src/b.w", "src/note.w", "src/old.w"], by_name
    assert by_name["src/main.w"]["old"] and by_name["src/main.w"]["new"] and by_name["src/main.w"]["added"]
    assert by_name["src/a.w"]["old"] is None and by_name["src/a.w"]["new"] and by_name["src/a.w"]["added"]
    assert by_name["src/b.w"]["old"] is None and by_name["src/b.w"]["new"] and by_name["src/b.w"]["added"] == 0
    assert by_name["src/a.w"]["new"] != by_name["src/b.w"]["new"]
    assert by_name["src/note.w"]["old"] != by_name["src/note.w"]["new"] and by_name["src/note.w"]["added"] > 0
    assert by_name["src/old.w"]["old"] and by_name["src/old.w"]["new"] is None and by_name["src/old.w"]["added"] == 0
    assert out["index"] > 0
    assert out["added"] == out["index"] + sum(file["added"] for file in by_name.values())

    call(project, store, "proposal", root, attempt, target="different-target",
         refusal="not the current source attempt")
    call(project, store.with_name("empty"), "proposal-admission", root, attempt,
         refusal="bound base CST predecessor is not published")
    snap = (metadata_db.read_bytes(), nodes_db.read_bytes())
    accepted = call(project, store, "proposal-admission", root, attempt)
    receipt = accepted["result"]["admission"]
    assert receipt["base_records"] == baseline_nodes and receipt["novel_records"] == out["added"]
    assert receipt["validated_files"] == 4 and not receipt["stale_attempt_rejected"]
    assert receipt["noncanonical_order_rejected"]
    assert (metadata_db.read_bytes(), nodes_db.read_bytes()) == snap
    again = call(project, store, "proposal-stale", root, attempt)
    assert again["pending"] == root and again["attempt"] > attempt
    assert again["result"]["admission"]["stale_attempt_rejected"]
    call(project, store, "proposal", root, attempt, refusal="not the current source attempt")
    actual = call(project, store, "proposal-oracle", root, again["attempt"])
    assert (actual["selected"], actual["pending"], actual["attempt"]) == (selected["selected"], root, again["attempt"])
    assert actual["result"]["oracle"] is True
    assert actual["result"]["files"] == out["files"]
    assert actual["result"]["wire_bytes"] == out["wire_bytes"]

    assert call(project, store, "submit-pending", "app", "src/main.w")["selected"] == root
    # A configuration-only edit produces a root and metadata changes, not
    # fabricated per-file edits.
    (project / "wosy.toml").write_text(manifest + "\n# configuration observation only\n")
    config = call(project, store, "stage")
    assert (config["result"]["parsed"], config["result"]["reused"]) == (0, 5)
    config_accept = call(project, store, "proposal-admission", config["pending"], config["attempt"])["result"]["admission"]
    assert config_accept["validated_files"] == 0 and config_accept["novel_records"] == 2
    assert config_accept["noncanonical_order_rejected"]
    config_proposal = call(project, store, "proposal-oracle", config["pending"], config["attempt"])
    assert config_proposal["result"]["files"] == []
    assert config_proposal["result"]["oracle"]
    assert config_proposal["result"]["added"] == config_proposal["result"]["index"] > 0
    print(json.dumps({"changed": out, "admission": receipt, "config_only": config_proposal["result"],
                      "config_admission": config_accept}))

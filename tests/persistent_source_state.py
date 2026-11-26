"""Reopen real project SQLite state across clients and a lost source store."""
import json
import os
from pathlib import Path
import sqlite3
import subprocess
import sys
import tempfile
import time


example = Path(sys.argv[1]).resolve()
standard = (Path(__file__).parent /
            "fixtures/build/shared_stdlib_local_dependency/project/wosy.toml").read_text()
standard = standard.replace("categories = []", 'categories = ["WHY"]')
second = "[targets.second]" + standard.split("[targets.app]", 1)[1].replace("targets.app", "targets.second")
suffix = ("std", "src/packet.w")


def packet(width, why):
    return f"%%start\n# WHY: {why}\nstruct copy Packet {{ u{width} count; }}\n%%end\n"


def invoke(root, store, target, action, *extra, failure=None):
    start = time.perf_counter()
    with subprocess.Popen(
        [example, root, store, "--persistent-source", target, action, *extra],
        stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True,
    ) as proc:
        stdout = proc.stdout.read()
        stderr = proc.stderr.read()
        _, status, usage = os.wait4(proc.pid, 0)
        proc.returncode = os.waitstatus_to_exitcode(status)
    if failure is not None:
        assert proc.returncode != 0 and not stdout, (proc.returncode, stdout, stderr)
        assert failure in stderr, stderr
        return
    assert proc.returncode == 0 and not stderr, (proc.returncode, stdout, stderr)
    outcome = json.loads(stdout)
    outcome["wall_ms"] = (time.perf_counter() - start) * 1000
    outcome["peak_kib"] = usage.ru_maxrss
    return outcome


with tempfile.TemporaryDirectory() as directory:
    base = Path(directory)
    root = base / "app"
    source = root / "src"
    lib = root / "stdlib/src"
    source.mkdir(parents=True)
    lib.mkdir(parents=True)
    active_manifest = standard + "\n" + second
    (root / "wosy.toml").write_text(active_manifest)
    (lib.parent / "wosy.toml").write_text('[package]\nname="std"\nsource_root="src"\n')
    leaves = 24
    for i in range(leaves):
        (lib / f"leaf{i}.w").write_text(
            "%%start\n" + "".join(f"u32 value{j} = {i * 128 + j};\n" for j in range(128)) + "%%end\n"
        )
    (source / "main.w").write_text(
        '%%start\npayload = namespace std "packet.w";\n'
        + "".join(f'leaf{i} = namespace std "leaf{i}.w";\n' for i in range(leaves))
        + "payload.Packet packet = { .count = 19; };\n%%end\n"
    )
    original = packet(32, "comentário ☕")
    editing = packet(64, "edição 🚀")
    canceled = packet(16, "cancelado 🌱")
    file = lib / "packet.w"
    file.write_text(original)
    primary = base / "primary"
    replacement = base / "replacement"
    db = root / ".wosy/client.sqlite"
    nodes = root / ".wosy/cst/nodes.sqlite"
    view = nodes.with_name("current.json")
    view.parent.mkdir(parents=True)
    view.write_bytes(b'{"ordinary_tool_view":"preserved"}\n')

    initial = invoke(root, primary, "app", "stage")
    initial_root = initial["pending"]
    assert initial["attempt"] == initial["result"]["attempt"] == 1
    assert initial["selected"] is None and initial_root == initial["result"]["root"]
    assert initial["result"]["parsed"] == leaves + 2 and initial["result"]["reused"] == 0
    assert initial["result"]["new_nodes"] > 5000 and initial["result"]["new_bytes"] > 500_000
    assert initial["result"]["storage_reads"] == 0
    assert db.stat().st_dev == nodes.stat().st_dev
    assert db.read_bytes()[:16] == nodes.read_bytes()[:16] == b"SQLite format 3\x00"
    invoke(root, primary, "app", "stage", failure="already pending")

    # Loss of the client after daemon publication, before local promotion.
    interrupted = invoke(root, primary, "app", "publish-before-confirm", *suffix)
    assert not interrupted["result"]["known"] and interrupted["result"]["written"] > 0
    assert interrupted["pending"] == initial_root and interrupted["selected"] is None
    confirmed = invoke(root, primary, "app", "submit-pending", *suffix)
    assert confirmed["result"]["known"] and confirmed["result"]["written"] == 0
    assert confirmed["result"]["confirmed"] and confirmed["result"]["authored"] == original
    assert confirmed["result"]["modules"][1]["structs"][0]["layout"]["size"] == 4
    assert confirmed["pending"] is None and confirmed["selected"] == initial_root

    def inventory():
        with sqlite3.connect(nodes) as conn:
            return conn.execute("SELECT COUNT(*), SUM(length(payload)) FROM nodes").fetchone()

    prior_count = inventory()
    assert prior_count == (initial["result"]["new_nodes"], initial["result"]["new_bytes"])
    warm = invoke(root, primary, "app", "stage")
    assert warm["result"]["parsed"] == 0 and warm["result"]["reused"] == leaves + 2
    assert warm["selected"] == initial_root and warm["result"]["attempt"] is None
    assert warm["result"]["new_nodes"] == warm["result"]["new_bytes"] == 0 and inventory() == prior_count
    assert warm["result"]["storage_reads"] == leaves + 4

    # The fast observation path still enforces both of the current caller's
    # limits. A missing/malformed receipt cannot turn into valid budget data.
    args = (str(prior_count[0]), str(prior_count[1]))
    exact = invoke(root, primary, "app", "stage-limited", *args)
    assert exact["result"]["root"] == initial_root and exact["result"]["storage_reads"] == leaves + 4
    nodes_low, bytes_low = prior_count[0] - 1, prior_count[1] - 1
    invoke(root, primary, "app", "stage-limited", str(nodes_low), args[1],
           failure=f"LimitExceeded {{ field: Nodes, limit: {nodes_low} }}")
    invoke(root, primary, "app", "stage-limited", args[0], str(bytes_low),
           failure=f"LimitExceeded {{ field: EncodedBytes, limit: {bytes_low} }}")
    with sqlite3.connect(db) as client_db:
        (receipt,) = client_db.execute("SELECT receipt FROM source_root_usage WHERE root = ?",
                                      (bytes.fromhex(initial_root),)).fetchone()
        client_db.execute("UPDATE source_root_usage SET receipt = ? WHERE root = ?",
                          (bytes([receipt[0] ^ 1]) + receipt[1:], bytes.fromhex(initial_root)))
    invoke(root, primary, "app", "stage", failure="invalid source observation usage receipt")
    intact = invoke(root, primary, "app", "submit-selected", *suffix)
    assert intact["selected"] == initial_root and intact["result"]["authored"] == original
    # Reproduce the actual v1 schema on this owned SQLite pair, preserving
    # the confirmed selection and every canonical CST row.
    with sqlite3.connect(db) as client_db:
        client_db.executescript("ALTER TABLE source_targets DROP COLUMN selected_generation; DROP TABLE source_pins; DROP TABLE source_root_usage; PRAGMA user_version=1;")
    migrated = invoke(root, primary, "app", "stage")
    assert migrated["selected"] == initial_root and migrated["pending"] is None
    assert migrated["result"]["parsed"] == 0 and migrated["result"]["new_nodes"] == 0
    assert migrated["result"]["storage_reads"] == prior_count[0] and inventory() == prior_count
    migrated_warm = invoke(root, primary, "app", "stage")
    assert migrated_warm["result"]["storage_reads"] == leaves + 4 and inventory() == prior_count

    # Same authored source, separate actual configured target and selection.
    other = invoke(root, primary, "second", "stage")
    other_root = other["pending"]
    assert other_root != initial_root
    assert other["result"]["new_nodes"] == 2
    assert invoke(root, primary, "second", "submit-pending", *suffix)["selected"] == other_root
    other_count = inventory()

    file.write_text(editing)
    changed = invoke(root, primary, "app", "stage")
    changed_root = changed["pending"]
    assert changed["attempt"] == 2
    assert changed["selected"] == initial_root and changed_root != initial_root
    assert (changed["result"]["parsed"], changed["result"]["reused"]) == (1, leaves + 1)
    assert changed["result"]["storage_reads"] == prior_count[0]
    assert 0 < changed["result"]["new_nodes"] < 100
    assert inventory() == tuple(a + b for a, b in zip(other_count, (
        changed["result"]["new_nodes"], changed["result"]["new_bytes"])))

    # Change the live workspace without observing it; reopening the exact
    # pending root must not parse either current project or dependency bytes.
    file.write_text("deliberately not a Wosy source file")
    refused = base / "not-a-directory"
    refused.write_text("Cannot be opened as a graph store")
    invoke(root, refused, "app", "submit-pending", *suffix, failure="Error")
    saved = invoke(root, primary, "app", "state")
    assert (saved["selected"], saved["pending"]) == (initial_root, changed_root)
    assert not invoke(root, primary, "app", "abandon", other_root, "1")["result"]["abandoned"]

    recovered = invoke(root, replacement, "app", "submit-pending", *suffix)
    assert recovered["pending"] is None and recovered["selected"] == changed_root
    result = recovered["result"]
    assert not result["known"] and result["nodes"] > 5000
    assert result["namespace_derived"] > 0 and result["written"] == result["nodes"] + result["namespace_derived"]
    assert result["confirmed"] and result["parsed"] == 0
    assert result["authored"] == editing and "edição" in result["comments"][0]["raw_payload"]
    assert result["modules"][1]["structs"][0]["layout"]["size"] == 8
    history = invoke(root, replacement, "second", "submit-selected", *suffix)
    assert history["selected"] == other_root and history["result"]["authored"] == original
    assert history["result"]["modules"][1]["structs"][0]["layout"]["size"] == 4
    assert file.read_text() == "deliberately not a Wosy source file"

    file.write_text(canceled)
    abandoned = invoke(root, replacement, "app", "stage")
    abandoned_root = abandoned["pending"]
    old_attempt = abandoned["attempt"]
    assert abandoned_root not in (None, changed_root)
    delayed = invoke(root, replacement, "app", "publish-before-confirm", *suffix)
    assert delayed["pending"] == abandoned_root and delayed["attempt"] == old_attempt
    assert delayed["result"]["authored"] == canceled and delayed["result"]["written"] > 0
    discard = invoke(root, replacement, "app", "abandon", abandoned_root, str(old_attempt))
    assert discard["result"]["abandoned"] and discard["selected"] == changed_root and discard["pending"] is None
    restaged = invoke(root, replacement, "app", "stage")
    assert restaged["pending"] == abandoned_root and restaged["attempt"] == old_attempt + 1
    assert restaged["result"]["new_nodes"] == restaged["result"]["new_bytes"] == 0
    # This reply corresponds to an actual completed daemon publish above,
    # before cancelling and restaging the exact same immutable source root.
    ack = invoke(root, replacement, "app", "delayed-ack", abandoned_root, str(old_attempt))
    assert not ack["result"]["confirmed"] and ack["attempt"] == restaged["attempt"]
    assert ack["selected"] == changed_root and ack["pending"] == abandoned_root
    earlier_abandon = invoke(root, replacement, "app", "abandon", abandoned_root, str(old_attempt))
    assert not earlier_abandon["result"]["abandoned"] and earlier_abandon["attempt"] == restaged["attempt"]
    discard = invoke(root, replacement, "app", "abandon", abandoned_root, str(restaged["attempt"]))
    assert discard["result"]["abandoned"] and discard["selected"] == changed_root and discard["pending"] is None

    # Falling back to a historical canonical root still compares actual
    # duplicate records. Its novel token is outside the selected base.
    canceled_token = "# WHY: cancelado 🌱".encode()
    with sqlite3.connect(nodes) as conn:
        [(canceled_id, payload)] = conn.execute(
            "SELECT digest, payload FROM nodes WHERE substr(payload, ?) = ?",
            (-len(canceled_token), canceled_token),
        ).fetchall()
        conn.execute("UPDATE nodes SET payload = ? WHERE digest = ?", (payload + b"damaged", canceled_id))
    damaged_count = inventory()
    invoke(root, replacement, "app", "stage", failure="stored CST bytes differ from admitted source")
    state = invoke(root, replacement, "app", "state")
    assert state["selected"] == changed_root and state["pending"] is None and inventory() == damaged_count
    with sqlite3.connect(nodes) as conn:
        conn.execute("UPDATE nodes SET payload = ? WHERE digest = ?", (payload, canceled_id))
    with sqlite3.connect(db) as client_db:
        historical_key = bytes.fromhex(abandoned_root)
        (historical_receipt,) = client_db.execute(
            "SELECT receipt FROM source_root_usage WHERE root = ?", (historical_key,),
        ).fetchone()
        client_db.execute("UPDATE source_root_usage SET receipt = ? WHERE root = ?",
                          (historical_receipt + b"damaged", historical_key))
    invoke(root, replacement, "app", "stage", failure="stored source observation receipt differs from admitted source")
    state = invoke(root, replacement, "app", "state")
    assert state["selected"] == changed_root and state["pending"] is None
    with sqlite3.connect(db) as client_db:
        client_db.execute("UPDATE source_root_usage SET receipt = ? WHERE root = ?",
                          (historical_receipt, historical_key))

    fresh = base / "fresh"
    selected = invoke(root, fresh, "app", "submit-selected", *suffix)
    assert selected["result"]["authored"] == editing and not selected["result"]["known"]
    assert not selected["result"]["confirmed"] and selected["selected"] == changed_root
    assert view.read_bytes() == b'{"ordinary_tool_view":"preserved"}\n'

    # Same path, source bytes and profile, but changing the configured parser
    # policy must choose new file identities and actually parse every file.
    file.write_text(editing)
    (root / "wosy.toml").write_text(active_manifest.replace('["WHY"]', '["WHY", "NOTE"]'))
    policy = invoke(root, replacement, "app", "stage")
    assert (policy["result"]["parsed"], policy["result"]["reused"]) == (leaves + 2, 0)
    assert policy["pending"] not in (None, changed_root) and policy["selected"] == changed_root
    invoke(root, replacement, "app", "abandon", policy["pending"], str(policy["attempt"]))
    (root / "wosy.toml").write_text(active_manifest)

    # Damage a descendant outside the verified observation metadata. A
    # no-op observation is not a health attestation, but full replay and
    # every new staged generation still reject corrupt base source bytes.
    authored_token = "# WHY: edição 🚀".encode()
    with sqlite3.connect(nodes) as conn:
        [(token_id, token)] = conn.execute("SELECT digest, payload FROM nodes WHERE substr(payload, ?) = ?",
                                          (-len(authored_token), authored_token)).fetchall()
        conn.execute("UPDATE nodes SET payload = ? WHERE digest = ?", (token + b"damaged", token_id))
    observer = invoke(root, replacement, "app", "stage")
    assert observer["result"]["storage_reads"] == leaves + 4
    assert observer["selected"] == changed_root and observer["pending"] is None
    no_store = base / "no-store"
    invoke(root, no_store, "app", "submit-selected", *suffix, failure="persisted source node hash mismatch")
    assert not no_store.exists()
    file.write_text(original)
    invoke(root, replacement, "app", "stage", failure="persisted source node hash mismatch")
    state = invoke(root, replacement, "app", "state")
    assert state["selected"] == changed_root and state["pending"] is None
    with sqlite3.connect(nodes) as conn:
        conn.execute("UPDATE nodes SET payload = ? WHERE digest = ?", (token, token_id))

    # Model bad local disk bytes in this owned disposable fixture only; they
    # must not silently act as old parser state or get sent to a fresh store.
    with sqlite3.connect(nodes) as conn:
        (old,) = conn.execute("SELECT payload FROM nodes WHERE digest = ?", (bytes.fromhex(changed_root),)).fetchone()
        conn.execute("UPDATE nodes SET payload = ? WHERE digest = ?", (old + b"damaged", bytes.fromhex(changed_root)))
    invoke(root, no_store, "app", "submit-selected", *suffix, failure="persisted source node hash mismatch")
    assert not no_store.exists()
    invoke(root, replacement, "app", "stage", failure="persisted source node hash mismatch")
    assert invoke(root, replacement, "app", "state")["selected"] == changed_root
    assert invoke(root, replacement, "second", "submit-selected", *suffix)["result"]["authored"] == original

    # A normal redirected cache layout is out of scope for the two-file
    # SQLite commit contract. Both paths are part of this disposable fixture.
    redirected = base / "redirected"
    redirected.mkdir()
    detached = base / "separate-cst"
    detached.mkdir()
    (redirected / ".wosy").mkdir()
    (redirected / ".wosy/cst").symlink_to(detached, target_is_directory=True)
    invoke(redirected, primary, "app", "state", failure="redirected local SQLite directory")
    assert not list(detached.iterdir()) and not (redirected / ".wosy/client.sqlite").exists()

    misplaced = base / "misplaced"
    (misplaced / ".wosy/cst").mkdir(parents=True)
    harmless = base / "unrelated-fixture-file"
    harmless.write_bytes(b"unchanged")
    (misplaced / ".wosy/client.sqlite-journal").symlink_to(harmless)
    invoke(misplaced, primary, "app", "state", failure="redirected local SQLite file")
    assert harmless.read_bytes() == b"unchanged" and not (misplaced / ".wosy/client.sqlite").exists()

    print("SQLite client recovery with exact pending and selected roots", {
        "client_sqlite_bytes": db.stat().st_size, "cst_sqlite_bytes": nodes.stat().st_size,
        "initial_nodes": initial["result"]["new_nodes"], "initial_node_bytes": initial["result"]["new_bytes"],
        "cold_stage_ms": round(initial["result"]["stage_ms"], 3), "warm_stage_ms": round(warm["result"]["stage_ms"], 3),
        "warm_cst_reads": warm["result"]["storage_reads"], "migration_cst_reads": migrated["result"]["storage_reads"],
        "migration_open_ms": round(migrated["open_ms"], 3),
        "migration_stage_ms": round(migrated["result"]["stage_ms"], 3),
        "post_migration_warm_stage_ms": round(migrated_warm["result"]["stage_ms"], 3),
        "edit_stage_ms": round(changed["result"]["stage_ms"], 3), "edit_new_nodes": changed["result"]["new_nodes"],
        "edit_new_bytes": changed["result"]["new_bytes"], "replay_ms": round(result["replay_ms"], 3),
        "restore_store_ms": round(result["publish_ms"], 3), "restore_process_ms": round(recovered["wall_ms"], 3),
        "peak_kib_cold": initial["peak_kib"], "peak_kib_warm": warm["peak_kib"], "peak_kib_restore": recovered["peak_kib"],
    })

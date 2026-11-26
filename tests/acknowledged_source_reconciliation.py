"""Restore exact acknowledged canonical source into a real empty replacement store."""
import json
from pathlib import Path
import subprocess
import sys
import tempfile


example = Path(sys.argv[1]).resolve()
manifest = (Path(__file__).parent /
            "fixtures/build/shared_stdlib_local_dependency/project/wosy.toml").read_text()
manifest = manifest.replace("categories = []", 'categories = ["WHY"]')


def request(*args, refusal=None):
    result = subprocess.run([example, *map(str, args)], text=True, capture_output=True, timeout=30)
    if refusal:
        assert result.returncode and refusal in result.stderr and not result.stdout, result.stderr
        return
    result.check_returncode()
    return json.loads(result.stdout)


with tempfile.TemporaryDirectory() as tmp:
    directory = Path(tmp).resolve()
    project, before_loss, replacement, session_replacement = (
        directory / entry for entry in ("project", "storage-before-loss", "replacement", "session-replacement")
    )
    (project / "src").mkdir(parents=True)
    (project / "stdlib/src").mkdir(parents=True)
    (project / "wosy.toml").write_text(manifest)
    (project / "stdlib/wosy.toml").write_text('[package]\nname="std"\nsource_root="src"\n')
    main, leaf = project / "src/main.w", project / "stdlib/src/value.w"
    original = '%%start\n# WHY: sessão anterior ☕\nnum = namespace std "value.w";\nu32 selected = num.count;\n%%end\n'
    main.write_text(original)
    leaf.write_text("%%start\nu32 count = 9;\n%%end\n")

    def state(store, mode, *args, target="app", refusal=None):
        return request(project, store, "--persistent-source", target, mode, *args, refusal=refusal)

    def client_bytes():
        return tuple((project / name).read_bytes() for name in (".wosy/client.sqlite", ".wosy/cst/nodes.sqlite"))

    pending = state(before_loss, "stage")
    root = pending["pending"]
    assert pending["selected"] is None
    assert not state(before_loss, "replay-acknowledged-for", root, project)["result"]["matches"]
    request(project, replacement, "--persistent-source", "app", "reconcile-acknowledged",
            refusal="no acknowledged source")
    assert not replacement.exists() and not before_loss.exists()
    accepted = state(before_loss, "submit-pending", "app", "src/main.w")
    assert accepted["selected"] == root and accepted["result"]["confirmed"] and accepted["result"]["ready"]
    original_typed = request(project, before_loss, root, "--typed-project", "dev")
    original_cst = request(project, before_loss, root, "app", "src/main.w")
    assert original_cst["text"] == original
    assert state(replacement, "replay-acknowledged-for", root, project)["result"]["matches"]
    assert not state(replacement, "replay-acknowledged-for", root, directory)["result"]["matches"]
    assert not state(replacement, "replay-acknowledged-for", root, project, target="different")["result"]["matches"]
    assert not replacement.exists()
    frozen_client = client_bytes()

    # Scope is acknowledged source, not live worktree. A fresh parser would
    # project different bytes now, while the client's root is unchanged.
    main.write_text(original.replace("sessão anterior ☕", "alteração nova 🚀").replace("num.count", "17"))
    known = state(before_loss, "reconcile-acknowledged-limited")["result"]
    assert known["kind"] == "known" and known["root"] == root and not known["client_replayed"]
    assert known["written"] == known["daemon_validated_files"] == 0
    assert client_bytes() == frozen_client

    # Recovery must restore normal lease access, without an alternate way to
    # dispatch diagnostics or reread the modified live source.
    expected = state(before_loss, "leased-build", "dev")["result"]["response"]
    state(session_replacement, "leased-build", "dev", refusal="needs reconciliation")
    state(session_replacement, "reconcile-open-build-limited", "dev", refusal="authored byte budget")
    state(session_replacement, "leased-build", "dev", refusal="needs reconciliation")
    recovered_session = state(session_replacement, "reconcile-open-build", "dev")["result"]
    assert recovered_session["recovery"]["validated_files"] == 2
    assert recovered_session["recovery"]["written"] == accepted["result"]["written"]
    assert recovered_session["recovery"]["conflicts"] == 0
    warm_session = state(session_replacement, "reconcile-open-build-limited", "dev")["result"]
    assert warm_session["recovery"] is None
    quota = state(session_replacement, "reconcile-open-build-quota", "dev")["result"]
    assert quota["recovery"] is None and quota["lease_quota_and_scope"]
    for output in (recovered_session, warm_session, quota):
        assert {key: value for key, value in output["response"].items() if key != "origin"} == {
            key: value for key, value in expected.items() if key != "origin"
        }
    assert client_bytes() == frozen_client

    # Actual new SurrealKV/CAS directory, no artificial socket or deletion.
    # The authored-byte budget refuses after provisional rows in its own txn.
    state(replacement, "reconcile-acknowledged-limited", refusal="authored byte budget")
    request(project, replacement, root, "app", "src/main.w", refusal="unknown retained snapshot")
    assert client_bytes() == frozen_client
    restored = state(replacement, "reconcile-acknowledged")["result"]
    assert restored["kind"] == "restored" and restored["root"] == root and restored["client_replayed"]
    assert restored["client_parsed"] == restored["conflicts"] == 0
    assert restored["daemon_validated_files"] == 2
    assert restored["written"] == accepted["result"]["written"] > 0  # no stranded intern records after refusal
    assert restored["namespace_derived"] == accepted["result"]["namespace_derived"] == 2
    assert client_bytes() == frozen_client

    restarted = state(replacement, "reconcile-acknowledged")["result"]
    assert restarted["kind"] == "known" and restarted["root"] == root and not restarted["client_replayed"]
    assert restarted["written"] == restarted["daemon_validated_files"] == 0
    new_typed = request(project, replacement, root, "--typed-project", "dev")
    for key in ("snapshot", "project_ast", "prepared_identity", "layout", "modules", "ready", "diagnostics"):
        assert new_typed[key] == original_typed[key], key
    new_cst = request(project, replacement, root, "app", "src/main.w")
    for key in ("cst", "comments", "text"):
        assert new_cst[key] == original_cst[key], key
    assert client_bytes() == frozen_client

    fresh = state(replacement, "stage")
    assert fresh["pending"] != root and fresh["selected"] == root
    assert state(replacement, "replay-acknowledged-for", root, project)["result"]["matches"]
    assert state(replacement, "submit-pending", "app", "src/main.w")["selected"] == fresh["pending"]
    assert not state(replacement, "replay-acknowledged-for", root, project)["result"]["matches"]
    print(json.dumps({
        "require_replay_ms": restored["probe_ms"], "sqlite_replay_ms": restored["replay_ms"],
        "daemon_restore_ms": restored["recover_ms"], "reopened_known_probe_ms": restarted["probe_ms"],
        "validated_files": restored["daemon_validated_files"], "canonical_and_namespace_nodes_written": restored["written"],
        "old_source_preserved_despite_live_edit": True, "last_acknowledgement_boundary": True,
        "restore_and_open_ms": recovered_session["recovery"]["restore_and_open_ms"],
        "warm_lease_open_ms": warm_session["probe_ms"], "scoped_lease_quota": quota["lease_quota_and_scope"],
    }))

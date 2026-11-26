"""Compare acknowledged SQLite tooling bytes with actual optimized wosy parse."""
import json
from pathlib import Path
import subprocess
import sys
import tempfile


example, ordinary = (Path(argument).resolve() for argument in sys.argv[1:3])
manifest = (Path(__file__).parent /
            "fixtures/build/shared_stdlib_local_dependency/project/wosy.toml").read_text()
manifest = manifest.replace("categories = []", 'categories = ["WHY"]')


def entry(note):
    return f'%%start\n# WHY: {note}\nnum = namespace std "value.w";\nu32 result = num.answer;\n%%end\n'


with tempfile.TemporaryDirectory() as temporary:
    root = Path(temporary).resolve()
    project, store = root / "project", root / "catalog"
    (project / "src").mkdir(parents=True)
    (project / "stdlib/src").mkdir(parents=True)
    (project / "wosy.toml").write_text(manifest)
    (project / "stdlib/wosy.toml").write_text('[package]\nname="std"\nsource_root="src"\n')
    (project / "stdlib/src/value.w").write_text("%%start\nu32 answer = 42;\n%%end\n")
    authored = project / "src/main.w"
    public = project / ".wosy/cst/current.json"
    first, second = entry("antes ☕"), entry("mais detalhes sobre a revisão 🚀")
    authored.write_text(first)

    def stage(*arguments, error=None):
        child = subprocess.run([example, project, store, "--persistent-source", "app", *map(str, arguments)],
                               capture_output=True, timeout=30)
        if error:
            assert child.returncode and not child.stdout and error in child.stderr, child.stderr
            return
        assert child.returncode == 0, child.stderr
        return json.loads(child.stdout)

    def parse():
        child = subprocess.run([ordinary, "parse", "app"], cwd=project, capture_output=True, timeout=30)
        assert child.returncode == 0 and not child.stdout and not child.stderr, child.stderr
        return public.read_bytes()

    def current(output=1 << 20, encoded=2 << 20, authored_bytes=1 << 20, restage=None, error=None):
        action = "current-entry-cst" if restage is None else "current-entry-cst-restage"
        suffix = [] if restage is None else [restage]
        answer = stage(action, output, encoded, authored_bytes, *suffix, error=error)
        if not error:
            assert answer["result"]["root"] == answer["selected"]
            return answer["result"]

    expected_a = parse()
    a = stage("stage")
    assert a["result"]["parsed"] == 2
    assert stage("submit-pending", "app", "src/main.w")["selected"] == a["pending"]
    got_a = current(output=len(expected_a))
    assert got_a["json"].encode() == expected_a
    assert got_a["generation"] == a["attempt"]
    assert json.loads(expected_a)["comments"][0]["raw_payload"] == " antes ☕"
    assert not json.loads(expected_a)["diagnostics"]

    authored.write_text(second)
    b = stage("stage")
    assert (b["result"]["parsed"], b["result"]["reused"]) == (1, 1)
    assert b["selected"] == a["pending"] and b["pending"] != a["pending"]
    expected_b = parse()
    saved = authored.with_suffix(".fixture")
    authored.rename(saved)  # Only the rooted SQLite source can now supply the entry.
    before = stage("state")
    pending_a = current()
    assert pending_a["json"].encode() == expected_a and pending_a["generation"] == got_a["generation"]
    current(output=len(expected_a) - 1, error=b"JSON byte budget")
    current(output=1, error=b"JSON byte budget")
    current(encoded=1, error=b"expanded record read budget")
    current(authored_bytes=1, error=b"authored byte budget")
    after = stage("state")
    assert [(before[key], after[key]) for key in ("selected", "pending", "attempt")] == [
        (a["pending"], a["pending"]), (b["pending"], b["pending"]), (b["attempt"], b["attempt"])]
    assert public.read_bytes() == expected_b
    saved.rename(authored)

    assert stage("submit-pending", "app", "src/main.w")["selected"] == b["pending"]
    got_b = current()
    assert got_b["json"].encode() == expected_b and got_b["generation"] == b["attempt"]
    assert json.loads(expected_b)["comments"][0]["raw_payload"] == " mais detalhes sobre a revisão 🚀"
    impact = subprocess.run([example, project, store, a["pending"], "--source-impact", b["pending"]],
                            capture_output=True, check=True, timeout=30)
    result = json.loads(impact.stdout)
    assert result["annotation_tree_changed"] and not result["executable_tree_changed"]

    fresh = current(restage=first)
    assert fresh["root"] == got_b["root"] and fresh["generation"] > fresh["prior_generation"] == got_b["generation"]
    assert fresh["json"].encode() == expected_b and public.read_bytes() == expected_b
    assert not (project / ".wosy/artifacts").exists()
    print(json.dumps({"old_json_bytes": len(expected_a), "new_json_bytes": len(expected_b),
                      "old_view_ms": got_a["view_ms"], "new_view_ms": got_b["view_ms"],
                      "exact_ordinary_bytes": True, "read_without_live_entry": True,
                      "same_root_old_generation_refused": True, "current_file_not_written": True,
                      "executable_tree_unchanged": True}))

"""Compare acknowledged SQLite tooling bytes with actual optimized wosy parse."""
import json
from pathlib import Path
import subprocess
import sys
import tempfile


example, ordinary = (Path(argument).resolve() for argument in sys.argv[1:3])
seed_writer = Path(sys.argv[3]).resolve() if len(sys.argv) > 3 else example
manifest = (Path(__file__).parent /
            "fixtures/build/shared_stdlib_local_dependency/project/wosy.toml").read_text()
manifest = manifest.replace("categories = []", 'categories = ["WHY"]')


def entry(note):
    return f'%%start\n# WHY: {note}\nnum = namespace std "value.w";\nu32 result = num.answer;\n%%end\n'


def write_project(project, text):
    (project / "src").mkdir(parents=True)
    (project / "stdlib/src").mkdir(parents=True)
    (project / "wosy.toml").write_text(manifest)
    (project / "stdlib/wosy.toml").write_text('[package]\nname="std"\nsource_root="src"\n')
    (project / "stdlib/src/value.w").write_text("%%start\nu32 answer = 42;\n%%end\n")
    (project / "src/main.w").write_text(text)


with tempfile.TemporaryDirectory() as temporary:
    root = Path(temporary).resolve()
    project, store, other = root / "project", root / "catalog", root / "other"
    authored = project / "src/main.w"
    public = project / ".wosy/cst/current.json"
    first, second = entry("antes ☕"), entry("mais detalhes sobre a revisão 🚀")
    write_project(project, first)

    def stage(*arguments, error=None, workspace=project, catalog=store, binary=example, target="app"):
        child = subprocess.run([binary, workspace, catalog, "--persistent-source", target, *map(str, arguments)],
                               capture_output=True, timeout=30)
        if error:
            assert child.returncode and not child.stdout and error in child.stderr, child.stderr
            return
        assert child.returncode == 0, child.stderr
        return json.loads(child.stdout)

    def parse(workspace=project):
        child = subprocess.run([ordinary, "parse", "app"], cwd=workspace, capture_output=True, timeout=30)
        assert child.returncode == 0 and not child.stdout and not child.stderr, child.stderr
        return (workspace / ".wosy/cst/current.json").read_bytes()

    def current(output=1 << 20, encoded=2 << 20, authored_bytes=1 << 20, restage=None, error=None):
        action = "current-entry-cst" if restage is None else "current-entry-cst-restage"
        suffix = [] if restage is None else [restage]
        answer = stage(action, output, encoded, authored_bytes, *suffix, error=error)
        if not error:
            assert answer["result"]["root"] == answer["selected"]
            return answer["result"]

    expected_a = parse()
    a = stage("stage", binary=seed_writer)
    assert a["result"]["parsed"] == 2
    assert stage("submit-pending", "app", "src/main.w", binary=seed_writer)["selected"] == a["pending"]
    name = "authored/Δ"
    pin_a = stage("pin-local", name, binary=seed_writer)["result"]
    assert pin_a == {"name": name, "target": "app", "root": a["pending"], "generation": 1}
    no_change = stage("stage")
    assert no_change["result"]["root"] == a["pending"] and no_change["pending"] is None
    assert (no_change["result"]["parsed"], no_change["result"]["reused"]) == (0, 2)
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

    unused = root / "never-opened-catalog"

    def pinned(pin=name, output=1 << 20, encoded=2 << 20, authored_bytes=1 << 20, **kwargs):
        return stage("pinned-entry-cst", pin, output, encoded, authored_bytes, catalog=unused, **kwargs)

    # A new process opens this exact former root solely through its local
    # pin even when the new source is acknowledged and the file isn't live.
    authored.rename(saved)
    dbs = (project / ".wosy/client.sqlite", project / ".wosy/cst/nodes.sqlite")
    before = tuple(path.read_bytes() for path in dbs)
    historical = pinned(output=len(expected_a), target="not-the-pin-target")
    assert historical["result"]["pin"] == pin_a
    assert historical["result"]["json"].encode() == expected_a
    assert historical["selected"] is None
    assert pinned("unknown")["result"] is None
    pinned(output=len(expected_a) - 1, error=b"JSON byte budget")
    pinned(encoded=1, error=b"expanded record read budget")
    pinned(authored_bytes=1, error=b"authored byte budget")
    assert tuple(path.read_bytes() for path in dbs) == before
    assert public.read_bytes() == expected_b
    saved.rename(authored)

    write_project(other, entry("outra raiz 🌱"))
    other_expected = parse(other)
    other_root = stage("stage", workspace=other, binary=seed_writer)["pending"]
    assert stage("submit-pending", "app", "src/main.w", workspace=other, binary=seed_writer)["selected"] == other_root
    other_pin = stage("pin-local", name, workspace=other, binary=seed_writer)["result"]
    assert other_pin["root"] != pin_a["root"]
    other_view = pinned(workspace=other)["result"]
    assert other_view["pin"] == other_pin and other_view["json"].encode() == other_expected
    assert pinned()["result"]["pin"] == pin_a

    assert stage("unpin-local", name, pin_a["root"], pin_a["generation"])["result"]["released"]
    assert pinned()["result"] is None
    pin_b = stage("pin-local", name)["result"]
    assert pin_b["root"] == b["pending"] and pin_b["generation"] > pin_a["generation"]
    assert not stage("unpin-local", name, pin_a["root"], pin_a["generation"])["result"]["released"]
    replacement = pinned()["result"]
    assert replacement["pin"] == pin_b and replacement["json"].encode() == expected_b
    assert not unused.exists()

    fresh = current(restage=first)
    assert fresh["root"] == got_b["root"] and fresh["generation"] > fresh["prior_generation"] == got_b["generation"]
    assert fresh["json"].encode() == expected_b and public.read_bytes() == expected_b
    assert not (project / ".wosy/artifacts").exists()
    print(json.dumps({"old_json_bytes": len(expected_a), "new_json_bytes": len(expected_b),
                      "old_view_ms": got_a["view_ms"], "new_view_ms": got_b["view_ms"],
                      "exact_ordinary_bytes": True, "read_without_live_entry": True,
                      "same_root_old_generation_refused": True, "current_file_not_written": True,
                      "executable_tree_unchanged": True, "pinned_json_exact": True,
                      "pinned_scope_and_generation": True, "separate_seed_writer": seed_writer != example}))

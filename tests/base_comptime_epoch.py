"""Seed real V3 artifacts, then verify V4 with the same build target."""
import json
from pathlib import Path
import subprocess
import sys


mode, root_arg, example_arg, cli_arg = sys.argv[1:]
root, example, cli = (Path(arg).resolve() for arg in (root_arg, example_arg, cli_arg))
project, store = root / "project", root / "catalog"
entry = project / "src/main.w"
public = project / ".wosy/cst/current.json"
pin_name = "base-v3/Δ"
saved_name = "v3-ordinary-build"
prior_source = '''%%start
# WHY: o mesmo código ☕
num = namespace std "value.w";
u32 result = num.answer;
%%end
'''


def command(program, *args, error=None):
    process = subprocess.run([program, *map(str, args)], cwd=project, capture_output=True, timeout=30)
    if error is not None:
        assert process.returncode and not process.stdout and error in process.stderr, process.stderr
        return
    assert process.returncode == 0 and not process.stderr, process.stderr
    return process.stdout


def action(*args, error=None):
    stdout = command(example, project, store, "--persistent-source", "app", *args, error=error)
    return json.loads(stdout) if error is None else None


def retained(scope, *roots):
    return json.loads(command(example, project, store, scope, "--source-plan", 20000, *roots))


def publication():
    assert command(cli, "parse", "app") == b""
    return public.read_bytes()


def submit():
    return action("submit-pending", "app", "src/main.w")


def state_metadata():
    state = action("state")
    return tuple(state[key] for key in ("selected", "pending", "attempt"))


if mode == "seed":
    (project / "src").mkdir(parents=True)
    (project / "stdlib/src").mkdir(parents=True)
    fixture = Path(__file__).parent / "fixtures/build/shared_stdlib_local_dependency/project/wosy.toml"
    (project / "wosy.toml").write_text(fixture.read_text().replace("categories = []", 'categories = ["WHY"]'))
    (project / "stdlib/wosy.toml").write_text('[package]\nname = "std"\nsource_root = "src"\n')
    (project / "stdlib/src/value.w").write_text("%%start\nu32 answer = 42;\n%%end\n")
    entry.write_text(prior_source)
    (root / "v3-entry.json").write_bytes(publication())
    first = action("stage")
    assert (first["result"]["parsed"], first["result"]["reused"]) == (2, 0)
    assert submit()["selected"] == first["pending"]
    pin = action("pin-local", pin_name)["result"]
    assert pin["root"] == first["pending"]
    (root / "v3-pin.json").write_text(json.dumps(pin))
    entry.write_text(prior_source.replace("num.answer;", "true;"))
    rejected = action("stage")["pending"]
    assert rejected != pin["root"] and submit()["selected"] == rejected
    report = action("save-build-diagnostic", saved_name, 64 << 10, "dev")["result"]
    assert report["report"]["diagnostics"] and report["rendered"]
    assert bytes(report["report"]["source_root"]).hex() == rejected
    (root / "v3-diagnostic.json").write_text(json.dumps(report))
    entry.write_text(prior_source)
    assert action("stage")["pending"] == pin["root"]
    assert submit()["selected"] == pin["root"]
elif mode == "verify":
    pin = json.loads((root / "v3-pin.json").read_bytes())
    old_json = (root / "v3-entry.json").read_bytes()
    assert publication() == old_json
    assert state_metadata() == (pin["root"], None, None)
    # No historical source may be promoted into the active grammar.
    action("submit-selected", "app", "src/main.w", error=b"unsupported persisted source schema")
    action("read-saved-build-diagnostic", saved_name, 64 << 10, error=b"unsupported acknowledged source schema")
    action("stage-limited", 1, 1, error=b"LimitExceeded { field: EncodedBytes, limit: 1 }")
    assert state_metadata() == (pin["root"], None, None)
    staged = action("stage")
    assert staged["selected"] == pin["root"] and staged["pending"] != pin["root"]
    stats = staged["result"]
    assert (stats["parsed"], stats["reused"], stats["storage_reads"]) == (2, 0, 2)
    assert stats["requires_complete_upload"]
    action("proposal", staged["pending"], staged["attempt"], error=b"unsupported persisted source schema")
    assert state_metadata() == (pin["root"], staged["pending"], staged["attempt"])
    assert action("abandon", staged["pending"], staged["attempt"])["result"]["abandoned"]
    retry = action("stage")
    assert retry["selected"] == pin["root"] and retry["pending"] == staged["pending"]
    assert retry["attempt"] > staged["attempt"]
    assert not action("delayed-ack", staged["pending"], staged["attempt"])["result"]["confirmed"]
    acknowledged = submit()
    assert acknowledged["selected"] == retry["pending"] and acknowledged["pending"] is None
    assert acknowledged["result"]["authored"] == prior_source
    assert acknowledged["result"]["confirmed"] and acknowledged["result"]["ready"]
    warm = action("stage")["result"]
    assert (warm["parsed"], warm["reused"], warm["new_nodes"]) == (0, 2, 0)
    assert not warm["requires_complete_upload"]
    assert action("current-entry-cst", 1 << 20, 2 << 20, 1 << 20)["result"]["json"].encode() == old_json
    # Archived authored view needs neither the live file nor the old parser.
    entry.rename(entry.with_suffix(".fixture"))
    databases = project / ".wosy/client.sqlite", project / ".wosy/cst/nodes.sqlite"
    before = tuple(path.read_bytes() for path in databases)
    historical = action("pinned-entry-cst", pin_name, 1 << 20, 2 << 20, 1 << 20)["result"]
    assert historical["pin"] == pin and historical["json"].encode() == old_json
    saved = json.loads((root / "v3-diagnostic.json").read_bytes())
    archived = action("read-saved-build-diagnostic", saved_name, 64 << 10)["result"]
    assert (archived["report"], archived["rendered"]) == (saved["report"], saved["rendered"])
    archive_root = bytes(saved["report"]["source_root"]).hex()
    census = retained(retry["pending"], pin["root"], archive_root)
    assert (census["protected_roots"], census["candidate_roots"]) == (2, 1)
    assert census["nominated_roots"] == 2 and census["shared_nodes"] > 0
    assert retained(archive_root, pin["root"])["candidate_roots"] == 1
    assert tuple(path.read_bytes() for path in databases) == before and public.read_bytes() == old_json
    # Only Parse tooling recognizes the new authored construct. Build must
    # name its unsupported compile-time phase before invoking any builder.
    entry.write_text("%%start\n# WHY: referência ☕\nu32 result = comptime measured { 42 };\n%%end\n")
    assert b'"ComptimeExpr"' in publication()
    command(cli, "build", "app", "--profile", "dev", error=b"requires specialization")
    assert not (project / ".wosy/artifacts").exists()
    assert state_metadata() == (retry["pending"], None, None)
    print(json.dumps({
        "v3_root": pin["root"], "v4_root": retry["pending"],
        "historical_authored_json_bytes": len(old_json),
        "metadata_reads": stats["storage_reads"], "upgrade_parsed": stats["parsed"],
        "warm_reused": warm["reused"], "v3_active_and_sparse_refused": True,
        "old_generation_refused": True, "build_requires_specialization": True,
        "v3_named_report_rendered_exact": True,
        "mixed_census": {key: census[key] for key in ("protected_roots", "candidate_roots", "nodes_visited", "shared_nodes")},
    }))
else:
    raise ValueError("expected seed or verify")

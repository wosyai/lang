"""Check the rooted opt-in compiler preparation in optimized real processes."""
import json
import os
from pathlib import Path
from statistics import median
import subprocess
import sys
import tempfile
import time


binary = Path(sys.argv[1]).resolve()
template = Path(__file__).parent / "fixtures/build/shared_stdlib_local_dependency/project/wosy.toml"
manifest = template.read_text().replace("categories = []", 'categories = ["WHY"]') + """
[builders.llvm-native]
command = ["python", "scripts/build-native"]
args = []
toolchain_identity = "llvm-18.native"

[runners.native]
command = ["python", "scripts/run-native"]
args = []

[targets.app.artifacts.main.profiles.native]
backend = "llvm"
output = "native"
builder = "llvm-native"
run_runner = "native"

[targets.app.profiles.native]
main_artifact = "main"
"""


def run(*args):
    with tempfile.TemporaryFile() as stdout:
        start = time.perf_counter()
        child = subprocess.Popen([binary, *args], stdout=stdout)
        _, status, usage = os.wait4(child.pid, 0)
        child.returncode = os.waitstatus_to_exitcode(status)
        if child.returncode:
            raise subprocess.CalledProcessError(child.returncode, child.args)
        elapsed = (time.perf_counter() - start) * 1000
        stdout.seek(0)
        return json.load(stdout), elapsed, usage.ru_maxrss


def create(root):
    (root / "src").mkdir(parents=True)
    (root / "stdlib/src").mkdir(parents=True)
    (root / "wosy.toml").write_text(manifest)
    (root / "stdlib/wosy.toml").write_text('[package]\nname="std"\nsource_root="src"\n')
    return root / "src/main.w", root / "stdlib/src"


def source(body, why=None):
    comment = "" if why is None else "# WHY: " + why + "\n"
    return "%%start\n" + comment + body + "%%end\n"


def snapshot(root, store, prior=None):
    return run(root, store, *([prior] if prior else []))[0]["snapshot"]["source_root"]


def prepare(root, store, revision, profile, *args):
    return run(root, store, revision, "--typed-project", profile, *args)


def sizes(store):
    files = [(p, p.stat().st_size) for p in store.rglob("*") if p.is_file()]
    return [sum(n for p, n in files if (p.parent.name == "cas") == cas) for cas in (False, True)]


with tempfile.TemporaryDirectory() as work:
    root, store = Path(work) / "app", Path(work) / "store"
    main, deps = create(root)
    provider = (
        "struct Pair { *?u8 address; u32 count; }\n"
        "struct copy Packet { u32 count; }\n"
        "u32(Packet) read = fn(packet) { packet.count };\n"
    )
    left, right = deps / "left.w", deps / "right.w"
    left.write_text(source(provider, "same source"))
    right.write_text(source(provider, "same source"))
    imports = ('one = namespace std "left.w";\n'
               'two = namespace std "right.w";\n')
    body = imports + (
        "one.Packet one_packet = { .count = 17; };\n"
        "two.Packet two_packet = { .count = 19; };\n"
        "u32 from_one = one.read(one_packet);\n"
        "u32 from_two = two.read(two_packet);\n"
    )
    main.write_text(source(body, "primeiro ☕"))
    first = snapshot(root, store)
    wasm, _, _ = prepare(root, store, first, "dev", "--oracle")
    native, _, _ = prepare(root, store, first, "native", "--oracle")
    assert wasm["ready"] and native["ready"]
    assert wasm["layout"] == {"pointer_size": 4, "pointer_alignment": 4}
    assert native["layout"] == {"pointer_size": 8, "pointer_alignment": 8}
    assert wasm["project_ast"] == native["project_ast"]
    assert wasm["prepared_identity"] != native["prepared_identity"]
    for reply, size, alignment in ((wasm, 8, 4), (native, 16, 8)):
        app, left_module, right_module = reply["modules"]
        left_owner, right_owner = left_module["owner"], right_module["owner"]
        assert left_owner["revision"] == right_owner["revision"] and left_owner != right_owner
        assert app["members"]["one_packet"] == left_owner
        assert app["members"]["two_packet"] == right_owner
        for provider_module in (left_module, right_module):
            pair, packet = provider_module["structs"]
            assert pair["layout"] == {"size": size, "alignment": alignment}
            assert packet["copy_policy"] == "Copy"
            assert pair["owner"] == packet["owner"] == provider_module["owner"]
            assert all(owner == provider_module["owner"] for owner in pair["field_owners"])
    stable_size = sizes(store)
    pair, _, _ = prepare(root, store, first, "native", "--held", first, "dev")
    assert not any(reply["reused_live"] for reply in pair)
    same, _, _ = prepare(root, store, first, "dev", "--held")
    assert sum(reply["reused_live"] for reply in same) == 1
    released, _, _ = prepare(root, store, first, "dev", "--released")
    assert not any(reply["reused_live"] for reply in released)
    assert sizes(store) == stable_size

    main.write_text(source(body, "comentário revisado, mais longo 🚀🚀"))
    annotated = snapshot(root, store, first)
    anno, _, _ = prepare(root, store, annotated, "dev", "--oracle")
    assert annotated != first and anno["prepared_identity"] == wasm["prepared_identity"]
    assert anno["modules"][0]["owner"] == wasm["modules"][0]["owner"]
    assert anno["modules"][0]["author"] != wasm["modules"][0]["author"]
    views, _, _ = prepare(root, store, first, "dev", "--held", annotated)
    assert views[0]["prepared_identity"] == views[1]["prepared_identity"]
    assert views[0]["modules"][0]["author"] != views[1]["modules"][0]["author"]
    assert sum(reply["reused_live"] for reply in views) == 1

    # A rooted configuration change must not use the live old typed handle,
    # even when the layout and complete executable AST are the same.
    (root / "wosy.toml").write_text(manifest.replace("llvm-18.native", "llvm-18.revised"))
    changed_config = snapshot(root, store, annotated)
    configs, _, _ = prepare(root, store, annotated, "dev", "--held", changed_config)
    assert configs[0]["project_ast"] == configs[1]["project_ast"]
    assert configs[0]["layout"] == configs[1]["layout"]
    assert configs[0]["prepared_identity"] != configs[1]["prepared_identity"]
    assert not any(result["reused_live"] for result in configs)

    # Selection errors do not trigger new compiler projections or writes.
    prior_size = sizes(store)
    absent = subprocess.run(
        [binary, root, store, changed_config, "--typed-project", "not-a-profile"],
        capture_output=True, text=True,
    )
    assert absent.returncode != 0 and "not present in the rooted source configuration" in absent.stderr
    assert sizes(store) == prior_size

    broken_body = imports + "i32 requested = one.unlisted;\n"
    main.write_text(source(broken_body, "detalhe ☕"))
    error_a = snapshot(root, store, changed_config)
    broken, _, _ = prepare(root, store, error_a, "dev", "--oracle")
    assert not broken["ready"]
    member = next(d for d in broken["diagnostics"] if d["code"] == "M0002")
    assert len(member["labels"]) == 2
    assert [label["span"]["source"]["path"] for label in member["labels"]] == [
        "src/main.w", "src/left.w",
    ]
    span = member["labels"][0]["span"]["range"]
    assert main.read_bytes()[span["start"]:span["end"]] == b"unlisted"
    main.write_text(source(broken_body, "detalhe muito mais longo 🚀🚀"))
    left.write_text(source(provider, "alteração só de comentário ☕"))
    error_b = snapshot(root, store, error_a)
    later, _, _ = prepare(root, store, error_b, "dev", "--oracle")
    same_error, _, _ = prepare(root, store, error_a, "dev", "--held", error_b)
    assert broken["prepared_identity"] == later["prepared_identity"]
    assert same_error[0]["diagnostics"] == broken["diagnostics"]
    assert same_error[1]["diagnostics"] == later["diagnostics"]
    assert sum(reply["reused_live"] for reply in same_error) == 1
    second = next(d for d in later["diagnostics"] if d["code"] == "M0002")["labels"]
    assert second[0]["span"]["range"] != span
    assert second[1]["span"]["source"]["revision"] != member["labels"][1]["span"]["source"]["revision"]
    print("typed identities, Native64/Wasm32 layout/Copy, qualified owners/calls, UTF-8 labels, weak reuse: PASS",
          flush=True)

    samples = []
    for _ in range(3):
        with tempfile.TemporaryDirectory(dir=work) as current:
            workspace, database = Path(current) / "bulk", Path(current) / "store"
            main, deps = create(workspace)
            main.write_text(source("".join(f'm{j} = namespace std "body{j:02}.w";\n' for j in range(24))))
            for j in range(24):
                (deps / f"body{j:02}.w").write_text(source(
                    "".join(f"u32 item{i} = {j * 144 + i};\n" for i in range(144))
                ))
            revision = snapshot(workspace, database)
            cached_ast, _, _ = run(workspace, database, revision, "--project-declarations")
            assert cached_ast["bodies_derived"] == 25
            before = sizes(database)
            reopened, elapsed, rss = prepare(workspace, database, revision, "dev")
            concurrent, concurrent_wall, concurrent_rss = prepare(workspace, database, revision, "dev", "--held")
            assert reopened["ready"] and reopened["bodies_derived"] == 0
            assert all(reply["ready"] for reply in concurrent)
            reused = next(reply for reply in concurrent if reply["reused_live"])
            assert reused["compiler_ms"] == reused["materialized_ms"] == 0
            after = sizes(database)
            samples.append({
                "projected_ms": reopened["projected_ms"], "materialized_ms": reopened["materialized_ms"],
                "compiler_ms": reopened["compiler_ms"], "query_ms": reopened["query_ms"],
                "wall_ms": elapsed, "rss_kib": rss, "in_process_hit_projection_ms": reused["projected_ms"],
                "concurrent_wall_ms": concurrent_wall, "concurrent_rss_kib": concurrent_rss,
                "extra_kv_bytes": after[0] - before[0], "extra_cas_bytes": after[1] - before[1],
            })
    print("25 modules / 3,456 bindings with persisted AST median", {
        key: round(median(row[key] for row in samples), 3) for key in samples[0]
    }, flush=True)

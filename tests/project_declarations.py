"""Exercise the real rooted declaration DAG in separately reopened release processes."""
from pathlib import Path
from statistics import median
import json
import os
import subprocess
import sys
import tempfile
import time


binary = Path(sys.argv[1]).resolve()
fixture = Path(__file__).parent / "fixtures/build/shared_stdlib_local_dependency/project/wosy.toml"
manifest = fixture.read_text().replace("categories = []", 'categories = ["WHY"]')


def run(*args):
    with tempfile.TemporaryFile() as stdout:
        start = time.perf_counter()
        child = subprocess.Popen([binary, *args], stdout=stdout)
        _, status, usage = os.wait4(child.pid, 0)
        child.returncode = os.waitstatus_to_exitcode(status)
        if child.returncode:
            raise subprocess.CalledProcessError(child.returncode, child.args)
        wall_ms = (time.perf_counter() - start) * 1000
        stdout.seek(0)
        return json.load(stdout), wall_ms, usage.ru_maxrss


def create_project(root):
    (root / "src").mkdir(parents=True)
    (root / "stdlib/src").mkdir(parents=True)
    (root / "wosy.toml").write_text(manifest)
    (root / "stdlib/wosy.toml").write_text('[package]\nname="std"\nsource_root="src"\n')
    return root / "src/main.w", root / "stdlib/src"


def source(body, comment=None):
    context = "" if comment is None else "# WHY: " + comment + "\n"
    return "%%start\n" + context + body + "%%end\n"


def snapshot(root, store, prior=None):
    args = [prior] if prior is not None else []
    return run(root, store, *args)[0]["snapshot"]["source_root"]


def projection(root, store, revision, *args):
    return run(root, store, revision, "--project-declarations", *args)


def counts(view):
    return (view["modules_derived"], view["modules_reused"], view["bodies_derived"])


def bypath(view):
    return {mod["key"]["path"]: mod for mod in view["modules"]}


def stats(store):
    paths = [(p, p.stat().st_size) for p in store.rglob("*") if p.is_file()]
    return (sum(size for p, size in paths if p.parent != store / "cas"),
            sum(size for p, size in paths if p.parent == store / "cas"))


with tempfile.TemporaryDirectory() as work:
    root, store = Path(work) / "diamond", Path(work) / "disk"
    main, dep = create_project(root)
    main_body = ('first = namespace std "z.w";\n'
                 'second = namespace std "a.w";\n'
                 'sibling = namespace std "sibling.w";\n')
    main.write_text(source(main_body, "initial ☕"))
    (dep / "z.w").write_text(source('common = namespace std "leaf.w";\n'
                                    'other = namespace std "leaf.w";\n'))
    (dep / "a.w").write_text(source('common = namespace std "leaf.w";\n'))
    leaf = dep / "leaf.w"
    # Both the small and the CAS payload branches are exercised in one graph.
    leaf_body = "".join(f"u32 signal{i} = {i};\n" for i in range(190))
    leaf.write_text(source(leaf_body))
    (dep / "sibling.w").write_text(source("u32 alone = 99;\n"))

    initial = snapshot(root, store)
    annotated = snapshot(root, store, initial)
    assert initial == annotated
    # Materialize a single module before the project. The project must share
    # this AST with the existing independently rooted query, not derive again.
    standalone, _, _ = run(root, store, initial, "std", "src/sibling.w", "--declarations")
    before = stats(store)
    cold, time_cold, rss_cold = projection(root, store, initial, "--oracle")
    after = stats(store)
    assert counts(cold) == (5, 0, 4)
    assert cold["modules"][4]["body"] == standalone["declarations"]
    assert list(bypath(cold)) == [
        "src/main.w", "src/z.w", "src/leaf.w", "src/a.w", "src/sibling.w",
    ]
    assert [m["target"] for m in cold["modules"][0]["imports"]] == [1, 3, 4]
    assert [m["target"] for m in cold["modules"][1]["imports"]] == [2, 2]
    assert [m["authored"]["binding"] for m in cold["modules"][1]["imports"]] == ["common", "other"]
    assert cold["materialized"]["bodies_derived"] == 0
    warm, _, _ = projection(root, store, initial)
    assert counts(warm) == (0, 5, 0)
    assert warm["projection"] == cold["projection"] and warm["modules"] == cold["modules"]

    main.write_text(source(main_body, "mudança de contexto com acentos e foguetes 🚀🚀"))
    annotation = snapshot(root, store, initial)
    annotated, _, _ = projection(root, store, annotation, "--oracle")
    assert annotation != initial and annotated["projection"] == cold["projection"]
    assert counts(annotated) == (0, 5, 0)
    old_entry, new_entry = cold["modules"][0], annotated["modules"][0]
    assert old_entry["source"]["revision"] != new_entry["source"]["revision"]
    assert old_entry["body"] == new_entry["body"]
    assert old_entry["imports"][0]["authored"]["span"] != new_entry["imports"][0]["authored"]["span"]
    text = main.read_bytes()
    span = new_entry["imports"][0]["authored"]["span"]
    assert text[span["start"]:span["end"]] == b'first = namespace std "z.w";'

    leaf.write_text(source(leaf_body.replace("signal0 = 0;", "signal0 = 101;")))
    changed_root = snapshot(root, store, annotation)
    edited, _, _ = projection(root, store, changed_root, "--oracle")
    assert edited["projection"] != annotated["projection"] and counts(edited) == (4, 1, 1)
    for name in ("src/main.w", "src/z.w", "src/a.w"):
        assert bypath(edited)[name]["body"] == bypath(annotated)[name]["body"]
        assert bypath(edited)[name]["projection"] != bypath(annotated)[name]["projection"]
    assert bypath(edited)["src/sibling.w"] == bypath(annotated)["src/sibling.w"]
    assert bypath(edited)["src/leaf.w"]["body"] != bypath(annotated)["src/leaf.w"]["body"]
    old, _, _ = projection(root, store, initial, "--load")
    assert counts(old) == (0, 5, 0)
    assert old["projection"] == cold["projection"] and old["modules"] == cold["modules"]
    assert old["materialized"]["items"] == cold["materialized"]["items"]

    # Recover only an omitted, owned, recomputable CAS body, lazily on demand.
    blobs = list((store / "cas").iterdir())
    assert len(blobs) == 2
    for path in blobs:
        path.unlink()
    manifest_only, _, _ = projection(root, store, initial)
    assert counts(manifest_only) == (0, 5, 0)
    assert manifest_only["projection"] == cold["projection"]
    assert not list((store / "cas").iterdir())
    recovered, _, _ = projection(root, store, initial, "--load")
    assert recovered["projection"] == cold["projection"] and recovered["modules"] == cold["modules"]
    assert recovered["materialized"]["bodies_derived"] == 1 and len(list((store / "cas").iterdir())) == 1

    # Two cold revisions can share independently validated projections. Every
    # equal body / contextual node appears once across both publications.
    race_store = Path(work) / "race"
    r0 = snapshot(root, race_store)
    leaf.write_text(source(leaf_body))
    r1 = snapshot(root, race_store, r0)
    reports, _, _ = projection(root, race_store, r0, "--concurrent", r1)
    assert sum(view["modules_derived"] for view in reports) == 9
    assert sum(view["bodies_derived"] for view in reports) == 6
    assert all(counts(projection(root, race_store, revision)[0]) == (0, 5, 0)
               for revision in (r0, r1))
    print("ordered diamond, duplicate alias, ordinary compiler oracle, old roots, UTF-8 authored spans:",
          {"cold": counts(cold), "warm": counts(warm), "edit": counts(edited),
           "annotation": counts(annotated), "concurrent": [counts(v) for v in reports],
           "disk_delta_before_prefetched_body": [right-left for left, right in zip(before, after)],
           "cold_including_oracle_wall_ms": round(time_cold, 2), "cold_peak_kib": rss_cold},
          flush=True)

    # Larger fixture: prebuild the source graph but not the project projection,
    # distinguish inline/query writes in KV from large AST files in CAS, and
    # measure cold/warm metadata separately from loading 6,144 real declarations.
    benchmark_rows = []
    for n in range(3):
        with tempfile.TemporaryDirectory(dir=work) as sample:
            bulk, bulk_store = Path(sample) / "bulk", Path(sample) / "store"
            main, dep = create_project(bulk)
            main.write_text(source("".join(f'n{j} = namespace std "body{j:02}.w";\n' for j in range(32))))
            for j in range(32):
                (dep / f"body{j:02}.w").write_text(source(
                    "".join(f"u32 entry{i} = {j * 192 + i};\n" for i in range(192))
                ))
            revision = snapshot(bulk, bulk_store)
            before = stats(bulk_store)
            cold, cold_wall, cold_rss = projection(bulk, bulk_store, revision)
            after = stats(bulk_store)
            warm, warm_wall, warm_rss = projection(bulk, bulk_store, revision)
            loaded, load_wall, load_rss = projection(bulk, bulk_store, revision, "--load")
            assert counts(cold) == (33, 0, 33) and counts(warm) == (0, 33, 0)
            assert loaded["materialized"]["items"] == 32 * 192 + 32
            assert loaded["materialized"]["bodies_derived"] == 0
            benchmark_rows.append({
                "cold_query_ms": cold["query_ms"], "warm_query_ms": warm["query_ms"],
                "cold_wall_ms": cold_wall, "warm_wall_ms": warm_wall, "materialize_ms": loaded["materialized"]["load_ms"],
                "materialized_wall_ms": load_wall, "cold_rss_kib": cold_rss, "warm_rss_kib": warm_rss,
                "materialized_rss_kib": load_rss, "original_store_bytes": before[0],
                "extra_kv_bytes": after[0] - before[0], "extra_cas_bytes": after[1] - before[1],
                "warm_disk_delta_bytes": sum(stats(bulk_store)) - sum(after),
            })
    print("33 modules / 6144 bindings median", {
        k: round(median(row[k] for row in benchmark_rows), 3) for k in benchmark_rows[0]
    }, flush=True)

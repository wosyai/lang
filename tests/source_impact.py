"""Compare real rooted source graphs while retaining historical inputs."""
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import time


binary = Path(sys.argv[1]).resolve()
manifest = (Path(__file__).parent /
            "fixtures/build/shared_stdlib_local_dependency/project/wosy.toml").read_text()
manifest = manifest.replace("categories = []", 'categories = ["WHY"]')


def invoke(*args, failure=None):
    start = time.perf_counter()
    with subprocess.Popen([binary, *args], stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True) as proc:
        stdout, stderr = proc.stdout.read(), proc.stderr.read()
        _, status, usage = os.wait4(proc.pid, 0)
        proc.returncode = os.waitstatus_to_exitcode(status)
    if failure is not None:
        assert proc.returncode and not stdout and failure.lower() in stderr.lower(), (stdout, stderr)
        return
    assert proc.returncode == 0 and not stderr, (proc.returncode, stderr)
    return json.loads(stdout), (time.perf_counter() - start) * 1000, usage.ru_maxrss


def project(root, source_manifest=manifest):
    (root / "src").mkdir(parents=True)
    (root / "stdlib/src").mkdir(parents=True)
    (root / "wosy.toml").write_text(source_manifest)
    (root / "stdlib/wosy.toml").write_text('[package]\nname="std"\nsource_root="src"\n')
    return root / "src/main.w", root / "stdlib/src"


def write(file, body, why=None):
    comments = "" if why is None else "# WHY: " + why + "\n"
    file.write_text("%%start\n" + comments + body + "%%end\n")


def publish(root, store, prior=None):
    return invoke(root, store, *([prior] if prior else []))[0]["snapshot"]["source_root"]


def impact(root, store, before, after, *limits, **kwargs):
    return invoke(root, store, before, "--source-impact", after, *limits, **kwargs)


def files(report):
    return [diff["file"]["path"] for diff in report["files"]]


def paths(report, side):
    return [file["path"] for file in report[side]]


with tempfile.TemporaryDirectory() as work:
    directory = Path(work)
    app, foreign, remap, store = (directory / name for name in ("app", "foreign", "remap", "store"))
    main, dep = project(app)
    shared, other_dep = project(foreign)
    entry = ('first = namespace std "z.w";\n'
             'second = namespace std "a.w";\n'
             'sibling = namespace std "sibling.w";\n')
    left = 'common = namespace std "leaf.w";\nagain = namespace std "leaf.w";\n'
    right = 'common = namespace std "leaf.w";\n'
    body = "u32 signal = 17;\n"
    write(main, entry, "original ☕")
    # Published with distinct executable entry but the identical providers.
    # Its reverse records share the same child IDs as the selected roots.
    write(shared, entry + "u32 separate_workspace = 22;\n")
    for package in (dep, other_dep):
        write(package / "z.w", left)
        write(package / "a.w", right)
        write(package / "leaf.w", body)
        write(package / "sibling.w", "u32 unrelated = 99;\n")

    base = publish(app, store)
    outside = publish(foreign, store)
    assert outside != base
    write(main, entry, "só anotação 🚀🚀")
    comment = publish(app, store, base)
    note, _, _ = impact(app, store, base, comment)
    assert not note["configuration_changed"] and not note["executable_tree_changed"] and note["annotation_tree_changed"]
    assert files(note) == ["src/main.w"] and note["files"][0]["annotation_changed"]
    assert not note["files"][0]["executable_changed"] and not note["files"][0]["imports_changed"]
    assert note["before"] == note["after"] == [] and note["selected_reverse_rows"] == 0

    write(dep / "leaf.w", body.replace("17", "31"))
    edit = publish(app, store, comment)
    before_plan = invoke(app, store, base, "--source-plan", "10000", base, comment, edit)[0]
    report, wall, peak = impact(app, store, comment, edit)
    assert report["executable_tree_changed"] and not report["annotation_tree_changed"]
    assert not report["configuration_changed"] and files(report) == ["src/leaf.w"]
    assert report["files"][0]["executable_changed"] and not report["files"][0]["imports_changed"]
    expected = ["src/main.w", "src/a.w", "src/leaf.w", "src/z.w"]
    assert paths(report, "before") == paths(report, "after") == expected
    assert report["forward_occurrences"] == 12  # Duplicate aliases remain occurrences.
    assert report["selected_reverse_rows"] == 8  # Physical reverse edges are deduplicated.
    again, _, _ = impact(app, store, comment, edit)
    assert {k: v for k, v in again.items() if k != "query_ms"} == {
        k: v for k, v in report.items() if k != "query_ms"
    }
    after_plan = invoke(app, store, base, "--source-plan", "10000", base, comment, edit)[0]
    assert {k: v for k, v in before_plan.items() if k != "query_ms"} == {
        k: v for k, v in after_plan.items() if k != "query_ms"
    }
    assert not (store / "cas").exists()
    assert invoke(app, store, edit)[0]["snapshot"]["written"] == 0
    impact(app, store, comment, edit, "9", "100", failure="file budget")
    impact(app, store, comment, edit, "10", "11", failure="adjacency budget")
    # The two globally shared providers also have a foreign incoming row.
    # Its name/root may never enter the result, but scanning it has a cost.
    impact(app, store, comment, edit, "10", "20", failure="adjacency budget")
    impact(app, store, comment, outside, failure="same workspace and target")
    assert impact(app, store, comment, "a" * 64)[0] is None
    identical, _, _ = impact(app, store, edit, edit)
    assert files(identical) == [] and identical["before"] == identical["after"] == []

    # Configuration-only change neither fabricates file deltas nor permits
    # assuming semantic reuse beyond this source graph query.
    config = (app / "wosy.toml")
    config.write_text(manifest.replace('llvm-18.1.3"', 'llvm-18.fixture"'))
    revised = publish(app, store, edit)
    config_only, _, _ = impact(app, store, edit, revised)
    assert config_only["configuration_changed"] and not config_only["executable_tree_changed"]
    assert files(config_only) == [] and config_only["before"] == config_only["after"] == []

    # Remove both importer branches. No after-side traversal starts on files
    # that are no longer selected.
    write(main, 'sibling = namespace std "sibling.w";\n', "só anotação 🚀🚀")
    shrinking = publish(app, store, revised)
    removed, _, _ = impact(app, store, revised, shrinking)
    assert paths(removed, "before") == expected and paths(removed, "after") == ["src/main.w"]
    assert [diff["file"]["path"] for diff in removed["files"] if diff["after"] is None] == expected[1:]
    reverse, _, _ = impact(app, store, shrinking, revised)
    assert paths(reverse, "before") == ["src/main.w"] and paths(reverse, "after") == expected

    # The ordinary source loader, not the impact query, rejects cyclic
    # revisions. Existing accepted roots remain comparable.
    write(main, entry)
    write(dep / "leaf.w", 'cycle = namespace std "z.w";\n')
    invoke(app, store, revised, failure="cycle")
    still, _, _ = impact(app, store, revised, shrinking)
    assert paths(still, "before") == expected

    importer, deps = project(remap)
    write(importer, 'provider = namespace std "packet.w";\n')
    write(deps / "packet.w", "u32 width = 4;\n")
    first = publish(remap, store)
    # A genuine change of provider owner with identical bytes and identical
    # relative paths must affect the provider and its unchanged importer.
    mirror = remap / "mirror/src"
    mirror.mkdir(parents=True)
    (mirror.parent / "wosy.toml").write_text('[package]\nname="std"\nsource_root="src"\n')
    write(mirror / "packet.w", "u32 width = 4;\n")
    (remap / "wosy.toml").write_text(manifest.replace('path = "stdlib"', 'path = "mirror"'))
    owner = publish(remap, store, first)
    changed_owner, _, _ = impact(remap, store, first, owner)
    assert changed_owner["configuration_changed"]
    assert not changed_owner["executable_tree_changed"] and not changed_owner["annotation_tree_changed"]
    assert files(changed_owner) == ["src/main.w", "src/packet.w"]
    unchanged_importer, moved_provider = changed_owner["files"]
    assert unchanged_importer["before"] == unchanged_importer["after"] and unchanged_importer["imports_changed"]
    assert not unchanged_importer["source_owner_changed"] and not unchanged_importer["executable_changed"]
    assert moved_provider["source_owner_changed"] and not moved_provider["executable_changed"]
    assert moved_provider["before"] != moved_provider["after"]
    assert paths(changed_owner, "before") == paths(changed_owner, "after") == ["src/main.w", "src/packet.w"]

    alt = remap / "alt/custom"
    alt.mkdir(parents=True)
    (alt.parent / "wosy.toml").write_text('[package]\nname="std"\nsource_root="custom"\n')
    write(alt / "packet.w", "u64 width = 8;\n")
    remapped_manifest = manifest.replace('path = "stdlib"', 'path = "alt"')
    (remap / "wosy.toml").write_text(remapped_manifest)
    second = publish(remap, store, owner)
    changed, _, _ = impact(remap, store, first, second)
    entry_diff = changed["files"][0]
    assert changed["configuration_changed"] and entry_diff["file"]["path"] == "src/main.w"
    assert entry_diff["before"] == entry_diff["after"] and not entry_diff["executable_changed"]
    assert entry_diff["imports_changed"] and not entry_diff["annotation_changed"]
    assert paths(changed, "before") == ["src/main.w", "src/packet.w"]
    assert paths(changed, "after") == ["src/main.w", "custom/packet.w"]

    # Distinguish target scope in the exact same workspace.
    alternate_target = remapped_manifest.replace('default_target = "app"', 'default_target = "other"')
    alternate_target = alternate_target.replace("[targets.app", "[targets.other")
    (remap / "wosy.toml").write_text(alternate_target)
    different_target = publish(remap, store)
    impact(remap, store, second, different_target, failure="same workspace and target")
    print("rooted source impact, reverse diamond, and exact alias remap", {
        "affected_per_side": len(report["before"]), "forward_occurrences": report["forward_occurrences"],
        "selected_reverse_rows": report["selected_reverse_rows"], "query_ms": round(report["query_ms"], 3),
        "process_ms": round(wall, 3), "process_peak_kib": peak,
    })

"""Run the internal release source_snapshot example across genuine process reopens."""
import json
import os
from pathlib import Path
import shutil
import struct
import subprocess
import sys
import tempfile
import time

binary = Path(sys.argv[1]).resolve()
template = Path(__file__).parent / "fixtures/build/shared_stdlib_local_dependency/project/wosy.toml"


def measured_run(*args, executable=binary):
    # Per-child rusage, not the largest of all preceding child processes.
    with tempfile.TemporaryFile() as stdout:
        start = time.perf_counter()
        child = subprocess.Popen([executable, *args], stdout=stdout)
        _, status, usage = os.wait4(child.pid, 0)
        child.returncode = os.waitstatus_to_exitcode(status)
        if child.returncode:
            raise subprocess.CalledProcessError(child.returncode, child.args)
        wall_ms = (time.perf_counter() - start) * 1000
        stdout.seek(0)
        return json.load(stdout), wall_ms, usage.ru_maxrss


with tempfile.TemporaryDirectory() as work:
    root = Path(work) / "app"
    store = Path(work) / "store"
    (root / "src").mkdir(parents=True)
    dep = root / "stdlib"
    (dep / "src/nested").mkdir(parents=True)
    manifest = template.read_text().replace("categories = []", 'categories = ["WHY"]')
    (root / "wosy.toml").write_text(manifest)
    (dep / "wosy.toml").write_text('[package]\nname="std"\nsource_root="src"\n')
    names = ["leaf.w", "sibling.w", "nested/view.w"]
    if sys.platform.startswith("linux"):
        names.append("nested\\view.w")
    names.extend(f"module{n:02}.w" for n in range(32))
    entry = "%%start\n# WHY: entry context\n" + "".join(
        f"module{n} = namespace std {json.dumps(name)};# WHY: after this import\n"
        if n == 0 else f"module{n} = namespace std {json.dumps(name)};\n"
        for n, name in enumerate(names)
    ) + "%%end\n"
    (root / "src/main.w").write_text(entry)
    src = "%%start\n# WHY: first\nu8 sample = 17;\n%%end\n"
    leaf = dep / "src/leaf.w"
    leaf.write_text(src)
    for n, name in enumerate(names[1:], start=1):
        (dep / "src" / name).write_text(
            "%%start\n" + "".join(f"u32 sample{i} = {n*100+i};\n" for i in range(48)) + "%%end\n"
        )
    files_total = 1 + len(names)

    def snapshot(name, *args, project=root, database=store):
        response, elapsed, rss = measured_run(project, database, *args)
        data = response["snapshot"]
        size = sum(p.stat().st_size for p in database.rglob("*") if p.is_file())
        stats = {key: value for key, value in data.items() if key != "files"}
        print(name, {"process_wall_ms": elapsed, "store_bytes": size, "peak_child_kib": rss,
                     "close_ms": response["close_ms"], **stats}, flush=True)
        return data

    def authored(root_id, path, *, oracle=False):
        args = ["--oracle"] if oracle else []
        value, elapsed, rss = measured_run(root, store, root_id, "std", path, *args)
        print("typed readback", {k:v for k,v in value.items() if k.endswith("_ms")},
              {"wall_ms": elapsed, "peak_child_kib": rss, "oracle": oracle}, flush=True)
        return {key: value[key] for key in ("cst", "comments", "text")}

    def declarations(root_id, package, file, *, verify=False, project=root, database=store):
        extra = ["--oracle"] if verify else []
        value, elapsed, rss = measured_run(
            project, database, root_id, package, file, "--declarations", *extra
        )
        print("base declarations", {k:v for k,v in value.items() if k != "diagnostics"},
              {"wall_ms": elapsed, "peak_child_kib": rss, "oracle": verify}, flush=True)
        return value

    def bypath(reply):
        return {f["path"]: f for f in reply["files"]}

    cold = snapshot("cold")
    warm = snapshot("reopen", cold["source_root"])
    assert cold["parsed"] == files_total and cold["written"] > 0
    assert cold["namespace_derived"] == files_total and cold["namespace_reused"] == 0
    assert warm["source_root"] == cold["source_root"]
    assert (warm["parsed"], warm["reused"], warm["written"]) == (0, files_total, 0)
    assert (warm["namespace_derived"], warm["namespace_reused"]) == (0, 0)
    assert bypath(warm)["src/nested/view.w"]["path"] == "src/nested/view.w"
    if sys.platform.startswith("linux"):
        assert bypath(warm)["src/nested\\view.w"]["module"] != bypath(warm)["src/nested/view.w"]["module"]
    old = authored(cold["source_root"], "src/leaf.w", oracle=True)
    assert old["text"] == src and old["comments"][0]["raw_payload"].strip() == "first"

    first_leaf = declarations(warm["source_root"], "std", "src/leaf.w", verify=True)
    first_main = declarations(warm["source_root"], "app", "src/main.w", verify=True)
    assert first_leaf["disposition"] == first_main["disposition"] == "Derived"
    reused = declarations(warm["source_root"], "std", "src/leaf.w")
    assert reused["disposition"] == "Reused" and reused["declarations"] == first_leaf["declarations"]

    leaf.write_text(src.replace("17", "18"))
    changed = snapshot("dependency", warm["source_root"])
    assert (changed["parsed"], changed["reused"]) == (1, files_total - 1)
    assert 0 < changed["written"] < cold["written"]
    assert (changed["namespace_derived"], changed["namespace_reused"]) == (1, 0)
    assert cold["executable"] != changed["executable"]
    unaffected = (set(bypath(cold)) - {"src/main.w", "src/leaf.w"})
    assert all(bypath(changed)[p] == bypath(cold)[p] for p in unaffected)
    assert bypath(changed)["src/main.w"]["authored"] == bypath(cold)["src/main.w"]["authored"]
    assert bypath(changed)["src/main.w"]["module"] != bypath(cold)["src/main.w"]["module"]
    assert authored(cold["source_root"], "src/leaf.w") == old
    assert authored(changed["source_root"], "src/leaf.w", oracle=True)["text"] == leaf.read_text()

    second_leaf = declarations(changed["source_root"], "std", "src/leaf.w", verify=True)
    second_main = declarations(changed["source_root"], "app", "src/main.w", verify=True)
    assert second_leaf["disposition"] == "Derived" and second_leaf["declarations"] != first_leaf["declarations"]
    assert second_main["disposition"] == "Reused" and second_main["declarations"] == first_main["declarations"]
    assert second_main["module"] != first_main["module"]

    leaf.write_text(leaf.read_text().replace("first", "amended"))
    note = snapshot("typed-comment", changed["source_root"])
    assert (note["parsed"], note["reused"]) == (1, files_total - 1)
    assert note["executable"] == changed["executable"] and note["annotations"] != changed["annotations"]
    assert (note["namespace_derived"], note["namespace_reused"]) == (0, 1)
    assert authored(note["source_root"], "src/leaf.w", oracle=True)["comments"][0]["raw_payload"].strip() == "amended"
    main = root / "src/main.w"
    before = main.read_text()
    main.write_text(before.replace("entry context", "more detailed entry context"))
    annotation_edit = snapshot("comment-shifts-import-span", note["source_root"])
    assert (annotation_edit["namespace_derived"], annotation_edit["namespace_reused"]) == (0, 1)
    assert annotation_edit["executable"] == note["executable"]
    older, current = bypath(note)["src/main.w"], bypath(annotation_edit)["src/main.w"]
    assert older["module"] == current["module"] and older["imports"] != current["imports"]
    for state, text in ((older, before), (current, main.read_text())):
        first_import = state["imports"][0]["span"]
        assert text[first_import["start"]:first_import["end"]] == 'module0 = namespace std "leaf.w";'

    annotated_main = declarations(annotation_edit["source_root"], "app", "src/main.w", verify=True)
    assert annotated_main["disposition"] == "Reused"
    assert annotated_main["declarations"] == first_main["declarations"]
    assert annotated_main["source"]["revision"] != first_main["source"]["revision"]

    (root / "wosy.toml").write_text(manifest.replace('["WHY"]', '["WHY", "INTENT"]'))
    policy = snapshot("category-policy", annotation_edit["source_root"])
    assert policy["source_root"] != annotation_edit["source_root"] and policy["parsed"] == files_total
    assert policy["executable"] == annotation_edit["executable"] and policy["annotations"] == annotation_edit["annotations"]
    assert (policy["namespace_derived"], policy["namespace_reused"]) == (0, files_total)

    other_project = Path(work) / "another-consumer"
    shutil.copytree(root, other_project)
    other = snapshot("equal-content second project", project=other_project)
    assert other["source_root"] != policy["source_root"]
    assert other["executable"] == policy["executable"]
    assert other["parsed"] == files_total
    assert (other["namespace_derived"], other["namespace_reused"]) == (0, files_total)

    other_main = declarations(
        other["source_root"], "app", "src/main.w", verify=True, project=other_project
    )
    assert other_main["disposition"] == "Reused"
    assert other_main["declarations"] == first_main["declarations"]
    assert other_main["source"]["project"] != first_main["source"]["project"]

    # A separate small project can exercise authored diagnostics and big typed
    # declaration results without making the ordinary 37-module fixture larger.
    payload_root = Path(work) / "payload-app"
    payload_db = Path(work) / "payload-store"
    (payload_root / "src").mkdir(parents=True)
    (payload_root / "stdlib/src").mkdir(parents=True)
    (payload_root / "wosy.toml").write_text(manifest)
    (payload_root / "stdlib/wosy.toml").write_text((dep / "wosy.toml").read_text())
    payload_file = payload_root / "src/main.w"
    bad = (Path(__file__).parent / "fixtures/build/project_derivation_invalid_escape/project/src/child.w").read_text()
    payload_file.write_text(bad.replace("%%start\n", "%%start\n# WHY: antes ☕\n"))
    diagnostics_root = snapshot("diagnostic source", project=payload_root, database=payload_db)
    diagnostic = declarations(
        diagnostics_root["source_root"], "app", "src/main.w",
        verify=True, project=payload_root, database=payload_db,
    )
    assert diagnostic["disposition"] == "Derived" and len(diagnostic["diagnostics"]) == 1
    assert diagnostic["diagnostics"][0]["code"] == "B0003"
    payload_file.write_text(payload_file.read_text().replace("antes ☕", "muito mais contexto 🚀🚀"))
    annotated_diagnostics_root = snapshot(
        "diagnostic span shift", diagnostics_root["source_root"], project=payload_root, database=payload_db
    )
    shifted = declarations(
        annotated_diagnostics_root["source_root"], "app", "src/main.w",
        verify=True, project=payload_root, database=payload_db,
    )
    assert shifted["disposition"] == "Reused" and shifted["declarations"] == diagnostic["declarations"]
    assert shifted["diagnostics"] != diagnostic["diagnostics"]

    finite_values = ["1.2345678901234567e-230", "2.2250738585072014e-308",
                     "5.0e-324", "-0.0", "1.7976931348623157e308"]
    expected_bits = [struct.pack(">d", float(text)).hex() for text in finite_values]
    float_source = "".join(f"f64 finite{n} = {text};\n" for n, text in enumerate(finite_values))
    payload_file.write_text("%%start\n" + float_source + "%%end\n")
    float_root = snapshot("finite declaration source", project=payload_root, database=payload_db)
    for phase in ("Derived", "Reused"):
        finite = declarations(float_root["source_root"], "app", "src/main.w",
                              verify=True, project=payload_root, database=payload_db)
        assert finite["disposition"] == phase and finite["float_bits"] == expected_bits

    large_text = (
        "%%start\nstruct Datum {\n\tu32 value;\n}\n"
        "enum Mode { off; on; }\nu32() answer = fn() {\n\t42\n};\n"
        + float_source + "".join(f"u32 v{n} = {n};\n" for n in range(720)) + "%%end\n"
    )
    payload_file.write_text(large_text)
    large_source = snapshot("large declaration source", project=payload_root, database=payload_db)
    def large_query():
        return declarations(large_source["source_root"], "app", "src/main.w",
                            verify=True, project=payload_root, database=payload_db)
    pair, _, _ = measured_run(payload_root, payload_db, large_source["source_root"],
                              "app", "src/main.w", "--declarations-pair", "--stale-temp")
    print("concurrent base declarations", pair, flush=True)
    assert pair["derived"] == 1 and pair["float_bits"] == expected_bits
    large = large_query()
    assert large["disposition"] == "Reused" and large["declarations"] == pair["declarations"]
    assert large["float_bits"] == expected_bits
    blobs = list((payload_db / "cas").iterdir())
    assert len(blobs) == 1 and blobs[0].stat().st_size > 64 * 1024
    print("large derived CAS bytes", blobs[0].stat().st_size, flush=True)
    large_warm = large_query()
    assert large_warm["disposition"] == "Reused" and large_warm["declarations"] == large["declarations"]
    # Losing a recomputable projection does not make the source root stale.
    blobs[0].unlink()
    recovered = large_query()
    assert recovered["disposition"] == "Derived" and recovered["declarations"] == large["declarations"]
    assert recovered["float_bits"] == expected_bits
    assert blobs[0].is_file()
    assert large_query()["disposition"] == "Reused"

    if len(sys.argv) == 3:
        # Only when the original release binary is already available: verify
        # schema migration with the actual previous writer, not graph stubs.
        previous = Path(sys.argv[2]).resolve()
        legacy_store = Path(work) / "legacy-store"
        old_reply, _, _ = measured_run(root, legacy_store, executable=previous)
        legacy = old_reply["snapshot"]
        migrated = snapshot("legacy schema migration", legacy["source_root"], database=legacy_store)
        assert migrated["source_root"] != legacy["source_root"]
        assert migrated["executable"] == legacy["executable"]
        assert (migrated["parsed"], migrated["reused"]) == (files_total, 0)
        assert migrated["namespace_derived"] == files_total
        assert migrated["written"] == files_total + 1  # one new rooted authority schema and its namespace facts

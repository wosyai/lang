"""Verify the opt-in rooted Build analysis without invoking any emitter."""
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
        wall_ms = (time.perf_counter() - start) * 1000
        stdout.seek(0)
        return json.load(stdout), wall_ms, usage.ru_maxrss


def source(body, note=None):
    why = f"# WHY: {note}\n" if note else ""
    return "%%start\n" + why + body + "%%end\n"


with tempfile.TemporaryDirectory() as work:
    project, store = Path(work) / "project", Path(work) / "store"
    (project / "src").mkdir(parents=True)
    dep = project / "stdlib/src"
    dep.mkdir(parents=True)
    (project / "wosy.toml").write_text(manifest)
    (dep.parent / "wosy.toml").write_text('[package]\nname="std"\nsource_root="src"\n')
    main, provider = project / "src/main.w", dep / "provider.w"
    main.write_text(source(
        'picked = namespace std "provider.w";\n'
        'picked.Payload input = { .value = 3; };\n'
        'picked.Payload picked_result = picked.consume(input);\n'
    ))
    valid = ("struct Payload { u8 value; }\n"
             "Payload(Payload) consume = fn(value) { next = value; next };\n")

    def snapshot(prior=None):
        return run(project, store, *([prior] if prior else []))[0]["snapshot"]

    def build(root, profile, *options):
        return run(project, store, root, "--typed-project", profile, *options)

    profiles = {"dev": 4, "native": 8}
    accepted = {}
    accepted_keys = {}
    provider.write_text(source(valid, "original ☕"))
    good = snapshot()["source_root"]
    for profile, width in profiles.items():
        verdict, _, _ = build(good, profile, "--build-oracle")
        assert verdict["phase"] == "Analyzed" and verdict["diagnostics"] == []
        assert verdict["preparation_diagnostics"] == []
        assert verdict["target_layout"]["pointer_size"] == width
        assert verdict["analysis_disposition"] == "Derived"
        accepted[profile] = verdict["prepared_identity"]
        accepted_keys[profile] = verdict["analysis_key"]
        assert accepted_keys[profile] != accepted[profile]
    assert len(set(accepted.values())) == 2
    assert len(set(accepted_keys.values())) == 2
    other_layout, _, _ = build(good, "dev", "--build-held", good, "native")
    assert [result["analysis_key"] for result in other_layout] == [accepted_keys["dev"], accepted_keys["native"]]
    assert all(result["analysis_disposition"] == "Derived" for result in other_layout)

    # The established compiler test's harmless illegal move is rejected only
    # by ordinary Build analysis; parsing and preparation still succeed.
    invalid = valid.replace("next = value; next", "next = value; next; next")
    old_text = source(invalid, "origem ☕")
    provider.write_text(old_text)
    old = snapshot(good)["source_root"]
    prior = {}
    for profile in profiles:
        verdict, _, _ = build(old, profile, "--build-oracle")
        assert verdict["phase"] == "Analyzed" and verdict["preparation_diagnostics"] == []
        assert verdict["analysis_disposition"] == "Derived"
        assert verdict["analysis_key"] != accepted_keys[profile]
        moved, = verdict["diagnostics"]
        assert moved["message"] == "read or move of unavailable place"
        assert [label["kind"] for label in moved["labels"]] == ["Primary", "Secondary"]
        for label in moved["labels"]:
            assert label["span"]["source"]["path"] == "src/provider.w"
            span = label["span"]["range"]
            assert old_text.encode()[span["start"]:span["end"]] == b"next"
        prior[profile] = verdict

    new_text = source(invalid, "comentário de origem maior com UTF-8 🚀🚀")
    provider.write_text(new_text)
    new = snapshot(old)["source_root"]
    assert old != new
    updated = {}
    concurrent_wait_ms = []
    for profile in profiles:
        current, _, _ = build(new, profile, "--build-oracle")
        updated[profile] = current["diagnostics"]
        assert current["prepared_identity"] == prior[profile]["prepared_identity"]
        assert current["analysis_key"] == prior[profile]["analysis_key"]
        historical, _, _ = build(old, profile, "--build")
        assert historical["diagnostics"] == prior[profile]["diagnostics"]
        held, _, _ = build(old, profile, "--build-held", new)
        assert sum(view["reused_live"] for view in held) == 1
        assert sorted(view["analysis_disposition"] for view in held) == ["Derived", "Reused"]
        assert all(view["analysis_key"] == current["analysis_key"] for view in held)
        assert held[0]["diagnostics"] == prior[profile]["diagnostics"]
        assert held[1]["diagnostics"] == current["diagnostics"]
        concurrent_wait_ms.extend(
            view["analysis_ms"] for view in held if view["analysis_disposition"] == "Reused"
        )
        for label in current["diagnostics"][0]["labels"]:
            span = label["span"]["range"]
            assert new_text.encode()[span["start"]:span["end"]] == b"next"
        assert current["diagnostics"] != historical["diagnostics"]
    released, _, _ = build(new, "dev", "--build-released")
    assert all(view["analysis_disposition"] == "Derived" for view in released)
    assert released[0]["analysis_key"] == released[1]["analysis_key"] == prior["dev"]["analysis_key"]
    assert all(view["diagnostics"] == updated["dev"] for view in released)

    # A real input to canonical preparation changes even if the diagnostics
    # for this source happen to stay equal. Do not share the concurrent work.
    (project / "wosy.toml").write_text(manifest.replace("llvm-18.1.3", "llvm-18.revised"))
    reconfigured = snapshot(new)["source_root"]
    assert new != reconfigured
    contexts, _, _ = build(new, "dev", "--build-held", reconfigured)
    assert all(view["analysis_disposition"] == "Derived" for view in contexts)
    assert contexts[0]["analysis_key"] != contexts[1]["analysis_key"]
    assert contexts[0]["diagnostics"] == contexts[1]["diagnostics"] == updated["dev"]

    # Existing compiler examples for the ordinary mixed-order static-rejection
    # path: an unresolved callable prevents preparation, while the earlier
    # module's array-move diagnostic still belongs to Build's rejection report.
    main.write_text(source(
        'first = namespace std "z-origin.w";\nsecond = namespace std "a-lookup.w";\n'
    ))
    (dep / "z-origin.w").write_text(source(
        "struct Payload { i32 value; }\n"
        "Payload(Payload[2], u64) take = fn(values, index) { values[index] };\n",
        "primeiro módulo ☕",
    ))
    (dep / "a-lookup.w").write_text(source(
        "struct Callback { i32(i32) run; }\n"
        "i32(i32) identity = fn(input) { input };\n"
        "Callback callback = { .run = identity; };\n"
        "i32(i32) invoke = fn(input) { callback.run(input) };\n",
        "segundo módulo 🚀",
    ))
    mixed = snapshot(new)["source_root"]
    for profile in profiles:
        verdict, _, _ = build(mixed, profile, "--build-oracle")
        assert verdict["phase"] == "PreparationRejected"
        assert [d["message"] for d in verdict["preparation_diagnostics"]] == ["missing resolved call target"]
        assert [d["message"] for d in verdict["diagnostics"]] == [
            "array elements cannot be moved individually", "missing resolved call target",
        ]
        assert [d["labels"][0]["span"]["source"]["path"] for d in verdict["diagnostics"]] == [
            "src/z-origin.w", "src/a-lookup.w",
        ]
        shared, _, _ = build(mixed, profile, "--build-held", mixed)
        assert sorted(view["analysis_disposition"] for view in shared) == ["Derived", "Reused"]
        assert all(view["diagnostics"] == verdict["diagnostics"] for view in shared)

    assert not (project / ".wosy/artifacts").exists()
    observations = []
    for _ in range(4):
        verdict, wall_ms, rss = build(new, "dev", "--build")
        assert verdict["diagnostics"] == updated["dev"]
        observations.append({k: verdict[k] for k in ("preparation_ms", "analysis_ms", "authored_projection_ms")}
                            | {"wall_ms": wall_ms, "peak_child_kib": rss})
    print("ordinary Build parity on Native64/Wasm32: accepted, post-preparation diagnostics, mixed rejection, historical UTF-8:",
          {k: round(median(sample[k] for sample in observations), 3) for k in observations[0]}, flush=True)
    print("diagnostic-only Build uses exact live preparation, concurrent share, layout/config isolation, release and remap:",
          {"shared_request_including_wait_ms": [round(value, 3) for value in concurrent_wait_ms],
           "fresh_request_analysis_ms": round(median(sample["analysis_ms"] for sample in observations), 3)},
          flush=True)

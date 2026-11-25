"""Recover one active client's source generation into a different real store."""
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import time


example = Path(sys.argv[1]).resolve()
manifest = (Path(__file__).parent /
            "fixtures/build/shared_stdlib_local_dependency/project/wosy.toml").read_text()
manifest = manifest.replace("categories = []", 'categories = ["WHY"]')


def source(body, why):
    return f"%%start\n# WHY: {why}\n{body}%%end\n"


with tempfile.TemporaryDirectory() as temp:
    root = Path(temp) / "app"
    main = root / "src"
    lib = root / "stdlib/src"
    main.mkdir(parents=True)
    lib.mkdir(parents=True)
    (root / "wosy.toml").write_text(manifest)
    (lib.parent / "wosy.toml").write_text('[package]\nname = "std"\nsource_root = "src"\n')
    count = 12
    for i in range(count):
        (lib / f"leaf{i}.w").write_text(source(f"u32 constant = {i};\n", f"leaf {i}"))
    (main / "main.w").write_text(source(
        'payload = namespace std "packet.w";\n'
        + "".join(f'leaf{i} = namespace std "leaf{i}.w";\n' for i in range(count))
        + "payload.Packet packet = { .count = 19; };\n", "application"
    ))
    original = source("struct copy Packet { u32 count; }\n", "original ☕")
    edited = source("struct copy Packet { u64 count; }\n", "selecionado 🚀")
    unobserved = source("struct copy Packet { u32 count; }\n", "não selecionado")
    path = lib / "packet.w"
    path.write_text(original)
    start = time.perf_counter()
    with subprocess.Popen(
        [example, root, Path(temp) / "primary", "--reconcile-source",
         Path(temp) / "replacement", "std", "src/packet.w"],
        stdin=subprocess.PIPE, stdout=subprocess.PIPE, text=True,
    ) as child:
        phase1 = json.loads(child.stdout.readline())
        path.write_text(edited)
        child.stdin.write("\n")
        child.stdin.flush()
        phase2 = json.loads(child.stdout.readline())
        path.write_text(unobserved)
        child.stdin.write("\n")
        child.stdin.flush()
        phase3 = json.loads(child.stdout.readline())
        assert child.stdout.read() == ""
        _, status, usage = os.wait4(child.pid, 0)
        child.returncode = os.waitstatus_to_exitcode(status)
        assert child.returncode == 0
    wall = (time.perf_counter() - start) * 1000

    initial, warm = phase1["initial"], phase1["warm"]
    assert (initial["parsed"], initial["reused"]) == (count + 2, 0)
    assert (warm["parsed"], warm["reused"]) == (0, count + 2)
    assert warm["root"] == initial["root"]
    assert warm["retained_node_bytes"] == initial["retained_node_bytes"]
    assert phase1["publish_initial"]["written"] > 0
    assert phase1["reopened"]["written"] == phase1["reopened"]["namespace_derived"] == 0
    assert phase1["initial_state"]["modules"][1]["structs"][0]["layout"]["size"] == 4
    assert phase1["initial_state"]["authored"] == original

    selection = phase2["edited"]
    assert (selection["parsed"], selection["reused"]) == (1, count + 1)
    assert phase2["drops_prior_root"] and phase2["historical"] == original
    assert selection["root"] != initial["root"]
    assert 0 < phase2["publish_edit"]["written"] < phase1["publish_initial"]["written"]
    assert phase2["edited_state"]["modules"][1]["structs"][0]["layout"]["size"] == 8
    assert phase2["edited_state"]["authored"] == edited
    assert "selecionado" in phase2["edited_state"]["comments"][0]["raw_payload"]

    assert phase3["missing"] and phase3["selected"] == selection
    assert phase3["recovered"]["written"] > phase2["publish_edit"]["written"]
    assert phase3["recovered"]["namespace_derived"] > 0
    assert phase3["repeat"]["written"] == phase3["repeat"]["namespace_derived"] == 0
    assert phase3["restored"] == phase2["edited_state"]
    assert path.read_text() == unobserved and not (root / ".wosy").exists()

    print("live client reopens known store and resubmits exact CST to real empty store", {
        "files": count + 2, "cold_warm_edit_parse": [initial["parsed"], warm["parsed"], selection["parsed"]],
        "retained_nodes": [initial["retained_nodes"], selection["retained_nodes"]],
        "retained_encoded_bytes": [initial["retained_node_bytes"], selection["retained_node_bytes"]],
        "observe_ms": [round(group[key], 3) for group, key in (
            (phase1, "cold_observe_ms"), (phase1, "warm_observe_ms"), (phase2, "edit_observe_ms"))],
        "recovered_ms": round(phase3["recovered"]["elapsed_ms"], 3),
        "recovered_open_ms": round(phase3["replacement_open_ms"], 3),
        "process_wall_ms": round(wall, 3), "process_peak_kib": usage.ru_maxrss,
    }, flush=True)

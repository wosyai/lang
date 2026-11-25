"""Admit real staged updates, interleave two clients and recover a missing base."""
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


def packet(width, why):
    return f"%%start\n# WHY: {why}\nstruct copy Packet {{ u{width} count; }}\n%%end\n"


with tempfile.TemporaryDirectory() as directory:
    root = Path(directory) / "app"
    main = root / "src"
    lib = root / "stdlib/src"
    main.mkdir(parents=True)
    lib.mkdir(parents=True)
    (root / "wosy.toml").write_text(manifest)
    (lib.parent / "wosy.toml").write_text('[package]\nname="std"\nsource_root="src"\n')
    count = 12
    for i in range(count):
        (lib / f"leaf{i}.w").write_text(f"%%start\nu32 constant = {i};\n%%end\n")
    (main / "main.w").write_text(
        '%%start\npayload = namespace std "packet.w";\n'
        + "".join(f'leaf{i} = namespace std "leaf{i}.w";\n' for i in range(count))
        + "payload.Packet packet = { .count = 19; };\n%%end\n"
    )
    original = packet(32, "original ☕")
    edit = packet(64, "concorrente 🌱 " + "ç" * 512)
    next_edit = packet(16, "selecionado 🚀")
    unobserved = packet(32, "não selecionado")
    source = lib / "packet.w"
    source.write_text(original)
    start = time.perf_counter()
    with subprocess.Popen(
        [example, root, Path(directory) / "base", "--staged-source-deltas",
         Path(directory) / "replacement", "std", "src/packet.w"],
        stdin=subprocess.PIPE, stdout=subprocess.PIPE, text=True,
    ) as child:
        reports = []
        for text in (edit, next_edit, unobserved):
            reports.append(json.loads(child.stdout.readline()))
            source.write_text(text)
            child.stdin.write("\n")
            child.stdin.flush()
        reports.append(json.loads(child.stdout.readline()))
        assert child.stdout.read() == ""
        _, status, usage = os.wait4(child.pid, 0)
        child.returncode = os.waitstatus_to_exitcode(status)
        assert child.returncode == 0
    wall = (time.perf_counter() - start) * 1000

    start, concurrent, staged, final = reports
    original_root = start["initial"]["root"]
    assert start["initial"]["parsed"] == count + 2 and start["published"]["written"] > 0
    assert start["original"]["authored"] == original
    assert start["original"]["modules"][1]["structs"][0]["layout"]["size"] == 4
    assert concurrent["constrained"] == start["initial"]
    assert concurrent["historical"] == start["original"]
    assert concurrent["constrained_repeat"]["written"] == 0
    assert concurrent["concurrent_state"]["authored"] == edit
    assert concurrent["concurrent_state"]["modules"][1]["structs"][0]["layout"]["size"] == 8
    one, two = sorted(concurrent["cohort"], key=lambda output: output["mode"])
    assert one["mode"] == "Existing" and one["written"] == 0
    assert two["mode"] == "Incremental" and two["written"] > 0
    assert 0 < one["added_nodes"] == two["added_nodes"] < concurrent["concurrent"]["retained_nodes"]
    assert 0 < one["added_bytes"] == two["added_bytes"] < concurrent["concurrent"]["retained_node_bytes"]
    assert concurrent["known"]["mode"] == "Existing" and concurrent["known"]["added_nodes"] == 0
    active = concurrent["concurrent"]["root"]
    assert concurrent["second"]["root"] == active and active != original_root
    assert (concurrent["concurrent"]["parsed"], concurrent["concurrent"]["reused"]) == (1, count + 1)

    recovered = final["recovered"]
    assert final["fresh"] and final["still_no_predecessor"]
    assert staged["base"] == active and staged["candidate"] not in (active, original_root)
    assert final["selected"]["root"] == staged["candidate"]
    assert final["selected"]["retained_nodes"] == staged["candidate_nodes"]
    assert final["selected"]["retained_node_bytes"] == staged["candidate_bytes"]
    assert final["other_client"] == concurrent["second"]
    assert recovered["mode"] == "Recovered" and recovered["written"] > recovered["added_nodes"]
    assert 0 < recovered["added_bytes"] < staged["candidate_bytes"]
    assert final["repeat"]["mode"] == "Existing" and final["repeat"]["written"] == 0
    assert final["restored"]["authored"] == next_edit
    assert "selecionado" in final["restored"]["comments"][0]["raw_payload"]
    assert final["restored"]["modules"][1]["structs"][0]["layout"]["size"] == 2
    assert source.read_text() == unobserved and not (root / ".wosy").exists()
    print("two clients, sparse commit, stale-base recovery without republishing old roots", {
        "delta_nodes": two["added_nodes"], "delta_bytes": two["added_bytes"],
        "full_payload_bytes": concurrent["concurrent"]["retained_node_bytes"],
        "incremental_ms": round(two["elapsed_ms"], 3),
        "concurrent_retries": one["conflicts"] + two["conflicts"],
        "recovered_ms": round(recovered["elapsed_ms"], 3),
        "recovered_nodes": recovered["written"], "recovered_payload_bytes": staged["candidate_bytes"],
        "process_wall_ms": round(wall, 3), "process_peak_kib": usage.ru_maxrss,
    }, flush=True)

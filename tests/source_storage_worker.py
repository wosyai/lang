"""Use real temporary storage and a deterministic one-thread blocking pool."""
import hashlib
import json
import os
from pathlib import Path
from statistics import median
import subprocess
import sys
import tempfile
import time


example = Path(sys.argv[1]).resolve()
manifest = (Path(__file__).parent /
            "fixtures/build/shared_stdlib_local_dependency/project/wosy.toml").read_text()
manifest = manifest.replace("categories = []", 'categories = ["WHY"]')


def run(*args):
    with tempfile.TemporaryFile() as output:
        start = time.perf_counter()
        child = subprocess.Popen([example, *args], stdout=output)
        _, status, usage = os.wait4(child.pid, 0)
        child.returncode = os.waitstatus_to_exitcode(status)
        if child.returncode:
            raise subprocess.CalledProcessError(child.returncode, child.args)
        wall = (time.perf_counter() - start) * 1_000
        output.seek(0)
        return json.load(output), wall, usage.ru_maxrss


def blobs(store):
    return {p.name: hashlib.sha256(p.read_bytes()).hexdigest() for p in (store / "cas").iterdir()}


with tempfile.TemporaryDirectory() as temporary:
    root, store = (Path(temporary) / name for name in ("app", "store"))
    (root / "src").mkdir(parents=True)
    (root / "stdlib/src").mkdir(parents=True)
    (root / "wosy.toml").write_text(manifest)
    (root / "stdlib/wosy.toml").write_text('[package]\nname="std"\nsource_root="src"\n')
    (root / "src/main.w").write_text(
        '%%start\nprovider = namespace std "values.w";\nu32 result = provider.value;\n%%end\n'
    )
    file = root / "stdlib/src/values.w"
    old_text = ("%%start\n# WHY: older retained\nu32 value = 19;\n"
                + "".join(f"u32 fact{i} = {i};\n" for i in range(180)) + "%%end\n")
    file.write_text(old_text)
    old = run(root, store)[0]["snapshot"]["source_root"]
    run(root, store, old, "--project-declarations")
    new_text = old_text.replace("value = 19", "value = 31")
    file.write_text(new_text)
    new = run(root, store, old)[0]["snapshot"]["source_root"]
    run(root, store, new, "--project-declarations")
    original_blobs = blobs(store)
    assert len(original_blobs) == 2

    report, wall, rss = run(root, store, "--source-worker", new, old)
    assert report["heartbeat_while_blocking_pool_full"] and report["queued_cancellation_kept_slot"]
    assert report["node_budget_enforced"] and report["exact"]
    assert report["candidate_cas_references"] == report["protected_cas_references"] == 1
    assert report["nodes"] > 600 and blobs(store) == original_blobs
    assert run(root, store, old, "std", "src/values.w")[0]["text"] == old_text
    assert run(root, store, new, "std", "src/values.w")[0]["text"] == new_text
    samples = report["warm_paired_ms"]
    print("bounded worker, deterministic admission/cancel, async heartbeat, exact read-only SurrealKV/CAS", {
        "nodes": report["nodes"],
        "warm_direct_ms": round(median(row[0] for row in samples), 3),
        "warm_queue_ms": round(median(row[1] for row in samples), 3),
        "warm_worker_ms": round(median(row[2] for row in samples), 3),
        "process_wall_ms": round(wall, 3), "process_peak_kib": rss,
    }, flush=True)

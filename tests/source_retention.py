"""Verify explicit pins using real disposable stores and separately reopened processes."""
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
manifest = template.read_text().replace("categories = []", 'categories = ["WHY"]')


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


def project(root, leaf_text):
    (root / "src").mkdir(parents=True)
    (root / "stdlib/src").mkdir(parents=True)
    (root / "wosy.toml").write_text(manifest)
    (root / "stdlib/wosy.toml").write_text('[package]\nname="std"\nsource_root="src"\n')
    (root / "src/main.w").write_text(
        '%%start\nprovider = namespace std "values.w";\nu32 chosen = provider.value;\n%%end\n'
    )
    leaf = root / "stdlib/src/values.w"
    leaf.write_text(leaf_text)
    return leaf


def listed(root, store, scope):
    return run(root, store, scope, "--source-pins")[0]["pins"]


def file_bytes(store):
    return sum(p.stat().st_size for p in store.rglob("*") if p.is_file())


with tempfile.TemporaryDirectory() as work:
    root, other, store = (Path(work) / part for part in ("app", "other", "store"))
    old_text = "%%start\n# WHY: first root ☕\nu32 value = 17;\n%%end\n"
    new_text = old_text.replace("17", "19")
    leaf = project(root, old_text)
    project(other, new_text)
    name = "release/Δ"

    # The helper exits without Tree::close immediately after both an Eventual
    # source publication and an Immediate pin acknowledge in the same process.
    primary, publish_wall, publish_rss = run(root, store, "--publish-pin-exit", name)
    old = primary["root"]
    assert primary["name"] == name and primary["published_nodes"] > 0
    assert listed(root, store, old) == [{"name": name, "root": old}]
    assert not (store / "cas").exists()
    authored, _, _ = run(root, store, old, "std", "src/values.w", "--oracle")
    assert authored["text"] == old_text
    assert "first root" in authored["comments"][0]["raw_payload"]

    # Workspace-scoped namespaces, not package alias, pin name or path of the
    # helper's invocation. Real source content may be shared between owners.
    another, _, _ = run(other, store, "--publish-pin-exit", name)
    other_root = another["root"]
    assert old != other_root
    assert listed(other, store, other_root) == [{"name": name, "root": other_root}]
    assert listed(root, store, old) == [{"name": name, "root": old}]

    leaf.write_text(new_text)
    revision, _, _ = run(root, store, old)
    latest = revision["snapshot"]["source_root"]
    assert latest != old
    assert listed(root, store, latest) == [{"name": name, "root": old}]
    retained, _, _ = run(root, store, old, "std", "src/values.w")
    assert retained["text"] == old_text
    current, _, _ = run(root, store, latest, "std", "src/values.w", "--oracle")
    assert current["text"] == new_text

    # A graceful pin also works; rebind the original name to the new source
    # root and verify both names resolve after a second unclosed exit.
    scratch, _, _ = run(root, store, old, "--source-pin", "historical")
    assert scratch["root"] == old
    moved, _, _ = run(root, store, latest, "--source-pin", name, "--without-close")
    assert moved["root"] == latest
    assert listed(root, store, latest) == [
        {"name": "historical", "root": old}, {"name": name, "root": latest},
    ]
    assert listed(other, store, other_root) == [{"name": name, "root": other_root}]
    again, _, _ = run(root, store, latest, "--source-pin", name, "--without-close")
    assert again["root"] == latest

    removed, _, _ = run(root, store, latest, "--source-unpin", name, "--without-close")
    assert removed["removed"]
    assert listed(root, store, latest) == [{"name": "historical", "root": old}]
    absent, _, _ = run(root, store, latest, "--source-unpin", name)
    assert not absent["removed"]
    assert listed(other, store, other_root) == [{"name": name, "root": other_root}]
    assert run(root, store, latest, "std", "src/values.w")[0]["text"] == new_text
    assert not (store / "cas").exists()

    before = file_bytes(store)
    pins = []
    unpins = []
    for n in range(5):
        value, _, _ = run(root, store, latest, "--source-pin", f"measurement-{n}", "--without-close")
        pins.append(value["ack_ms"])
        value, _, _ = run(root, store, latest, "--source-unpin", f"measurement-{n}", "--without-close")
        unpins.append(value["ack_ms"])
        assert value["removed"]
    after = file_bytes(store)
    assert listed(root, store, latest) == [{"name": "historical", "root": old}]
    print("eventual publication before durable pin, no-close reopen, separate owners, rebind, durable unpin:",
          {"published_nodes": primary["published_nodes"], "source_publish_ms": round(primary["published_ms"], 3),
           "initial_pin_ack_ms": round(primary["ack_ms"], 3), "publish_and_ack_wall_ms": round(publish_wall, 3),
           "publish_and_ack_peak_kib": publish_rss, "median_pin_ack_ms": round(median(pins), 3),
           "median_unpin_ack_ms": round(median(unpins), 3), "kv_bytes_delta": after-before,
           "cas_created": False}, flush=True)

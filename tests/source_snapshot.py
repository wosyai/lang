"""Run the internal release source_snapshot example across genuine process reopens."""
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import time

binary = Path(sys.argv[1]).resolve()
template = Path(__file__).parent / "fixtures/build/shared_stdlib_local_dependency/project/wosy.toml"


def measured_run(*args):
    # Per-child rusage, not the largest of all preceding child processes.
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
    entry = "%%start\n" + "".join(
        f"module{n} = namespace std {json.dumps(name)};\n" for n, name in enumerate(names)
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

    def snapshot(name, *args):
        response, elapsed, rss = measured_run(root, store, *args)
        data = response["snapshot"]
        size = sum(p.stat().st_size for p in store.rglob("*") if p.is_file())
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

    def bypath(reply):
        return {f["path"]: f for f in reply["files"]}

    cold = snapshot("cold")
    warm = snapshot("reopen", cold["source_root"])
    assert cold["parsed"] == files_total and cold["written"] > 0
    assert warm["source_root"] == cold["source_root"]
    assert (warm["parsed"], warm["reused"], warm["written"]) == (0, files_total, 0)
    assert bypath(warm)["src/nested/view.w"]["path"] == "src/nested/view.w"
    if sys.platform.startswith("linux"):
        assert bypath(warm)["src/nested\\view.w"]["module"] != bypath(warm)["src/nested/view.w"]["module"]
    old = authored(cold["source_root"], "src/leaf.w", oracle=True)
    assert old["text"] == src and old["comments"][0]["raw_payload"].strip() == "first"

    leaf.write_text(src.replace("17", "18"))
    changed = snapshot("dependency", warm["source_root"])
    assert (changed["parsed"], changed["reused"]) == (1, files_total - 1)
    assert 0 < changed["written"] < cold["written"]
    assert cold["executable"] != changed["executable"]
    unaffected = (set(bypath(cold)) - {"src/main.w", "src/leaf.w"})
    assert all(bypath(changed)[p] == bypath(cold)[p] for p in unaffected)
    assert bypath(changed)["src/main.w"]["authored"] == bypath(cold)["src/main.w"]["authored"]
    assert bypath(changed)["src/main.w"]["module"] != bypath(cold)["src/main.w"]["module"]
    assert authored(cold["source_root"], "src/leaf.w") == old
    assert authored(changed["source_root"], "src/leaf.w", oracle=True)["text"] == leaf.read_text()

    leaf.write_text(leaf.read_text().replace("first", "amended"))
    note = snapshot("typed-comment", changed["source_root"])
    assert (note["parsed"], note["reused"]) == (1, files_total - 1)
    assert note["executable"] == changed["executable"] and note["annotations"] != changed["annotations"]
    assert authored(note["source_root"], "src/leaf.w", oracle=True)["comments"][0]["raw_payload"].strip() == "amended"
    (root / "wosy.toml").write_text(manifest.replace('["WHY"]', '["WHY", "INTENT"]'))
    policy = snapshot("category-policy", note["source_root"])
    assert policy["source_root"] != note["source_root"] and policy["parsed"] == files_total
    assert policy["executable"] == note["executable"] and policy["annotations"] == note["annotations"]

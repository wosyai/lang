"""A selected SQLite root uses the actual leased diagnostic dispatcher, not an emitter."""
import hashlib
import json
from pathlib import Path
import subprocess
import sys
import tempfile


example = Path(sys.argv[1]).resolve()
manifest = (Path(__file__).parent /
            "fixtures/build/shared_stdlib_local_dependency/project/wosy.toml").read_text()
manifest = manifest.replace("categories = []", 'categories = ["WHY"]')
manifest += """
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


def call(project, store, *args):
    child = subprocess.run([example, project, store, "--persistent-source", "app", *args],
                           text=True, capture_output=True, check=True)
    return json.loads(child.stdout)


def response(result):
    return result["result"]["response"]


def script(note, result):
    bindings = "".join(f"u32 fact{i} = {i};\n" for i in range(180))
    return f"""%%start
# WHY: {note}
struct Item {{ u8 value; }}
Item(Item) take = fn(input) {{ next = input; {result} }};
{bindings}%%end
"""


with tempfile.TemporaryDirectory() as temp:
    project, store = Path(temp) / "project", Path(temp) / "store"
    (project / "src").mkdir(parents=True)
    (project / "stdlib/src").mkdir(parents=True)
    (project / "wosy.toml").write_text(manifest)
    (project / "stdlib/wosy.toml").write_text('[package]\nname="std"\nsource_root="src"\n')
    entry, provider = project / "src/main.w", project / "stdlib/src/item.w"
    peers = "".join(f'peer{i} = namespace std "peer{i}.w";\n' for i in range(17))
    entry.write_text(f'%%start\nlocal = namespace std "item.w";\n{peers}%%end\n')
    for i in range(17):
        (provider.parent / f"peer{i}.w").write_text(f"%%start\nu32 value = {100 + i};\n%%end\n")
    old_text = script("origem ☕", "next; next")
    provider.write_text(old_text)
    first = call(project, store, "stage")
    old = call(project, store, "submit-pending", "app", "src/main.w")
    assert old["selected"] == first["pending"]

    def run(profile="dev", oracle=True, framed=True):
        options = (["--oracle"] if oracle else []) + (["--framed"] if framed else [])
        out = call(project, store, "leased-build", profile, *options)
        assert out["result"]["framed"] == framed
        answer = response(out)
        assert bytes(answer["source_root"]).hex() == out["selected"]
        assert answer["phase"] == "Analyzed" and answer["version"] == 1 and answer["profile"] == profile
        return answer, out["result"]["dispatch_ms"]

    orig, _ = run()
    native, _ = run("native")
    assert orig["origin"] == native["origin"] == "Derived"
    assert orig["receipt"] != native["receipt"]
    assert orig["diagnostics"] == native["diagnostics"]
    direct, _ = run(framed=False)
    assert direct["origin"] == "Stored"
    assert {k: v for k, v in direct.items() if k != "origin"} == {k: v for k, v in orig.items() if k != "origin"}
    only_blob, = (store / "cas").iterdir()
    assert only_blob.stat().st_size > 64 * 1024
    digest = hashlib.sha256(only_blob.read_bytes()).digest()
    # Lost recomputable fixture CAS: the next real leased Build restores the
    # same artifact bytes and keeps the stored canonical diagnostics.
    only_blob.unlink()
    repaired, _ = run()
    assert repaired["origin"] == "Stored" and repaired["diagnostics"] == orig["diagnostics"]
    assert hashlib.sha256(only_blob.read_bytes()).digest() == digest
    old_labels = orig["diagnostics"][0]["labels"]
    assert [label["kind"] for label in old_labels] == ["Primary", "Secondary"]
    for label in old_labels:
        span = label["span"]
        assert span["source"]["path"] == "src/item.w"
        assert old_text.encode()[span["range"]["start"]:span["range"]["end"]] == b"next"

    new_text = script("origem atualizada, texto Unicode maior 🚀", "next; next")
    provider.write_text(new_text)
    staged = call(project, store, "stage")
    still_old, _ = run(oracle=False)
    assert still_old["origin"] == "Stored" and still_old["diagnostics"] == orig["diagnostics"]
    assert still_old["source_root"] == orig["source_root"] and staged["pending"] != staged["selected"]
    updated = call(project, store, "submit-pending", "app", "src/main.w")
    assert updated["selected"] == staged["pending"]
    remapped, warm_ms = run()
    assert remapped["origin"] == "Stored" and remapped["receipt"] == orig["receipt"]
    assert remapped["source_root"] != orig["source_root"]
    for label, previous in zip(remapped["diagnostics"][0]["labels"], old_labels):
        span = label["span"]
        assert span["source"] != previous["span"]["source"]
        assert new_text.encode()[span["range"]["start"]:span["range"]["end"]] == b"next"
    provider.write_text(script("origem atualizada, texto Unicode maior 🚀", "next"))
    call(project, store, "stage")
    call(project, store, "submit-pending", "app", "src/main.w")
    fixed, _ = run()
    assert fixed["origin"] == "Derived" and not fixed["diagnostics"] and fixed["receipt"] != orig["receipt"]
    assert not (project / ".wosy/artifacts").exists()
    print("acknowledged SQLite root → real framed in-process leased worker → ordinary compiler diagnostics", {
        "warm_dispatch_ms": round(warm_ms, 3), "resolved_profiles": ["dev", "native"],
        "historic_UTF8": True, "newer_pending_not_selected": True, "cas_rebuilt": True,
        "rooted_files": 19, "cas_bytes": only_blob.stat().st_size,
    })

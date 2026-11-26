"""Real ordinary diagnostic identities anchor only their configured profile's receipt."""
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile


example, daemon_test = (Path(arg).resolve() for arg in sys.argv[1:3])
manifest = (Path(__file__).parent /
            "fixtures/build/shared_stdlib_local_dependency/project/wosy.toml").read_text()
manifest = manifest.replace("categories = []", 'categories = ["WHY"]')
manifest += """
[builders.native]
command = ["unused"]
args = []
toolchain_identity = "native-fixture"

[runners.native]
command = ["unused"]
args = []

[targets.app.artifacts.main.profiles.native]
backend = "llvm"
output = "native"
builder = "native"
run_runner = "native"

[targets.app.artifacts.main.profiles.uncached]
backend = "llvm"
output = "wasm-wasip1"
builder = "llvm-wasm"
run_runner = "wasmtime-wasip1"

[targets.app.artifacts.main.profiles.unsupported]
backend = "not-a-fixture-backend"
output = "wasm-wasip1"
builder = "llvm-wasm"
run_runner = "wasmtime-wasip1"

[targets.app.profiles.native]
main_artifact = "main"

[targets.app.profiles.uncached]
main_artifact = "main"

[targets.app.profiles.unsupported]
main_artifact = "main"
"""
with tempfile.TemporaryDirectory() as temp:
    base = Path(temp).resolve()
    project, store = base / "project", base / "source"
    manifest_path = project / "wosy.toml"
    entry, leaf = project / "src/main.w", project / "stdlib/src/value.w"
    entry.parent.mkdir(parents=True)
    leaf.parent.mkdir(parents=True)
    manifest_path.write_text(manifest)
    (project / "stdlib/wosy.toml").write_text('[package]\nname="std"\nsource_root="src"\n')
    initial = '''%%start
# WHY: origem ☕
dep = namespace std "value.w";
struct Item { u8 value; }
Item(Item) take = fn(input) { next = input; next; next };
%%end
'''
    entry.write_text(initial)
    leaf.write_text("%%start\nu32 value = 1;\n%%end\n")

    def call(*args):
        run = subprocess.run([example, project, store, "--persistent-source", "app", *args],
                             text=True, capture_output=True, check=False, timeout=30)
        assert run.returncode == 0, run.stderr
        return json.loads(run.stdout)

    def accepted():
        pending = call("stage")["pending"]
        assert pending and call("submit-pending", "app", "src/main.w")["selected"] == pending
        return pending

    old = accepted()
    entry.write_text(initial.replace("origem ☕", "anotação revista em português 🚀"))
    annotation = accepted()
    leaf.write_text("%%start\nu32 value = 2;\n%%end\n")
    dependency = accepted()
    manifest_path.write_text(manifest.replace("native-fixture", "native-fixture-config"))
    configuration = accepted()
    assert len({old, annotation, dependency, configuration}) == 4
    result = subprocess.run([daemon_test, "census_requires_exact_configured_profile_layout_key",
                             "--ignored", "--nocapture"], capture_output=True, text=True,
                            check=False, timeout=30, env={**os.environ, "WOSY_TEST_IMAGE_STORE": str(store),
                                "WOSY_TEST_IMAGE_ROOT": old, "WOSY_SEMANTIC_CENSUS_ANNOTATION": annotation,
                                "WOSY_SEMANTIC_CENSUS_DEPENDENCY": dependency,
                                "WOSY_SEMANTIC_CENSUS_CONFIGURATION": configuration})
    assert result.returncode == 0 and "1 passed" in result.stdout, result.stdout + result.stderr
    print("Verified semantic retention for Native64 and WASI; annotation-only reuse with authored remap, "
          "dependency/config separation; absent, wrong-layout, unsupported and malformed receipts not retained.")

"""Exercise opaque type-resolution handoff using real leased compiler workers."""
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile


example, daemon_test = (Path(arg).resolve() for arg in sys.argv[1:3])
template = Path(__file__).parent / "fixtures/build/shared_stdlib_local_dependency/project/wosy.toml"
manifest = template.read_text().replace("categories = []", 'categories = ["WHY"]') + """
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

[targets.app.profiles.native]
main_artifact = "main"
"""
source = """%%start
# WHY: original ☕
dep = namespace std "item.w";
struct Item { u8 value; }
Item(Item) take = fn(input) { next = input; next };
i32 number = dep.value();
%%end
"""
with tempfile.TemporaryDirectory() as temp:
    parent = Path(temp).resolve()
    project, store = parent / "project", parent / "store"
    main = project / "src/main.w"
    main.parent.mkdir(parents=True)
    leaf = project / "stdlib/src/item.w"
    leaf.parent.mkdir(parents=True)
    (project / "wosy.toml").write_text(manifest)
    (project / "stdlib/wosy.toml").write_text('[package]\nname="std"\nsource_root="src"\n')
    leaf.write_text("%%start\nstruct Packet { *?u8 address; u32 count; }\ni32() value = fn { 42 };\n%%end\n")

    def call(*args):
        proc = subprocess.run([example, project, store, "--persistent-source", "app", *args],
                              capture_output=True, text=True, check=False, timeout=30)
        assert proc.returncode == 0, proc.stderr
        return json.loads(proc.stdout)

    def accepted():
        pending = call("stage")["pending"]
        assert pending and call("submit-pending", "app", "src/main.w")["selected"] == pending
        return pending

    main.write_text(source)
    first = accepted()
    main.write_text(source.replace("original ☕", "anotação maior em português 🚀"))
    annotated = accepted()
    assert first != annotated
    pressure = []
    for index in range(5):
        main.write_text(source.replace("original ☕", f"anotação de pressão {index} 🚀"))
        pressure.append(accepted())
    main.write_text(source.replace("%%end", "".join(f"u32 value{i} = {i};\n" for i in range(350)) + "%%end"))
    oversized = accepted()
    assert len({first, annotated, oversized, *pressure}) == 8
    check = subprocess.run(
        [daemon_test, "leased_resolution_reuses_only_compiler_owned_exact_positive_state", "--ignored", "--nocapture"],
        capture_output=True, text=True, check=False, timeout=30,
        env={**os.environ, "WOSY_RESOLUTION_PROJECT": str(project), "WOSY_RESOLUTION_STORE": str(store),
             "WOSY_RESOLUTION_FIRST": first, "WOSY_RESOLUTION_ANNOTATED": annotated,
             "WOSY_RESOLUTION_PRESSURE": ",".join(pressure), "WOSY_RESOLUTION_OVERSIZED": oversized},
    )
    assert check.returncode == 0 and "1 passed" in check.stdout, check.stdout + check.stderr
    assert not (project / ".wosy/artifacts").exists()
    print(check.stdout, end="")

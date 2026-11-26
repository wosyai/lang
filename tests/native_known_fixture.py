"""Shared local project for the NativeKnown diagnostic gates."""
from pathlib import Path


def write_project(project):
    template = Path(__file__).parent / "fixtures/build/shared_stdlib_local_dependency/project/wosy.toml"
    manifest = template.read_text().replace("categories = []", 'categories = ["WHY"]')
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

[targets.app.profiles.native]
main_artifact = "main"
"""
    (project / "src").mkdir(parents=True)
    (project / "stdlib/src").mkdir(parents=True)
    (project / "wosy.toml").write_text(manifest)
    (project / "stdlib/wosy.toml").write_text('[package]\nname="std"\nsource_root="src"\n')
    (project / "src/main.w").write_text('%%start\ndep = namespace std "item.w";\n%%end\n')
    (project / "stdlib/src/item.w").write_text(
        "%%start\n# WHY: sessão local ☕\nstruct Item { u8 value; }\n"
        "Item(Item) take = fn(input) { next = input; next; next };\n%%end\n",
        encoding="utf-8",
    )

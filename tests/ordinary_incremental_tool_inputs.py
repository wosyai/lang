"""Real Native64/Wasm32 Build/Run; compiler proof is not native tool authority."""
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import time

COMPILER = Path(sys.argv[1]).resolve()
WASM_START = Path(__file__).parent / "fixtures/build/shared_stdlib_local_dependency/project/scripts/start.ll"
DEFAULT_ENV = {k: v for k, v in os.environ.items() if k != "WOSY_INCREMENTAL"}
OPT_IN_ENV = {**DEFAULT_ENV, "WOSY_INCREMENTAL": "1"}

BUILD_RECIPE = '''import json
from pathlib import Path
import subprocess
import sys
import tempfile
import time

layout, optimize, partition, output, pending, runtime = sys.argv[1:]
manifest = json.loads(Path(pending).read_bytes())
version = "recipe-v1"
started = time.perf_counter_ns()
if layout == "native":
    subprocess.run(["clang", optimize, "-x", "ir", partition, runtime, "-o", output + "/main"], check=True)
else:
    with tempfile.TemporaryDirectory(dir=output) as stage:
        operands = [(partition, "main.o"), (runtime, "runtime.o")]
        exports = ["--export-all"] if layout == "wasm-export" else ["--export=_start"]
        if layout != "wasm-export":
            operands.append(("start.ll", "start.o"))
        objects = []
        for source, filename in operands:
            dest = str(Path(stage, filename))
            subprocess.run(["clang", optimize, "--target=wasm32-wasi", "-c", "-x", "ir",
                source, "-o", dest], check=True)
            objects.append(dest)
        subprocess.run(["wasm-ld", "--no-entry", "--allow-undefined", *exports, *objects,
            "-o", output + "/main"], check=True)
with Path("build-ledger").open("a", encoding="utf-8") as ledger:
    print(json.dumps([version, optimize, manifest["identity"]["builder_toolchain_identity"],
        (time.perf_counter_ns() - started) / 1e6]), file=ledger)
'''

RUN_RECIPE = '''import json
from pathlib import Path
import subprocess
import sys

layout, label, manifest, delimiter, *arguments = sys.argv[1:]
assert delimiter == "--"
version = "runner-v1"
executable = json.loads(Path(manifest).read_bytes())["executable"]
if layout == "native":
    subprocess.run([executable, *arguments], check=True)
elif layout == "wasm-export":
    result = subprocess.run(["wasmtime", "run", "--invoke", label, executable, *arguments],
        capture_output=True, check=True)
    assert result.stdout.strip() == b"34", result.stdout
else:
    subprocess.run(["wasmtime", "run", executable, *arguments], check=True)
with Path("run-ledger").open("a", encoding="utf-8") as ledger:
    print(json.dumps([version, label]), file=ledger)
'''

def setup(root, layout, output):
    root.mkdir()
    (root / "main.w").write_text("%%start\ni32 value = 34;\n%%end\n")
    (root / "builder.py").write_text(BUILD_RECIPE)
    (root / "runner.py").write_text(RUN_RECIPE)
    if layout == "wasm":
        shutil.copyfile(WASM_START, root / "start.ll")
    (root / "wosy.toml").write_text(f"""
[package]
name = "accounted"
default_target = "app"
[comments]
categories = []
[builders.compiler]
command = ["python3", "builder.py"]
args = ["{layout}", "-O0"]
toolchain_identity = "declared-llvm-initial"
[runners.actual]
command = ["python3", "runner.py"]
args = ["{layout}", "alpha"]
[targets.app]
root = "main.w"
[targets.app.artifacts.main]
artifact = "Main"
[targets.app.artifacts.main.profiles.dev]
backend = "llvm"
output = "{output}"
builder = "compiler"
run_runner = "actual"
[targets.app.artifacts.main.profiles.verify]
backend = "llvm"
output = "{output}"
builder = "compiler"
run_runner = "actual"
[targets.app.profiles.dev]
main_artifact = "main"
[targets.app.profiles.verify]
main_artifact = "main"
""")

def read_ledger(root, name):
    path = root / name
    return [json.loads(line) for line in path.read_text().splitlines()] if path.exists() else []

def invoke(root, command, profile="dev", opt_in=True, status=0):
    started = time.perf_counter_ns()
    proc = subprocess.run([COMPILER, command, "app", "--profile", profile], cwd=root,
        env=OPT_IN_ENV if opt_in else DEFAULT_ENV, capture_output=True, text=True, timeout=45)
    assert proc.returncode == status, (command, profile, proc.returncode, proc.stderr)
    return proc.stderr, (time.perf_counter_ns() - started) / 1e6

def artifact(root, profile="dev"):
    disk = root / f".wosy/artifacts/app/{profile}/main/incremental/artifact-manifest.json"
    current = json.loads(disk.read_bytes())
    binary = Path(current["executable"])
    assert binary.is_file() and binary.with_name("compiler-partition-v1").is_file()
    digest = hashlib.sha256(binary.with_name("partition.ll").read_bytes()).hexdigest()
    return current, digest

def build(root, expected_reuse, profile="dev"):
    calls = len(read_ledger(root, "build-ledger"))
    message, elapsed = invoke(root, "build", profile)
    assert ("reused compiler LLVM" in message) == expected_reuse, message
    assert ("owned compiler process" in message) != expected_reuse, message
    current, digest = artifact(root, profile)
    ledger = read_ledger(root, "build-ledger")
    assert len(ledger) == calls + 1, ledger
    return current, digest, elapsed, ledger[-1]

def edit(root, file, old, new):
    path = root / file
    saved = path.read_text()
    assert saved.count(old) == 1
    path.write_text(saved.replace(old, new))
    return saved

def require_runner(root, version, choice, profile="dev"):
    calls = len(read_ledger(root, "run-ledger"))
    _, elapsed = invoke(root, "run", profile)
    ledger = read_ledger(root, "run-ledger")
    assert len(ledger) == calls + 1 and ledger[-1] == [version, choice]
    return elapsed

def expect_stale(root):
    failure, _ = invoke(root, "run", status=1)
    assert "artifact manifest is stale" in failure, failure

def verify(root, layout, output):
    setup(root, layout, output)
    default, _ = invoke(root, "build", opt_in=False)
    invoke(root, "run", opt_in=False)
    assert "incremental" not in default and not (root / ".wosy/client.sqlite").exists()
    assert len(read_ledger(root, "build-ledger")) == 1

    initial, digest, cold_ms, first_recipe = build(root, False)
    assert first_recipe[:3] == ["recipe-v1", "-O0", "declared-llvm-initial"]
    require_runner(root, "runner-v1", "alpha")
    _, warm_digest, warm_ms, _ = build(root, True)
    assert warm_digest == digest

    edit(root, "wosy.toml", '"alpha"', '"beta"')
    expect_stale(root)
    runner_change, runner_digest, runner_ms, _ = build(root, True)
    assert runner_digest == digest
    assert runner_change["identity"]["runner_identity"] != initial["identity"]["runner_identity"]
    run_ms = require_runner(root, "runner-v1", "beta")

    edit(root, "wosy.toml", '"declared-llvm-initial"', '"same-label-not-a-tool-proof"')
    toolchain_change, toolchain_digest, toolchain_ms, actual = build(root, True)
    assert toolchain_digest == digest and actual[:3] == ["recipe-v1", "-O0", "same-label-not-a-tool-proof"]
    assert toolchain_change["identity"]["builder_toolchain_identity"] != initial["identity"]["builder_toolchain_identity"]
    require_runner(root, "runner-v1", "beta")
    edit(root, "wosy.toml", '"-O0"', '"-O1"')
    changed_builder, changed_builder_digest, builder_ms, actual = build(root, True)
    assert changed_builder_digest == digest and actual[1] == "-O1"
    assert changed_builder["identity"]["builder_identity"] != toolchain_change["identity"]["builder_identity"]
    require_runner(root, "runner-v1", "beta")

    # Neither unchanged declared label nor same command path freezes bytes.
    # Real tools are called again, and Run uses the current configured recipe.
    edit(root, "builder.py", '"recipe-v1"', '"recipe-v2"')
    same_label, same_digest, same_label_ms, actual = build(root, True)
    assert same_digest == digest and actual[:3] == ["recipe-v2", "-O1", "same-label-not-a-tool-proof"]
    assert same_label["identity"] == changed_builder["identity"]
    assert same_label["executable"] != changed_builder["executable"]
    edit(root, "runner.py", '"runner-v1"', '"runner-v2"')
    require_runner(root, "runner-v2", "beta")

    _, profile_digest, _, _ = build(root, False, "verify")
    assert profile_digest == digest
    build(root, True, "verify")
    require_runner(root, "runner-v2", "beta", "verify")

    source = edit(root, "main.w", "34", "37")
    expect_stale(root)
    _, changed_digest, _, _ = build(root, False)
    assert changed_digest != digest
    require_runner(root, "runner-v2", "beta")
    (root / "main.w").write_text(source)
    _, restored_digest, _, _ = build(root, False)
    assert restored_digest == digest
    require_runner(root, "runner-v2", "beta")

    # Grammar policy is part of the compiler projection even when the
    # particular program contains no comments and emits identical bytes.
    edit(root, "wosy.toml", "categories = []", 'categories = ["WHY"]')
    _, grammar_digest, _, _ = build(root, False)
    assert grammar_digest == digest
    require_runner(root, "runner-v2", "beta")
    assert len(read_ledger(root, "build-ledger")) == 12
    print(json.dumps({"layout": layout, "status": "passed", "actual_builders": 12,
        "default_source_state_untouched": True, "elapsed_ms": {"cold": cold_ms, "warm": warm_ms,
        "runner_config_edit": runner_ms, "toolchain_label_edit": toolchain_ms,
        "builder_args_edit": builder_ms, "builder_same_path_edit": same_label_ms, "real_run": run_ms}}))

def setup_exports(root):
    setup(root, "wasm-export", "wasm-wasip1")
    path = root / "wosy.toml"
    raw = path.read_text().replace('"alpha"', '"first_export"').replace(
        "[targets.app.profiles.dev]",
        '[targets.app.artifacts.main.profiles.dev.exports]\nfirst_export = "value"\n[targets.app.profiles.dev]',
    )
    path.write_text(raw)
    (root / "main.w").write_text("%%start\ni32() value = fn { 34 };\n%%end\n")

def verify_exports(root):
    setup_exports(root)
    _, first_digest, _, _ = build(root, False)
    require_runner(root, "runner-v1", "first_export")
    assert build(root, True)[1] == first_digest
    path = root / "wosy.toml"
    saved = path.read_text()
    assert saved.count("first_export") == 2
    path.write_text(saved.replace("first_export", "new_export"))
    expect_stale(root)
    _, new_digest, _, _ = build(root, False)
    assert new_digest != first_digest
    require_runner(root, "runner-v1", "new_export")
    assert build(root, True)[1] == new_digest
    assert len(read_ledger(root, "build-ledger")) == 4
    print(json.dumps({"wasm_export_identity": "changed compiler input; new export actually executed", "actual_builders": 4}))

def verify_layout_switch(root):
    setup(root, "native", "native")
    path = root / "wosy.toml"
    saved = path.read_text()
    native, native_digest, _, _ = build(root, False)
    first = Path(native["executable"])
    native_bytes = first.read_bytes()
    assert native_bytes.startswith(b"\x7fELF")
    require_runner(root, "runner-v1", "alpha")
    assert build(root, True)[1] == native_digest
    shutil.copyfile(WASM_START, root / "start.ll")
    path.write_text(saved.replace('"native"', '"wasm"').replace('output = "wasm"', 'output = "wasm-wasip1"'))
    expect_stale(root)
    wasm, wasm_digest, _, _ = build(root, False)
    assert wasm_digest != native_digest and Path(wasm["executable"]).read_bytes().startswith(b"\x00asm")
    require_runner(root, "runner-v1", "alpha")
    assert build(root, True)[1] == wasm_digest
    path.write_text(saved)
    expect_stale(root)
    _, restored_digest, _, _ = build(root, False)
    assert restored_digest == native_digest and first.read_bytes() == native_bytes
    require_runner(root, "runner-v1", "alpha")
    assert len(read_ledger(root, "build-ledger")) == 5
    print(json.dumps({"same_profile_layout_switch": "Native64 to Wasm32 and back", "actual_builders": 5}))

with tempfile.TemporaryDirectory(prefix="wosy-accounted-tool-inputs-") as directory:
    root = Path(directory)
    for layout, output in (("native", "native"), ("wasm", "wasm-wasip1")):
        verify(root / layout, layout, output)
    verify_exports(root / "exports")
    verify_layout_switch(root / "layout-switch")

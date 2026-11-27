"""Private controlled-driver publication gate. Not a shipping builder or reuse path."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile


def verified_tool(spec):
    name, expected = spec
    path = Path(name)
    assert path.is_absolute() and path.is_file()
    assert hashlib.sha256(path.read_bytes()).hexdigest() == expected, path
    # Preserve wasm-ld's driver basename; do not execute the resolved 'lld'.
    return path


parser = argparse.ArgumentParser()
parser.add_argument("example", type=Path)
parser.add_argument("ordinary", type=Path)
parser.add_argument("--clang", nargs=2, metavar=("PATH", "SHA256"), required=True)
parser.add_argument("--wasm-ld", nargs=2, metavar=("PATH", "SHA256"), required=True)
parser.add_argument("--wasmtime", nargs=2, metavar=("PATH", "SHA256"), required=True)
parser.add_argument("--llvm-libs", type=Path, required=True)
args = parser.parse_args()
example, ordinary = args.example.resolve(strict=True), args.ordinary.resolve(strict=True)
clang, linker, runner = map(verified_tool, (args.clang, args.wasm_ld, args.wasmtime))
assert args.llvm_libs.is_absolute() and args.llvm_libs.is_dir()
bootstrap = (Path(__file__).parent / "fixtures/build/shared_stdlib_local_dependency/project/scripts/start.ll").read_bytes()


with tempfile.TemporaryDirectory(prefix="wosy-controlled-wasi-") as temp:
    scratch = Path(temp).resolve()
    root = scratch / "project"
    source = root / "main.w"
    configuration = root / "wosy.toml"
    output = root / ".wosy/artifacts/app/dev/main"
    final = output / "artifact-manifest.json"
    store = scratch / "catalog"
    root.mkdir()
    configuration.write_text("""
[package]
name = "app"
default_target = "app"
[comments]
categories = ["WHY"]
[builders.test]
command = ["never-the-ordinary-builder"]
toolchain_identity = "controlled scratch gate only"
[runners.test]
command = [""" + json.dumps(str(runner)) + """]
args = []
[targets.app]
root = "main.w"
[targets.app.artifacts.main]
artifact = "Main"
[targets.app.artifacts.main.profiles.dev]
backend = "llvm"
output = "wasm-wasip1"
builder = "test"
run_runner = "test"
[targets.app.artifacts.main.profiles.dev.exports]
fixture_answer = "answer"
[targets.app.profiles.dev]
main_artifact = "main"
""")
    # The real ordinary Run expects its runner to consume the manifest. Use
    # the repository's fixture adapter, with interpreter and Wasmtime explicit.
    # Retain the same manifest from source capture to final Run.
    wasmtime_adapter = root / "fixture-runner.py"
    wasmtime_adapter.write_text("""import json, subprocess, sys
from pathlib import Path
manifest = json.loads(Path(sys.argv[2]).read_bytes())
assert sys.argv[3] == '--'
raise SystemExit(subprocess.run([sys.argv[1], "run", manifest["executable"], *sys.argv[4:]]).returncode)
""")
    text = configuration.read_text().replace(
        "command = [" + json.dumps(str(runner)) + "]",
        "command = " + json.dumps([os.path.realpath(os.sys.executable), str(wasmtime_adapter), str(runner)]),
    )
    configuration.write_text(text)
    authored = b"%%start\n# WHY: known fixture\ni32() answer = fn { 42 };\n%%end\n"
    source.write_bytes(authored)

    def state(*parts):
        out = subprocess.check_output([example, root, store, "--persistent-source", "app", *map(str, parts)],
                                      cwd=root, timeout=30)
        return json.loads(out)

    def run(stale=False):
        result = subprocess.run([ordinary, "run", "app", "--profile", "dev"], cwd=root, timeout=30,
                                env=tools_env, capture_output=True)
        if stale:
            assert result.returncode == 1 and result.stderr == b"wosy: artifact manifest is stale\n", result
        else:
            assert result.returncode == 0 and not result.stdout and not result.stderr, result

    tools_env = {"LD_LIBRARY_PATH": str(args.llvm_libs), "XDG_CACHE_HOME": str(scratch / "cache"),
                 "LANG": "C", "NO_COLOR": "1"}
    staged = state("stage")["pending"]
    assert state("submit-pending", "app", "main.w")["selected"] == staged
    source_inputs = (source, configuration, wasmtime_adapter)
    exact_inputs = tuple(path.read_bytes() for path in source_inputs)
    invoked = []

    def generation(name, fail_link=False, before_commit=None):
        destination = output / name
        plan = state("executable-builder-inputs", "dev", destination, example)["result"]
        partition, directory, pending, runtime = map(Path, plan["arguments"])
        assert directory == destination and not directory.exists()
        assert all(path.parent == directory for path in (partition, pending, runtime))
        manifest = json.loads(plan["pending"])
        assert manifest["identity"]["artifact_identity"] == "app:dev:Main"
        assert manifest["identity"]["llvm_input_identity"] == plan["executable_partition_digest"]
        assert Path(manifest["executable"]) == destination / "main"
        assert tuple(path.read_bytes() for path in source_inputs) == exact_inputs
        directory.mkdir(parents=True)
        committed = False
        temporary_manifest = output / ".fixture-next-manifest"
        try:
            start = directory / "start.ll"
            for path, data in ((partition, plan["partition"].encode()), (runtime, plan["runtime"].encode()),
                               (start, bootstrap)):
                path.write_bytes(data)
            invoked.append(name)
            objects = []
            for ir in (partition, runtime, start):
                obj = ir.with_suffix(".o")
                subprocess.run([clang, "--target=wasm32-wasi", "-c", "-x", "ir", ir, "-o", obj],
                               cwd=directory, env=tools_env, check=True, capture_output=True, timeout=30)
                objects.append(obj)
            if fail_link:
                objects[0].unlink()
            binary = directory / "main"
            result = subprocess.run([linker, "--no-entry", "--allow-undefined", "--export=_start",
                                     "--export=fixture_answer", *objects, "-o", binary],
                                    cwd=directory, env=tools_env, capture_output=True, timeout=30)
            if fail_link:
                assert result.returncode and not binary.exists(), result
                return "link-failed"
            result.check_returncode()
            assert binary.is_file() and binary.read_bytes().startswith(b"\x00asm")
            if before_commit:
                before_commit()
            if tuple(path.read_bytes() for path in source_inputs) != exact_inputs:
                return "source-changed"
            temporary_manifest.write_text(plan["pending"])
            temporary_manifest.replace(final)
            committed = True
            return "published"
        finally:
            if not committed:
                temporary_manifest.unlink(missing_ok=True)
                shutil.rmtree(directory)

    assert generation("cold") == "published"
    assert invoked == ["cold"]
    run()
    cold_binary = output / "cold/main"
    baseline = (final.read_bytes(), cold_binary.read_bytes(), cold_binary.stat().st_ino)
    export = subprocess.check_output([runner, "run", "--invoke", "fixture_answer", cold_binary],
                                     cwd=root, env=tools_env, timeout=30)
    assert export.strip() == b"42"

    assert generation("failed", fail_link=True) == "link-failed"
    assert not (output / "failed").exists()
    assert (final.read_bytes(), cold_binary.read_bytes(), cold_binary.stat().st_ino) == baseline
    run()
    assert generation("stale", before_commit=lambda: source.write_bytes(authored.replace(b"42", b"43"))) == "source-changed"
    assert not (output / "stale").exists()
    assert (final.read_bytes(), cold_binary.read_bytes(), cold_binary.stat().st_ino) == baseline
    run(stale=True)
    source.write_bytes(authored)
    run()
    assert invoked == ["cold", "failed", "stale"]
    assert sorted(path.name for path in output.iterdir()) == ["artifact-manifest.json", "cold"]
    print("Controlled WASI scratch gate: real cold link and export 42, actual linker failure and stale source"
          " preserve the previous binary/manifest, real Run accepts only matching observations.")

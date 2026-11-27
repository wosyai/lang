"""Independent real builder receipt and execution of opt-in no-cache inputs."""
import json
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile


example, ordinary = (Path(arg).resolve() for arg in sys.argv[1:3])
fixtures = Path(__file__).resolve().parent / "fixtures/build"
with tempfile.TemporaryDirectory() as temp:
    base = Path(temp).resolve()
    project, store = base / "project", base / "source"
    shutil.copytree(fixtures / "short_circuit_project/project", project)
    scripts = project / "scripts"
    for name in ("build-wasm", "run-wasmtime", "start.ll"):
        shutil.copyfile(fixtures / "shared_stdlib_local_dependency/project/scripts" / name, scripts / name)
    wasm_builder = scripts / "build-wasm"
    wasm_builder.write_text(wasm_builder.read_text().replace('"--export=_start",',
                                                             '"--export=_start", "--export=fixture_answer",'))
    entry = project / "src/main.w"
    entry.write_text(entry.read_text().replace("%%end", "i32() answer = fn { 42 };\n%%end"))
    child = project / "src/child.w"
    child.write_text(child.read_text().replace("i32 calls", "# WHY: original note\ni32 calls"))
    profile = project / "wosy.toml"
    text = profile.read_text().replace("categories = []", 'categories = ["WHY"]')
    text = text.replace('command = ["python", "scripts/build-native"]\nargs = []',
                        'command = ["python", "scripts/capture-native"]\nargs = ["native-profile"]')
    profile.write_text(text + """
[builders.wasm]
command = ["python", "scripts/capture-wasm"]
args = ["wasm-profile"]
toolchain_identity = "llvm-18.1.3"

[runners.wasmtime]
command = ["python", "scripts/run-wasmtime"]
args = []

[targets.app.artifacts.main.profiles.wasm]
backend = "llvm"
output = "wasm-wasip1"
builder = "wasm"
run_runner = "wasmtime"

[targets.app.artifacts.main.profiles.wasm.exports]
fixture_answer = "answer"

[targets.app.profiles.wasm]
main_artifact = "main"
""")
    # Record both the original command's arguments and the real pending
    # manifest from the builder process, then delegate to the ordinary
    # repository fixture builder with its unchanged four protected inputs.
    for backend in ("native", "wasm"):
        (scripts / f"capture-{backend}").write_text(f"""
import json
from pathlib import Path
import subprocess
import sys
assert sys.argv[1] == "{backend}-profile"
partition, output, pending, runtime = sys.argv[2:]
Path(output, "builder-observation.json").write_text(json.dumps({{
    "argv": sys.argv,
    "pending": Path(pending).read_text(),
    "runtime": Path(runtime).read_text(),
    "partition": Path(partition).read_text(),
}}))
subprocess.run([sys.executable, "scripts/build-{backend}", *sys.argv[2:]], check=True)
""")

    def call(*args, refusal=None, database=store):
        proc = subprocess.run([example, project, database, "--persistent-source", "app", *map(str, args)],
                              capture_output=True, text=True, check=False, timeout=30)
        if refusal:
            assert proc.returncode != 0 and not proc.stdout and refusal in proc.stderr, (proc.stdout, proc.stderr)
            return
        assert proc.returncode == 0, proc.stderr
        return json.loads(proc.stdout)

    pending = call("stage")["pending"]
    receipt = call("submit-pending", "short_circuit_project", "src/main.w")
    assert receipt["selected"] == pending
    before = [(project / child).read_bytes() for child in (".wosy/client.sqlite", ".wosy/cst/nodes.sqlite")]
    empty = base / "unknown-source"
    blocked = base / "blocked"
    for expected in (ordinary, example):
        terminal = call("ordinary-builder-inputs", "dev", blocked, expected, database=empty)
        assert terminal["result"] == {"kind": "replay-required", "opens": 1, "known": False}
        assert terminal["selected"] == pending
        assert not blocked.exists()
    call("ordinary-builder-inputs", "dev", base / "refused", ordinary,
         refusal="different compiler image")
    assert not (base / "refused").exists()

    def plan(name, destination):
        result = call("ordinary-builder-inputs", name, destination, example)["result"]
        assert result["root"] == pending and result["project"] == str(project)
        assert not destination.exists()
        return result

    timing = {}
    for name, backend, runner, magic in (("dev", "native", "native", b"\x7fELF"),
                                         ("wasm", "wasm", "wasmtime", b"\x00asm")):
        dest = project / ".wosy/artifacts/app" / name / "main"
        selected = plan(name, dest)
        cli = subprocess.run([ordinary, "build", "app", "--profile", name], cwd=project, capture_output=True, timeout=30)
        assert cli.returncode == 0, cli.stderr
        args = selected["command"][1:] + selected["configured_arguments"] + selected["arguments"]
        observed = json.loads((dest / "builder-observation.json").read_bytes())
        assert observed["argv"] == args
        assert isinstance(selected["pending"], str)
        for key in ("pending", "partition", "runtime"):
            assert selected[key] == observed[key], key
        assert (dest / "main").read_bytes().startswith(magic)
        # Independently execute the reviewed ordinary builder/runner with
        # the plan in a disposable directory, not the project's artifact.
        sandbox = base / f"builder-{backend}"
        opt = plan(name, sandbox)
        partition, out, pending_manifest, runtime = map(Path, opt["arguments"])
        assert out == sandbox and out.parent == base
        out.mkdir()
        for path, data in zip((partition, pending_manifest, runtime), (opt["partition"], opt["pending"], opt["runtime"])):
            assert path.parent == sandbox
            path.write_text(data)
        command = opt["command"] + opt["configured_arguments"] + opt["arguments"]
        build = subprocess.run(command, cwd=project, capture_output=True, timeout=30)
        assert build.returncode == 0, build.stderr
        assert (sandbox / "main").read_bytes().startswith(magic)
        run = subprocess.run(["python", f"scripts/run-{runner}", pending_manifest, "--"], cwd=project,
                             capture_output=True, timeout=30)
        assert run.returncode == 0 and not run.stdout and not run.stderr, run
        assert not (sandbox / "artifact-manifest.json").exists()
        timing[name] = round(selected["elapsed_ms"], 3)

    for held, sibling in (("dev", "wasm"), ("wasm", "dev")):
        output = base / f"cancel-{held}-keep-{sibling}"
        isolated = plan(sibling, output)
        observed = call("ordinary-builder-inputs-cancel", held, sibling, output, example)
        assert observed["selected"] == pending and observed["pending"] is None
        concurrent = observed["result"]
        assert concurrent["canceled"] == "PeerClosed" and concurrent["sibling"] == "Disconnected"
        assert not output.exists() and not output.with_suffix(".held").exists()
        assert ({key: value for key, value in isolated.items() if key != "elapsed_ms"} ==
                {key: value for key, value in concurrent["plan"].items() if key != "elapsed_ms"})

    assert before == [(project / child).read_bytes() for child in (".wosy/client.sqlite", ".wosy/cst/nodes.sqlite")]
    print("V2 no-cache plan equals ordinary pending manifest, argv and both IR inputs; valid disposable Native64/WASI builder and runner,"
          " independent known-image refusal, no unknown-source replay, no opt-in write or publication.", timing)
    print("Cancel after the real LLVM reply is held: Native64 and WASI in both roles, same lease scope and unchanged SQLite,"
          " surviving plan byte-equal to isolated, actual PeerClosed plus Disconnected, no output.")

    executable = base / "experimental"
    call("executable-builder-inputs", "dev", executable, ordinary, refusal="different compiler image")
    assert not executable.exists()
    assert call("executable-builder-inputs", "dev", blocked, example, database=empty)["result"]["kind"] == "replay-required"
    assert not blocked.exists()

    def executable_plans():
        return {name: call("executable-builder-inputs", name, executable, example)["result"]
                for name in ("dev", "wasm")}

    initial = executable_plans()
    assert not executable.exists()
    child.write_text(child.read_text().replace("original note", "updated annotation with a longer body ☕"))
    call("stage")
    call("submit-pending", "short_circuit_project", "src/main.w")
    annotated = executable_plans()
    for name in initial:
        assert annotated[name]["root"] != initial[name]["root"]
        assert annotated[name]["executable_partition_digest"] == initial[name]["executable_partition_digest"]
        assert annotated[name]["partition"] == initial[name]["partition"]
    child.write_text(child.read_text().replace("i32 calls", "# WHY: inserted annotation\ni32 calls"))
    call("stage")
    call("submit-pending", "short_circuit_project", "src/main.w")
    inserted = executable_plans()
    for name in initial:
        assert inserted[name]["root"] != annotated[name]["root"]
        assert inserted[name]["executable_partition_digest"] == annotated[name]["executable_partition_digest"]
        assert inserted[name]["partition"] == annotated[name]["partition"]
    child.write_text(child.read_text().replace("calls + 1", "calls + 2"))
    entry.write_text(entry.read_text().replace("fn { 42 }", "fn { 43 }"))
    call("stage")
    call("submit-pending", "short_circuit_project", "src/main.w")
    changed = executable_plans()
    assert not executable.exists()
    for name, runner, magic in (("dev", "native", b"\x7fELF"), ("wasm", "wasmtime", b"\x00asm")):
        plan = changed[name]
        assert plan["executable_partition_digest"] != inserted[name]["executable_partition_digest"]
        partition, output, pending_manifest, runtime = map(Path, plan["arguments"])
        assert output == executable
        output.mkdir()
        for path, field in ((partition, "partition"), (pending_manifest, "pending"), (runtime, "runtime")):
            path.write_text(plan[field])
        subprocess.run(plan["command"] + plan["configured_arguments"] + plan["arguments"],
                       cwd=project, check=True, capture_output=True, timeout=30)
        assert (output / "main").read_bytes().startswith(magic)
        subprocess.run(["python", f"scripts/run-{runner}", pending_manifest, "--"],
                       cwd=project, check=True, capture_output=True, timeout=30)
        assert not (output / "artifact-manifest.json").exists()
        if name == "wasm":
            export = subprocess.check_output(["wasmtime", "run", "--invoke", "fixture_answer", output / "main"],
                                             timeout=30)
            assert export.strip() == b"43"
        shutil.rmtree(output)
    print("V3: comment body edit and comment line insertion preserve normalized bytes and digest; code change alters digest;"
          " disposable Native64 and WASI link and run; no publication.")

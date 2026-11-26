"""Real live selected client ↔ daemon authored IR and ordinary linked parity."""
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
    for filename in ("build-wasm", "run-wasmtime", "start.ll"):
        shutil.copyfile(fixtures / "shared_stdlib_local_dependency/project/scripts" / filename, project / "scripts" / filename)
    path = project / "wosy.toml"
    path.write_text(path.read_text().replace("categories = []", 'categories = ["WHY"]') + """
[builders.wasm]
command = ["python", "scripts/build-wasm"]
args = []
toolchain_identity = "llvm-18.1.3"

[runners.wasmtime]
command = ["python", "scripts/run-wasmtime"]
args = []

[targets.app.artifacts.main.profiles.wasm]
backend = "llvm"
output = "wasm-wasip1"
builder = "wasm"
run_runner = "wasmtime"

[targets.app.profiles.wasm]
main_artifact = "main"
""")
    entry = project / "src/main.w"
    original = entry.read_text().replace("%%start\n", "%%start\n# WHY: origem em português ☕\n", 1)
    alternate = original.replace("origem em português ☕", "segunda anotação com conteúdo 🚀")
    entry.write_text(original)

    def call(*args, database=store):
        result = subprocess.run([example, project, database, "--persistent-source", "app", *args],
                                capture_output=True, text=True, check=False, timeout=30)
        assert result.returncode == 0, result.stderr
        return json.loads(result.stdout)

    def query(profile="dev", mode="paired", database=store):
        return call("framed-ordinary-llvm", profile, mode, database=database)["result"]

    def assert_refused(view, kind, terminals):
        assert view["answer"] == {"kind": kind}
        assert view["opens"] == terminals

    assert_refused(query(), "not-acknowledged", [])
    pending = call("stage")["pending"]
    assert pending and call("submit-pending", "short_circuit_project", "src/main.w")["selected"] == pending
    assert_refused(query("nonexistent"), "profile-not-selected", [])
    assert_refused(query(mode="diagnostics-only"), "not-authorized-or-unavailable", [[False, "Disconnected"]])
    assert_refused(query(mode="server-limit"), "not-authorized-or-unavailable", [[False, "Disconnected"]])
    assert_refused(query(mode="client-limit"), "client-limit", [[False, "PeerClosed"]])
    assert_refused(query(mode="ignore-replay", database=base / "deliberately-not-restored"), "replay-not-accepted",
                   [[False, "ReplayRequired"], [True, "ReplayRequired"]])
    entry.write_text(alternate)
    assert_refused(query(), "workspace-changed", [])
    entry.write_text(original)

    assert not (project / ".wosy/artifacts").exists()
    timings = {}
    for profile, magic in (("dev", b"\x7fELF"), ("wasm", b"\x00asm")):
        cold_catalog = base / f"restore-{profile}"
        first = query(profile, database=cold_catalog)
        second = query(profile, database=cold_catalog)
        assert first["opens"] == [[False, "ReplayRequired"], [True, "Disconnected"]]
        assert second["opens"] == [[False, "Disconnected"]]
        assert first["answer"] == second["answer"]
        rooted = first["answer"]
        assert rooted["kind"] == "partition" and rooted["root"] == pending and rooted["profile"] == profile
        assert len(bytes.fromhex(rooted["digest"])) == len(bytes.fromhex(rooted["compiler_image"])) == 32
        artifact = project / ".wosy/artifacts/app" / profile / "main"
        assert not artifact.exists()
        for operation in ("build", "run"):
            result = subprocess.run([ordinary, operation, "app", "--profile", profile], capture_output=True,
                                    check=False, timeout=30, cwd=project)
            assert result.returncode == 0, result.stderr
            if operation == "run":
                assert not result.stdout and not result.stderr
        assert (artifact / "main").read_bytes().startswith(magic)
        assert (artifact / "partition.ll").read_bytes() == rooted["partition"].encode()
        manifest = json.loads((artifact / "artifact-manifest.json").read_bytes())
        assert manifest["identity"]["llvm_input_identity"] == rooted["digest"]
        timings[profile] = dict(cold_ms=round(first["elapsed_ms"], 3), warm_ms=round(second["elapsed_ms"], 3),
                                partition_bytes=len(rooted["partition"].encode()))

    stale = call("framed-ordinary-llvm-stale", "dev", alternate)["result"]
    assert stale["kind"] == "stale" and stale["root"] == pending
    assert stale["latest_generation"] > stale["initial_generation"]
    fresh = query()
    assert fresh["answer"]["generation"] == stale["latest_generation"]
    assert fresh["answer"]["root"] == pending
    print("Native64/WASI actual framed LLVM equals ordinary linked bytes/digest; fresh store replay, exact generation ABA,"
          "Live mismatch, capability and both byte limits verified.", timings)

"""Linux opt-in CLI: custody, local-dependency generations and sampled stage costs."""
import fcntl
import hashlib
import json
import os
from pathlib import Path
import shutil
import sqlite3
import statistics
import subprocess
import sys
import tempfile
import time

COMPILER = Path(sys.argv[1]).resolve()
FIXTURE = Path(__file__).parent / "fixtures/build/short_circuit_project/project"
DEFAULT_ENV = {key: val for key, val in os.environ.items() if key != "WOSY_INCREMENTAL"}
OPT_IN_ENV = {**DEFAULT_ENV, "WOSY_INCREMENTAL": "1"}

BUILDER = '''from pathlib import Path
import time

partition, output_dir, manifest, runtime = sys.argv[1:]
if Path("fail-builder").exists():
    Path(output_dir, "main").write_bytes(b"unpublished incomplete fixture output")
    print("fixture builder stopped after partial output", file=sys.stderr)
    sys.exit(72)
started = time.perf_counter_ns()
subprocess.run(["clang", "-x", "ir", partition, runtime, "-o", output_dir + "/main"], check=True)
with open("builder-ms", "a", encoding="utf-8") as ledger:
    print((time.perf_counter_ns() - started) / 1e6, file=ledger)
'''

RUNNER_HOLD = '''from pathlib import Path
import time

if Path("hold-runner").exists():
    Path("runner-captured").write_text(manifest["executable"])
    deadline = time.monotonic() + 10
    while not Path("release-runner").exists():
        if time.monotonic() >= deadline:
            sys.exit(79)
        time.sleep(.01)
'''

def setup(root, dependency_functions=0):
    shutil.copytree(FIXTURE, root)
    build = root / "scripts/build-native"
    build.write_text(build.read_text().split("partition, output_dir", 1)[0] + BUILDER)
    runner = root / "scripts/run-native"
    runner.write_text(runner.read_text().replace("result = subprocess.run(", RUNNER_HOLD + "\nresult = subprocess.run("))
    source = root / "src/main.w"
    source.write_text(source.read_text().replace('namespace short_circuit_project "src/child.w"', 'namespace dependency "child.w"'))
    dependency = root / "dependency"
    (dependency / "src").mkdir(parents=True)
    (dependency / "wosy.toml").write_text('[package]\nname = "dependency"\nsource_root = "src"\n')
    authored = (root / "src/child.w").read_text()
    if dependency_functions:
        authored = authored.replace("%%end", "\n".join(
            f"i32() unused_{index:04d} = fn {{ {index} }};" for index in range(dependency_functions)
        ) + "\n%%end")
    (dependency / "src/child.w").write_text(authored)
    with (root / "wosy.toml").open("a") as manifest:
        manifest.write('''
[dependencies.dependency]
path = "dependency"
[targets.app.artifacts.main.profiles.verify]
backend = "llvm"
output = "native"
builder = "native"
run_runner = "native"
[targets.app.profiles.verify]
main_artifact = "main"
''')

def command(root, verb, profile="dev", opted=True, status=0):
    started = time.perf_counter_ns()
    result = subprocess.run([COMPILER, verb, "app", "--profile", profile],
        cwd=root, env=OPT_IN_ENV if opted else DEFAULT_ENV, capture_output=True, text=True, timeout=30)
    assert result.returncode == status, result.stderr
    return result.stderr, (time.perf_counter_ns() - started) / 1e6

def pointer(root, profile="dev"):
    return root / f".wosy/artifacts/app/{profile}/main/incremental/artifact-manifest.json"

def current(root, profile="dev"):
    return json.loads(pointer(root, profile).read_bytes())

def generation(root):
    path = (root / ".wosy/client.sqlite").as_uri() + "?mode=ro"
    with sqlite3.connect(path, uri=True) as connection:
        return connection.execute("select selected, selected_generation from source_targets where target = ?", ("app",)).fetchone()

def partition(manifest):
    return hashlib.sha256(Path(manifest["executable"]).with_name("partition.ll").read_bytes()).hexdigest()

def has_receipt(manifest):
    return Path(manifest["executable"]).with_name("compiler-partition-v1").is_file()

def expect_cold(root, profile="dev"):
    message, elapsed = command(root, "build", profile)
    assert f"local Unix socket for app:{profile} in an owned compiler process" in message, message
    assert "reused compiler LLVM" not in message and has_receipt(current(root, profile)), message
    return message, elapsed

def expect_warm(root, profile="dev"):
    message, elapsed = command(root, "build", profile)
    assert "reused compiler LLVM" in message and "owned compiler process" not in message, message
    assert has_receipt(current(root, profile))
    return elapsed

def verify_lifecycle(path):
    setup(path)
    expect_cold(path)
    a = generation(path)
    old = current(path)
    old_path = Path(old["executable"])
    old_bytes = old_path.read_bytes()
    expect_warm(path)
    assert generation(path) == a and current(path)["executable"] != str(old_path)

    # A distinct runner process gets the prior immutable generation path.
    # The next Build publishes while that reader is still using the older one.
    (path / "hold-runner").touch()
    held = current(path)
    runner = subprocess.Popen([COMPILER, "run", "app"], cwd=path, env=OPT_IN_ENV,
        stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
    try:
        deadline = time.monotonic() + 10
        while not (path / "runner-captured").exists():
            assert runner.poll() is None, runner.communicate()
            assert time.monotonic() < deadline, "runner never reached its captured path"
            time.sleep(.01)
        expect_warm(path)
        assert current(path)["executable"] != held["executable"]
        (path / "release-runner").touch()
        _, stderr = runner.communicate(timeout=10)
        assert runner.returncode == 0, stderr
        assert (path / "runner-captured").read_text() == held["executable"]
    finally:
        if runner.poll() is None:
            runner.kill()
            runner.wait()
        (path / "hold-runner").unlink()

    expect_cold(path, "verify")
    expect_warm(path, "verify")
    assert generation(path) == a
    command(path, "run", "verify")

    # The predecessor pointer and its binary remain valid if the configured
    # builder writes a partial new binary and fails. The stage must be gone.
    before = pointer(path).read_bytes()
    before_entries = set(pointer(path).parent.iterdir())
    (path / "fail-builder").touch()
    aborted, _ = command(path, "build", status=1)
    assert "fixture builder stopped after partial output" in aborted, aborted
    assert "72" in aborted, aborted
    assert pointer(path).read_bytes() == before and set(pointer(path).parent.iterdir()) == before_entries
    (path / "fail-builder").unlink()
    command(path, "run")

    # Another cooperating OS process holds source-store custody. Both the
    # unchanged default command and incremental miss still run the ordinary
    # builder; the latter must not consume or produce a compiler receipt.
    directory = os.open(path / ".wosy", os.O_RDONLY | os.O_DIRECTORY)
    try:
        fcntl.flock(directory, fcntl.LOCK_EX | fcntl.LOCK_NB)
        holder = subprocess.Popen([sys.executable, "-c", "import os; os.fstat(1); assert os.read(0, 1) == b''"],
            stdin=subprocess.PIPE, stdout=directory, stderr=subprocess.PIPE)
    finally:
        os.close(directory)
    try:
        # The coordinator's descriptor is gone. The child inherited the
        # same locked description and releases it only when EOF ends it.
        default, _ = command(path, "build", opted=False)
        command(path, "run", opted=False)
        assert "local Unix socket" not in default and "reused compiler LLVM" not in default
        fallback, _ = command(path, "build")
        assert "local Unix socket" not in fallback and "reused compiler LLVM" not in fallback
        assert not has_receipt(current(path))
        assert generation(path) == a
        command(path, "run")
    finally:
        try:
            _, stderr = holder.communicate(timeout=10)
            assert holder.returncode == 0, stderr
        except subprocess.TimeoutExpired:
            holder.kill()
            holder.communicate()
            raise
    expect_cold(path)
    assert generation(path) == a

    # The source cut spans a package boundary rather than simply changing
    # the same target's entry. Edit, run each generation, and restore bytes.
    dependency = path / "dependency/src/child.w"
    original = dependency.read_bytes()
    changed = original.replace(b"calls + 1", b"calls + 2")
    assert changed != original
    dependency.write_bytes(changed)
    stale, _ = command(path, "run", status=1)
    assert "artifact manifest is stale" in stale, stale
    expect_cold(path)
    b = generation(path)
    assert b[0] != a[0] and b[1] > a[1]
    assert partition(current(path)) != partition(old)
    command(path, "run")
    expect_warm(path)
    dependency.write_bytes(original)
    expect_cold(path)
    c = generation(path)
    assert c[0] == a[0] and c[1] > b[1]
    assert partition(current(path)) == partition(old)
    command(path, "run")
    assert old_path.read_bytes() == old_bytes

    # Remove only this disposable fixture's store after every process exited.
    # SQLite remains: one child reports replay, EOF finishes, and the next
    # child receives the root under the same inherited owner lock.
    shutil.rmtree(path / ".wosy/incremental-daemon")
    message, _ = expect_cold(path)
    assert "after source replay" in message and generation(path) == c, message
    command(path, "run")
    expect_warm(path)
    assert generation(path) == c
    # Changing the resolved dependency's package path changes identity
    # even when the imported bytes are the same.
    manifest = path / "wosy.toml"
    original_manifest = manifest.read_text()
    prior_identity = current(path)["identity"]["configuration_identity"]
    (path / "dependency").rename(path / "dependency-moved")
    manifest.write_text(original_manifest.replace('path = "dependency"', 'path = "dependency-moved"'))
    expect_cold(path)
    d = generation(path)
    assert d[0] != c[0] and d[1] > c[1]
    assert current(path)["identity"]["configuration_identity"] != prior_identity
    command(path, "run")
    (path / "dependency-moved").rename(path / "dependency")
    manifest.write_text(original_manifest)
    expect_cold(path)
    e = generation(path)
    assert e[0] == c[0] and e[1] > d[1]
    assert current(path)["identity"]["configuration_identity"] == prior_identity
    command(path, "run")
    print(json.dumps({"package_lifecycle": "pass", "generations": [a[1], b[1], c[1], d[1], e[1]],
        "pinned_runner": "pass", "partial_output_rollback": "pass", "busy_custody_fallback": "pass",
        "recovery_fresh_child": "pass", "builder_invocations_completed": len((path / "builder-ms").read_text().splitlines())}))

def measure(root, count):
    rows = []
    for sample in range(4):
        path = root / f"profile-{count}-{sample}"
        setup(path, count)
        message, cold = expect_cold(path)
        warm = expect_warm(path)
        _, run = command(path, "run")
        phase = message.split("(source store ", 1)[1].split(" us)", 1)[0]
        source, child = phase.split(" us; owned comparison ")
        cold_builder, warm_builder = map(float, (path / "builder-ms").read_text().splitlines())
        rows.append((cold, warm, run, int(source) / 1000, int(child) / 1000, cold_builder, warm_builder))
    names = ["cold_build", "warm_build", "run", "cold_source_store", "cold_owned_comparison",
        "cold_clang_builder", "warm_clang_builder"]
    print(json.dumps({"additional_functions": count, "samples": len(rows),
        "median_ms": dict(zip(names, map(statistics.median, zip(*rows)))), "samples_ms": rows}))

with tempfile.TemporaryDirectory(prefix="wosy-ordinary-generations-") as folder:
    root = Path(folder)
    verify_lifecycle(root / "lifecycle")
    for count in (0, 240):
        measure(root, count)

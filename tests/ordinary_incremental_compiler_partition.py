"""Exercise the public Linux Build/Run with retained LLVM and a real Clang builder."""
import hashlib
import json
import os
from pathlib import Path
import shutil
import socket
import sqlite3
import subprocess
import sys
import tempfile

compiler = Path(sys.argv[1]).resolve()
fixture = Path(__file__).parent / "fixtures/build/short_circuit_project/project"

with tempfile.TemporaryDirectory() as directory:
    project = Path(directory) / "project"
    shutil.copytree(fixture, project)
    configured_builder = project / "scripts/build-native"
    configured_builder.write_text(configured_builder.read_text().replace(
        "partition, output_dir", 'with open("invocations", "a") as f: f.write("a\\n")\npartition, output_dir', 1
    ))
    manifest = project / "wosy.toml"
    with manifest.open("a") as file:
        file.write("""
[targets.app.artifacts.main.profiles.verify]
backend = "llvm"
output = "native"
builder = "native"
run_runner = "native"
[targets.app.profiles.verify]
main_artifact = "main"
""")
    source = project / "src/child.w"
    original = source.read_text()
    output = project / ".wosy/artifacts/app/dev/main/incremental"
    generations = []
    digests = []

    def selected_source():
        database = (project / ".wosy/client.sqlite").as_uri() + "?mode=ro"
        with sqlite3.connect(database, uri=True) as connection:
            return connection.execute(
                "select selected, selected_generation from source_targets where target = ?", ("app",),
            ).fetchone()

    build_messages = []

    def invoke(command, opt_in, profile="dev"):
        environment = os.environ.copy()
        if opt_in:
            environment["WOSY_INCREMENTAL"] = "1"
        else:
            environment.pop("WOSY_INCREMENTAL", None)
        result = subprocess.run([compiler, command, "app", "--profile", profile], cwd=project, env=environment,
                                capture_output=True, text=True, timeout=60)
        assert result.returncode == 0, result.stderr
        if command == "build":
            build_messages.append((opt_in, result.stderr))
        return result.stderr

    # A caller with neither the inherited socket nor the selected same-image
    # parent cannot enter the internal child path or open the project store.
    hidden = [compiler, "__wosy-linked-ordinary-child-v1"]
    default_env = os.environ.copy()
    default_env.pop("WOSY_INCREMENTAL", None)
    declined = subprocess.run(hidden, cwd=project, stdin=subprocess.DEVNULL, env=default_env,
        capture_output=True, text=True, timeout=15)
    assert declined.returncode == 2 and "unrecognized subcommand" in declined.stderr, declined.stderr
    # This Python process owns a real Unix socket but is not the
    # compiler executable entitled to activate a scoped child.
    foreign, supplied = socket.socketpair()
    with foreign, supplied:
        declined = subprocess.run(hidden, cwd=project, stdin=supplied, env={**default_env, "WOSY_INCREMENTAL": "1"},
            capture_output=True, text=True, timeout=15)
        assert declined.returncode == 1 and "not from its same-image parent" in declined.stderr, declined.stderr
    assert not (project / ".wosy").exists()

    for count, case in enumerate(("cold", "warm", "change", "warm-new-builder", "corrupt", "recover-warm"), 1):
        if case == "change":
            source.write_text(source.read_text().replace("calls + 1", "calls + 2"))
        if case == "warm-new-builder":
            configured_builder.write_text(configured_builder.read_text().replace('f.write("a', 'f.write("b'))
        if case == "corrupt":
            (generations[-1] / "partition.ll").write_text("invalid LLVM")
        message = invoke("build", True)
        assert ("compared over local Unix socket for app:dev" in message) == (case in ("cold", "change", "corrupt")), message
        if case in ("cold", "change", "corrupt"):
            assert "in an owned compiler process" in message, message
        if case == "cold":
            initial = selected_source()
        if case == "change":
            changed = selected_source()
            assert changed[1] > initial[1] and changed[0] != initial[0]
        assert ("reused compiler LLVM" in message) == (case in ("warm", "warm-new-builder", "recover-warm")), message
        assert not invoke("run", True)
        current = json.loads((output / "artifact-manifest.json").read_text())
        executable = Path(current["executable"])
        assert executable.parent not in generations
        generations.append(executable.parent)
        assert all((generation / "main").is_file() for generation in generations)
        assert (executable.parent / "compiler-partition-v1").is_file()
        calls = (project / "invocations").read_text().splitlines()
        assert calls == ["a"] * min(count, 3) + ["b"] * max(count - 3, 0)
        digests.append(hashlib.sha256((executable.parent / "partition.ll").read_bytes()).digest())

    assert digests[0] == digests[1] != digests[2]
    assert digests[2:] == [digests[2]] * 4
    # The client retains the exact acknowledged source but the disposable
    # project cache is gone. One child sees ReplayRequired and closes;
    # only a fresh child may compare after the client replays that root.
    before_replay = selected_source()
    shutil.rmtree(project / ".wosy/incremental-daemon")
    replayed = invoke("build", True)
    assert "in an owned compiler process after source replay" in replayed, replayed
    assert "reused compiler LLVM" not in replayed and selected_source() == before_replay
    assert not invoke("run", True)

    # In this disposable private project, model a store owner the scoped
    # child cannot accept. The ordinary configured builder must still run
    # and publish a valid executable, but no compiler proof may be retained.
    owned_dir = project / ".wosy"
    original_mode = owned_dir.stat().st_mode & 0o777
    try:
        owned_dir.chmod(original_mode | 0o020)
        fallback = invoke("build", True, "verify")
        assert "requires a project-owned source store" in fallback, fallback
        assert "compared over local Unix socket" not in fallback and "reused compiler LLVM" not in fallback
        assert not invoke("run", True, "verify")
        fallback_manifest = json.loads((project / ".wosy/artifacts/app/verify/main/incremental/artifact-manifest.json").read_text())
        assert not Path(fallback_manifest["executable"]).with_name("compiler-partition-v1").exists()
    finally:
        owned_dir.chmod(original_mode)

    # Separate CLI commands reconnect to the same persistent selected source.
    # Switching artifact profiles doesn't grant the previous profile's receipt.
    edited, edit_generation = selected_source()
    assert "compared over local Unix socket for app:verify" in invoke("build", True, "verify")
    assert selected_source() == (edited, edit_generation)
    assert not invoke("run", True, "verify")
    assert "reused compiler LLVM" in invoke("build", True, "verify")
    assert selected_source() == (edited, edit_generation)
    assert not invoke("run", True, "verify")

    # Reverting bytes creates another generation and cannot use the changed
    # generation's partition, even though it recovers the initial digest.
    source.write_text(original)
    reverted_msg = invoke("build", True)
    assert "compared over local Unix socket for app:dev" in reverted_msg, reverted_msg
    assert "reused compiler LLVM" not in reverted_msg, reverted_msg
    reverted, revert_generation = selected_source()
    assert revert_generation > edit_generation and reverted != edited
    assert reverted == initial[0]
    current = json.loads((output / "artifact-manifest.json").read_text())
    assert hashlib.sha256(Path(current["executable"]).with_name("partition.ll").read_bytes()).digest() == digests[0]
    assert not invoke("run", True)
    assert all((generation / "main").is_file() for generation in generations)

    default = invoke("build", False)
    assert "local Unix socket" not in default and "reused compiler LLVM" not in default
    assert not invoke("run", False)
    invocations = (project / "invocations").read_text().splitlines()
    assert len(invocations) == 12
    comparisons = sum("compared over local Unix socket" in message for _, message in build_messages)
    reuses = sum("reused compiler LLVM" in message for _, message in build_messages)
    assert (comparisons, reuses) == (6, 4)
    print(json.dumps({"fresh_socket_comparisons": comparisons, "retained_llvm_hits": reuses,
        "configured_builds": len(invocations), "ordinary_default_builds": sum(not opt for opt, _ in build_messages),
        "guarded_fallback_builds": sum("requires a project-owned source store" in message for _, message in build_messages),
        "selected_generations": [initial[1], edit_generation, revert_generation]}))

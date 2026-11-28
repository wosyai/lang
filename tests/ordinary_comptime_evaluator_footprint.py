"""Measure real temporary evaluator footprint for a multi-mode WASI artifact.

Run in a configured clang/wasm-ld/wasmtime environment:
    python3 tests/ordinary_comptime_evaluator_footprint.py /path/to/wosy 1
The second argument is the expected number of simultaneous stage directories.
"""
from __future__ import annotations

import hashlib
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import time


def main(compiler: Path, expected_stages: int, unselected: int = 0) -> None:
    modes = [
        "standby", "manual", "yaw_port", "yaw_starboard", "pitch_up", "pitch_down",
        "roll_port", "roll_starboard", "brake", "boost", "approach", "retreat",
        "docking", "undocking", "translation", "stabilize", "hold", "failover",
    ]
    tuples = [(mode, 112, code) for code, mode in enumerate(modes)]
    manifest = """
[package]
name = "station"
default_target = "control"
[comments]
categories = ["WHY"]
[builders.closed]
command = ["python3", "ordinary_wasip1_pair.py", "build"]
toolchain_identity = "ordinary WASI station fixture"
[runners.closed]
command = ["python3", "ordinary_wasip1_pair.py", "ordered-run"]
[builders.artifact]
command = ["python3", "ordinary_wasip1_pair.py", "artifact-build-footprint"]
toolchain_identity = "ordinary WASI station artifact"
[runners.artifact]
command = ["python3", "ordinary_wasip1_pair.py", "artifact-run"]
[comptime.calculate]
watch = ["interlock.txt"]
[targets.control]
root = "control.w"
[targets.control.profiles.dev]
main_artifact = "main"
comptime_builder = "closed"
comptime_runner = "closed"
build_permissions = []
build_mounts = []
[targets.control.artifacts.main]
artifact = "StationControl"
[targets.control.artifacts.main.profiles.dev]
backend = "llvm"
output = "wasm-wasip1"
builder = "artifact"
run_runner = "artifact"
[targets.control.artifacts.main.profiles.dev.exports]
"""
    source = "\n".join(
        ["%%start", "u32(u32, u32) solve = fn(left, right) { comptime calculate { right ^ left } };"]
        + [f"u32() {mode} = fn {{ solve({base}, {code}) }};" for mode, base, code in tuples]
        + [f"u32() unused{i:04} = fn {{ {i} }};" for i in range(unselected)]
        + ["u32() nonliteral = fn { 1 };", "%%end\n"]
    )
    expected = {f"station_{mode}": base ^ code for mode, base, code in tuples}
    roster = {f"station_{mode}": mode for mode in modes}
    # An alias is a second root for the same typed call, not a new evaluator.
    roster["station_safe"] = modes[0]
    expected["station_safe"] = expected["station_standby"]
    manifest += "".join(f'{export} = "{name}"\n' for export, name in roster.items())

    with tempfile.TemporaryDirectory() as tmp:
        root = Path(tmp)
        (root / "wosy.toml").write_text(manifest)
        (root / "control.w").write_text(source)
        (root / "interlock.txt").write_text("nominal\n")
        (root / "expected-exports.json").write_text(json.dumps(expected))
        (root / "audit-evaluator-footprint").touch()
        tool = Path(__file__).resolve().parents[1] / "crates/wosy-cli/tests/fixtures/ordinary_wasip1_pair.py"
        (root / tool.name).write_bytes(tool.read_bytes())
        count_file = root / "evaluator-order"
        build_file = root / "footprint-final-builds"
        metrics = root / "evaluator-footprint"
        stage_parent = root / ".wosy/incremental-daemon"
        pointer = root / ".wosy/artifacts/control/dev/main/incremental/artifact-manifest.json"

        def call(verb):
            started = time.perf_counter_ns()
            result = subprocess.run(
                [compiler, verb, "control"], cwd=root, capture_output=True, text=True,
                env={**os.environ, "WOSY_INCREMENTAL": "1"}, timeout=110,
            )
            return result, (time.perf_counter_ns() - started) / 1e6

        def sampled_cycle(label, expected_warm, expected_count, expected_output=True):
            order_index = len(count_file.read_text().splitlines()) if count_file.exists() else 0
            metric_index = len(metrics.read_text().splitlines()) if metrics.exists() else 0
            final_index = len(build_file.read_text().splitlines()) if build_file.exists() else 0
            built, build_ms = call("build")
            assert (built.returncode == 0) == expected_output, built.stderr
            if expected_output:
                assert ("reused compiler LLVM after executing current comptime" in built.stderr) == expected_warm
                ran, run_ms = call("run")
                assert ran.returncode == 0, ran.stderr
            else:
                assert "selected comptime runner failed" in built.stderr, built.stderr
                run_ms = None
            final = len(build_file.read_text().splitlines())
            assert final == final_index + int(expected_output)
            results = [int(line) for line in count_file.read_text().splitlines()[order_index:]]
            assert results == [base ^ code for _, base, code in tuples[:expected_count]], results
            samples = [tuple(map(int, line.split())) for line in metrics.read_text().splitlines()[metric_index:]]
            assert len(samples) == expected_count
            peak_stages, peak_bytes = (max(values) for values in zip(*samples))
            assert peak_stages == expected_stages, (label, peak_stages, expected_stages)
            assert not list(stage_parent.glob("wosy-evaluator-*")), "owned stages leaked"
            print(json.dumps({"case": label, "build_ms": build_ms, "run_ms": run_ms,
                "evaluators": len(results), "final_builders": int(expected_output),
                "peak_stages": peak_stages, "peak_reserved_file_bytes": peak_bytes,
                "phase_metrics": next((x for x in built.stderr.splitlines() if "evaluator tools" in x), None)}),
                flush=True)

        sampled_cycle("cold", False, len(tuples))
        first = json.loads(pointer.read_text())
        first_output = Path(first["executable"])
        first_wasm = first_output.read_bytes()
        first_llvm = first_output.with_name("partition.ll").read_bytes()
        print(json.dumps({"case": "cold_artifact_identity", "unselected": unselected,
            "llvm_sha256": hashlib.sha256(first_llvm).hexdigest(),
            "wasm_sha256": hashlib.sha256(first_wasm).hexdigest()}), flush=True)
        sampled_cycle("warm", True, len(tuples))
        current = pointer.read_bytes()
        current_output = Path(json.loads(current)["executable"])
        assert current_output != first_output
        assert current_output.with_name("partition.ll").read_bytes() == first_llvm
        (root / "refuse-evaluator-output").write_text(f"{expected['station_failover']}\n")
        sampled_cycle("last_evaluator_failure", False, len(tuples), False)
        assert pointer.read_bytes() == current and Path(first["executable"]).read_bytes() == first_wasm
        (root / "refuse-evaluator-output").unlink()

        # This opt-in host watch explicitly enters the scratch subtree.
        # Full staging must retain the incumbent pre-execution refusal.
        ledger = metrics.read_bytes(), count_file.read_bytes(), build_file.read_bytes()
        with_scratch_watch = manifest.replace('watch = ["interlock.txt"]',
            'watch = ["interlock.txt", ".wosy/incremental-daemon/wosy-evaluator-*/evaluator.ll"]')
        (root / "wosy.toml").write_text(with_scratch_watch)
        refused_watch, reject_ms = call("build")
        assert refused_watch.returncode != 0, refused_watch.stderr
        assert "declared comptime inputs changed after evaluator staging" in refused_watch.stderr, refused_watch.stderr
        assert (metrics.read_bytes(), count_file.read_bytes(), build_file.read_bytes()) == ledger
        assert pointer.read_bytes() == current
        assert not list(stage_parent.glob("wosy-evaluator-*"))
        (root / "wosy.toml").write_text(manifest)
        print(json.dumps({"case": "explicit_scratch_watch_preserves_incumbent_preexecution_refusal",
            "build_ms": reject_ms, "evaluators": 0, "final_builders": 0}), flush=True)

        # Make the last selected edge unsupported and prove the complete
        # plan rejected it before even the first previously valid process.
        old_ledger = metrics.read_bytes(), count_file.read_bytes(), build_file.read_bytes()
        (root / "control.w").write_text(source.replace("solve(112, 17)", "solve(112, nonliteral())"))
        rejected, reject_ms = call("build")
        assert rejected.returncode != 0
        assert (metrics.read_bytes(), count_file.read_bytes(), build_file.read_bytes()) == old_ledger
        assert pointer.read_bytes() == current
        assert not list(stage_parent.glob("wosy-evaluator-*"))
        print(json.dumps({"case": "last_edge_rejected_before_any_process", "build_ms": reject_ms,
            "evaluators": 0, "final_builders": 0}), flush=True)


if __name__ == "__main__":
    main(Path(sys.argv[1]).resolve(), int(sys.argv[2]), int(sys.argv[3]) if len(sys.argv) > 3 else 0)

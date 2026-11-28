"""Exercise the public Linux Build/Run with retained LLVM and a real Clang builder."""
import hashlib
import json
import os
from pathlib import Path
import shutil
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
    source = project / "src/child.w"
    output = project / ".wosy/artifacts/app/dev/main/incremental"
    generations = []
    digests = []

    def invoke(command, opt_in):
        environment = os.environ.copy()
        if opt_in:
            environment["WOSY_INCREMENTAL"] = "1"
        else:
            environment.pop("WOSY_INCREMENTAL", None)
        result = subprocess.run([compiler, command, "app"], cwd=project, env=environment,
                                capture_output=True, text=True, timeout=60)
        assert result.returncode == 0, result.stderr
        return result.stderr

    for count, case in enumerate(("cold", "warm", "change", "warm-new-builder", "corrupt", "recover-warm"), 1):
        if case == "change":
            source.write_text(source.read_text().replace("calls + 1", "calls + 2"))
        if case == "warm-new-builder":
            configured_builder.write_text(configured_builder.read_text().replace('f.write("a', 'f.write("b'))
        if case == "corrupt":
            (generations[-1] / "partition.ll").write_text("invalid LLVM")
        message = invoke("build", True)
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
    assert "reused compiler LLVM" not in invoke("build", False)
    assert not invoke("run", False)
    assert len((project / "invocations").read_text().splitlines()) == 7

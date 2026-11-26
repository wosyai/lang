"""Exercise NativeKnown's real shared framed handler without opening a Unix socket."""
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile


example, unit_tests = (Path(arg).resolve() for arg in sys.argv[1:3])
manifest = (Path(__file__).parent /
            "fixtures/build/shared_stdlib_local_dependency/project/wosy.toml").read_text()
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
with tempfile.TemporaryDirectory() as path:
    temp = Path(path).resolve()
    project, store, missing = (temp / sub for sub in ("project", "source", "empty"))
    (project / "src").mkdir(parents=True)
    (project / "stdlib/src").mkdir(parents=True)
    (project / "wosy.toml").write_text(manifest)
    (project / "stdlib/wosy.toml").write_text('[package]\nname="std"\nsource_root="src"\n')
    (project / "stdlib/src/value.w").write_text("%%start\nu32 value = 1;\n%%end\n")
    (project / "src/main.w").write_text('''%%start
dep = namespace std "value.w";
struct Item { u8 value; }
Item(Item) take = fn(input) { next = input; next; next };
%%end
''')

    def run(*args):
        out = subprocess.run([example, project, store, "--persistent-source", "app", *args],
                             text=True, capture_output=True, check=False, timeout=30)
        assert out.returncode == 0, out.stderr
        return json.loads(out.stdout)

    pending = run("stage")["pending"]
    known = run("submit-pending", "app", "src/main.w")["selected"]
    assert known == pending and not missing.exists()
    result = subprocess.run([unit_tests, "native_known_frames_renew_analyze_and_refuse_source_replay",
                             "--ignored", "--nocapture"],
                            env={**os.environ, "WOSY_NATIVE_KNOWN_PROJECT": str(project),
                                 "WOSY_NATIVE_KNOWN_STORE": str(store), "WOSY_NATIVE_KNOWN_EMPTY_STORE": str(missing),
                                 "WOSY_NATIVE_KNOWN_ROOT": known}, check=False, capture_output=True, text=True, timeout=30)
    assert result.returncode == 0 and "1 passed" in result.stdout, result.stdout + result.stderr
    print("NativeKnown shared framed handler: real owned published root, Renew + Analyze + Close in WASI/Native64, "
          "same root absent in actual replacement store returns terminal ReplayRequired, expired ready Start sends no frame. "
          "No socket operation.")

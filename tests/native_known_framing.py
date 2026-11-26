"""Exercise NativeKnown's real shared framed handler without opening a Unix socket."""
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile

from native_known_fixture import write_project

example, unit_tests = (Path(arg).resolve() for arg in sys.argv[1:3])
with tempfile.TemporaryDirectory() as path:
    temp = Path(path).resolve()
    project, store, missing = (temp / sub for sub in ("project", "source", "empty"))
    write_project(project)

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

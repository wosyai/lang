"""Fixture for the ordinary selected tool ABI, not the private known-image gate."""
import json
from pathlib import Path
import subprocess
import sys
import tempfile

operation, *args = sys.argv[1:]
if operation == "build":
    ir, output, manifest = map(Path, args)
    module = Path(json.loads(manifest.read_text())["executable"])
    assert module.parent == output
    with tempfile.TemporaryDirectory(dir=output) as temp:
        directory = Path(temp)
        start = directory / "start.ll"
        start.write_text('''target triple = "wasm32-wasi"
declare i32 @main()
declare void @proc_exit(i32) #0
define void @_start() {
entry:
    %status = call i32 @main()
    call void @proc_exit(i32 %status)
    unreachable
}
attributes #0 = { noreturn "wasm-import-module"="wasi_snapshot_preview1" "wasm-import-name"="proc_exit" }
''')
        allowed = directory / "allowed.txt"
        allowed.write_text("fd_write\nproc_exit\n")
        for source, name in ((ir, "evaluator.o"), (start, "start.o")):
            subprocess.run(["clang", "--target=wasm32-wasi", "-O2", "-c", "-x", "ir",
                            source, "-o", directory / name], check=True)
        subprocess.run(["wasm-ld", "--no-entry", "--export=_start", "--export-memory",
                        f"--allow-undefined-file={allowed}", directory / "evaluator.o",
                        directory / "start.o", "-o", module], check=True)
elif operation == "run":
    manifest, = map(Path, args)
    module = json.loads(manifest.read_text())["executable"]
    sys.exit(subprocess.run(["wasmtime", "run", "-C", "cache=n", module]).returncode)
else:
    raise ValueError(operation)

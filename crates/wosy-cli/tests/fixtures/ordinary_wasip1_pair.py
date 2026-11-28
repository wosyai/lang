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
elif operation == "artifact-check":
    ir, runtime, output = map(Path, args[:3])
    expected = int(args[3])
    with tempfile.TemporaryDirectory(dir=output.parent) as temp:
        directory = Path(temp)
        observed = directory / "observed.ll"
        observed.write_text(ir.read_text() + f"""
; Test-only observer in the same module as the ordinary private globals.
define i32 @wosy_test_startup_values() {{
entry:
    %a = load i32, ptr @bypass
    %b = load i32, ptr @first
    %c = load i32, ptr @repeated
    %d = load i32, ptr @different
    %qa = icmp eq i32 %a, 37
    %qb = icmp eq i32 %b, 34
    %qc = icmp eq i32 %c, 34
    %qd = icmp eq i32 %d, {expected}
    %ab = and i1 %qa, %qb
    %cd = and i1 %qc, %qd
    %all = and i1 %ab, %cd
    %status = select i1 %all, i32 0, i32 73
    ret i32 %status
}}
""")
        start = directory / "artifact-start.ll"
        start.write_text('''target triple = "wasm32-wasi"
declare i32 @__original_main()
declare i32 @wosy_test_startup_values()
declare void @proc_exit(i32) #0
define void @_start() {
entry:
    %main = call i32 @__original_main()
    %checked = call i32 @wosy_test_startup_values()
    %status = or i32 %main, %checked
    call void @proc_exit(i32 %status)
    unreachable
}
attributes #0 = { noreturn "wasm-import-module"="wasi_snapshot_preview1" "wasm-import-name"="proc_exit" }
''')
        allowed = directory / "allowed.txt"
        allowed.write_text("fd_write\nproc_exit\n")
        objects = []
        for source, name in ((observed, "artifact.o"), (start, "artifact-start.o"), (runtime, "runtime.o")):
            object_file = directory / name
            subprocess.run(["clang", "--target=wasm32-wasi", "-c", "-x", "ir",
                            source, "-o", object_file], check=True)
            objects.append(object_file)
        subprocess.run(["wasm-ld", "--no-entry", "--export=_start", "--export-memory",
                        f"--allow-undefined-file={allowed}", *objects, "-o", output], check=True)
    sys.exit(subprocess.run(["wasmtime", "run", "-C", "cache=n", output]).returncode)
elif operation == "run":
    manifest, = map(Path, args)
    module = json.loads(manifest.read_text())["executable"]
    sys.exit(subprocess.run(["wasmtime", "run", "-C", "cache=n", module]).returncode)
else:
    raise ValueError(operation)

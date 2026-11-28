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
elif operation in ("artifact-check", "artifact-build", "artifact-build-min", "artifact-build-ordered", "artifact-build-exported"):
    if operation == "artifact-check":
        ir, runtime, output = map(Path, args[:3])
        expected = int(args[3])
    else:
        ir, directory, manifest, runtime = map(Path, args)
        metadata = json.loads(manifest.read_text())
        output = Path(metadata["executable"])
        assert output.parent == directory
        expected = -2147483648 if operation == "artifact-build-min" else 10
        if Path("refuse-final-builder").is_file():
            sys.exit(72)
    if operation == "artifact-build-ordered":
        checks = [("first", 14), ("second", 10), ("tied", 4)]
    elif operation == "artifact-build-exported":
        profile = metadata["identity"]["artifact_identity"].split(":")[1]
        checks = [("north", 10)] if profile == "subset" else [
            ("west", 34), ("south", 34), ("north", 10), ("mirror", 34),
        ]
    else:
        checks = [("bypass", 37), ("first", 34), ("repeated", 34), ("different", expected)]
    checker = []
    previous = "true"
    for index, (binding, value) in enumerate(checks):
        read = f"call i32 @{binding}()" if operation == "artifact-build-exported" else f"load i32, ptr @{binding}"
        checker.append(f"  %v{index} = {read}")
        checker.append(f"  %q{index} = icmp eq i32 %v{index}, {value}")
        checker.append(f"  %all{index} = and i1 {previous}, %q{index}")
        previous = f"%all{index}"
    with tempfile.TemporaryDirectory(dir=output.parent) as temp:
        directory = Path(temp)
        observed = directory / "observed.ll"
        observed.write_text(ir.read_text() + f"""
; Test-only observer in the same module as the ordinary private globals.
define i32 @wosy_test_startup_values() {{
entry:
{chr(10).join(checker)}
    %status = select i1 {previous}, i32 0, i32 73
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
        exports = [f"--export={name}" for name, _ in checks] if operation == "artifact-build-exported" else []
        subprocess.run(["wasm-ld", "--no-entry", "--export=_start", "--export-memory", *exports,
                        f"--allow-undefined-file={allowed}", *objects, "-o", output], check=True)
    if operation == "artifact-check":
        sys.exit(subprocess.run(["wasmtime", "run", "-C", "cache=n", output]).returncode)
elif operation == "artifact-run":
    manifest, separator = args
    assert separator == "--"
    module = json.loads(Path(manifest).read_text())["executable"]
    sys.exit(subprocess.run(["wasmtime", "run", "-C", "cache=n", module]).returncode)
elif operation == "ordered-run":
    manifest, = map(Path, args)
    module = json.loads(manifest.read_text())["executable"]
    completed = subprocess.run(["wasmtime", "run", "-C", "cache=n", module], capture_output=True)
    sys.stderr.buffer.write(completed.stderr)
    if not completed.returncode:
        with Path("evaluator-order").open("ab") as log:
            log.write(completed.stdout)
        if completed.stdout == b"10\n" and Path("refuse-second-evaluator").exists():
            sys.exit(74)
    sys.stdout.buffer.write(completed.stdout)
    sys.exit(completed.returncode)
elif operation == "run":
    manifest, = map(Path, args)
    module = json.loads(manifest.read_text())["executable"]
    sys.exit(subprocess.run(["wasmtime", "run", "-C", "cache=n", module]).returncode)
else:
    raise ValueError(operation)

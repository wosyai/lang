"""Reusable command recipes for the private WASIp1 stdout tool fixture.

A recipe is data; the trusted launcher owns tool provenance, writable
scratch, and process isolation. Do not invoke it as an ordinary runner.
"""

START = """target triple = "wasm32-wasi"

declare i32 @main()
declare void @proc_exit(i32) #0

define void @_start() {
entry:
  %status = call i32 @main()
  call void @proc_exit(i32 %status)
  unreachable
}

attributes #0 = { noreturn "wasm-import-module"="wasi_snapshot_preview1" "wasm-import-name"="proc_exit" }
"""
ALLOW_UNDEFINED = b"fd_write\nproc_exit\n"

def builder_commands(clang, linker):
    compile_ir = [clang, "--target=wasm32-wasi", "-O2", "-c", "-x", "ir"]
    return (
        [*compile_ir, "evaluator.ll", "-o", "evaluator.o"],
        [*compile_ir, "start.ll", "-o", "start.o"],
        [linker, "--no-entry", "--export=_start", "--export-memory", "--fatal-warnings",
         "--allow-undefined-file=allow-undefined.txt", "evaluator.o", "start.o", "-o", "report.wasm"],
    )

def runner_command(wasmtime, module):
    return [wasmtime, "run",
        "-C", "cache=n,parallel-compilation=n",
        "-D", "logging=n",
        "-W", "fuel=100000,timeout=1s,max-memory-size=1048576,max-wasm-stack=262144,max-instances=1,max-memories=1,max-tables=1,max-table-elements=64,threads=n,shared-memory=n,component-model=n",
        "-O", "memory-init-cow=n,memory-reservation=1048576,memory-reservation-for-growth=0,memory-guard-size=65536",
        "-S", "cli=y,preview2=y,listenfd=n,http=n,tcp=n,udp=n,inherit-network=n,allow-ip-name-lookup=n,inherit-env=n,inherit-stdin=n,inherit-stdout=y,inherit-stderr=y",
        module]

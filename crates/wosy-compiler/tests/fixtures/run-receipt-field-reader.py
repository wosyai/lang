"""Run with the IR directory from WOSY_RECEIPT_FIELD_IR_DIR/{single,project} and Linux LLVM toolchain root."""
import os
from pathlib import Path
import subprocess
import sys
import tempfile

ir_dir, toolchain = (Path(arg).resolve() for arg in sys.argv[1:3])
root = toolchain / "llvm/usr"
clang = root / "bin/clang-18"
ld = root / "bin/wasm-ld-18"
env = os.environ | {"LD_LIBRARY_PATH": str(root / "lib/x86_64-linux-gnu")}

def run(*cmd):
    subprocess.run(list(map(str, cmd)), check=True, env=env)

runtime = r"""
typedef unsigned long long u64;
static _Alignas(64) unsigned char backing[64];
static unsigned active, allocated, released;

void *__wosy_core_alloc(u64 size, u64 alignment) {
    if (active || allocated || size != 3 || !alignment || alignment > 64 || 64 % alignment)
        __builtin_trap();
    ++allocated;
    active = 1;
    return backing;
}
void __wosy_core_free(void *pointer) {
    if (!active || pointer != backing || backing[0] != 19)
        __builtin_trap();
    active = 0;
    ++released;
}
void __wosy_core_system_panic(void) { __builtin_trap(); }
extern u64 wosy_field_witness(void);

int check(void) {
    u64 observed = wosy_field_witness();
    return observed != 4 || allocated != 1 || released != 1 || active;
}
#ifndef WASI
int main(void) { return check(); }
#endif
"""

expected = 4
if sys.argv[3:] == ["metadata"]:
    expected = 10
    runtime = runtime.replace("observed != 4", "observed != 10")
elif sys.argv[3:]:
    raise SystemExit("optional third argument must be metadata")

with tempfile.TemporaryDirectory(prefix="receipt-field-", dir=ir_dir) as tmp:
    work = Path(tmp)
    trace = work / "trace.c"
    trace.write_text(runtime)
    native = work / "check-native"
    run(clang, "-O0", trace, ir_dir / "native64.ll", "-o", native)
    run(native)
    print(f"Native64: checked reader result {expected} and exactly one original backing release", flush=True)

    triple = "--target=wasm32-unknown-wasi"
    wosy, stub, wasm = work / "wosy.o", work / "trace.o", work / "check.wasm"
    run(clang, triple, "-O0", "-c", ir_dir / "wasm32.ll", "-o", wosy)
    run(clang, triple, "-O0", "-DWASI", "-ffreestanding", "-c", trace, "-o", stub)
    run(ld, "--no-entry", "--export=check", wosy, stub, "-o", wasm)
    run("node", "--input-type=module", "-e", """
import fs from "node:fs";
const {instance:{exports:e}} = await WebAssembly.instantiate(fs.readFileSync(process.argv[1]));
if (e.check() !== 0) throw new Error("checked field receipt trace failed");
""", wasm)
    print(f"Wasm32: checked reader result {expected} and exactly one original backing release", flush=True)

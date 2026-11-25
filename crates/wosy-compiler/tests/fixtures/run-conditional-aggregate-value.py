"""Run with the IR directory from WOSY_VALUE_RECEIPT_IR_DIR and Linux LLVM toolchain root."""
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
static _Alignas(64) unsigned char pool[6][64];
static unsigned allocated, released;
// False transfers the third receipt, then cleans up second and first at root.
// True transfers first and second in authored order, then cleans up third.
static const unsigned expected[] = {2, 1, 0, 3, 4, 5};
void *__wosy_core_alloc(u64 size, u64 alignment) {
    if (allocated == 6 || size != 2 || !alignment || alignment > 64 || 64 % alignment)
        __builtin_trap();
    return pool[allocated++];
}
void __wosy_core_free(void *pointer) {
    if (released == 6 || expected[released] >= allocated ||
        pointer != pool[expected[released]] || pool[expected[released]][0] != 19)
        __builtin_trap();
    ++released;
}
void __wosy_core_system_panic(void) { __builtin_trap(); }
extern u64 wosy_conditional_retain(void);
extern u64 wosy_conditional_transfer(void);

int check(void) {
    if (wosy_conditional_retain() != 3 || allocated != 3 || released != 3)
        return 1;
    if (wosy_conditional_transfer() != 6 || allocated != 6 || released != 6)
        return 2;
    return 0;
}
#ifndef WASI
int main(void) { return check(); }
#endif
"""

with tempfile.TemporaryDirectory(prefix="conditional-receipt-", dir=ir_dir) as tmp:
    work = Path(tmp)
    trace = work / "trace.c"
    trace.write_text(runtime)
    native = work / "check-native"
    run(clang, "-O0", trace, ir_dir / "native64.ll", "-o", native)
    run(native)
    print("Native64: all six original receipts released exactly once in the expected order", flush=True)

    triple = "--target=wasm32-unknown-wasi"
    wosy, stub, wasm = work / "wosy.o", work / "trace.o", work / "check.wasm"
    run(clang, triple, "-O0", "-c", ir_dir / "wasm32.ll", "-o", wosy)
    run(clang, triple, "-O0", "-DWASI", "-ffreestanding", "-c", trace, "-o", stub)
    run(ld, "--no-entry", "--export=check", wosy, stub, "-o", wasm)
    run("node", "--input-type=module", "-e", """
import fs from "node:fs";
const {instance:{exports:e}} = await WebAssembly.instantiate(fs.readFileSync(process.argv[1]));
if (e.check() !== 0) throw new Error("conditional receipt trace failed");
""", wasm)
    print("Wasm32: all six original receipts released exactly once in the expected order", flush=True)

"""Run with the IR directory from WOSY_LEXICAL_RECEIPT_IR_DIR and Linux LLVM toolchain root."""
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
static _Alignas(64) unsigned char pool[16][64];
static unsigned allocated, released;
// The plain-block receipt leaves first; active sibling receipts leave in their
// branch, before parent cleanup unless a consuming call has already taken it.
static const unsigned expected[] = {1, 2, 3, 0, 5, 6, 7, 4, 9, 8, 11, 10, 13, 14, 12, 15};
void *__wosy_core_alloc(u64 size, u64 alignment) {
    if (allocated == 16 || size != 2 || !alignment || alignment > 64 || 64 % alignment)
        __builtin_trap();
    return pool[allocated++];
}
void __wosy_core_free(void *pointer) {
    if (released == 16 || expected[released] >= allocated ||
        pointer != pool[expected[released]] || pool[expected[released]][0] != 19)
        __builtin_trap();
    ++released;
}
void __wosy_core_system_panic(void) { __builtin_trap(); }
extern u64 wosy_case_ff(void), wosy_case_ft(void), wosy_case_tf(void), wosy_case_tt(void);

int check(void) {
    if (wosy_case_ff() != 7 || allocated != 4 || released != 4)
        return 1;
    if (wosy_case_ft() != 8 || allocated != 8 || released != 8)
        return 2;
    if (wosy_case_tf() != 9 || allocated != 12 || released != 12)
        return 3;
    if (wosy_case_tt() != 11 || allocated != 16 || released != 16)
        return 4;
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
    print("Native64: all sixteen original receipts released exactly once in the expected order", flush=True)

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
    print("Wasm32: all sixteen original receipts released exactly once in the expected order", flush=True)

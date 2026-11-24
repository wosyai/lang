"""Run with the IR directory from WOSY_D0024_OWNERSHIP_IR_DIR and Linux LLVM toolchain root."""
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
static _Alignas(64) unsigned char pool[8][64];
static unsigned allocated, released, live, peak;
static const unsigned sizes[] = {4, 3, 5, 4, 3, 2, 2, 2};
static const unsigned marker[] = {2, 1, 3, 2, 1, 30, 40, 50};
static const unsigned release_order[] = {1, 0, 4, 3, 2, 5, 6, 7};

void *__wosy_core_alloc(u64 size, u64 alignment) {
    if (allocated == 8 || size != sizes[allocated] || !alignment || alignment > 64 || 64 % alignment)
        __builtin_trap();
    if (++live > peak)
        peak = live;
    return pool[allocated++];
}
void __wosy_core_free(void *pointer) {
    if (!live || released == 8 || release_order[released] >= allocated)
        __builtin_trap();
    unsigned id = release_order[released++];
    if (pointer != pool[id] || pool[id][0] != marker[id] || (id < 5 && pool[id][1] != marker[id] + 10))
        __builtin_trap();
    // This is a static trace pool, so release deliberately overwrites both
    // possible holder bytes. A misplaced early release cannot leave the old
    // contents intact and accidentally satisfy the post-recursion checksum.
    pool[id][0] = 0xee;
    pool[id][1] = 0xef;
    --live;
}
void __wosy_core_system_panic(void) { __builtin_trap(); }
extern unsigned char wosy_frame_zero(void), wosy_frame_two(void), wosy_frame_three(void);
extern unsigned char wosy_carry_zero(void), wosy_carry_two(void), wosy_carry_three(void);

int check(void) {
    if (wosy_frame_zero() != 0 || live || allocated || released || peak)
        return 1;
    if (wosy_frame_two() != 13 || live || allocated != 2 || released != 2 || peak != 2)
        return 2;
    if (wosy_frame_three() != 26 || live || allocated != 5 || released != 5 || peak != 3)
        return 3;
    peak = 0;
    if (wosy_carry_zero() != 30 || live || allocated != 6 || released != 6 || peak != 1)
        return 4;
    if (wosy_carry_two() != 40 || live || allocated != 7 || released != 7 || peak != 1)
        return 5;
    if (wosy_carry_three() != 50 || live || allocated != 8 || released != 8 || peak != 1)
        return 6;
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
    print("Native64: recursive checked selections and eight exact receipts released in order", flush=True)

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
    print("Wasm32: recursive checked selections and eight exact receipts released in order", flush=True)

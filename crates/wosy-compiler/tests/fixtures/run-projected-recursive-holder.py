"""Run with the IR directory from WOSY_PROJECTED_HOLDER_IR_DIR and Linux LLVM toolchain root."""
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
static _Alignas(64) unsigned char pool[5][64];
static unsigned allocated, live, peak, released;
static const unsigned extent[] = {4, 3, 5, 4, 3};
static const unsigned tag[] = {2, 1, 3, 2, 1};
static const unsigned order[] = {1, 0, 4, 3, 2};

void *__wosy_core_alloc(u64 size, u64 alignment) {
    if (allocated == 5 || size != extent[allocated] || !alignment || alignment > 64 || 64 % alignment)
        __builtin_trap();
    if (++live > peak)
        peak = live;
    return pool[allocated++];
}
void __wosy_core_free(void *pointer) {
    if (!live || released == 5 || order[released] >= allocated)
        __builtin_trap();
    unsigned id = order[released++];
    if (pointer != pool[id] || pool[id][0] != tag[id] + 3 ||
        pool[id][1] != tag[id] + 1 || pool[id][2] != tag[id] + 4)
        __builtin_trap();
    pool[id][0] = 0xe0;
    pool[id][1] = 0xe1;
    pool[id][2] = 0xe2;
    --live;
}
void __wosy_core_system_panic(void) { __builtin_trap(); }
extern unsigned char wosy_frame_zero(void), wosy_frame_two(void), wosy_frame_three(void);

int check(void) {
    if (wosy_frame_zero() != 0 || live || allocated || released || peak)
        return 1;
    if (wosy_frame_two() != 39 || live || allocated != 2 || released != 2 || peak != 2)
        return 2;
    if (wosy_frame_three() != 66 || live || allocated != 5 || released != 5 || peak != 3)
        return 3;
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
    print("Native64: projected retirement preserved both exact reads and five frame releases", flush=True)

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
    print("Wasm32: projected retirement preserved both exact reads and five frame releases", flush=True)

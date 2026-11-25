"""Run with the IR directory from WOSY_RECURSIVE_FALLIBLE_IR_DIR and Linux LLVM toolchain root."""
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
static _Alignas(64) u64 pool[4];
static unsigned attempts, allocated, live, peak, released;
static const unsigned tag[] = {3, 3, 2, 1};
static const unsigned order[] = {0, 3, 2, 1};

void *__wosy_core_alloc(u64 size, u64 alignment) {
    if (attempts == 5 || size != 8 || alignment != 8)
        __builtin_trap();
    if (attempts++ == 1)
        return (void *)0;
    if (allocated == 4)
        __builtin_trap();
    if (++live > peak)
        peak = live;
    return &pool[allocated++];
}
void __wosy_core_free(void *pointer) {
    if (!live || released == 4 || order[released] >= allocated)
        __builtin_trap();
    unsigned id = order[released++];
    if (pointer != &pool[id] || pool[id] != tag[id])
        __builtin_trap();
    // A successful ancestor reads its value after the child returns.
    pool[id] = 0xeeeeeeeeeeeeeeeeULL;
    --live;
}
void __wosy_core_system_panic(void) { __builtin_trap(); }
extern u64 wosy_no_allocation(void), wosy_three_frames(void);

int check(void) {
    if (wosy_no_allocation() != 0 || attempts || live || allocated || released)
        return 1;
    // The second allocator attempt fails: no initialization, descent or free
    // for it; the original successful ancestor still observes its own value.
    if (wosy_three_frames() != 503 || attempts != 2 || live || allocated != 1 || released != 1 || peak != 1)
        return 2;
    if (wosy_three_frames() != 123 || attempts != 5 || live || allocated != 4 || released != 4 || peak != 3)
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
    print("Native64: forced recursive allocation failure released only the four successful allocations", flush=True)

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
    print("Wasm32: forced recursive allocation failure released only the four successful allocations", flush=True)

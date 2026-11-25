"""Run with the IR directory from WOSY_PAIRED_HANDOFF_IR_DIR and Linux LLVM toolchain root."""
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
static _Alignas(64) unsigned char pool[4][64];
// Both array owners end in the same lexical scope. The allocator model fixes
// that endpoint, not the relative order of equivalent releases there. Record
// exact identities once each, and use the weighted result to verify lane order.
static unsigned allocated, live, peak, released;
static const unsigned marker[] = {19, 37, 19, 37};

void *__wosy_core_alloc(u64 size, u64 alignment) {
    if (allocated == 4 || size != 2 || !alignment || alignment > 64 || 64 % alignment)
        __builtin_trap();
    if (++live > peak)
        peak = live;
    return pool[allocated++];
}
void __wosy_core_free(void *pointer) {
    for (unsigned id = 0; id < allocated; ++id) {
        if (pointer == pool[id]) {
            if (!live || (released & (1u << id)) || pool[id][0] != marker[id])
                __builtin_trap();
            released |= 1u << id;
            pool[id][0] = 0xee;
            --live;
            return;
        }
    }
    __builtin_trap();
}
void __wosy_core_system_panic(void) { __builtin_trap(); }
extern unsigned char wosy_choose_left(void), wosy_choose_right(void);

int check(void) {
    if (wosy_choose_right() != 186 || live || allocated != 2 || released != 3 || peak != 2)
        return 1;
    if (wosy_choose_left() != 171 || live || allocated != 4 || released != 15 || peak != 2)
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
    print("Native64: each checked-holder choice kept both original array identities", flush=True)

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
    print("Wasm32: each checked-holder choice kept both original array identities", flush=True)

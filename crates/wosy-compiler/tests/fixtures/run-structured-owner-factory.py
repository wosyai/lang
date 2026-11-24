"""Run with the IR directory from WOSY_STRUCTURED_FACTORY_IR_DIR and Linux LLVM toolchain root."""
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
static const unsigned sizes[] = {2, 4, 3, 2, 5, 4, 3, 2};
static const unsigned marker[] = {29, 19, 19, 19, 19, 19, 19, 29};
static const unsigned release_order[] = {0, 2, 1, 3, 6, 5, 4, 7};

void *__wosy_core_alloc(u64 size, u64 alignment) {
    if (allocated == 8 || size != sizes[allocated] || !alignment || alignment > 64 || 64 % alignment)
        __builtin_trap();
    if (++live > peak)
        peak = live;
    return pool[allocated++];
}
void __wosy_core_free(void *pointer) {
    if (!live || released == 8 || release_order[released] >= allocated ||
        pointer != pool[release_order[released]] || pool[release_order[released]][0] != marker[release_order[released]])
        __builtin_trap();
    ++released;
    --live;
}
void __wosy_core_system_panic(void) { __builtin_trap(); }
extern u64 wosy_case_zero(void), wosy_case_two(void), wosy_case_three(void);

int check(void) {
    if (wosy_case_zero() != 2 || live || allocated != 1 || released != 1 || peak != 1)
        return 1;
    if (wosy_case_two() != 2 || live || allocated != 4 || released != 4 || peak != 3)
        return 2;
    if (wosy_case_three() != 2 || live || allocated != 8 || released != 8 || peak != 4)
        return 3;
    return 0;
}
#ifndef WASI
int main(void) { return check(); }
#endif
"""

# The named variant deliberately puts a different length in the alternate
# descriptor without changing its backing size or the allocation trace.
if sys.argv[3:] == ["named"]:
    runtime = runtime.replace("wosy_case_zero() != 2", "wosy_case_zero() != 12")
    runtime = runtime.replace("wosy_case_three() != 2", "wosy_case_three() != 12")
elif sys.argv[3:]:
    raise SystemExit("optional third argument must be named")

with tempfile.TemporaryDirectory(prefix="conditional-receipt-", dir=ir_dir) as tmp:
    work = Path(tmp)
    trace = work / "trace.c"
    trace.write_text(runtime)
    native = work / "check-native"
    run(clang, "-O0", trace, ir_dir / "native64.ll", "-o", native)
    run(native)
    print("Native64: alternate selected bases and eight exact receipts released in order", flush=True)

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
    print("Wasm32: alternate selected bases and eight exact receipts released in order", flush=True)

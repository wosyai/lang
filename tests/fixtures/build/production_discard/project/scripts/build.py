import subprocess
import sys

target, partition, output, manifest, runtime = sys.argv[1:]
native = target == "native"
clang = ["clang", "-O0"] if native else ["clang", "--target=wasm32-wasi", "-O0"]
main, core, observer, start = (f"{output}/{name}.o" for name in ("main", "runtime", "observer", "start"))
sources = [(partition, main), (runtime, core)]
if not native:
    sources.append(("scripts/start.ll", start))
for source, obj in sources:
    subprocess.run([*clang, "-c", "-x", "ir", source, "-o", obj], check=True)

symbols = subprocess.check_output(
    ["llvm-nm", "--undefined-only", "--just-symbol-name", main], text=True,
).splitlines()


def encoded(*parts):
    return "__".join(part.encode().hex() for part in parts)


def symbol(package, path, binding, module, member):
    prefix = "wosy_extern__" + encoded(package, path) + "__"
    suffix = "__" + encoded(binding, module, member)
    (match,) = (name for name in symbols if name.startswith(prefix) and name.endswith(suffix))
    return match


definitions = {
    "CHECKPOINT": symbol("production_discard", "src/main.w", "observer", "fixture_discard", "checkpoint"),
}
for member in ("_fd_read", "fd_write", "_poll_oneoff", "_proc_exit"):
    definitions[member.lstrip("_").upper()] = symbol(
        "std", "src/wasi/preview1.w", "_wasi", "wasi_snapshot_preview1", member,
    )
ops = ("__wosy_core_alloc", "__wosy_core_free")
if native:
    redirected = f"{output}/runtime-redirected.o"
    subprocess.run(["llvm-objcopy", *(f"--redefine-sym={op}=__real_{op}" for op in ops), core, redirected], check=True)
    definitions.update({f"__wrap_{op}": op for op in ops})
subprocess.run([
    *clang, "-ffreestanding", "-Wall", "-Wextra", "-Werror", "-c",
    *(f"-D{key}={value}" for key, value in definitions.items()),
    "scripts/observer.c", "-o", observer,
], check=True)
if native:
    builtins = subprocess.check_output([*clang, "--rtlib=compiler-rt", "--print-libgcc-file-name"], text=True).strip()
    link = [*clang, "-fuse-ld=lld", main, redirected, observer, builtins, "-Xlinker", "/stack:1048576,65536"]
else:
    link = ["wasm-ld", "--no-entry", "--export=_start", "-z", "stack-size=32768",
            *(f"--wrap={op}" for op in ops), main, start, core, observer]
subprocess.run([*link, "-o", f"{output}/main"], check=True)

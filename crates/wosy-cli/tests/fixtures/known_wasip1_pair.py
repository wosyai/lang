"""Private WASIp1 known-positive tool pair, not an ordinary or generic runner.

Requires the already reviewed stable local Linux tool image and one Wosy-owned
scratch directory. This tool is not installed or invoked by a Wosy command.
Only the previously inspected evaluator and linked module are admitted.
"""
import ctypes as C
import errno
import hashlib
import json
import os
from pathlib import Path
import resource
import selectors
import signal
import stat
import subprocess
import sys
import tempfile
import time

EVALUATOR = "a9a93423c9c29f60ee4b4bf8d24014f4b515f2bb1ce18228ef77794001e37ec4"
MODULE = "c84f592ce0a7e48557a3c1c2d90d553b9b61d46e56be8594d4bffb18d479ffa9"
CLANG = "llvm/usr/lib/llvm-18/bin/clang"
LINKER = "llvm/usr/lib/llvm-18/bin/wasm-ld"
WASMTIME = "wasmtime/wasmtime"
TEMPLATE = "400e5ab21c1b874a2c297fdd4a48c80dfb76d17127ba1df31d1083641879d922"

BASE_IMAGE = (
    ("/usr/bin/python3", "889c603f0d17cb54060951bcf4c4f9b8c9ebd9e52b392c70209bbb9755d797d9"),
    ("/usr/bin/bwrap", "573236e5328ac2ebb08f59ae3a9805b4f8d12bdef14be8af4450d5463294985f"),
    ("/usr/bin/env", "eb1c64bbc9908afcdf8cb48524f17112d8dd76a249f80dee2f01da14747b3b6e"),
    ("/lib/x86_64-linux-gnu/libseccomp.so.2", "c851a2378b4ef3a42ada5f8ff1604ad38f6bbcafa2aeea3f6680160d628c2f27"),
    ("/lib64/ld-linux-x86-64.so.2", "8642358c60bdf4579911381070c69cc7b1a447809b73d483ce799c4421609d21"),
    ("/lib/x86_64-linux-gnu/libc.so.6", "cc7377e8816cefadbb4ee083f5387f67a4a883d90bc273160842a6ac075af662"),
    ("/lib/x86_64-linux-gnu/libselinux.so.1", "5cb5293b4e7590463b94f7b23152d54807a6a2ccaf8cac9e343908c6692c086c"),
    ("/lib/x86_64-linux-gnu/libcap.so.2", "41217b0af128444f66dee8adda52b2ceaf19da00e5b0b155e2b87b857671286b"),
    ("/lib/x86_64-linux-gnu/libpcre2-8.so.0", "92e9b4167f9bbc237ca8f5526d82ee99ec9f8dbd0270ef1dd0ab24e2217f48e3"),
    ("/lib/x86_64-linux-gnu/libz.so.1", "6c13931760210e8b391b63a337dfe4071346c4ad6e3827694edb7b6989bb16dd"),
    ("/lib/x86_64-linux-gnu/libexpat.so.1", "36e7e50d21edcabed383bfddc594bb261c1c7cce456afdf23c5f53e849cda67c"),
)
BUILDER_IMAGE = (
    (CLANG, "f560497bdaa7590c3111ec89f2df56288f6b5d03b1cba579e9c052ede2e28e19"),
    (LINKER, "44d2e0a9ba012ba9727f9338b05fde68ce6d2f93f79f3a04e7140b448c01e1ab"),
    ("llvm/usr/lib/llvm-18/lib/libclang-cpp.so.18.1", "9bcf6d52c626468cebd6e2411e9630d5ecc6d051f4c51d1a452359c9915e0a6d"),
    ("llvm/usr/lib/x86_64-linux-gnu/libLLVM-18.so.18.1", "d3c61980156773d285ea71c0968e9a69feae9777c59f24c6d589d1c78a846fbf"),
    ("/usr/lib/x86_64-linux-gnu/libbsd.so.0", "167e3fa2b6320ce61534a48002e6d0921f61d61eae88a669c117e23ed3de9f37"),
    ("/usr/lib/x86_64-linux-gnu/libedit.so.2", "4cde661dc336a1a835e605532086c629f6279a312374c91795215b8489ad4bcf"),
    ("/usr/lib/x86_64-linux-gnu/libffi.so.8", "922024ec5a23f7de1095b1cff2f6cbfb377b5d58cd51fd0ec2e8fe35427653e2"),
    ("/usr/lib/x86_64-linux-gnu/libgcc_s.so.1", "30c61ab012a4241bed033725a09b61f5fdd3bb7df95ee852d0b096520524c7af"),
    ("/usr/lib/x86_64-linux-gnu/liblzma.so.5", "7ca392e92ebb94b6f8ee094cbd5243185e4452377b94acc657f7ba8c8f9dda2c"),
    ("/usr/lib/x86_64-linux-gnu/libm.so.6", "6d567d53e895273ca14a1f9dc164fc6c8d39aed2f60aa46a733c2784228915f3"),
    ("/usr/lib/x86_64-linux-gnu/libmd.so.0", "1c8c51474c0eb67dabd9f1605097eb58195cdabef64427ae17cfcd2053c37479"),
    ("/usr/lib/x86_64-linux-gnu/libstdc++.so.6", "972bb2a18b71140dab0240f8a1f68ab3fb1d56bcd4c4f824a91b70888faf5a00"),
    ("/usr/lib/x86_64-linux-gnu/libtinfo.so.6", "4013e7651ac68f547d8333517def25b2366c97eec7de7f06e105dbf3444ffebf"),
    ("/usr/lib/x86_64-linux-gnu/libxml2.so.2", "8f2643af68a9eba917929046806dabd1848867068d0eee0f6799288a8e886963"),
    ("/usr/lib/x86_64-linux-gnu/libz.so.1", "6c13931760210e8b391b63a337dfe4071346c4ad6e3827694edb7b6989bb16dd"),
    ("/usr/lib/x86_64-linux-gnu/libz3.so.4", "7accc397d4ac387468b09489bdaff9d82bfac4ca6a7e339b8fbb2167edf9c1c0"),
    ("/usr/lib/x86_64-linux-gnu/libzstd.so.1", "27f07c9a49c2c956bcfb64cd4712976586a66facbf15fc7f09bc37413b5f2b21"),
)
RUNNER_IMAGE = (
    (WASMTIME, "9f3f3e1b1b048f802c1d35a17607c064425a9c746f00bda3c2466b92ddcb5c92"),
    ("/lib/x86_64-linux-gnu/libdl.so.2", "295fa521a03cd2faa99974f378c9e23dd622021ec7d32bcad4f8ea61aec8a872"),
    ("/lib/x86_64-linux-gnu/libgcc_s.so.1", "30c61ab012a4241bed033725a09b61f5fdd3bb7df95ee852d0b096520524c7af"),
    ("/lib/x86_64-linux-gnu/libpthread.so.0", "85e21f7dba0394411d00959176fd18b470e575b0b05f1f4f41e5636802ce0500"),
    ("/lib/x86_64-linux-gnu/libm.so.6", "6d567d53e895273ca14a1f9dc164fc6c8d39aed2f60aa46a733c2784228915f3"),
)

def require(ok, message):
    if not ok:
        raise RuntimeError(message)

def read_stage_file(path, limit):
    fd = os.open(path, os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK | os.O_CLOEXEC)
    with os.fdopen(fd, "rb") as file:
        meta = os.fstat(file.fileno())
        require(stat.S_ISREG(meta.st_mode) and meta.st_uid == os.geteuid() and meta.st_nlink == 1,
                "unexpected file in compiler stage")
        require(meta.st_size <= limit, "compiler stage file exceeds its bound")
        data = file.read(limit + 1)
        require(len(data) <= limit, "compiler stage file exceeds its bound")
        return data

def pin(path, expected, capture_limit=0):
    fd = os.open(path, os.O_RDONLY | os.O_NONBLOCK | os.O_CLOEXEC)
    with os.fdopen(fd, "rb") as file:
        meta = os.fstat(file.fileno())
        require(stat.S_ISREG(meta.st_mode) and not meta.st_mode & 0o022, "unexpected tool image file")
        require(os.fstatvfs(file.fileno()).f_flag & os.ST_RDONLY, "private tool image must be read-only")
        if capture_limit:
            source = file.read(capture_limit + 1)
            require(len(source) <= capture_limit, "private recipe exceeds its bound")
            digest = hashlib.sha256(source)
        else:
            source = None
            digest = hashlib.file_digest(file, "sha256")
        require(digest.hexdigest() == expected, "private tool image differs")
        return source

def load_manifest(path):
    require(path.is_absolute() and path.name == "comptime-manifest", "private Wosy manifest is required")
    stage = path.parent
    meta = stage.stat(follow_symlinks=False)
    require(stat.S_ISDIR(meta.st_mode) and meta.st_uid == os.geteuid()
            and stat.S_IMODE(meta.st_mode) == 0o700, "private Wosy stage is required")
    expected = stage / "evaluator.module"
    contents = json.loads(read_stage_file(path, 16384))
    require(contents == {"executable": str(expected), "parameters": {}, "permissions": [], "mounts": []},
            "private comptime manifest has different inputs")
    return expected

def no_network():
    # Same restrictive host helper policy as the independently verified
    # fixture; Wasmtime's process-private notifier may use an AF_UNIX pair.
    sec = C.CDLL("/lib/x86_64-linux-gnu/libseccomp.so.2")
    sec.seccomp_init.argtypes, sec.seccomp_init.restype = [C.c_uint32], C.c_void_p
    sec.seccomp_syscall_resolve_name.argtypes = [C.c_char_p]
    sec.seccomp_rule_add.argtypes = [C.c_void_p, C.c_uint32, C.c_int, C.c_uint]
    sec.seccomp_load.argtypes = [C.c_void_p]
    ctx = sec.seccomp_init(0x7fff0000)
    require(ctx, "cannot instantiate private host policy")
    deny = 0x00050000 | errno.EPERM
    for name in b"socket connect bind listen accept accept4 sendto sendmsg sendmmsg recvfrom recvmsg recvmmsg io_uring_setup io_uring_enter io_uring_register".split():
        nr = sec.seccomp_syscall_resolve_name(name)
        require(nr >= 0 and sec.seccomp_rule_add(ctx, deny, nr, 0) == 0, "cannot restrict host network")
    class Argument(C.Structure):
        _fields_ = [("arg", C.c_uint), ("op", C.c_int), ("a", C.c_uint64), ("b", C.c_uint64)]
    sec.seccomp_rule_add_array.argtypes = [C.c_void_p, C.c_uint32, C.c_int, C.c_uint, C.POINTER(Argument)]
    nr = sec.seccomp_syscall_resolve_name(b"socketpair")
    require(nr >= 0 and sec.seccomp_rule_add_array(ctx, deny, nr, 1, C.byref(Argument(0, 1, 1, 0))) == 0,
            "cannot restrict host socket families")
    return lambda: sec.seccomp_load(ctx)

def child(command, directory, load_policy, writes=None, stdout_cap=0, environment=()):
    # No inherited environment, stdin, capabilities or extra file descriptors.
    # The builder alone has a writable bind of its own disposable object dir.
    box = ["/usr/bin/bwrap", "--ro-bind", "/", "/", "--dev", "/dev", "--proc", "/proc",
           "--die-with-parent", "--clearenv", "--chdir", str(directory)]
    if writes is not None:
        box.extend(("--bind", str(writes), str(writes)))
    box.extend(("--", "/usr/bin/env", "-i", *(f"{key}={value}" for key, value in environment), *map(str, command)))
    def limits():
        if load_policy() != 0:
            os._exit(126)
        resource.setrlimit(resource.RLIMIT_AS, (1 << 30, 1 << 30))
        resource.setrlimit(resource.RLIMIT_CPU, (2, 3))
        resource.setrlimit(resource.RLIMIT_NOFILE, (32, 32))
        resource.setrlimit(resource.RLIMIT_CORE, (0, 0))
        limit = (1 << 20) if writes is not None else 0
        resource.setrlimit(resource.RLIMIT_FSIZE, (limit, limit))

    halted = time.monotonic() + 5
    proc = subprocess.Popen(box, cwd=directory, env={}, stdin=subprocess.DEVNULL, stdout=subprocess.PIPE,
                            stderr=subprocess.PIPE, close_fds=True, start_new_session=True, preexec_fn=limits)
    try:
        with selectors.DefaultSelector() as select:
            streams = ((proc.stdout, bytearray(), stdout_cap), (proc.stderr, bytearray(), 16384))
            for stream, data, cap in streams:
                select.register(stream, selectors.EVENT_READ, (data, cap))
            while select.get_map():
                remaining = halted - time.monotonic()
                require(remaining > 0, "private tool deadline elapsed")
                available = select.select(remaining)
                require(available, "private tool deadline elapsed")
                for key, _ in available:
                    data, cap = key.data
                    span = os.read(key.fd, cap + 1 - len(data))
                    if not span:
                        select.unregister(key.fileobj)
                        continue
                    data.extend(span)
                    require(len(data) <= cap, "private tool exceeded its output bound")
            status = proc.wait(timeout=max(0, halted - time.monotonic()))
            return status, streams[0][1], streams[1][1]
    finally:
        if proc.poll() is None:
            try:
                os.killpg(proc.pid, signal.SIGKILL)
            except ProcessLookupError:
                pass
            proc.wait(timeout=2)
        proc.stdout.close()
        proc.stderr.close()

def load_template():
    path = Path(__file__).with_name("wasip1_pair_template.py")
    # Compile the exact single bounded read, never a second pathname
    # lookup, an import search path, or a bytecode cache.
    source = pin(path, TEMPLATE, capture_limit=4096)
    recipe = {}
    exec(compile(source, str(path), "exec"), recipe)
    return recipe

def build(root, evaluator, output, module, policy, recipe):
    require(output == module.parent and evaluator == output / "evaluator.ll", "builder paths do not match the stage")
    ir = read_stage_file(evaluator, 13981)
    require(hashlib.sha256(ir).hexdigest() == EVALUATOR, "evaluator is not the inspected input")
    with tempfile.TemporaryDirectory(dir=output, prefix="known-p1-objects.") as tmp:
        stage = Path(tmp)
        for path, data in (("evaluator.ll", ir), ("start.ll", recipe["START"].encode()),
                           ("allow-undefined.txt", recipe["ALLOW_UNDEFINED"])):
            with (stage / path).open("xb") as file:
                file.write(data)
        compile_env = (("LD_LIBRARY_PATH", ":".join(map(str, (
            root / "llvm/usr/lib/llvm-18/lib", root / "llvm/usr/lib/x86_64-linux-gnu")))),)
        for argv in recipe["builder_commands"](root / CLANG, root / LINKER):
            status, _, errors = child(argv, stage, policy, writes=stage, environment=compile_env)
            sys.stderr.buffer.write(errors)
            require(status == 0, "private builder failed")
        wasm = read_stage_file(stage / "report.wasm", 1430)
        require(hashlib.sha256(wasm).hexdigest() == MODULE, "linked module is not the inspected artifact")
        fd = os.open(module, os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW | os.O_CLOEXEC, 0o600)
        with os.fdopen(fd, "wb") as file:
            file.write(wasm)

def run(root, module, policy, recipe):
    wasm = read_stage_file(module, 1430)
    require(hashlib.sha256(wasm).hexdigest() == MODULE, "runner module is not the inspected artifact")
    argv = recipe["runner_command"](root / WASMTIME, module)
    status, stdout, stderr = child(argv, module.parent, policy, stdout_cap=34)
    sys.stderr.buffer.write(stderr)
    if status == 0:
        sys.stdout.buffer.write(stdout)
    return status if 0 <= status <= 255 else 1

def main():
    require(len(sys.argv) >= 3, "missing private tool mode")
    mode, root, *args = sys.argv[1:]
    require((mode == "build" and len(args) == 3) or (mode == "run" and len(args) == 1),
            "expected private 3/1 comptime arguments")
    root = Path(root)
    require(root.is_absolute() and not Path("/etc/ld.so.preload").exists(), "expected pinned host tool image")
    module = load_manifest(Path(args[-1]))
    for path, digest in (*BASE_IMAGE, *(BUILDER_IMAGE if mode == "build" else RUNNER_IMAGE)):
        pin(root / path, digest)
    recipe = load_template()
    policy = no_network()
    if mode == "build":
        build(root, Path(args[0]), Path(args[1]), module, policy, recipe)
        return 0
    return run(root, module, policy, recipe)

if __name__ == "__main__":
    try:
        sys.exit(main())
    except (OSError, RuntimeError, ValueError, subprocess.SubprocessError) as error:
        print(str(error), file=sys.stderr)
        sys.exit(1)

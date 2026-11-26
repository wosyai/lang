"""Inspect closed scalar IR, then link only the independently accepted bytes."""
import hashlib
import json
import os
from pathlib import Path
import selectors
import signal
import subprocess
import sys
import time


mode, fixture, *arguments = sys.argv[1:]
root = Path(fixture).resolve(strict=True)
report = root / "inspection.json"
source = '''%%start
# WHY: parâmetros estáticos ☕
i32 result = comptime evaluate_limit {
    i32 limit = core.comp_param("limit", 6);
    i32 factor = core.comp_param("factor", 2);
    (limit ^ factor) | 32
};
bool flag = comptime evaluate_flag {
    bool enabled = core.comp_param("flag", true);
    enabled == true
};
i32 unavailable = comptime required { core.comp_param("no_default") };
i32 rejected = comptime wrong_default { core.comp_param("limit", true) };
i32 overflow_contract = comptime checked_operator { i32 operand = 2; operand + 1 };
i32 negate_contract = comptime checked_negate { i32 operand = 2; -operand };
bool ordered = comptime ordered_comparison { u32 operand = 2; (1 | operand) < operand };
%%end
'''
roundtrip_source = source.replace(
    "i32 unavailable", 'u32 unsigned = comptime evaluate_unsigned { core.comp_param("unsigned", 4294967295) };\ni32 unavailable',
    1,
)
manifest = '''
[package]
name = "fixture"
default_target = "app"

[comments]
categories = ["WHY"]

[builders.never]
command = ["fixture-command-must-not-run"]
toolchain_identity = "fixture"

[runners.never]
command = ["fixture-runner-must-not-run"]

[targets.app]
root = "src/main.w"

[targets.app.artifacts.main]
artifact = "Main"
'''
profiles = {
    "dev": 'limit = 9\nfactor = 3\nflag = false\n"std.environment" = "unread"',
    "other": "limit = 11",
    "defaults": None,
    "wrong": "limit = true",
    "unsupported": 'limit = "not the i32 value"',
}
for profile, params in profiles.items():
    manifest += f'''
[targets.app.artifacts.main.profiles.{profile}]
backend = "llvm"
output = "native"
builder = "never"
run_runner = "never"

[targets.app.profiles.{profile}]
main_artifact = "main"
comptime_builder = "never"
comptime_runner = "never"
'''
    if params:
        manifest += f"\n[targets.app.profiles.{profile}.params]\n{params}\n"


def run(*args, refusal=None, cwd=None):
    result = subprocess.run(args, cwd=cwd, capture_output=True, timeout=30)
    if refusal is not None:
        assert result.returncode == 1 and not result.stdout and refusal in result.stderr, result.stderr
        return None
    assert result.returncode == 0, result.stderr
    return result.stdout


cases = [("dev", "evaluate_limit", 42), ("other", "evaluate_limit", 41), ("defaults", "evaluate_limit", 36),
         ("dev", "evaluate_flag", 0), ("defaults", "evaluate_flag", 1)]
layouts = ("native", "wasi")
roundtrip_cases = [("dev", "evaluate_limit", 42, "I32"),
                   ("defaults", "evaluate_unsigned", 4294967295, "U32"),
                   ("dev", "evaluate_flag", False, "Bool"),
                   ("defaults", "evaluate_flag", True, "Bool")]
MAX_REPORT = 1 << 16
MAX_IR = 1 << 20
MAX_PROGRAM = 4 << 20
MAX_TOOL = 128 << 20
MAX_WORD = 12
native_bool_adapter = b"""declare i1 @evaluate()

define i32 @evaluate_bool_i32() {
entry:
  %result = call i1 @evaluate()
  %extended = zext i1 %result to i32
  ret i32 %extended
}
"""


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def owned_bytes(path, limit):
    if not path.is_relative_to(root) or path.resolve(strict=True) != path or not path.is_file():
        raise RuntimeError(f"not an owned fixture file: {path}")
    with path.open("rb") as file:
        data = file.read(limit + 1)
    if len(data) > limit:
        raise RuntimeError(f"fixture file exceeds its bound: {path}")
    return data


def sha(data):
    return hashlib.sha256(data).hexdigest()


def require(condition, message):
    if not condition:
        raise RuntimeError(message)


def installed_digest(path):
    require(path.is_absolute() and path.is_file() and path.stat().st_size <= MAX_TOOL,
            "expected a bounded locally approved tool")
    amount = 0
    hash_value = hashlib.sha256()
    with path.open("rb") as file:
        while data := file.read(1 << 16):
            amount += len(data)
            require(amount <= MAX_TOOL, "locally approved tool grew past its fixture bound")
            hash_value.update(data)
    return hash_value.hexdigest()


def capture_owned(argv, cwd, env, *, input_word=None, stdout_max=MAX_IR * 2, timeout=30,
                  allowed_exit=0):
    """Bound both pipes and the direct owned process group; no shell or captured command."""
    require(Path(argv[0]).is_absolute(), "fixture executable must be absolute")
    require(input_word is None or len(input_word) <= MAX_WORD, "fixture stdin exceeds word bound")
    deadline = time.monotonic() + timeout
    child = subprocess.Popen(argv, cwd=cwd, env=env, start_new_session=True,
                             stdin=subprocess.PIPE if input_word is not None else subprocess.DEVNULL,
                             stdout=subprocess.PIPE, stderr=subprocess.PIPE)
    try:
        if time.monotonic() >= deadline:
            raise TimeoutError("owned fixture process deadline")
        if input_word is not None:
            child.stdin.write(input_word)
            child.stdin.close()
        buffers = [bytearray(), bytearray()]
        limits = [stdout_max, MAX_REPORT]
        with selectors.DefaultSelector() as waiting:
            for index, pipe in enumerate((child.stdout, child.stderr)):
                waiting.register(pipe, selectors.EVENT_READ, index)
            while waiting.get_map():
                remaining = deadline - time.monotonic()
                if remaining <= 0:
                    raise TimeoutError("owned fixture process deadline")
                for key, _ in waiting.select(remaining):
                    index = key.data
                    request = min(8192, limits[index] + 1 - len(buffers[index]))
                    chunk = os.read(key.fd, request)
                    if not chunk:
                        waiting.unregister(key.fileobj)
                        continue
                    buffers[index].extend(chunk)
                    require(len(buffers[index]) <= limits[index], "owned fixture process output exceeds bound")
        remaining = deadline - time.monotonic()
        if remaining <= 0:
            raise TimeoutError("owned fixture process deadline")
        code = child.wait(timeout=remaining)
        require(code == allowed_exit, f"owned fixture process exit {code}: {bytes(buffers[1])!r}")
        return tuple(bytes(buffer) for buffer in buffers)
    except BaseException:
        # Only the session created for this trusted test tool.
        if child.returncode is None:
            try:
                os.killpg(child.pid, signal.SIGKILL)
            except ProcessLookupError:
                pass
            child.wait()
        raise
    finally:
        child.stdout.close()
        child.stderr.close()
        if child.stdin is not None:
            child.stdin.close()


class FixtureTools:
    def __init__(self, toolchain):
        top = Path(toolchain).resolve(strict=True)
        llvm = top / "llvm/usr/lib/llvm-18/bin"
        self.executables = {name: llvm / name for name in ("clang", "llvm-as", "llc", "wasm-ld")}
        self.executables["wasmtime"] = top / "wasmtime/wasmtime"
        require(all(tool.is_file() and tool.resolve(strict=True).is_relative_to(top)
                    for tool in self.executables.values()),
                "expected the authorized installed LLVM/Wasmtime fixture tools")
        self.env = {"PATH": f"{llvm}:/usr/bin:/bin", "LANG": "C.UTF-8", "LC_ALL": "C.UTF-8",
                    "HOME": str(root), "XDG_CACHE_HOME": str(root / "fixture-cache"),
                    "LD_LIBRARY_PATH": str(top / "llvm/usr/lib/x86_64-linux-gnu"),
                    "LIBRARY_PATH": str(top / "system-libs")}

    def run(self, tool, *args, **kwargs):
        return capture_owned([self.executables[tool], *args], root, self.env, **kwargs)[0]

    def driver(self, executable, *args, **kwargs):
        return capture_owned([executable, *args], root, self.env, **kwargs)

    def stamps(self, driver):
        return {name: installed_digest(path) for name, path in [*self.executables.items(), ("driver", driver)]}


def fixture_cases():
    for index, (layout, (profile, id, want)) in enumerate(
        (layout, case) for layout in layouts for case in cases
    ):
        yield layout, profile, id, want, root / f"fragment-{index}.ll"


def roundtrip_fixture_cases():
    for ordinal, (layout, (profile, id, want, kind)) in enumerate(
        (layout, case) for layout in layouts for case in roundtrip_cases
    ):
        yield ordinal, layout, profile, id, want, kind, root / f"fragment-{ordinal}.ll"


def runtime_path(layout):
    return root / f"runtime-{layout}.ll"


def inspect(driver, ordinary):
    # Begin with one disposable directory owned by the caller.
    assert not list(root.iterdir())
    project = root / "project"
    (project / "src").mkdir(parents=True)
    (project / "wosy.toml").write_text(manifest)
    (project / "src/main.w").write_text(source)

    def fragment(profile, id, layout, **kwargs):
        data = run(driver, project, "app", profile, id, layout, **kwargs)
        return json.loads(data) if data else None

    observed = []
    for layout, profile, id, want, ir in fixture_cases():
        start = time.perf_counter()
        value = fragment(profile, id, layout)
        plan_ms = 1000 * (time.perf_counter() - start)
        span = value["site"]["range"]
        assert value["id"] == id
        assert source.encode()[span["start"]:span["end"]].startswith(f"comptime {id} {{".encode())
        assert value["result_type"] == ("Bool" if id.endswith("flag") else "I32")
        ir.write_text(value["llvm"])
        runtime = runtime_path(layout)
        if not runtime.exists():
            runtime.write_text(value["runtime"])
        assert runtime.read_text() == value["runtime"]
        observed.append({"layout": layout, "profile": profile, "site": id,
                         "fixture_expected": want, "type": value["result_type"],
                         "file": ir.name, "sha256": digest(ir), "plan_ms": plan_ms})
    fragment("dev", "required", "native", refusal=b'reason: Missing')
    fragment("dev", "wrong_default", "native", refusal=b'reason: DefaultType')
    fragment("wrong", "evaluate_limit", "wasi", refusal=b"expression type does not match expected type")
    fragment("unsupported", "evaluate_limit", "native", refusal=b'reason: UnsupportedProfileValue')
    fragment("not_configured", "evaluate_limit", "wasi", refusal=b'not_configured')
    for id in ("checked_operator", "checked_negate", "ordered_comparison"):
        fragment("dev", id, "wasi", refusal=b"not a closed scalar expression")
    assert not (project / ".wosy").exists()
    run(ordinary, "build", "app", "--profile", "dev", cwd=project, refusal=b"requires specialization")
    assert not (project / ".wosy/artifacts").exists()
    report.write_text(json.dumps({"fragments": observed, "runtime": {layout: digest(runtime_path(layout)) for layout in layouts}}))
    print(json.dumps({"inspection_sha256": digest(report), "fragments": len(observed), "linked": False}))


def link(layout, ir, result_type, ordinal, invoke=run):
    output = root / f"link-{ordinal}"
    output.mkdir()
    runtime = runtime_path(layout)
    program = output / "evaluation"
    if layout == "native":
        harness = output / "harness.c"
        native_ir = [ir, runtime]
        symbol = "evaluate"
        if result_type == "Bool":
            # Wosy's bare i1 is not a C _Bool ABI. Adapt it within LLVM,
            # and expose only the exact i32 boundary to the C harness.
            adapter = output / "native-bool-adapter.ll"
            adapter.write_bytes(native_bool_adapter)
            native_ir.append(adapter)
            symbol = "evaluate_bool_i32"
        harness.write_text(f"""#include <stdint.h>
#include <stdio.h>
extern int __real_main(void);
extern int32_t {symbol}(void);
int __wrap_main(void) {{
    if (__real_main() != 0) return 1;
    printf("%d\\n", (int){symbol}());
    return 0;
}}
""")
        invoke("clang", "-x", "ir", *native_ir, "-x", "c", harness, "-Wl,--wrap=main", "-o", program)
    else:
        objects = []
        for index, module in enumerate((ir, runtime)):
            bc, obj = (output / f"module-{index}.{ext}" for ext in ("bc", "o"))
            invoke("llvm-as", module, "-o", bc)
            invoke("llc", "-mtriple=wasm32-wasi", "-filetype=obj", bc, "-o", obj)
            objects.append(obj)
        invoke("wasm-ld", "--no-entry", "--export=evaluate", *objects, "-o", program)
    return program


def execute(accepted_inspection, accepted_adapter):
    assert digest(report) == accepted_inspection
    assert hashlib.sha256(native_bool_adapter).hexdigest() == accepted_adapter
    receipt = json.loads(report.read_bytes())
    for layout in layouts:
        assert digest(runtime_path(layout)) == receipt["runtime"][layout]
    admitted = list(zip(fixture_cases(), receipt["fragments"], strict=True))
    # Verify the complete reviewed set before invoking any external tool.
    for (layout, profile, id, want, ir), recorded in admitted:
        assert (layout, profile, id, want, ir.name) == tuple(
            recorded[key] for key in ("layout", "profile", "site", "fixture_expected", "file")
        )
        assert digest(ir) == recorded["sha256"]
    observed = {}
    for ordinal, ((layout, profile, id, want, ir), recorded) in enumerate(admitted):
        start = time.perf_counter()
        program = link(layout, ir, recorded["type"], ordinal)
        output = run(program) if layout == "native" else run("wasmtime", "run", "--invoke", "evaluate", program)
        assert output.strip() == str(want).encode(), (layout, output)
        observed.setdefault(layout, []).append(
            {"profile": profile, "site": id, "value": want, "link_run_ms": 1000 * (time.perf_counter() - start)}
        )
    print(json.dumps(observed))


def fixture_driver_arg(path, toolchain, profile, site, layout, mode, *arguments, **options):
    return toolchain.driver(path, root / "project", "app", profile, site, layout, mode,
                            *arguments, **options)


def save_report(path, value):
    data = json.dumps(value, sort_keys=True, separators=(",", ":")).encode()
    require(len(data) <= MAX_REPORT, "closed fixture report exceeded cap")
    with path.open("xb") as file:
        file.write(data)
    return sha(data)


def reviewed_report(path, expected):
    data = owned_bytes(path, MAX_REPORT)
    require(sha(data) == expected, f"fixture review digest mismatch: {path.name}")
    return json.loads(data)


def inspect_roundtrip(driver, toolchain):
    require(root.is_dir() and root.stat().st_uid == os.getuid() and not list(root.iterdir()),
            "expected a fresh owned fixture directory")
    project = root / "project"
    (project / "src").mkdir(parents=True)
    (project / "wosy.toml").write_text(manifest, encoding="utf-8")
    (project / "src/main.w").write_text(roundtrip_source, encoding="utf-8")
    tools = FixtureTools(toolchain)
    stamps = tools.stamps(driver)
    rows, runtimes = [], {}
    for ordinal, layout, profile, site, _, kind, ir in roundtrip_fixture_cases():
        output, _ = fixture_driver_arg(driver, tools, profile, site, layout, "fixture-stage")
        staged = json.loads(output)
        runtime = staged["runtime"].encode()
        if layout not in runtimes:
            with runtime_path(layout).open("xb") as file:
                file.write(runtime)
            runtimes[layout] = sha(runtime)
        require(sha(runtime) == runtimes[layout], "fixture runtime differed within layout")
        llvm = staged["llvm"].encode()
        require(len(llvm) <= MAX_IR and len(runtime) <= MAX_IR, "fixture IR exceeds cap")
        require(staged["id"] == site and staged["result_type"] == kind, "wrong compiler staging result")
        span = staged["site"]["range"]
        authored = roundtrip_source.encode()[span["start"]:span["end"]]
        require(authored.startswith(f"comptime {site} ".encode()), "fixture did not select the authored site")
        with ir.open("xb") as file:
            file.write(llvm)
        rows.append(dict(ordinal=ordinal, layout=layout, profile=profile, site=site, kind=kind,
                         ir=sha(llvm), observation=staged["observation"]))
    require(not (project / ".wosy").exists(), "fixture unexpectedly touched Build state")
    inspected = save_report(root / "staged.json", dict(
        rows=rows, runtimes=runtimes, tools=stamps, bool_adapter=sha(native_bool_adapter),
    ))
    print(json.dumps(dict(staging_sha256=inspected, fragments=len(rows), linked=False, executed=False)))


def verify_staged_fixture(driver, tools, accepted):
    staged = reviewed_report(root / "staged.json", accepted)
    require(staged["tools"] == tools.stamps(driver), "locally approved fixture tools changed")
    require(staged["bool_adapter"] == sha(native_bool_adapter), "fixture Bool adapter changed")
    rows = list(zip(roundtrip_fixture_cases(), staged["rows"], strict=True))
    for (ordinal, layout, profile, site, _, kind, ir), row in rows:
        require((ordinal, layout, profile, site, kind) ==
                tuple(row[key] for key in ("ordinal", "layout", "profile", "site", "kind")),
                "unexpected staged fixture scope")
        require(sha(owned_bytes(ir, MAX_IR)) == row["ir"], "staged fixture IR changed")
    for layout in layouts:
        require(sha(owned_bytes(runtime_path(layout), MAX_IR)) == staged["runtimes"][layout],
                "fixture runtime changed")
    return rows, staged["runtimes"]


def link_roundtrip(driver, toolchain, accepted_stage):
    tools = FixtureTools(toolchain)
    rows, _ = verify_staged_fixture(driver, tools, accepted_stage)
    programs = []
    for (ordinal, layout, _, _, _, kind, ir), _ in rows:
        program = link(layout, ir, kind, ordinal, tools.run)
        data = owned_bytes(program, MAX_PROGRAM)
        require(data.startswith(b"\x7fELF" if layout == "native" else b"\0asm"),
                "wrong fixture binary format")
        programs.append(dict(ordinal=ordinal, bytes=len(data), sha256=sha(data)))
    checked = save_report(root / "linked.json", dict(staged=accepted_stage, programs=programs))
    print(json.dumps(dict(linked_sha256=checked, artifacts=len(programs), executed=False)))


def verify_fresh_fixture(driver, tools, rows, runtimes):
    owned_bytes(root / "project/wosy.toml", MAX_REPORT)
    owned_bytes(root / "project/src/main.w", MAX_REPORT)
    for (_, layout, profile, site, _, kind, _), row in rows:
        output, _ = fixture_driver_arg(driver, tools, profile, site, layout, "fixture-stage")
        fresh = json.loads(output)
        require(fresh["observation"] == row["observation"] and fresh["result_type"] == kind
                and sha(fresh["llvm"].encode()) == row["ir"]
                and sha(fresh["runtime"].encode()) == runtimes[layout],
                "fixture source or selection changed before launch")


def stale_control(driver, tools, rows, runtimes):
    (_, layout, profile, site, _, _, _), row = rows[0]
    path = root / "project/wosy.toml"
    original = owned_bytes(path, MAX_REPORT)
    try:
        changed = original.replace(b"limit = 9\n", b"limit = 10\n", 1)
        require(changed != original, "expected authored selected parameter in fixture")
        path.write_bytes(changed)
        try:
            verify_fresh_fixture(driver, tools, rows, runtimes)
        except RuntimeError as refusal:
            require(str(refusal) == "fixture source or selection changed before launch",
                    f"unexpected stale preflight refusal: {refusal}")
        else:
            raise RuntimeError("stale manifest did not block fixture launch preflight")
        output, log = fixture_driver_arg(driver, tools, profile, site, layout, "fixture-readback",
                                         row["observation"], allowed_exit=1)
        require(not output and b"fixture source or selection changed" in log,
                "stale manifest did not block typed fixture readback")
    finally:
        path.write_bytes(original)


def readback_roundtrip(driver, toolchain, accepted_stage, accepted_link):
    tools = FixtureTools(toolchain)
    rows, runtimes = verify_staged_fixture(driver, tools, accepted_stage)
    linked = reviewed_report(root / "linked.json", accepted_link)
    require(linked["staged"] == accepted_stage, "linked fixture belongs to another staging review")
    launch = []
    for ((ordinal, layout, profile, site, expected, kind, _), row), binary in zip(
        rows, linked["programs"], strict=True,
    ):
        program = root / f"link-{ordinal}" / "evaluation"
        data = owned_bytes(program, MAX_PROGRAM)
        require(binary == dict(ordinal=ordinal, bytes=len(data), sha256=sha(data)),
                "reviewed executable fixture changed")
        launch.append((layout, profile, site, expected, kind, program, row["observation"]))
    # Check the complete approved set and the live compiler observations
    # before any compiled fixture starts. All inputs here are caller-owned.
    stale_control(driver, tools, rows, runtimes)
    verify_fresh_fixture(driver, tools, rows, runtimes)
    observed = []
    for layout, profile, site, expected, kind, program, observation in launch:
        started = time.perf_counter()
        if layout == "native":
            raw = tools.driver(program, stdout_max=MAX_WORD)[0]
        else:
            raw = tools.run("wasmtime", "run", "--invoke", "evaluate", program, stdout_max=MAX_WORD)
        actual_bytes, _ = fixture_driver_arg(driver, tools, profile, site, layout, "fixture-readback",
                                             observation, input_word=raw)
        actual = json.loads(actual_bytes)
        require(actual["observation"] == observation and actual["result_type"] == kind
                and type(actual["value"]) is type(expected) and actual["value"] == expected,
                "owned fixture returned a different typed value")
        observed.append(dict(layout=layout, profile=profile, site=site, kind=kind,
                             value=actual["value"], raw=raw.decode(),
                             run_and_readback_ms=1000 * (time.perf_counter() - started)))
    require(not (root / "project/.wosy").exists(), "fixture unexpectedly touched Build state")
    print(json.dumps(dict(results=observed, stale_before_readback=True)))


def process_controls():
    # No generated binary or manifest command. Only the currently running
    # test interpreter and literal benign test bodies exercise the transport.
    python = Path(sys.executable).resolve(strict=True)
    env = {"HOME": str(root), "LANG": "C.UTF-8"}
    def attempt(code, **options):
        return capture_owned([python, "-I", "-c", code], root, env, **options)
    require(attempt("import sys; sys.stdout.buffer.write(sys.stdin.buffer.read())",
                    input_word=b"-2147483648\n", stdout_max=MAX_WORD)[0] == b"-2147483648\n",
            "word pipe lost bytes")
    for code, options, error, text in [
        ("print('x' * 12)", dict(stdout_max=MAX_WORD), RuntimeError, "output exceeds bound"),
        ("import sys; sys.stderr.write('x' * 65537)", {}, RuntimeError, "output exceeds bound"),
        ("raise SystemExit(5)", {}, RuntimeError, "exit 5"),
        ("import time; time.sleep(2)", dict(timeout=0.1), TimeoutError, "deadline"),
    ]:
        try:
            attempt(code, **options)
        except error as failure:
            require(text in str(failure), f"unexpected process refusal: {failure}")
        else:
            raise RuntimeError(f"expected owned process refusal: {text}")
    print(json.dumps(dict(process_controls=5, linked=False, executed_generated=False)))


if mode == "inspect":
    inspect(*(Path(path).resolve(strict=True) for path in arguments))
elif mode == "execute":
    execute(*arguments)
elif mode in ("inspect-roundtrip", "link-roundtrip", "readback-roundtrip"):
    driver, toolchain, *reviews = arguments
    operation = {"inspect-roundtrip": inspect_roundtrip, "link-roundtrip": link_roundtrip,
                 "readback-roundtrip": readback_roundtrip}[mode]
    operation(Path(driver).resolve(strict=True), toolchain, *reviews)
elif mode == "process-controls" and not arguments:
    process_controls()
else:
    raise ValueError("unknown fixture phase")

"""Inspect closed scalar IR, then link only the independently accepted bytes."""
import hashlib
import json
from pathlib import Path
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


def fixture_cases():
    for index, (layout, (profile, id, want)) in enumerate(
        (layout, case) for layout in layouts for case in cases
    ):
        yield layout, profile, id, want, root / f"fragment-{index}.ll"


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


def link(layout, ir, result_type, want, ordinal):
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
        run("clang", "-x", "ir", *native_ir, "-x", "c", harness, "-Wl,--wrap=main", "-o", program)
        output_bytes = run(program)
    else:
        objects = []
        for index, module in enumerate((ir, runtime)):
            bc, obj = (output / f"module-{index}.{ext}" for ext in ("bc", "o"))
            run("llvm-as", module, "-o", bc)
            run("llc", "-mtriple=wasm32-wasi", "-filetype=obj", bc, "-o", obj)
            objects.append(obj)
        run("wasm-ld", "--no-entry", "--export=evaluate", *objects, "-o", program)
        output_bytes = run("wasmtime", "run", "--invoke", "evaluate", program)
    assert output_bytes.strip() == str(want).encode(), (layout, output_bytes)


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
        link(layout, ir, recorded["type"], want, ordinal)
        observed.setdefault(layout, []).append(
            {"profile": profile, "site": id, "value": want, "link_run_ms": 1000 * (time.perf_counter() - start)}
        )
    print(json.dumps(observed))


if mode == "inspect":
    inspect(*(Path(path).resolve(strict=True) for path in arguments))
elif mode == "execute":
    execute(*arguments)
else:
    raise ValueError("expected inspect or execute")

from pathlib import Path
import subprocess
import sys
import tempfile


MANIFEST = """
[package]
name = "app"
default_target = "app"

[comments]
categories = []

[builders]

[runners]

[targets.app]
root = "main.w"

[targets.app.artifacts.main]
artifact = "RuntimeStdio"

[targets.app.artifacts.main.profiles.dev]
backend = "llvm"
output = "native"
builder = "never"
run_runner = "never"

[targets.app.profiles.dev]
main_artifact = "main"
"""

PROGRAM = """%%start
    i16(i16, i16) choose = fn(left, right) { if (right < 0) { left } else { right } };
    proof identity for choose(entry, selector) -> (out)
        where (entry >= -5) ensures (out == core.old(entry)) {}
    i16 answer = choose(2, -3);
%%end
"""

EFFECT = """    core.require(false, { .stdout = "must not emit"; .stderr = "must not emit"; .code = 71;
        .files = [{ .path = "must-not-create.txt"; .text = "must not write"; }]; });
"""


STREAMS = """    core.require(false, { .stdout = "out-a:"; .stderr = "err-a:"; .code = 0; .files = []; });
    core.require(false, { .stdout = "out-b café:"; .stderr = "err-b:"; .code = 71; .files = []; });
    core.require(false, { .stdout = "out-c"; .stderr = "err-c"; .code = 19; .files = []; });
"""


def main():
    binary = Path(sys.argv[1]).resolve(strict=True)
    with tempfile.TemporaryDirectory() as directory:
        root = Path(directory).resolve()
        manifest = root / "wosy.toml"
        manifest.write_text(MANIFEST, encoding="utf-8")
        source = root / "main.w"

        def check(*args):
            result = subprocess.run(
                [binary, "check", *args], cwd=root, capture_output=True, timeout=15
            )
            assert sorted(path.name for path in root.iterdir()) == ["main.w", "wosy.toml"]
            return result

        source.write_text(PROGRAM, encoding="utf-8")
        passed = check("--main-flow-only")
        assert (passed.returncode, passed.stdout, passed.stderr) == (0, b"", b""), passed

        omitted_scope = check("app", "--profile", "dev")
        assert omitted_scope.returncode == 2 and omitted_scope.stdout == b"", omitted_scope
        assert b"--main-flow-only" in omitted_scope.stderr, omitted_scope

        rejected = PROGRAM.replace("choose(2, -3);", "choose(4, 8);\n    i16 other = choose(3, 9);")
        source.write_text(rejected, encoding="utf-8")
        failed = check("app", "--profile", "dev", "--main-flow-only")
        assert failed.returncode == 1 and failed.stdout == b"", failed
        assert failed.stderr.count(b"proof obligation failed") == 2, failed
        for label in (
            b"proof declaration", b"applicable call", b"out == core.old(entry)",
            b"choose(4, 8)", b"choose(3, 9)", b"main.w",
            b"call returned 8", b"call returned 9",
        ):
            assert label in failed.stderr, failed

        source.write_text(rejected.replace("%%end", STREAMS + EFFECT + "%%end"), encoding="utf-8")
        blocked = check("--main-flow-only")
        assert (blocked.returncode, blocked.stdout) == (2, b""), blocked
        assert blocked.stderr == b"wosy: check: result requires unsupported output; no result published\n", blocked

        expected_stdout = "out-a:out-b café:out-c".encode()
        expected_stderr = b"err-a:err-b:err-c"
        source.write_text("%%start\n" + STREAMS + "%%end", encoding="utf-8")
        effects = check("--main-flow-only")
        assert (effects.returncode, effects.stdout, effects.stderr) == (71, expected_stdout, expected_stderr), effects
        merged = subprocess.run(
            [binary, "check", "--main-flow-only"], cwd=root,
            stdout=subprocess.PIPE, stderr=subprocess.STDOUT, timeout=15,
        )
        assert (merged.returncode, merged.stdout) == (71, "out-a:err-a:out-b café:err-b:out-cerr-c".encode()), merged
        assert not (root / ".wosy").exists()

        zero_codes = STREAMS.replace(".code = 71", ".code = 0").replace(".code = 19", ".code = 0")
        source.write_text("%%start\n" + zero_codes + "%%end", encoding="utf-8")
        zero = check("--main-flow-only")
        assert (zero.returncode, zero.stdout, zero.stderr) == (0, expected_stdout, expected_stderr), zero

        source.write_text(rejected.replace("%%end", STREAMS + "%%end"), encoding="utf-8")
        mixed = check("--main-flow-only")
        assert (mixed.returncode, mixed.stdout) == (71, expected_stdout), mixed
        assert mixed.stderr.startswith(expected_stderr) and mixed.stderr.count(b"proof obligation failed") == 2, mixed

        source.write_text(rejected.replace("%%end", zero_codes + "%%end"), encoding="utf-8")
        mixed_zero = check("--main-flow-only")
        assert (mixed_zero.returncode, mixed_zero.stdout) == (1, expected_stdout), mixed_zero
        assert mixed_zero.stderr.startswith(expected_stderr) and mixed_zero.stderr.count(b"proof obligation failed") == 2, mixed_zero

        out_of_range = STREAMS.replace(".code = 71", ".code = 256")
        source.write_text(rejected.replace("%%end", out_of_range + "%%end"), encoding="utf-8")
        bad_code = check("--main-flow-only")
        assert (bad_code.returncode, bad_code.stdout) == (2, b""), bad_code
        assert bad_code.stderr == b"wosy: check: selected exit code is outside 0..=255; no result published\n", bad_code

        unsupported = PROGRAM.replace("core.old(entry)", "core.old(unknown)")
        source.write_text(unsupported, encoding="utf-8")
        incomplete = check("--main-flow-only")
        assert (incomplete.returncode, incomplete.stdout) == (2, b""), incomplete
        assert b"check: unsupported analysis; no result published" in incomplete.stderr, incomplete


    with tempfile.TemporaryDirectory() as directory:
        root = Path(directory).resolve()
        source_root = root / "src"
        source_root.mkdir()
        manifest = root / "wosy.toml"
        declared = MANIFEST.replace('default_target = "app"', 'default_target = "app"\nsource_root = "src"')
        declared = declared.replace('root = "main.w"', 'root = "src/main.w"')
        manifest.write_text(declared, encoding="utf-8")
        source = source_root / "main.w"
        source.write_text(PROGRAM, encoding="utf-8")
        dormant = source_root / "fallback.w"
        dormant_program = """%%start
    u8(u8) fallback = fn(value) { value };
    proof every_return for fallback(entry) -> (out) ensures (out != 7) {}
""" + EFFECT + "%%end"
        dormant.write_text(dormant_program, encoding="utf-8")
        (root / "stray.w").write_text("invalid syntax outside the declared source tree", encoding="utf-8")

        def check(*args):
            result = subprocess.run(
                [binary, "check", *args], cwd=root, capture_output=True, timeout=15
            )
            assert sorted(path.relative_to(root).as_posix() for path in root.rglob("*") if path.is_file()) == [
                "src/fallback.w", "src/main.w", "stray.w", "wosy.toml"
            ]
            assert not (root / ".wosy").exists()
            return result

        main_flow = check("--main-flow-only")
        assert (main_flow.returncode, main_flow.stdout, main_flow.stderr) == (0, b"", b""), main_flow

        wider = check("app", "--profile", "dev", "--declared-package")
        assert wider.returncode == 1 and wider.stdout == b"", wider
        assert wider.stderr.count(b"proof obligation failed") == 1, wider
        for label in (
            b"proof declaration", b"out != 7", b"src/fallback.w",
            b"unused function checked symbolically", b"return with a counterexample",
            b"symbolic call can return 7",
        ):
            assert label in wider.stderr, wider
        assert b"must not emit" not in wider.stderr, wider

        source.write_text(PROGRAM.replace("choose(2, -3)", "choose(4, 8)"), encoding="utf-8")
        both = check("--declared-package")
        assert both.returncode == 1 and both.stdout == b"", both
        assert both.stderr.count(b"proof obligation failed") == 2, both
        assert both.stderr.index(b"call returned 8") < both.stderr.index(b"symbolic call can return 7"), both

        source.write_text(PROGRAM, encoding="utf-8")
        dormant.write_text(dormant_program.replace("out != 7", "out <= 255"), encoding="utf-8")
        passed = check("--declared-package")
        assert (passed.returncode, passed.stdout, passed.stderr) == (0, b"", b""), passed

        conflict = check("--main-flow-only", "--declared-package")
        assert conflict.returncode == 2 and conflict.stdout == b"", conflict
        assert b"--main-flow-only" in conflict.stderr and b"--declared-package" in conflict.stderr, conflict

        manifest.write_text(declared.replace('source_root = "src"\n', ''), encoding="utf-8")
        absent = check("--declared-package")
        assert absent.returncode == 2 and absent.stdout == b"", absent
        assert b"source_root" in absent.stderr, absent
        still_main = check("--main-flow-only")
        assert (still_main.returncode, still_main.stdout, still_main.stderr) == (0, b"", b""), still_main


if __name__ == "__main__":
    main()

from pathlib import Path
import os
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
    # Pass the executable from an ordinary `cargo build -p wosy-cli` (no --features).
    binary = Path(sys.argv[1]).resolve(strict=True)
    for args, expected in [
        (("--help",), (b"check", b"current package")),
        (("check", "--help"), (b"--main-flow-only", b"the default scope")),
    ]:
        result = subprocess.run([binary, *args], capture_output=True, timeout=15)
        assert result.returncode == 0 and not result.stderr, result
        assert all(text in result.stdout for text in expected), result
    with tempfile.TemporaryDirectory() as directory:
        root = Path(directory).resolve()
        manifest = root / "wosy.toml"
        manifest.write_text(MANIFEST, encoding="utf-8")
        source = root / "main.w"

        def check(*args):
            result = subprocess.run(
                [binary, "check", "--all-errors", *args], cwd=root, capture_output=True, timeout=15
            )
            assert sorted(path.name for path in root.iterdir()) == ["main.w", "wosy.toml"]
            return result

        source.write_text(PROGRAM, encoding="utf-8")
        passed = check("--main-flow-only")
        assert (passed.returncode, passed.stdout, passed.stderr) == (0, b"", b""), passed

        omitted_scope = check("app", "--profile", "dev")
        assert omitted_scope.returncode == 2 and omitted_scope.stdout == b"", omitted_scope
        assert b"source_root" in omitted_scope.stderr, omitted_scope

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

        source.write_text(rejected.replace("%%end", STREAMS + EFFECT.replace("must-not-create.txt", "nested/must-not-create.txt") + "%%end"), encoding="utf-8")
        blocked = check("--main-flow-only")
        assert (blocked.returncode, blocked.stdout) == (2, b""), blocked
        assert blocked.stderr == b"wosy: check: result requires unsupported output; no result published\n", blocked

        expected_stdout = "out-a:out-b café:out-c".encode()
        expected_stderr = b"err-a:err-b:err-c"
        source.write_text("%%start\n" + STREAMS + "%%end", encoding="utf-8")
        effects = check("--main-flow-only")
        assert (effects.returncode, effects.stdout, effects.stderr) == (71, expected_stdout, expected_stderr), effects
        merged = subprocess.run(
            [binary, "check", "--all-errors", "--main-flow-only"], cwd=root,
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
                [binary, "check", "--all-errors", *args], cwd=root, capture_output=True, timeout=15
            )
            assert sorted(path.relative_to(root).as_posix() for path in root.rglob("*") if path.is_file()) == [
                "src/fallback.w", "src/main.w", "stray.w", "wosy.toml"
            ]
            assert not (root / ".wosy").exists()
            return result

        main_flow = check("--main-flow-only")
        assert (main_flow.returncode, main_flow.stdout, main_flow.stderr) == (0, b"", b""), main_flow

        wider = check("app", "--profile", "dev")
        assert wider.returncode == 1 and wider.stdout == b"", wider
        explicit = check("app", "--profile", "dev", "--declared-package")
        assert (explicit.returncode, explicit.stdout, explicit.stderr) == (wider.returncode, wider.stdout, wider.stderr), explicit
        assert wider.stderr.count(b"proof obligation failed") == 1, wider
        for label in (
            b"proof declaration", b"out != 7", b"src/fallback.w",
            b"unused function checked symbolically", b"return with a counterexample",
            b"symbolic call can return 7",
        ):
            assert label in wider.stderr, wider
        assert b"must not emit" not in wider.stderr, wider

        source.write_text(PROGRAM.replace("choose(2, -3)", "choose(4, 8)"), encoding="utf-8")
        both = check()
        assert both.returncode == 1 and both.stdout == b"", both
        assert both.stderr.count(b"proof obligation failed") == 2, both
        assert both.stderr.index(b"call returned 8") < both.stderr.index(b"symbolic call can return 7"), both

        source.write_text(PROGRAM, encoding="utf-8")
        dormant.write_text(dormant_program.replace("out != 7", "out <= 255"), encoding="utf-8")
        passed = check()
        assert (passed.returncode, passed.stdout, passed.stderr) == (0, b"", b""), passed

        refusing = PROGRAM.replace("%%end", STREAMS + """
            unit() recurse = fn { recurse(); };
            recurse();
        %%end""")
        source.write_text(refusing, encoding="utf-8")
        incomplete = check()
        assert (incomplete.returncode, incomplete.stdout) == (2, b""), incomplete
        assert incomplete.stderr == b"wosy: check: unsupported analysis; no result published\n", incomplete

        for malformed, expected in [
            (EFFECT.replace("must-not-create.txt", "nested/must-not-create.txt"),
                b"result requires unsupported output; no result published"),
            (STREAMS.replace(".code = 71", ".code = 256") + EFFECT,
                b"selected exit code is outside 0..=255; no result published"),
        ]:
            source.write_text(PROGRAM.replace("%%end", zero_codes + malformed + "%%end"),
                              encoding="utf-8")
            rejected = check()
            assert (rejected.returncode, rejected.stdout) == (2, b""), rejected
            assert rejected.stderr == b"wosy: check: " + expected + b"\n", rejected
        source.write_text(PROGRAM, encoding="utf-8")

        conflict = check("--main-flow-only", "--declared-package")
        assert conflict.returncode == 2 and conflict.stdout == b"", conflict
        assert b"--main-flow-only" in conflict.stderr and b"--declared-package" in conflict.stderr, conflict

        if sys.platform != "linux":
            source.write_text(PROGRAM.replace("%%end", STREAMS + EFFECT + "%%end"), encoding="utf-8")
            unsupported_file = check()
            assert (unsupported_file.returncode, unsupported_file.stdout) == (2, b""), unsupported_file
            assert unsupported_file.stderr == b"wosy: check: result requires unsupported output; no result published\n", unsupported_file

        manifest.write_text(declared.replace('source_root = "src"\n', ''), encoding="utf-8")
        absent = check("--declared-package")
        assert absent.returncode == 2 and absent.stdout == b"", absent
        assert b"source_root" in absent.stderr, absent
        still_main = check("--main-flow-only")
        assert (still_main.returncode, still_main.stdout, still_main.stderr) == (0, b"", b""), still_main

    if sys.platform == "linux":
        check_file_output(binary)


def check_file_output(binary):
    with tempfile.TemporaryDirectory() as directory:
        root = Path(directory).resolve()
        project = root / "project"
        project.mkdir()
        declared = MANIFEST.replace('default_target = "app"', 'default_target = "app"\nsource_root = "src"')
        declared = declared.replace('root = "main.w"', 'root = "src/main.w"')
        (project / "wosy.toml").write_text(declared, encoding="utf-8")
        caller = project / "caller"
        caller.mkdir()
        source = project / "src" / "main.w"
        source.parent.mkdir()
        output = project / ".wosy" / "outputs" / "check"
        outside = root / "outside"
        outside.mkdir()
        sentinel = outside / "protected.txt"
        sentinel.write_bytes(b"unchanged")
        one = EFFECT.replace("must-not-create.txt", "proof.txt").replace("must not write", "proof café")
        two = EFFECT.replace("must-not-create.txt", "empty.txt").replace("must not write", "")

        def check(code, scope=("--main-flow-only",)):
            source.write_text("%%start\n" + code + "%%end", encoding="utf-8")
            return subprocess.run(
                [binary, "check", "--all-errors", *scope], cwd=caller,
                capture_output=True, timeout=15,
            )

        bad_code = check(STREAMS.replace(".code = 71", ".code = 256") + one)
        assert bad_code.returncode == 2 and bad_code.stdout == b"", bad_code
        assert b"outside 0..=255" in bad_code.stderr and not output.exists(), bad_code

        for directory in (output.parent.parent, output.parent, output):
            directory.parent.mkdir(parents=True, exist_ok=True)
            directory.symlink_to(outside, target_is_directory=True)
            blocked = check(STREAMS + one)
            assert blocked.returncode == 2 and blocked.stdout == b"", blocked
            assert b"no result published" in blocked.stderr and not (outside / "proof.txt").exists(), blocked
            directory.unlink()

        success = check(STREAMS + one + two)
        assert success.returncode == 71, success
        assert success.stdout == "out-a:out-b café:out-cmust not emitmust not emit".encode(), success
        assert success.stderr == b"err-a:err-b:err-cmust not emitmust not emit", success
        assert sorted(path.name for path in output.iterdir()) == ["empty.txt", "proof.txt"]
        assert (output / "proof.txt").read_bytes() == "proof café".encode()
        assert (output / "empty.txt").read_bytes() == b""

        # Both requests have valid flat paths. The first is absent; the second
        # is an existing ordinary file reached through Linux file preparation.
        # Neither the earlier authored streams nor the first file may publish.
        late = check(STREAMS + one.replace("proof.txt", "not-published.txt")
                     + two.replace("empty.txt", "proof.txt"), scope=())
        assert (late.returncode, late.stdout) == (2, b""), late
        assert late.stderr == (
            b"wosy: check: could not prepare authored files; no result published: "
            b"proof.txt already exists\n"
        ), late
        assert sorted(path.name for path in output.iterdir()) == ["empty.txt", "proof.txt"]
        assert (output / "proof.txt").read_bytes() == "proof café".encode()
        assert (output / "empty.txt").read_bytes() == b""

        for create in (
            lambda target: target.symlink_to(sentinel),
            lambda target: os.link(sentinel, target),
            lambda target: os.mkfifo(target),
        ):
            conflict = output / "protected.txt"
            create(conflict)
            before = sorted(path.name for path in output.iterdir())
            blocked = check(STREAMS + one.replace("proof.txt", "new.txt") + two.replace("empty.txt", "protected.txt"))
            assert blocked.returncode == 2 and blocked.stdout == b"", blocked
            assert b"protected.txt already exists" in blocked.stderr, blocked
            assert sorted(path.name for path in output.iterdir()) == before
            assert sentinel.read_bytes() == b"unchanged"
            conflict.unlink()

        added = check(one.replace("proof.txt", "new.txt"))
        assert (added.returncode, added.stdout, added.stderr) == (71, b"must not emit", b"must not emit"), added
        assert sorted(path.name for path in output.iterdir()) == ["empty.txt", "new.txt", "proof.txt"]
        assert (output / "new.txt").read_bytes() == "proof café".encode()
        assert not (caller / ".wosy").exists()


def check_contextual_storage(binary):
    def require(condition, message, code):
        return (
            f'core.require({condition}, {{ .stdout = "{message}";'
            f' .stderr = ""; .code = {code}; .files = []; }});'
        )

    with tempfile.TemporaryDirectory() as scratch:
        root = Path(scratch)
        (root / "src").mkdir()
        manifest = MANIFEST.replace('default_target = "app"', 'default_target = "app"\nsource_root = "src"')
        manifest = manifest.replace('root = "main.w"', 'root = "src/main.w"')
        (root / "wosy.toml").write_text(manifest, encoding="utf-8")
        source = root / "src" / "main.w"
        program = f"""%%start
        unit(i8) state = fn(selector) {{
            i8 core.context.count = 1;
            if (selector < 0) {{
                i8 core.context.count = core.context.count;
                {require("core.context.count == 1", "bad init", 7)}
                core.context.count = 2;
                {require("core.context.count == 9", "inner:", 31)}
            }} else {{
                core.context.count = 3;
            }};
            if (selector < 0) {{
                {require("core.context.count == 1", "bad restore", 11)}
            }} else {{
                {require("core.context.count == 1", "outer", 37)}
            }};
        }};
        %%end"""

        def run(scope):
            return subprocess.run(
                [binary, "check", "--all-errors", scope], cwd=root, capture_output=True, timeout=15
            )

        source.write_text(program, encoding="utf-8")
        result = run("--main-flow-only")
        assert (result.returncode, result.stdout, result.stderr) == (0, b"", b""), result
        result = run("--declared-package")
        assert (result.returncode, result.stdout, result.stderr) == (31, b"inner:outer", b""), result
        assert not (root / ".wosy").exists()


if __name__ == "__main__":
    main()
    check_contextual_storage(Path(sys.argv[1]).resolve())

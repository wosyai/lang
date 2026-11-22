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
    i8(i8) identity = fn(value) { value };
    unit(i8) wrapper = fn(arg) {
        i8 result = identity(arg);
        core.require(result == 6, { .stdout = "must not emit"; .stderr = "not stderr"; .code = 71;
            .files = [{ .path = "must-not-create.txt"; .text = "ignored"; }]; });
    };
    i8(i8) unused = fn(value) { value };
    proof actual for identity(entry) -> (out) ensures (out <= 5) {}
    proof symbolic for unused(entry) -> (out) ensures (out != 7) {}
    wrapper(6);
    wrapper(9);
%%end
"""

PREIMAGE = """
    unit(u8) preimage = fn(value) {
        core.narrow(value < 4, { .stdout = ""; .stderr = ""; .code = 11; .files = []; });
        value = 9;
        core.require(value < 8, { .stdout = ""; .stderr = ""; .code = 71; .files = []; });
    };
"""

GUARDED = """%%start
    i16(i16) producer = fn(value) { if (value >= 0) { 7 } else { value + 1 } };
    unit(i16) caller = fn(value) {
        core.narrow(value >= 0, { .stdout = ""; .stderr = ""; .code = 11; .files = []; });
        i16 result = producer(value);
        core.require(result == 7, { .stdout = ""; .stderr = ""; .code = 71; .files = []; });
    };
    proof bounded for producer(entry) -> (out) where (entry >= 0) ensures (out == 7) {}
%%end
"""

EXPECTED = (
    b"ordered check: complete\n"
    b"directive failures: 1\n"
    b"actual proof failures: 2 (2 nested)\n"
    b"unused proof failures: 1\n"
)


def main():
    executable = Path(sys.argv[1]).resolve(strict=True)
    with tempfile.TemporaryDirectory() as directory:
        root = Path(directory)
        (root / "wosy.toml").write_text(MANIFEST, encoding="utf-8")
        source = root / "main.w"
        source.write_text(PROGRAM, encoding="utf-8")
        command = [executable, root, "app", "dev"]
        complete = subprocess.run(command, cwd=root, capture_output=True, timeout=15)
        assert complete.returncode == 1, complete
        assert complete.stdout == EXPECTED, complete
        assert complete.stderr == b"", complete

        source.write_text(f"%%start\n{PREIMAGE}\n%%end\n", encoding="utf-8")
        preimage = subprocess.run(command, cwd=root, capture_output=True, timeout=15)
        assert preimage.returncode == 1, preimage
        assert preimage.stdout == (
            b"ordered check: complete\ndirective failures: 1\n"
            b"actual proof failures: 0 (0 nested)\nunused proof failures: 0\n"
            b"directive 1 entry: 0..3; failure-time: 9\n"
        ), preimage
        assert preimage.stderr == b"", preimage

        mismatched = PREIMAGE.replace("unit(u8) preimage = fn(value)", "unit(u8, u8) preimage = fn(value, other)") \
            .replace("value = 9", "other = 9").replace("require(value", "require(other")
        source.write_text(f"%%start\n{mismatched}\n%%end\n", encoding="utf-8")
        wrong_axis = subprocess.run(command, cwd=root, capture_output=True, timeout=15)
        assert wrong_axis.returncode == 1, wrong_axis
        assert wrong_axis.stdout == preimage.stdout.split(b"directive 1 entry:", 1)[0], wrong_axis
        assert wrong_axis.stderr == b"", wrong_axis

        source.write_text(GUARDED, encoding="utf-8")
        admitted = subprocess.run(command, cwd=root, capture_output=True, timeout=15)
        assert admitted.returncode == 0, admitted
        assert admitted.stdout == (
            b"ordered check: complete\ndirective failures: 0\n"
            b"actual proof failures: 0 (0 nested)\nunused proof failures: 0\n"
        ), admitted
        assert admitted.stderr == b"", admitted
        disjunction = GUARDED.replace("result == 7", "result < 6 || result > 6")
        source.write_text(disjunction, encoding="utf-8")
        union_pass = subprocess.run(command, cwd=root, capture_output=True, timeout=15)
        assert (union_pass.returncode, union_pass.stdout, union_pass.stderr) == (0, admitted.stdout, b""), union_pass
        source.write_text(disjunction.replace("result < 6 || result > 6", "result < 7 || result > 7"), encoding="utf-8")
        union_hole = subprocess.run(command, cwd=root, capture_output=True, timeout=15)
        assert union_hole.returncode == 1 and union_hole.stderr == b"", union_hole
        assert union_hole.stdout == admitted.stdout.replace(b"directive failures: 0", b"directive failures: 1"), union_hole

        source.write_text(GUARDED.replace("core.narrow(value >= 0", "core.narrow(value >= -1"), encoding="utf-8")
        partial = subprocess.run(command, cwd=root, capture_output=True, timeout=15)
        assert partial.returncode == 2 and partial.stdout == b"", partial
        assert partial.stderr == b"ordered check: incomplete: checker did not complete analysis\n", partial

        incomplete_program = PROGRAM.replace("unused = fn(value) { value }", "unused = fn(value) { value + 1 }")
        assert incomplete_program != PROGRAM
        source.write_text(incomplete_program.replace("%%start", f"%%start\n{PREIMAGE}", 1), encoding="utf-8")
        incomplete = subprocess.run(command, cwd=root, capture_output=True, timeout=15)
        assert incomplete.returncode == 2, incomplete
        assert incomplete.stdout == b"", incomplete
        assert incomplete.stderr == b"ordered check: incomplete: checker did not complete analysis\n", incomplete
        assert sorted(entry.name for entry in root.iterdir()) == ["main.w", "wosy.toml"]

        module = root / "lib.w"
        source.write_text(
            '%%start lib = namespace app "lib.w"; i8 result = lib.echo(8); %%end',
            encoding="utf-8",
        )
        library = """%%start
            i8(i8) echo = fn(value) { value };
            i8(i8) unused = fn(value) { value };
            proof reached for echo(entry) -> (out) ensures (out < 5) {}
            proof uncovered for unused(entry) -> (out) ensures (out != 7) {}
        %%end"""
        module.write_text(library, encoding="utf-8")
        local = subprocess.run(command, cwd=root, capture_output=True, timeout=15)
        assert local.returncode == 1 and local.stderr == b"", local
        assert local.stdout == (
            b"ordered check: complete\ndirective failures: 0\n"
            b"actual proof failures: 1 (0 nested)\nunused proof failures: 1\n"
        ), local

        module.write_text(library.replace("unused = fn(value) { value }", "unused = fn(value) { value + 1 }"), encoding="utf-8")
        local_incomplete = subprocess.run(command, cwd=root, capture_output=True, timeout=15)
        assert local_incomplete.returncode == 2 and local_incomplete.stdout == b"", local_incomplete
        assert local_incomplete.stderr == b"ordered check: incomplete: checker did not complete analysis\n", local_incomplete
        main_command = [*command, "--main-flow-only"]
        local_main = subprocess.run(main_command, cwd=root, capture_output=True, timeout=15)
        expected_main = local.stdout.replace(b"unused proof failures: 1", b"unused proof failures: 0")
        assert (local_main.returncode, local_main.stdout, local_main.stderr) == (1, expected_main, b""), local_main

        source.write_text(
            '%%start lib = namespace app "lib.w"; i8 result = lib.echo(4); %%end',
            encoding="utf-8",
        )
        local_main_pass = subprocess.run(main_command, cwd=root, capture_output=True, timeout=15)
        expected_pass = expected_main.replace(b"actual proof failures: 1", b"actual proof failures: 0")
        assert (local_main_pass.returncode, local_main_pass.stdout, local_main_pass.stderr) == (
            0, expected_pass, b""
        ), local_main_pass
        assert sorted(entry.name for entry in root.iterdir()) == ["lib.w", "main.w", "wosy.toml"]


        source.write_text("%%start i8 startup = 1; %%end", encoding="utf-8")
        module.write_text("""%%start
            core.require(false, { .stdout = "must not emit"; .stderr = ""; .code = 71; .files = []; });
            i8(i8) unused = fn(value) { value };
            proof uncovered for unused(entry) -> (out) ensures (out != 7) {}
        %%end""", encoding="utf-8")
        dormant_command = [*command, "--dormant", "lib.w"]
        dormant = subprocess.run(dormant_command, cwd=root, capture_output=True, timeout=15)
        expected_dormant = expected_pass.replace(b"unused proof failures: 0", b"unused proof failures: 1")
        assert (dormant.returncode, dormant.stdout, dormant.stderr) == (1, expected_dormant, b""), dormant

        for invalid in ("main.w", "./lib.w", "deps/lib.w", "missing.w"):
            rejected = subprocess.run(
                [*command, "--dormant", invalid], cwd=root, capture_output=True, timeout=15
            )
            assert rejected.returncode == 2 and rejected.stdout == b"", rejected

        alias = root / "alias.w"
        if sys.platform != "win32":
            alias.symlink_to(module)
            rejected_alias = subprocess.run(
                [*command, "--dormant", alias.name], cwd=root, capture_output=True, timeout=15
            )
            assert rejected_alias.returncode == 2 and rejected_alias.stdout == b"", rejected_alias
            alias.unlink()

        module.write_text("not valid Wosy", encoding="utf-8")
        for path in ("lib.w", "missing.w"):
            ignored = subprocess.run(
                [*main_command, "--dormant", path], cwd=root, capture_output=True, timeout=15
            )
            assert (ignored.returncode, ignored.stdout, ignored.stderr) == (0, expected_pass, b""), ignored
        assert sorted(entry.name for entry in root.iterdir()) == ["lib.w", "main.w", "wosy.toml"]


        source.write_text("""%%start
            i8(i8) echo = fn(value) { value };
            proof reached for echo(entry) -> (out) ensures (out < 5) {}
            i8 result = echo(8);
            core.require(false, { .stdout = "ação"; .stderr = ""; .code = 0;
                .files = [{ .path = "preview.txt"; .text = "β"; }]; });
            core.require(false, { .stdout = ""; .stderr = "second"; .code = 340282366920938463463374607431768211456;
                .files = []; });
            core.require(false, { .stdout = "third"; .stderr = ""; .code = 33; .files = []; });
        %%end""", encoding="utf-8")
        module.write_text("""%%start
            i8(i8) unused = fn(value) { value };
            proof uncovered for unused(entry) -> (out) ensures (out != 7) {}
        %%end""", encoding="utf-8")
        effects_command = [*dormant_command, "--inspect-effects"]
        effects = subprocess.run(effects_command, cwd=root, capture_output=True, timeout=15)
        expected_effects = (
            'ordered check: complete\ndirective failures: 3\n'
            'actual proof failures: 1 (0 nested)\nunused proof failures: 1\n'
            'source texts: 2\nauthored status: 340282366920938463463374607431768211456\n'
            f'stdout: {"ação".encode().hex()}\nfile "preview.txt" from "main.w": {"β".encode().hex()}\n'
            f'stderr: {b"second".hex()}\nstdout: {b"third".hex()}\n'
        ).encode()
        assert (effects.returncode, effects.stdout, effects.stderr) == (1, expected_effects, b""), effects
        complete_source = source.read_text(encoding="utf-8")
        source.write_text(complete_source.replace('"preview.txt"', '"../preview.txt"'), encoding="utf-8")
        invalid_path = subprocess.run(effects_command, cwd=root, capture_output=True, timeout=15)
        assert (invalid_path.returncode, invalid_path.stdout, invalid_path.stderr) == (
            2, b"", b"ordered check: incomplete: checker rejected invalid input\n"
        ), invalid_path

        duplicate = complete_source.replace(
            '.stdout = "third"; .stderr = ""; .code = 33; .files = [];',
            '.stdout = "third"; .stderr = ""; .code = 33; .files = [{ .path = "preview.txt"; .text = ""; }];',
        )
        assert duplicate != complete_source
        source.write_text(duplicate, encoding="utf-8")
        duplicate_path = subprocess.run(effects_command, cwd=root, capture_output=True, timeout=15)
        assert (duplicate_path.returncode, duplicate_path.stdout, duplicate_path.stderr) == (
            2, b"", b'ordered check: duplicate output path "preview.txt" in "main.w"\n'
        ), duplicate_path
        source.write_text(complete_source, encoding="utf-8")
        module.write_text(module.read_text(encoding="utf-8").replace("{ value }", "{ value + 1 }"), encoding="utf-8")
        incomplete = subprocess.run(effects_command, cwd=root, capture_output=True, timeout=15)
        assert (incomplete.returncode, incomplete.stdout, incomplete.stderr) == (
            2, b"", b"ordered check: incomplete: checker did not complete analysis\n"
        ), incomplete
        assert sorted(entry.name for entry in root.iterdir()) == ["lib.w", "main.w", "wosy.toml"]


if __name__ == "__main__":
    main()

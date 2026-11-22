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


if __name__ == "__main__":
    main()

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

        source.write_text(rejected.replace("%%end", EFFECT + "%%end"), encoding="utf-8")
        blocked = check("--main-flow-only")
        assert (blocked.returncode, blocked.stdout) == (2, b""), blocked
        assert blocked.stderr == b"wosy: check: result requires unsupported output; no result published\n", blocked

        unsupported = PROGRAM.replace("core.old(entry)", "core.old(unknown)")
        source.write_text(unsupported, encoding="utf-8")
        incomplete = check("--main-flow-only")
        assert (incomplete.returncode, incomplete.stdout) == (2, b""), incomplete
        assert b"check: unsupported analysis; no result published" in incomplete.stderr, incomplete


if __name__ == "__main__":
    main()

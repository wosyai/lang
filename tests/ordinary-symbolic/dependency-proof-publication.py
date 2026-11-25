"""Exercise configured Check through its release CLI, not the Build command."""

import subprocess
import sys
import tempfile
from pathlib import Path

CLI = Path(sys.argv[1]).resolve()


def manifest(package, target, profile, output, dependency=None):
    text = f"""
[package]
name = "{package}"
default_target = "{target}"
source_root = "src"
[comments]
categories = []
[builders]
[runners]
[targets.{target}]
root = "src/primary.w"
default_profile = "{profile}"
[targets.{target}.artifacts.main]
artifact = "RuntimeStdio"
[targets.{target}.artifacts.main.profiles.{profile}]
backend = "llvm"
output = "{output}"
builder = "never"
run_runner = "never"
[targets.{target}.profiles.{profile}]
main_artifact = "main"
"""
    if dependency is not None:
        name, path = dependency
        text += f'\n[dependencies.{name}]\npath = "{path}"\n'
    return text


def package(directory, toml, source):
    (directory / "src").mkdir(parents=True)
    (directory / "wosy.toml").write_text(toml, encoding="utf-8")
    main = directory / "src/primary.w"
    main.write_text(source, encoding="utf-8")
    return main


ROOT_EFFECT = """%%start
core.require(false, { .stdout = "authored root"; .stderr = ""; .code = 0;
    .files = [{ .path = "selected.txt"; .text = "chosen by source"; }]; });
%%end"""
LEAF = """%%start
u8(u8) leaf = fn(input) { input };
proof leaf_goal for leaf(x) -> (out) ensures (out == 0) {}
%%end"""


def check(root, *options):
    return subprocess.run(
        [CLI, "check", "--profile", "release", *options],
        cwd=root, capture_output=True, timeout=30,
    )


def run_case(root_output, leaf_output):
    with tempfile.TemporaryDirectory(prefix="wosy-proof-publication-") as directory:
        root = Path(directory)
        package(root, manifest("app", "app_target", "release", root_output, ("relay", "relay")), ROOT_EFFECT)
        package(
            root / "relay",
            manifest("relay", "relay_target", "relay_profile", "native", ("leaf", "../leaf")),
            '%%start\ncore.require(true, { .stdout = ""; .stderr = ""; .code = 0; .files = []; });\n%%end',
        )
        leaf = package(root / "leaf", manifest("leaf", "leaf_target", "leaf_profile", leaf_output), LEAF)
        selected = root / ".wosy/outputs/check/selected.txt"

        root_only = check(root, "--main-flow-only")
        assert (root_only.returncode, root_only.stdout, root_only.stderr) == (0, b"authored root", b""), root_only
        assert selected.read_text(encoding="utf-8") == "chosen by source"

        for scope in ("--declared-package", "--main-flow-only"):
            selected.unlink()
            failed = check(root, scope, "--dependencies")
            assert (failed.returncode, failed.stdout) == (1, b"authored root"), failed
            assert failed.stderr.count(b"error: proof obligation failed") == 1, failed
            assert b"proof leaf_goal" in failed.stderr and b"out == 0" in failed.stderr, failed
            assert b"leaf::src/primary.w" in failed.stderr, failed
            assert b"proof declaration" in failed.stderr and b"not satisfied by this symbolic call" in failed.stderr, failed
            assert b"symbolic call can return 255" in failed.stderr, failed
            assert b"symbolic call entry: input = 255" in failed.stderr, failed
            assert selected.read_text(encoding="utf-8") == "chosen by source"
            assert not (root / "leaf/.wosy/outputs/check/selected.txt").exists()

        leaf.write_text(LEAF.replace("out == 0", "out == core.old(x)"), encoding="utf-8")
        selected.unlink()
        satisfied = check(root, "--dependencies")
        assert (satisfied.returncode, satisfied.stdout, satisfied.stderr) == (0, b"authored root", b""), satisfied

        # A later package fails before any explicit root output is published.
        selected.write_bytes(b"leave existing output alone\x00\xff")
        def output_snapshot():
            entries = {}
            for directory in root.rglob(".wosy/outputs/check"):
                for path in (directory, *directory.rglob("*")):
                    assert not path.is_symlink(), path
                    relative = path.relative_to(root)
                    if path.is_dir():
                        entries[relative] = ("directory", None)
                    else:
                        assert path.is_file(), path
                        entries[relative] = ("file", path.read_bytes())
            return entries

        before_invalid = output_snapshot()
        assert before_invalid == {
            selected.parent.relative_to(root): ("directory", None),
            selected.relative_to(root): ("file", b"leave existing output alone\x00\xff"),
        }
        leaf.write_text(LEAF.replace("{ input }", "{ unknown_value }"), encoding="utf-8")
        invalid = check(root, "--dependencies")
        assert invalid.returncode != 0 and not invalid.stdout, invalid
        assert b"primary.w" in invalid.stderr and b"unknown_value" in invalid.stderr, invalid
        assert output_snapshot() == before_invalid
        assert not list(root.rglob(".wosy/artifacts")), "Check must not create Build outputs"
        assert [path.relative_to(root).as_posix() for path in root.rglob("*.txt")] == [".wosy/outputs/check/selected.txt"]


if __name__ == "__main__":
    run_case("native", "wasm-wasip1")
    run_case("wasm-wasip1", "native")

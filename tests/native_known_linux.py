"""Explicit developer acceptance for the existing one-shot NativeKnown endpoint."""
import argparse
import json
import os
from pathlib import Path
import secrets
import selectors
import stat
import subprocess
import sys
import tempfile
import time

PLAN = """NativeKnown Linux developer acceptance (no implicit execution)

Use one explicitly authorized Linux host that permits ordinary AF_UNIX.
Build the existing release binaries in its usual Cargo target directory:
  cargo build --release --locked -p wosy-cli --bin wosy --example source_snapshot
  cargo build --release --locked -p wosy-cli --features native-diagnostics --example native_known_diagnostics_client
  cargo build --release --locked -p wosy-daemon --features native-diagnostics --example native_known_diagnostics_server
Then use:
  python -B tests/native_known_linux.py --prepare-only --release-dir <release>
  python -B tests/native_known_linux.py --run --release-dir <release> --endpoint-parent <protected-parent>

The supplied parent is short, absolute, writable by the test user, and
its ancestors must satisfy the existing NativeKnown ownership policy.
Only the existing Rust listener creates and removes its fresh child
endpoint. No fallback location, retries, permission changes, or forced
filesystem cleanup after an interrupted test. --prepare-only
seeds real SQLite and SurrealKV and checks the ordinary compiler oracle;
it opens no native endpoint. --run additionally requires byte equality
on Native64/WASI, clean close and reopen, terminal ReplayRequired on a
separate empty store, unchanged SQLite, and own endpoint cleanup.
This does not certify an unclean restart, a system-wide daemon, wire replay,
default Build/Check, or artifact acceptance.
"""


def prepare(release, work):
    from native_known_fixture import write_project

    project, store = work / "project", work / "source"
    write_project(project)
    example = release / "examples/source_snapshot"

    def selection(*args):
        return json.loads(subprocess.check_output(
            [example, project, store, "--persistent-source", "app", *args], timeout=30,
        ))

    root = selection("stage")["pending"]
    acknowledged = selection("submit-pending", "app", "src/main.w")
    assert root and acknowledged["selected"] == root and acknowledged["pending"] is None, acknowledged

    diagnostics = {}
    tail = b"wosy: build rejected by semantic diagnostics\n"
    # The native example uses Buffer::no_color. termcolor's Auto can
    # still emit ANSI on captured stderr when TERM is set. Normalize the
    # producer explicitly, not the diagnostic bytes under comparison.
    oracle_env = {**os.environ, "NO_COLOR": "1"}
    for profile in ("dev", "native"):
        observed = subprocess.run([release / "wosy", "build", "app", "--profile", profile],
                                  cwd=project, capture_output=True, timeout=30, env=oracle_env)
        assert observed.returncode == 1 and not observed.stdout and observed.stderr.endswith(tail), observed
        diagnostics[profile] = observed.stderr[:-len(tail)]
        assert b"item.w" in diagnostics[profile] and b"next" in diagnostics[profile]
    assert not (project / ".wosy/artifacts").exists()
    return project, store, root, diagnostics


def wait_until_listening(stdout, deadline):
    marker = b"LISTENING\n"
    received = bytearray()
    fd = stdout.fileno()
    os.set_blocking(fd, False)
    try:
        with selectors.DefaultSelector() as readiness:
            readiness.register(fd, selectors.EVENT_READ)
            while len(received) < len(marker):
                remaining = deadline - time.monotonic()
                if remaining <= 0 or not readiness.select(timeout=remaining) or time.monotonic() >= deadline:
                    raise TimeoutError(f"daemon readiness deadline with bytes {bytes(received)!r}")
                try:
                    part = os.read(fd, len(marker) - len(received))
                except BlockingIOError:
                    continue
                if time.monotonic() >= deadline:
                    raise TimeoutError("daemon readiness marker completed too late")
                received.extend(part)
                if not part or not marker.startswith(received):
                    raise RuntimeError(f"daemon sent unexpected readiness bytes {bytes(received)!r}")
    finally:
        os.set_blocking(fd, True)


def mismatch(expected, actual):
    offset = next((i for i, (left, right) in enumerate(zip(expected, actual)) if left != right),
                  min(len(expected), len(actual)))
    start = max(0, offset - 8)
    stop = offset + 8
    return (f"diagnostic bytes differ at offset {offset}; expected {len(expected)} bytes,"
            f" received {len(actual)} bytes; slices at {start}:"
            f" expected={expected[start:stop].hex()}, actual={actual[start:stop].hex()}")


def exchange(release, project, store, root, endpoint, profile, expected):
    started = time.monotonic()
    with subprocess.Popen(
        [release / "examples/native_known_diagnostics_server", store, project, "app", root, endpoint],
        stdout=subprocess.PIPE, stderr=subprocess.PIPE,
    ) as daemon:
        try:
            wait_until_listening(daemon.stdout, started + 12)
            socket = (endpoint / "diagnostics.sock").lstat()
            assert stat.S_ISSOCK(socket.st_mode) and socket.st_uid == os.geteuid()
            result = subprocess.run(
                [release / "examples/native_known_diagnostics_client", project, "app", profile, endpoint],
                capture_output=True, timeout=12,
            )
            remaining, terminal = daemon.communicate(timeout=10)
            assert daemon.returncode == 0 and not remaining, (remaining, terminal)
            if expected is None:
                assert terminal == b"ReplayRequired\n" and result.returncode == 1 and not result.stdout, (terminal, result)
                assert b"daemon needs source replay; this endpoint does not accept replay" in result.stderr, result
            else:
                assert terminal == b"Disconnected\n", terminal
                assert result.returncode == 0 and not result.stderr, result
                assert result.stdout == expected, mismatch(expected, result.stdout)
        finally:
            if daemon.poll() is None:
                daemon.kill()
                daemon.wait(timeout=5)
    assert not os.path.lexists(endpoint), "daemon did not clean up its endpoint"
    return round((time.monotonic() - started) * 1_000, 3)


def run(release, endpoint_parent, work, project, store, root, diagnostics):
    example = release / "examples/source_snapshot"
    dbs = (project / ".wosy/client.sqlite", project / ".wosy/cst/nodes.sqlite")
    before = tuple(path.read_bytes() for path in dbs)
    missing = work / "empty"
    missing.mkdir()
    endpoint = endpoint_parent / f"w6-{secrets.token_hex(5)}"
    if len(os.fsencode(endpoint / "diagnostics.sock")) > 107:
        raise ValueError("choose a shorter authorized endpoint parent for the Linux socket pathname")
    timings = {
        profile: exchange(release, project, store, root, endpoint, profile, report)
        for profile, report in diagnostics.items()
    }
    timings["reopen_same_profile"] = exchange(release, project, store, root, endpoint, "dev", diagnostics["dev"])
    timings["empty"] = exchange(release, project, missing, root, endpoint, "dev", None)
    absent = subprocess.run([example, project, missing, root, "app", "src/main.w"],
                            capture_output=True, timeout=30)
    assert absent.returncode != 0 and not absent.stdout and b"unknown retained snapshot" in absent.stderr, absent
    assert tuple(path.read_bytes() for path in dbs) == before
    assert not (project / ".wosy/artifacts").exists()
    return timings


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    mode = parser.add_mutually_exclusive_group()
    mode.add_argument("--prepare-only", action="store_true", help="no native process or socket")
    mode.add_argument("--run", action="store_true", help="run only on an authorized AF_UNIX Linux host")
    parser.add_argument("--release-dir", type=Path, help="the existing Linux Release directory with the four binaries")
    parser.add_argument("--endpoint-parent", type=Path, help="explicit protected parent required for --run")
    args = parser.parse_args()
    if not (args.prepare_only or args.run):
        print(PLAN, end="")
        return
    if not sys.platform.startswith("linux") or args.release_dir is None:
        parser.error("select Linux and supply the existing --release-dir")
    if args.run and (args.endpoint_parent is None or not args.endpoint_parent.is_absolute()
                     or ".." in args.endpoint_parent.parts):
        parser.error("--run requires an explicit absolute protected --endpoint-parent without '..'")
    release = args.release_dir.resolve(strict=True)
    with tempfile.TemporaryDirectory(prefix="w6-source-") as directory:
        work = Path(directory).resolve()
        project, store, root, diagnostics = prepare(release, work)
        result = {"root": root, "ordinary_diagnostic_bytes": {key: len(value) for key, value in diagnostics.items()}}
        if args.run:
            result["native_pair_wall_ms"] = run(release, args.endpoint_parent, work, project, store, root, diagnostics)
        else:
            result["native_executed"] = False
        print(json.dumps(result))


if __name__ == "__main__":
    main()

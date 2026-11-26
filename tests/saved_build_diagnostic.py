"""Durable archival of real ordinary Build diagnostics, not a replayed verdict."""
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile


example, ordinary, unit = (Path(arg).resolve() for arg in sys.argv[1:4])
manifest = (Path(__file__).parent /
            "fixtures/build/shared_stdlib_local_dependency/project/wosy.toml").read_text()
manifest = manifest.replace("categories = []", 'categories = ["WHY"]')
manifest += """
[builders.native]
command = ["unused"]
args = []
toolchain_identity = "native-fixture"

[runners.native]
command = ["unused"]
args = []

[targets.app.artifacts.main.profiles.native]
backend = "llvm"
output = "native"
builder = "native"
run_runner = "native"

[targets.app.profiles.native]
main_artifact = "main"
"""
old_source = """%%start
# WHY: origem guardada ☕
struct Item { u8 value; }
Item(Item) take = fn(input) { next = input; next; next };
%%end
"""
with tempfile.TemporaryDirectory() as name:
    tmp = Path(name).resolve()
    project, other = tmp / "project", tmp / "other"
    store, conditional = tmp / "store", tmp / "conditional-store"

    def make(root):
        (root / "src").mkdir(parents=True)
        (root / "stdlib/src").mkdir(parents=True)
        (root / "wosy.toml").write_text(manifest)
        (root / "stdlib/wosy.toml").write_text('[package]\nname="std"\nsource_root="src"\n')
        (root / "src/main.w").write_text('%%start\nlib = namespace std "item.w";\n%%end\n')
        leaf = root / "stdlib/src/item.w"
        leaf.write_text(old_source)
        return leaf

    def call(*args, owner=project, database=store, refusal=None):
        proc = subprocess.run([example, owner, database, "--persistent-source", "app", *args],
                              capture_output=True, check=False, timeout=30)
        if refusal:
            assert proc.returncode != 0 and not proc.stdout and refusal in proc.stderr, proc.stderr
            return
        assert proc.returncode == 0, proc.stderr
        return json.loads(proc.stdout)

    def admit(owner=project):
        pending = call("stage", owner=owner)["pending"]
        assert pending and call("submit-pending", "app", "src/main.w", owner=owner)["selected"] == pending
        return pending

    def build(profile):
        proc = subprocess.run([ordinary, "build", "app", "--profile", profile], cwd=project,
                              capture_output=True, check=False, timeout=30)
        marker = b"wosy: build rejected by semantic diagnostics\n"
        assert proc.returncode == 1 and proc.stderr.endswith(marker)
        return proc.stderr[:-len(marker)].decode()

    def graph(*args):
        return json.loads(subprocess.check_output([example, project, store, *args], timeout=30))

    leaf = make(project)
    old = admit()
    expected = {profile: build(profile) for profile in ("dev", "native")}
    call("reconcile-acknowledged", database=conditional)
    valid = subprocess.run([unit, "saved_build_diagnostic_commit_requires_live_lease_and_conditional_name",
                            "--ignored", "--nocapture"], capture_output=True, text=True, check=False,
                           timeout=30, env={**os.environ, "WOSY_SAVED_DIAGNOSTIC_PROJECT": str(project),
                               "WOSY_SAVED_DIAGNOSTIC_STORE": str(conditional), "WOSY_SAVED_DIAGNOSTIC_ROOT": old})
    assert valid.returncode == 0 and "1 passed" in valid.stdout, valid.stdout + valid.stderr

    max_bytes = str(64 << 10)
    # A Unicode profile exceeds the caller's budget by UTF-8 bytes, not
    # character count. A generous caller budget still obeys the inline cap.
    call("save-build-diagnostic", "profile-byte-budget", "3", "féé", refusal=b"report byte budget")
    call("save-build-diagnostic", "profile-inline-cap", str(128 << 10), "é" * ((32 << 10) + 1),
         refusal=b"report byte budget")
    # The profile itself fits; the fresh complete report does not.
    call("save-build-diagnostic", "under-budget", "3", "dev", refusal=b"report byte budget")
    for name in ("profile-byte-budget", "profile-inline-cap", "under-budget"):
        assert call("read-saved-build-diagnostic", name, max_bytes)["result"]["report"] is None

    saved = {}
    for profile in ("dev", "native"):
        key = f"historical-{profile}"
        response = call("save-build-diagnostic-exit", key, max_bytes, profile)
        assert response["rendered"] == expected[profile]
        assert bytes(response["report"]["source_root"]).hex() == old
        saved[profile] = response
        idempotent = call("save-build-diagnostic", key, max_bytes, profile)["result"]
        assert idempotent["report"] == response["report"] and idempotent["rendered"] == response["rendered"]
        call("read-saved-build-diagnostic", key, "1", refusal=b"report byte budget")
    call("save-build-diagnostic", "historical-dev", max_bytes, "native", refusal=b"name already retains")

    # A distinct authorized revision in this workspace can read the historical
    # name, but no current worktree bytes are used for its old diagnostics.
    leaf.write_text(old_source.replace("origem guardada ☕", "revisão atual 🚀").replace("next; next", "next"))
    current = admit()
    assert current != old
    for profile, saved_value in saved.items():
        restored = call("read-saved-build-diagnostic", f"historical-{profile}", max_bytes)
        assert restored["selected"] == current
        assert restored["result"]["report"] == saved_value["report"]
        assert restored["result"]["rendered"] == expected[profile]
    assert graph(current, "--source-pins")["pins"] == [
        {"name": "historical-dev", "root": old}, {"name": "historical-native", "root": old},
    ]
    retained = graph(current, "--source-plan", "20000", old)
    assert retained["candidate_roots"] == 0 and retained["protected_roots"] == 2
    make(other)
    assert admit(other) not in (old, current)
    assert call("read-saved-build-diagnostic", "historical-dev", max_bytes, owner=other)["result"]["report"] is None
    assert not (project / ".wosy/artifacts").exists()
    print("Native64/WASI saved reports equal ordinary rendered bytes after exit-before-close and source advance; "
          "authored pin retention, distinct owner, exact byte/name limits, dropped/expired/conditional writer all hold.",
          {profile: round(value["operation_ms"], 3) for profile, value in saved.items()})

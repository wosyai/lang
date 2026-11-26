"""Publish a real pending dependency edit using the typed client and scoped store worker."""
import json
from pathlib import Path
import subprocess
import sys
import tempfile


example, cli = (Path(arg).resolve() for arg in sys.argv[1:3])
manifest = (Path(__file__).parent /
            "fixtures/build/shared_stdlib_local_dependency/project/wosy.toml").read_text()
manifest = manifest.replace("categories = []", 'categories = ["WHY"]') + """
[builders.llvm-native]
command = ["python", "scripts/build-native"]
args = []
toolchain_identity = "llvm-18.native"

[runners.native]
command = ["python", "scripts/run-native"]
args = []

[targets.app.artifacts.main.profiles.native]
backend = "llvm"
output = "native"
builder = "llvm-native"
run_runner = "native"

[targets.app.profiles.native]
main_artifact = "main"
"""


def leaf(note, body):
    return f"""%%start
# WHY: {note}
struct Item {{ u8 value; }}
Item(Item) consume = fn(input) {{ next = input; {body} }};
%%end
"""


def case(directory, profile, restage):
    project, old_catalog, replacement = (directory / name for name in ("project", "old", "replacement"))
    (project / "src").mkdir(parents=True)
    (project / "stdlib/src").mkdir(parents=True)
    (project / "wosy.toml").write_text(manifest)
    (project / "stdlib/wosy.toml").write_text('[package]\nname="std"\nsource_root="src"\n')
    entry = project / "src/main.w"
    provider = project / "stdlib/src/leaf.w"
    sibling = project / "stdlib/src/sibling.w"
    entry.write_text('%%start\nleaf = namespace std "leaf.w";\nsibling = namespace std "sibling.w";\n%%end\n')
    sibling.write_text("%%start\nu32 unchanged = 7;\n%%end\n")
    before = leaf("anterior ☕", "next")
    after = leaf("novo diagnóstico 🚀", "next; next")
    provider.write_text(before)

    def state(store, *args):
        output = subprocess.run([example, project, store, "--persistent-source", "app", *args],
                                capture_output=True, check=True, text=True, timeout=30)
        return json.loads(output.stdout)

    initial = state(old_catalog, "stage")
    assert initial["result"]["parsed"] == 3
    old = initial["pending"]
    assert state(old_catalog, "submit-pending", "app", "src/main.w")["selected"] == old
    provider.write_text(after)
    pending = state(old_catalog, "stage")
    new = pending["pending"]
    assert new != old and pending["selected"] == old
    assert (pending["result"]["parsed"], pending["result"]["reused"]) == (1, 2)
    # A newer unstaged edit is intentionally not a publication input.
    if not restage:
        provider.write_text(leaf("not staged at all", "next"))
    action = "publish-pending-base-restage" if restage else "publish-pending-base"
    final = state(replacement, action, profile)
    answer = final["result"]
    assert final["selected"] == answer["root"] == new and final["pending"] is None
    assert answer["validated_files"] == 1
    assert answer["already_present"] == answer["stale_receipt_refused"] == restage
    assert (answer["written_records"] == 0) == restage
    assert answer["attempt"] == pending["attempt"] + int(restage)
    assert answer["previous_source"] == before and answer["next_source"] == after
    assert answer["presentation"]["response"]["profile"] == profile
    assert answer["presentation"]["response"]["diagnostics"]
    if not restage:
        assert "not staged at all" in provider.read_text()
        provider.write_text(after)
    actual = subprocess.run([cli, "build", "app", "--profile", profile], cwd=project,
                            capture_output=True, check=False, timeout=30)
    ending = b"wosy: build rejected by semantic diagnostics\n"
    assert actual.returncode == 1 and not actual.stdout and actual.stderr.endswith(ending), actual.stderr
    assert answer["presentation"]["rendered"] == actual.stderr[:-len(ending)].decode()
    assert not (project / ".wosy/artifacts").exists()
    reopened = state(replacement, "reconcile-acknowledged-limited")["result"]
    assert reopened["kind"] == "known" and reopened["root"] == new
    return answer["publication_ms"]


with tempfile.TemporaryDirectory() as temp:
    home = Path(temp).resolve()
    print(json.dumps({
        "wasi": case(home / "wasi", "dev", False),
        "native_same_root_restage": case(home / "native", "native", True),
        "changed_files_admitted": 1, "unchanged_files_reused": 2,
        "actual_cli_report_match": True,
    }))

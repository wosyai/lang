"""Exercise the host-bound ordinary base projection, with no source publication."""
import json
from pathlib import Path
import subprocess
import sys
import tempfile


example = Path(sys.argv[1]).resolve()
manifest = (Path(__file__).parent /
            "fixtures/build/shared_stdlib_local_dependency/project/wosy.toml").read_text()
manifest = manifest.replace("categories = []", 'categories = ["WHY"]')


def call(project, store, *args):
    child = subprocess.run([example, project, store, "--persistent-source", "app", *map(str, args)],
                           text=True, capture_output=True)
    assert child.returncode == 0, child.stderr
    return json.loads(child.stdout)


with tempfile.TemporaryDirectory() as temp:
    project, store = Path(temp) / "project", Path(temp) / "store"
    (project / "src").mkdir(parents=True)
    (project / "stdlib" / "src").mkdir(parents=True)
    (project / "wosy.toml").write_text(manifest)
    (project / "stdlib" / "wosy.toml").write_text('[package]\nname="std"\nsource_root="src"\n')
    entry = project / "src" / "main.w"
    entry.write_text('%%start\n# WHY: origem ☕\nlib = namespace std "module.w";\n%%end\n')
    library = project / "stdlib" / "src" / "module.w"
    library.write_text("%%start\nu32 value = 27;\n%%end\n")
    prior = call(project, store, "stage")
    selected = call(project, store, "submit-pending", "app", "src/main.w")["selected"]
    assert selected == prior["pending"]
    library.write_text("%%start\nu32 value = 83;\n%%end\n")
    candidate = call(project, store, "stage")
    assert candidate["result"]["parsed"] == candidate["result"]["reused"] == 1
    root, attempt = candidate["pending"], candidate["attempt"]
    answer = call(project, store, "framed-base-projection", root, attempt)
    assert (answer["selected"], answer["pending"], answer["attempt"]) == (selected, root, attempt + 1)
    projection = answer["result"]
    assert projection["stale_refused"] and projection["scope_refused"]
    assert projection["wrong_connection_and_id_refused"]
    assert projection["single_lease_at_quota"] and projection["close_eof_expiry_released"]
    assert projection["projection"]["attempt"] == attempt + 1
    assert bytes(projection["projection"]["proposed"]).hex() == root
    assert 0 < projection["response_bytes"] < 1024
    print(json.dumps(projection))

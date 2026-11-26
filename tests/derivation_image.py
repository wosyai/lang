"""Use two real optimized example images (one shared target), one store."""
import hashlib
import json
from pathlib import Path
import subprocess
import sys


example, fixture = (Path(text).resolve() for text in sys.argv[1:3])
phase = sys.argv[3]
project, store = fixture / "project", fixture / "store"
record = fixture / "phase-a.json"


def invoke(*args):
    proc = subprocess.run([example, project, store, *args], capture_output=True, text=True, check=True)
    assert not proc.stderr, proc.stderr
    return json.loads(proc.stdout)


def image():
    label = invoke("--fixture-compiler-image")
    with example.open("rb") as executable:
        return label, hashlib.file_digest(executable, "sha256").hexdigest()


def source(body, why=None):
    comment = "" if why is None else f"# WHY: {why}\n"
    return "%%start\n" + comment + body + "%%end\n"


def project_view(root, *args):
    return invoke(root, "--project-declarations", *args)


def census(root):
    return invoke(root, "--source-plan", "10000")


def reused(view, new_modules, new_bodies):
    assert view["modules_derived"] == new_modules and view["bodies_derived"] == new_bodies, view
    assert view["modules_reused"] == 3 - new_modules, view


entry = 'leaf = namespace std "leaf.w";\nsibling = namespace std "sibling.w";\nu32 answer = leaf.value;\n'
if phase == "seed":
    template = (Path(__file__).parent /
                "fixtures/build/shared_stdlib_local_dependency/project/wosy.toml").read_text()
    (project / "src").mkdir(parents=True)
    deps = project / "stdlib/src"
    deps.mkdir(parents=True)
    (project / "wosy.toml").write_text(template.replace("categories = []", 'categories = ["WHY"]'))
    (deps.parent / "wosy.toml").write_text('[package]\nname="std"\nsource_root="src"\n')
    (project / "src/main.w").write_text(source(entry, "original ☕"))
    (deps / "leaf.w").write_text(source("u32 value = 17;\n"))
    (deps / "sibling.w").write_text(source("u32 unaffected = 29;\n"))
    label, sha = image()
    assert label["fixture_image"] == "alpha"
    root = invoke()["snapshot"]["source_root"]
    cold_scope = census(root)
    start = project_view(root, "--oracle")
    reused(start, 3, 3)
    warm = project_view(root)
    reused(warm, 0, 0)
    assert warm["modules"] == start["modules"] and warm["projection"] == start["projection"]
    warm_scope = census(root)
    assert warm_scope["nodes_visited"] > cold_scope["nodes_visited"]
    record.write_text(json.dumps({
        "label": label, "sha": sha, "root": root, "modules": warm["modules"],
        "projection": warm["projection"], "cold_nodes": cold_scope["nodes_visited"],
        "warm_nodes": warm_scope["nodes_visited"], "epoch": warm_scope["protection_epoch"],
    }))
    print("image A: genuine cold/warm rooted declaration reuse", {"root": root, "sha256": sha})
elif phase == "compare":
    alpha = json.loads(record.read_text())
    label, sha = image()
    assert label["fixture_image"] == "beta"
    assert label["compiler_version"] == alpha["label"]["compiler_version"]
    assert sha != alpha["sha"]
    root = alpha["root"]
    read = invoke(root, "app", "src/main.w")
    assert read["text"] == source(entry, "original ☕")
    cold_scope = census(root)
    assert cold_scope["nodes_visited"] == alpha["cold_nodes"]
    assert cold_scope["protection_epoch"] != alpha["epoch"]
    derived = project_view(root, "--oracle")
    reused(derived, 3, 3)
    assert derived["modules"] == alpha["modules"] and derived["projection"] == alpha["projection"]
    warm = project_view(root)
    reused(warm, 0, 0)
    assert warm["modules"] == derived["modules"]
    assert census(root)["nodes_visited"] == alpha["warm_nodes"]
    prepared = invoke(root, "--typed-project", "dev", "--oracle")
    assert prepared["ready"] and not prepared["diagnostics"]
    # Annotations alone do not rerun compiler declarations inside this image.
    (project / "src/main.w").write_text(source(entry, "nova anotação 🚀"))
    newer = invoke(root)["snapshot"]["source_root"]
    assert newer != root
    note = project_view(newer, "--oracle")
    reused(note, 0, 0)
    assert note["projection"] == warm["projection"]
    (project / "stdlib/src/leaf.w").write_text(source("u32 value = 31;\n"))
    edit = invoke(newer)["snapshot"]["source_root"]
    selected = project_view(edit, "--oracle")
    reused(selected, 2, 1)
    assert selected["projection"] != warm["projection"]
    assert selected["modules"][2] == warm["modules"][2]
    historical = project_view(root)
    reused(historical, 0, 0)
    assert historical["modules"] == alpha["modules"]
    print("image B: same SemVer, distinct binary, genuine cold/warm/selective derivations", {
        "sha256": sha, "cold": derived["modules_derived"], "warm": warm["modules_reused"],
        "selective": selected["modules_derived"], "untouched": selected["modules_reused"],
        "cold_census": cold_scope["nodes_visited"], "warm_census": alpha["warm_nodes"],
    })
else:
    raise AssertionError(phase)

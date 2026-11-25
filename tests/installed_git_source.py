"""Exercise a legitimate installed dependency via the ordinary, file-only Git route."""
import json
import os
from pathlib import Path
import shlex
import shutil
import subprocess
import sys
import tempfile
import time


compiler, example = [Path(arg).resolve() for arg in sys.argv[1:]]
here = Path(__file__).parent
template = (here / "fixtures/build/shared_stdlib_local_dependency/project/wosy.toml").read_text()
manifest = template.replace('[dependencies.std]\npath = "stdlib"\n\n', '')
manifest = manifest.replace("categories = []", 'categories = ["WHY"]')


def capture(*args, cwd=None):
    return subprocess.run(args, cwd=cwd, env=env, capture_output=True, check=True, text=True).stdout


def snapshot(root, store, *args):
    with tempfile.TemporaryFile() as output:
        start = time.perf_counter()
        child = subprocess.Popen([example, root, store, *args], env=env, stdout=output)
        _, status, usage = os.wait4(child.pid, 0)
        child.returncode = os.waitstatus_to_exitcode(status)
        if child.returncode:
            raise subprocess.CalledProcessError(child.returncode, child.args)
        elapsed = (time.perf_counter() - start) * 1000
        output.seek(0)
        return json.load(output), elapsed, usage.ru_maxrss


with tempfile.TemporaryDirectory() as directory:
    work = Path(directory)
    origin, app, store, bin_dir = (work / key for key in ("upstream", "app", "store", "bin"))
    for path in (origin, app, bin_dir):
        path.mkdir()
    # Dev5 sets its own general protocol list for Git children. Narrow it
    # again through the fixture's PATH; also ignore host Git configurations.
    git = shlex.quote(str(Path(shutil.which("git")).resolve()))
    adapter = bin_dir / "git"
    adapter.write_text(f"#!/bin/sh\nGIT_ALLOW_PROTOCOL=file exec {git} \"$@\"\n")
    adapter.chmod(0o755)
    env = {key: value for key, value in os.environ.items() if not key.startswith("GIT_")}
    env.update(PATH=f"{bin_dir}:{os.environ['PATH']}", HOME=str(work), GIT_ALLOW_PROTOCOL="file",
               GIT_CONFIG_NOSYSTEM="1", GIT_CONFIG_GLOBAL="/dev/null")
    capture("git", "init", "--quiet", "--template=", "--object-format=sha1", cwd=origin)
    package = origin / "packages/remote_lib"
    (package / "src").mkdir(parents=True)
    (package / "wosy.toml").write_text('[package]\nname = "remote_lib"\nsource_root = "src"\n')
    pristine = "%%start\n# WHY: upstream ☕\nstruct copy Packet { u32 count; }\n%%end\n"
    edited = pristine.replace("upstream ☕", "alterado no projeto 🚀").replace("u32", "u64")
    (package / "src/packet.w").write_text(pristine)
    (package / "src/constants.w").write_text("%%start\nu32 value = 17;\n%%end\n")
    capture("git", "add", "--all", cwd=origin)
    capture("git", "-c", "user.name=Fixture", "-c", "user.email=fixture@example.invalid",
            "commit", "--quiet", "--no-verify", "--no-gpg-sign", "-m", "package", cwd=origin)

    (app / "wosy.toml").write_text(manifest)
    (app / "src").mkdir()
    (app / "src/main.w").write_text(
        '%%start\npayload = namespace remote_lib "packet.w";\n'
        'constants = namespace remote_lib "constants.w";\n'
        'payload.Packet packet = { .count = 19; };\nu32 marker = constants.value;\n%%end\n'
    )
    capture(compiler, "dep", "add", "../upstream", "--path", "packages/remote_lib", cwd=app)
    capture(compiler, "dep", "sync", cwd=app)
    lock = (app / "wosy.lock").read_bytes()
    installed = app / "deps/remote_lib/src/packet.w"
    assert installed.read_bytes() == pristine.encode()

    first, cold_wall, cold_rss = snapshot(app, store)
    first = first["snapshot"]
    initial = first["source_root"]
    assert (first["parsed"], first["reused"]) == (3, 0)
    selection = snapshot(app, store, initial, "--installed-git")[0]["remote_lib"]
    old_typed = snapshot(app, store, initial, "--typed-project", "dev", "--oracle")[0]
    old_modules = snapshot(app, store, initial, "--project-declarations")[0]["modules"]
    assert old_typed["ready"] and old_typed["modules"][1]["structs"][0]["layout"]["size"] == 4
    warm, warm_wall, warm_rss = snapshot(app, store, initial)
    warm = warm["snapshot"]
    assert warm["source_root"] == initial and warm["written"] == warm["parsed"] == 0
    assert warm["reused"] == 3

    installed.write_text(edited)
    dirty, dirty_wall, dirty_rss = snapshot(app, store, initial)
    dirty = dirty["snapshot"]
    authored = dirty["source_root"]
    assert initial != authored and (dirty["parsed"], dirty["reused"]) == (1, 2)
    assert snapshot(app, store, authored, "--installed-git")[0]["remote_lib"] == selection
    assert snapshot(app, store, authored, "--typed-project", "dev", "--oracle")[0]["modules"][1]["structs"][0]["layout"]["size"] == 8

    capture(compiler, "dep", "snapshot", cwd=app)
    assert (app / "wosy.lock").read_bytes() == lock and installed.read_bytes() == edited.encode()
    assert len(list((app / "patches/remote_lib").glob("*.patch"))) == 1
    overlay, patch_wall, patch_rss = snapshot(app, store, authored)
    overlay = overlay["snapshot"]
    patched = overlay["source_root"]
    assert patched not in (initial, authored) and (overlay["parsed"], overlay["reused"]) == (0, 3)
    assert overlay["executable"] == dirty["executable"]
    assert overlay["annotations"] == dirty["annotations"]
    patch = snapshot(app, store, patched, "--installed-git")[0]["remote_lib"]
    assert patch["locked"] == selection["locked"] and patch["patches"] != selection["patches"]
    assert snapshot(app, store, initial, "--installed-git")[0]["remote_lib"] == selection
    new_typed = snapshot(app, store, patched, "--typed-project", "dev", "--oracle")[0]
    new_project = snapshot(app, store, patched, "--project-declarations")[0]
    assert new_project["modules_derived"] == 0 and new_project["modules_reused"] == 3
    new_modules = new_project["modules"]
    assert old_modules[1]["body"] != new_modules[1]["body"]
    assert old_modules[2]["body"] == new_modules[2]["body"]
    assert new_typed["ready"] and new_typed["modules"][1]["structs"][0]["layout"]["size"] == 8
    old = snapshot(app, store, initial, "remote_lib", "src/packet.w")[0]
    old_again = snapshot(app, store, initial, "--typed-project", "dev")[0]
    assert old["text"] == pristine and old_again["prepared_identity"] == old_typed["prepared_identity"]
    assert old_again["modules"][1]["structs"][0]["layout"]["size"] == 4
    assert (package / "src/packet.w").read_text() == pristine
    capture(compiler, "parse", "app", cwd=app)
    assert (app / ".wosy/cst/current.json").is_file()

    print("installed pinned subpackage, edited live bytes, captured overlay, old rooted compiler parity", {
        "cold_wall_ms": round(cold_wall, 3), "warm_wall_ms": round(warm_wall, 3),
        "one_edited_file_wall_ms": round(dirty_wall, 3), "new_stack_same_sources_ms": round(patch_wall, 3),
        "observe_ms": [round(state["observe_ms"], 3) for state in (first, warm, dirty, overlay)],
        "publish_writes": [state["written"] for state in (first, warm, dirty, overlay)],
        "peak_child_kib": [cold_rss, warm_rss, dirty_rss, patch_rss], "observed_files": 3,
    }, flush=True)

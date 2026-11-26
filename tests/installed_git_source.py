"""Exercise a legitimate installed dependency via the ordinary, file-only Git route."""
import hashlib
import json
import os
from pathlib import Path
import shlex
import shutil
import subprocess
import sys
import tempfile
import time


compiler, example, *older = [Path(arg).resolve() for arg in sys.argv[1:]]
here = Path(__file__).parent
template = (here / "fixtures/build/shared_stdlib_local_dependency/project/wosy.toml").read_text()
manifest = template.replace('[dependencies.std]\npath = "stdlib"\n\n', '')
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


def consumer(root):
    (root / "wosy.toml").write_text(manifest)
    (root / "src").mkdir()
    (root / "src/main.w").write_text(
        '%%start\npayload = namespace remote_lib "packet.w";\n'
        'constants = namespace remote_lib "constants.w";\n'
        'payload.Packet packet = { .count = 19; };\nu32 marker = constants.value;\n%%end\n'
    )
    capture(compiler, "dep", "add", "../upstream", "--path", "packages/remote_lib", cwd=root)
    capture(compiler, "dep", "sync", cwd=root)
    capture(compiler, "parse", "app", cwd=root)
    assert (root / ".wosy/cst/current.json").is_file()
    return root / "deps/remote_lib/src/packet.w"


def workspace_evidence(root):
    paths = [
        root / name for name in ("wosy.toml", "wosy.lock", "src/main.w",
                                 "deps/.wosy-pins/remote_lib", "deps/remote_lib/wosy.toml",
                                 "deps/remote_lib/src/packet.w", "deps/remote_lib/src/constants.w")
    ]
    paths.extend((root / "patches/remote_lib").glob("*.patch"))
    paths.extend((root / ".wosy/cst").glob("*.json"))
    return {path.relative_to(root): hashlib.sha256(path.read_bytes()).hexdigest() for path in paths}


with tempfile.TemporaryDirectory() as directory:
    work = Path(directory)
    origin, app, peer, store, bin_dir = (work / key for key in ("upstream", "app", "peer", "store", "bin"))
    for path in (origin, app, peer, bin_dir):
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

    installed, peer_source = consumer(app), consumer(peer)
    lock = (app / "wosy.lock").read_bytes()
    assert (peer / "wosy.lock").read_bytes() == lock
    assert installed.read_bytes() == peer_source.read_bytes() == pristine.encode()
    initial_workspace, peer_workspace = workspace_evidence(app), workspace_evidence(peer)

    # The read-only observation needs no Git executable or reachable origin.
    # The true package tree is an already verified output of ordinary sync.
    revision = capture("git", "rev-parse", "HEAD", cwd=origin).strip()
    tree = capture("git", "rev-parse", "HEAD:packages/remote_lib", cwd=origin).strip()
    seal_a = app / "deps/.wosy-pins/remote_lib"
    sealed = json.loads(seal_a.read_text())
    assert (sealed["version"], sealed["verified_base"]["version"], sealed["verified_base"]["tree"]) == (1, 1, tree)
    read_env = {**env, "PATH": str(work / "no-programs")}
    if older:
        subprocess.run([older[0], "dep", "sync"], cwd=app, env=read_env, check=True, capture_output=True)
        assert workspace_evidence(app) == initial_workspace
    def observe(project, refusal=None):
        result = subprocess.run([example, project, store, "--observe-installed-git-base", "remote_lib"],
                                text=True, capture_output=True, env=read_env)
        if refusal:
            assert result.returncode and refusal in result.stderr, result.stderr
            return
        result.check_returncode()
        return json.loads(result.stdout)
    observed = observe(app)
    pristine_base = observed["base"]
    assert pristine_base == observe(peer)["base"]
    assert (pristine_base["revision"], pristine_base["tree"], pristine_base["package_path"]) == (
        revision, tree, "packages/remote_lib",
    )
    assert pristine_base["source_hash"].encode() in lock
    assert not store.exists()
    permissions = installed.stat().st_mode & 0o777
    try:
        installed.chmod(permissions ^ 0o100)
        observe(app, refusal="unsnapshotted edits")
    finally:
        installed.chmod(permissions)

    def pending(project, operation, *args, refusal=None, target="app"):
        cmd = [example, project, store, "--persistent-source", target, operation, *map(str, args)]
        child = subprocess.run(cmd, env=read_env, text=True, capture_output=True)
        if refusal:
            assert child.returncode and refusal in child.stderr, child.stderr
            return
        assert child.returncode == 0, child.stderr
        return json.loads(child.stdout)

    def source_db_evidence(project):
        return tuple((project / name).read_bytes() for name in (".wosy/client.sqlite", ".wosy/cst/nodes.sqlite"))

    staged = pending(peer, "stage")
    same = staged["pending"]
    old_attempt = staged["attempt"]
    assert staged["selected"] is None
    assert pending(peer, "abandon", same, old_attempt)["result"]["abandoned"]
    current_stage = pending(peer, "stage")
    current_attempt = current_stage["attempt"]
    assert current_stage["pending"] == same and current_attempt > old_attempt
    peer_state = source_db_evidence(peer)
    pending(peer, "git-base-projection", same, old_attempt, "remote_lib", refusal="not the current source attempt")
    pending(peer, "git-base-projection", same, current_attempt, "remote_lib", target="different",
            refusal="not the current source attempt")
    pending(peer, "git-base-projection-limited", 0, 16 << 20, same, current_attempt, "remote_lib", refusal="LimitExceeded")
    projected = pending(peer, "git-base-projection", same, current_attempt, "remote_lib")
    clean_projection = projected["result"]["projection"]
    assert (clean_projection["project"], clean_projection["target"], clean_projection["package"]) == (
        str(peer), "app", "remote_lib",
    )
    assert (clean_projection["root"], clean_projection["attempt"], projected["selected"]) == (same, current_attempt, None)
    assert (clean_projection["revision"], clean_projection["tree"], clean_projection["locked"]) == (
        revision, tree, pristine_base["locked"],
    )
    assert [f["path"] for f in clean_projection["files"]] == ["src/constants.w", "src/packet.w"]
    assert source_db_evidence(peer) == peer_state and not store.exists()
    assert pending(peer, "abandon", same, current_attempt)["result"]["abandoned"]

    # The current checkout is pristine again; the saved staged generation is
    # not. Checkout provenance must not bless a different authored payload.
    peer_source.write_text(edited)
    other_stage = pending(peer, "stage")
    peer_source.write_text(pristine)
    assert other_stage["pending"] != same
    dirty_state = source_db_evidence(peer)
    pending(peer, "git-base-projection", other_stage["pending"], other_stage["attempt"], "remote_lib",
            refusal="staged source revision does not match the installed Git base")
    assert source_db_evidence(peer) == dirty_state and not store.exists()
    assert pending(peer, "abandon", other_stage["pending"], other_stage["attempt"])["result"]["abandoned"]

    first, cold_wall, cold_rss = snapshot(app, store)
    first = first["snapshot"]
    initial = first["source_root"]
    assert (first["parsed"], first["reused"]) == (3, 0)
    selection = snapshot(app, store, initial, "--installed-git")[0]["remote_lib"]
    assert pristine_base["locked"] == selection["locked"]
    old_typed = snapshot(app, store, initial, "--typed-project", "dev", "--oracle")[0]
    old_modules = snapshot(app, store, initial, "--project-declarations")[0]["modules"]
    assert old_typed["ready"] and old_typed["modules"][1]["structs"][0]["layout"]["size"] == 4
    warm, warm_wall, warm_rss = snapshot(app, store, initial)
    warm = warm["snapshot"]
    assert warm["source_root"] == initial and warm["written"] == warm["parsed"] == 0
    assert warm["reused"] == 3
    peer_first = snapshot(peer, store)[0]["snapshot"]
    peer_root = peer_first["source_root"]
    assert peer_root == same
    observed_roots = [(f["path"], f["authored"], f["executable"]) for f in clean_projection["files"]]
    assert observed_roots == [(f["path"], f["authored"], f["executable"]) for f in peer_first["files"]
                              if f["package"] == "remote_lib"]
    assert peer_root != initial and peer_first["executable"] == first["executable"]
    assert snapshot(peer, store, peer_root, "--installed-git")[0]["remote_lib"] == selection
    peer_typed = snapshot(peer, store, peer_root, "--typed-project", "dev", "--oracle")[0]
    assert peer_typed["ready"] and peer_typed["project_ast"] == old_typed["project_ast"]
    assert peer_typed["prepared_identity"] != old_typed["prepared_identity"]
    assert peer_typed["modules"][1]["owner"]["project"] != old_typed["modules"][1]["owner"]["project"]
    assert peer_typed["modules"][1]["structs"][0]["layout"]["size"] == 4
    assert workspace_evidence(app) == initial_workspace
    assert workspace_evidence(peer) == peer_workspace

    installed.write_text(edited)
    dirty_workspace = workspace_evidence(app)
    observe(app, refusal="unsnapshotted edits")
    dirty, dirty_wall, dirty_rss = snapshot(app, store, initial)
    dirty = dirty["snapshot"]
    authored = dirty["source_root"]
    assert initial != authored and (dirty["parsed"], dirty["reused"]) == (1, 2)
    assert snapshot(app, store, authored, "--installed-git")[0]["remote_lib"] == selection
    assert snapshot(app, store, authored, "--typed-project", "dev", "--oracle")[0]["modules"][1]["structs"][0]["layout"]["size"] == 8
    assert workspace_evidence(app) == dirty_workspace

    capture(compiler, "dep", "snapshot", cwd=app)
    assert (app / "wosy.lock").read_bytes() == lock and installed.read_bytes() == edited.encode()
    assert len(list((app / "patches/remote_lib").glob("*.patch"))) == 1
    overlay_workspace = workspace_evidence(app)
    assert observe(app)["base"] is None
    assert json.loads(seal_a.read_text())["verified_base"]["tree"] == tree
    assert observe(peer)["base"] == pristine_base
    overlay_stage = pending(app, "stage")
    overlay_state = source_db_evidence(app)
    assert pending(app, "git-base-projection", overlay_stage["pending"], overlay_stage["attempt"],
                   "remote_lib")["result"]["projection"] is None
    assert source_db_evidence(app) == overlay_state
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

    # The already-cached importer text is equal in both consumers; neither
    # source view, selected stack nor resolved owner is interchangeable.
    peer_warm, peer_wall, peer_rss = snapshot(peer, store, peer_root)
    peer_warm = peer_warm["snapshot"]
    assert (peer_warm["source_root"], peer_warm["parsed"], peer_warm["written"]) == (peer_root, 0, 0)
    assert snapshot(peer, store, peer_root, "--installed-git")[0]["remote_lib"] == selection
    peer_after = snapshot(peer, store, peer_root, "--typed-project", "dev", "--oracle")[0]
    assert peer_after["prepared_identity"] == peer_typed["prepared_identity"]
    assert peer_after["modules"][1]["structs"][0]["layout"]["size"] == 4
    back = snapshot(app, store, patched)[0]["snapshot"]
    assert (back["source_root"], back["parsed"], back["written"]) == (patched, 0, 0)
    assert snapshot(app, store, patched, "--installed-git")[0]["remote_lib"] == patch
    patched_again = snapshot(app, store, patched, "--typed-project", "dev", "--oracle")[0]
    assert patched_again["prepared_identity"] == new_typed["prepared_identity"]
    assert patched_again["modules"][1]["structs"][0]["layout"]["size"] == 8
    assert new_modules[0]["body"] == old_modules[0]["body"]  # importer AST

    old = snapshot(app, store, initial, "remote_lib", "src/packet.w")[0]
    old_again = snapshot(app, store, initial, "--typed-project", "dev")[0]
    assert old["text"] == pristine and old_again["prepared_identity"] == old_typed["prepared_identity"]
    assert old_again["modules"][1]["structs"][0]["layout"]["size"] == 4
    assert (package / "src/packet.w").read_text() == pristine
    assert workspace_evidence(app) == overlay_workspace
    assert workspace_evidence(peer) == peer_workspace

    # The authorized installed ref is the old exact package pin, not the
    # current upstream HEAD. Advance only this owned file:// fixture.
    (package / "src/constants.w").write_text("%%start\nu32 value = 18;\n%%end\n")
    capture("git", "add", "--all", cwd=origin)
    capture("git", "-c", "user.name=Fixture", "-c", "user.email=fixture@example.invalid",
            "commit", "--quiet", "--no-verify", "--no-gpg-sign", "-m", "new unselected revision", cwd=origin)
    assert capture("git", "rev-parse", "HEAD", cwd=origin).strip() != revision

    app_stage = pending(app, "state")
    assert app_stage["pending"] == patched
    assert pending(app, "submit-pending", "app", "src/main.w")["selected"] == patched
    peer_stage = pending(peer, "stage")
    assert peer_stage["pending"] == peer_root
    assert pending(peer, "submit-pending", "app", "src/main.w")["selected"] == peer_root
    stale = pending(peer, "leased-installed-git-llvm", "remote_lib", "dev", "stale")["result"]
    assert stale["kind"] == "stale-unopened" and stale["root"] == peer_root
    assert stale["to_generation"] > stale["from_generation"]
    untouched = workspace_evidence(peer)
    sqlite_before = source_db_evidence(peer)
    native, wasi = {}, {}
    for profile, result in (("dev", wasi), ("native", native)):
        mode = "reject-incomplete" if profile == "dev" else ""
        result.update(pending(peer, "leased-installed-git-llvm", "remote_lib", profile, mode)["result"])
        assert (result["kind"], result["root"], result["revision"], result["tree"], result["witnessed_files"]) == (
            "partition", peer_root, revision, tree, 2)
        patched_refusal = pending(app, "leased-installed-git-llvm", "remote_lib", profile)["result"]
        assert patched_refusal == {"kind": "not-pristine-or-not-current"}
    pending(peer, "leased-installed-git-llvm", "remote_lib", "dev", "records", refusal="index exceeds the record budget")
    pending(peer, "leased-installed-git-llvm", "remote_lib", "dev", "bytes", refusal="index exceeds the byte budget")
    assert workspace_evidence(peer) == untouched and source_db_evidence(peer) == sqlite_before
    assert not (peer / ".wosy/artifacts").exists()

    # Only the ordinary Build oracle needs its normal compiler executables;
    # any Git invocation in that path is also refused.
    no_git = work / "no-git"
    no_git.mkdir()
    sentinel = no_git / "attempted"
    (no_git / "git").write_text(f"#!/bin/sh\n: > {shlex.quote(str(sentinel))}\nexit 1\n")
    (no_git / "git").chmod(0o755)
    build_env = {**env, "PATH": f"{no_git}:{os.environ['PATH']}"}
    wasm_scripts = here / "fixtures/build/shared_stdlib_local_dependency/project/scripts"
    native_scripts = here / "fixtures/build/short_circuit_project/project/scripts"
    for consumer_root in (app, peer):
        scripts = consumer_root / "scripts"
        scripts.mkdir()
        for directory, names in ((wasm_scripts, ("build-wasm", "start.ll", "run-wasmtime")),
                                 (native_scripts, ("build-native", "run-native"))):
            for name in names:
                shutil.copyfile(directory / name, scripts / name)
    for profile, result in (("dev", wasi), ("native", native)):
        for consumer_root in (peer, app):
            child = subprocess.run([compiler, "build", "app", "--profile", profile], cwd=consumer_root,
                                   env=build_env, capture_output=True, timeout=30)
            assert child.returncode == 0, child.stderr
            artifact = consumer_root / ".wosy/artifacts/app" / profile / "main"
            assert (artifact / "main").read_bytes().startswith(b"\0asm" if profile == "dev" else b"\x7fELF")
            oracle = json.loads((artifact / "artifact-manifest.json").read_bytes())
            if consumer_root == app:
                scoped = pending(app, "leased-ordinary-llvm", profile, 2 << 20)["result"]["answer"]
                assert scoped["kind"] == "llvm" and scoped["root"] == patched
            else:
                scoped = result
            assert (artifact / "partition.ll").read_bytes() == scoped["llvm"].encode()
            assert oracle["identity"]["llvm_input_identity"] == scoped["digest"]
    assert not sentinel.exists()

    print("shared store, one Git pin, two patch stacks, A→B→A, authored history, read-only consumers", {
        "cold_wall_ms": round(cold_wall, 3), "warm_wall_ms": round(warm_wall, 3),
        "one_edited_file_wall_ms": round(dirty_wall, 3), "new_stack_same_sources_ms": round(patch_wall, 3),
        "observe_ms": [round(state["observe_ms"], 3) for state in (first, warm, dirty, overlay)],
        "publish_writes": [state["written"] for state in (first, warm, dirty, overlay)],
        "peak_child_kib": [cold_rss, warm_rss, dirty_rss, patch_rss], "observed_files": 3,
        "peer_warm_ms": round(peer_wall, 3), "peer_warm_peak_kib": peer_rss,
        "verified_git_base_ms": round(observed["observe_ms"], 3),
        "older_release_reader": bool(older),
        "staged_git_base_projection_ms": round(projected["result"]["project_ms"], 3),
        "sizes_A_B_A_historical": [8, 4, 8, 4],
        "selected_old_ref_native_wasi_llvm": True, "patched_ordinary_without_pristine_lease": True,
        "installed_scope_open_ms": [round(wasi["scope_open_ms"], 3), round(native["scope_open_ms"], 3)],
        "stale_generation_never_opened": True, "git_not_invoked_on_either_read_route": True,
    }, flush=True)

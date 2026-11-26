"""Compare real optimized ordinary Build LLVM with a read-only leased projection."""
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile


example, ordinary = (Path(argument).resolve() for argument in sys.argv[1:3])
daemon_tests = Path(sys.argv[3]).resolve() if len(sys.argv) > 3 else None
fixtures = Path(__file__).parent / "fixtures/build"
wasi = fixtures / "shared_stdlib_local_dependency/project"
native = fixtures / "short_circuit_project/project"
manifest = (wasi / "wosy.toml").read_text().replace("categories = []", 'categories = ["WHY"]') + """
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

[targets.app.artifacts.main.profiles.dev.exports]
fixture_answer = "answer"

[targets.app.artifacts.main.profiles.absent_export]
backend = "llvm"
output = "wasm-wasip1"
builder = "llvm-wasm"
run_runner = "wasmtime-wasip1"

[targets.app.artifacts.main.profiles.absent_export.exports]
fixture_answer = "dependency_only"

[targets.app.profiles.native]
main_artifact = "main"

[targets.app.profiles.absent_export]
main_artifact = "main"
"""
profiles = ("dev", "native")
peers = 8
files = peers + 2


def entry(note, erroneous=""):
    imports = "".join(f'peer{n} = namespace std "peer{n}.w";\n' for n in range(peers))
    return f'''%%start
# WHY: {note}
dep = namespace std "value.w";
{imports}i32() answer = fn {{ dep.value() }};
{erroneous}%%end
'''


def provider(note):
    return f'''%%start
# WHY: {note}
i32() value = fn {{ 42 }};
i32() dependency_only = fn {{ 7 }};
%%end
'''


with tempfile.TemporaryDirectory() as temporary:
    directory = Path(temporary).resolve()
    project, catalog = directory / "project", directory / "catalog"
    for name in ("src", "stdlib/src", "scripts"):
        (project / name).mkdir(parents=True)
    (project / "wosy.toml").write_text(manifest)
    for script in ("build-wasm", "start.ll", "run-wasmtime"):
        data = (wasi / "scripts" / script).read_bytes()
        if script == "build-wasm":
            data = data.replace(b'"--export=_start",', b'"--export=_start",\n        "--export=fixture_answer",')
        (project / "scripts" / script).write_bytes(data)
    for script in ("build-native", "run-native"):
        (project / "scripts" / script).write_bytes((native / "scripts" / script).read_bytes())
    (project / "stdlib/wosy.toml").write_text('[package]\nname="std"\nsource_root="src"\n')
    library = project / "stdlib/src"
    leaf = library / "value.w"
    leaf.write_text(provider("proveniência original ☕"))
    for n in range(peers):
        values = "".join(f"i32() value{k} = fn {{ {n + k} }};\n" for k in range(75))
        (library / f"peer{n}.w").write_text(f"%%start\n{values}%%end\n")
    authored = project / "src/main.w"

    def state(*args, refusal=None):
        child = subprocess.run([example, project, catalog, "--persistent-source", "app", *map(str, args)],
                               capture_output=True, timeout=30)
        if refusal:
            assert child.returncode and not child.stdout and refusal in child.stderr, child.stderr
            return
        assert child.returncode == 0, child.stderr
        return json.loads(child.stdout)

    def projection(profile, budget=2 << 20, control="", refusal=None):
        value = state("leased-ordinary-llvm", profile, budget, control, refusal=refusal)
        if not refusal:
            return value["result"]

    def acknowledge(staged):
        first = state("submit-pending", "app", "src/main.w")
        assert first["selected"] == staged["pending"] and first["pending"] is None
        return first["selected"]

    def ordinary_build(profile, refusal=None):
        child = subprocess.run([ordinary, "build", "app", "--profile", profile], cwd=project,
                               capture_output=True, timeout=30)
        if refusal:
            assert child.returncode == 1 and not child.stdout and refusal in child.stderr, child.stderr
            return
        assert child.returncode == 0, child.stderr
        directory = project / ".wosy/artifacts/app" / profile / "main"
        binary = (directory / "main").read_bytes()
        assert binary.startswith(b"\x00asm" if profile == "dev" else b"\x7fELF")
        if profile == "dev":
            assert b"fixture_answer" in binary
        manifest = json.loads((directory / "artifact-manifest.json").read_bytes())
        return (directory / "partition.ll").read_bytes(), manifest["identity"]["llvm_input_identity"]

    def require_parity(selected, profile):
        result = projection(profile)
        view = result["answer"]
        assert view["kind"] == "llvm" and view["root"] == selected and view["profile"] == profile
        assert view["grammar_verified_files"] == view["grammar_reparsed_files"] == files
        assert len(bytes.fromhex(view["image"])) == 32
        expected, digest = ordinary_build(profile)
        assert expected == view["llvm"].encode() and digest == view["digest"]
        assert b"@fixture_answer(" in expected if profile == "dev" else b"@fixture_answer(" not in expected
        return view, result["projection_ms"], result["grammar_ms"]

    def link_executable_projection(profile, view, expected):
        output = directory / "candidate"
        output.mkdir(exist_ok=True)
        source = output / "partition.ll"
        source.write_text(view["llvm"])
        ordinary_main = project / ".wosy/artifacts/app" / profile / "main"
        executable = output / "main"
        runtime = ordinary_main / "runtime.ll"
        if profile == "dev":
            builder = [sys.executable, project / "scripts/build-wasm", source, output,
                       ordinary_main / "artifact-manifest.json", runtime]
        else:
            # The chosen Native fixture function has the ordinary i32(void)
            # ABI, but no public Native export table. Use its known checked
            # executable source owner, not textual discovery of an IR symbol.
            rooted = subprocess.check_output([example, project, catalog, view["root"],
                                              "app", "src/main.w", "--declarations"], timeout=30)
            revision = json.loads(rooted)["executable"]
            assert len(bytes.fromhex(revision)) == 32
            symbol = "wosy_fn__" + "__".join(part.encode().hex() for part in (
                "app", "src/main.w", revision, "answer",
            ))
            harness = output / "native-answer.c"
            harness.write_text(f"extern int __real_main(void);\nextern int {symbol}(void);\n"
                               f"int __wrap_main(void) {{ return __real_main() == 0 ? {symbol}() : 125; }}\n")
            builder = ["clang", "-x", "ir", source, runtime, "-x", "c", harness,
                       "-Wl,--wrap=main", "-o", executable]
        linked = subprocess.run(builder, cwd=project, capture_output=True, timeout=30)
        assert linked.returncode == 0, linked.stderr
        assert executable.read_bytes().startswith(b"\x00asm" if profile == "dev" else b"\x7fELF")
        command = (["wasmtime", "run", "--invoke", "fixture_answer", executable]
                   if profile == "dev" else [executable])
        run = subprocess.run(command, capture_output=True, timeout=30)
        assert run.returncode == (0 if profile == "dev" else expected), (run.returncode, run.stderr)
        assert run.stdout.strip() == (str(expected).encode() if profile == "dev" else b""), (run.stdout, run.stderr)

    def executable_view(selected, profile):
        view = projection(profile, control="executable")["answer"]
        assert view["kind"] == "llvm" and view["root"] == selected and view["profile"] == profile
        assert view["grammar_verified_files"] == files
        return view

    authored.write_text(entry("proveniência original ☕"))
    a = state("stage")
    assert (a["result"]["parsed"], a["result"]["reused"]) == (files, 0)
    root_a = acknowledge(a)
    if daemon_tests:
        subprocess.run([daemon_tests, "rooted_llvm::tests::closed_projection_cannot_install_grammar_or_llvm",
                        "--exact", "--ignored"], env={**os.environ, "WOSY_TEST_IMAGE_STORE": str(catalog),
                          "WOSY_TEST_LLVM_PROJECT": str(project), "WOSY_TEST_IMAGE_ROOT": root_a},
                       check=True, timeout=30)
    assert not (project / ".wosy/artifacts").exists()
    assert projection("dev", control="wrong-project")["kind"] == "scope-refused"
    assert projection("not-configured")["answer"] == {"kind": "profile-not-selected"}
    refused = projection("absent_export")["answer"]
    assert refused["kind"] == "emitter-rejected" and "no root function" in refused["reason"]
    ordinary_build("absent_export", refusal=b"no root function")
    projection("dev", control="index", refusal=b"index exceeds the record budget")
    projection("dev", control="authored", refusal=b"authored byte budget")
    projection("dev", budget=1, refusal=b"partition exceeds the output byte budget")
    assert not (project / ".wosy/artifacts").exists()
    before = {profile: require_parity(root_a, profile) for profile in profiles}
    stable_a = {profile: executable_view(root_a, profile) for profile in profiles}
    assert all(stable_a[profile]["digest"] != before[profile][0]["digest"] for profile in profiles)
    for profile, view in stable_a.items():
        link_executable_projection(profile, view, 42)

    leaf.write_text(provider("somente uma anotação mais longa 🚀"))
    b = state("stage")
    assert (b["result"]["parsed"], b["result"]["reused"]) == (1, files - 1)
    assert b["selected"] == root_a and b["pending"] != root_a
    offline = authored.with_suffix(".fixture")
    authored.rename(offline)
    for profile in profiles:
        assert projection(profile)["answer"] == before[profile][0]
    offline.rename(authored)
    memo = state("leased-llvm-grammar-memo")["result"]
    root_b = memo["root"]
    assert root_b == b["pending"] and memo["files"] == files
    assert memo["hot_admission_reparsed"] == 1
    if daemon_tests:
        subprocess.run([daemon_tests, "rooted_llvm::tests::executable_llvm_waiters_share_flight_and_release_slots",
                        "--exact", "--ignored"], env={**os.environ, "WOSY_TEST_IMAGE_STORE": str(catalog),
                          "WOSY_TEST_LLVM_PROJECT": str(project), "WOSY_TEST_IMAGE_ROOT": root_a,
                          "WOSY_TEST_IMAGE_OTHER_ROOT": root_b}, check=True, timeout=30)
    after = {profile: require_parity(root_b, profile) for profile in profiles}
    assert memo["wasi_digest"] == after["dev"][0]["digest"]
    assert memo["native_digest"] == after["native"][0]["digest"]
    assert all(after[profile][0]["digest"] != before[profile][0]["digest"] for profile in profiles)
    for profile in profiles:
        stable_b = executable_view(root_b, profile)
        assert stable_b["digest"] == stable_a[profile]["digest"] and stable_b["llvm"] == stable_a[profile]["llvm"]
        link_executable_projection(profile, stable_b, 42)
    impact = subprocess.run([example, project, catalog, root_a, "--source-impact", root_b],
                            capture_output=True, check=True, timeout=30)
    effect = json.loads(impact.stdout)
    assert effect["annotation_tree_changed"] and not effect["executable_tree_changed"]
    leaf.write_text(provider("somente uma anotação mais longa 🚀").replace("{ 42 }", "{ 43 }"))
    root_c = acknowledge(state("stage"))
    for profile in profiles:
        stable_c = executable_view(root_c, profile)
        assert stable_c["digest"] != stable_a[profile]["digest"]
        link_executable_projection(profile, stable_c, 43)

    # Same executable source and layout, different selected WASI export.
    # Verify C needed fresh derivation from just the dependency edit first.
    (project / "wosy.toml").write_text(manifest.replace('fixture_answer = "answer"', 'fixture_alternate = "answer"'))
    root_d = acknowledge(state("stage"))
    assert root_d != root_c
    if daemon_tests:
        subprocess.run([daemon_tests, "rooted_llvm::tests::registered_executable_waiter_reuses_after_eviction_and_failed_producer",
                        "--exact", "--ignored"], env={**os.environ, "WOSY_TEST_IMAGE_STORE": str(catalog),
                          "WOSY_TEST_LLVM_PROJECT": str(project), "WOSY_TEST_IMAGE_ROOT": root_a,
                          "WOSY_TEST_IMAGE_OTHER_ROOT": root_b, "WOSY_TEST_LLVM_EXECUTABLE_ROOT": root_c,
                          "WOSY_TEST_LLVM_CONFIGURED_ROOT": root_d}, check=True, timeout=30)
    receipts = state("leased-executable-llvm-receipts", root_a, root_b, root_c)["result"]
    assert receipts["files"] == files and receipts["annotated_reparsed"] == 1
    assert receipts["profile_dependency_export_and_reopen_distinct"] and receipts["hit_source_and_output_budgets"]
    policy = state("leased-llvm-grammar-policy")["result"]
    assert policy == {"new_policy_reparsed": files, "rollback_did_not_seed": True}

    # An existing ordinary ownership diagnostic must still prevent emission.
    authored.write_text(entry("reject before LLVM", "struct Item { u8 value; }\n"
        "Item(Item) take = fn(input) { next = input; next; next };\n"))
    acknowledge(state("stage"))
    for profile in profiles:
        refused = projection(profile)["answer"]
        assert refused["kind"] == "analysis-rejected" and refused["diagnostics"] and "llvm" not in refused
        stable_refusal = projection(profile, control="executable")["answer"]
        assert stable_refusal == refused
        ordinary_build(profile, refusal=b"build rejected by semantic diagnostics")
        path = project / ".wosy/artifacts/app" / profile / "main/partition.ll"
        assert path.read_bytes() == after[profile][0]["llvm"].encode()
    print(json.dumps({"native_and_wasi_exact": True, "ordinary_wasi_export": True,
                      "annotation_only_changes_current_ordinary_llvm": True,
                      "annotation_stable_internal_linked_projection": True,
                      "executable_edit_changes_internal_projection": True,
                      "retained_accepted_source_without_live_entry": True, "errors_emit_no_bytes": True,
                      "projection_ms": {key: [before[key][1], after[key][1]] for key in profiles},
                      "grammar_ms": {key: [before[key][2], after[key][2]] for key in profiles},
                      "same_process_grammar_memo": {key: memo[key] for key in
                          ("files", "hot_admission_reparsed", "cold_a", "warm_a", "hot_b", "reopened_b")},
                      "executable_receipts": receipts,
                      "policy_change_and_rollback": policy}))

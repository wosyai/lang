"""Compare real optimized ordinary Build LLVM with a read-only leased projection."""
import json
from pathlib import Path
import subprocess
import sys
import tempfile


example, ordinary = (Path(argument).resolve() for argument in sys.argv[1:3])
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

    authored.write_text(entry("proveniência original ☕"))
    a = state("stage")
    assert (a["result"]["parsed"], a["result"]["reused"]) == (files, 0)
    root_a = acknowledge(a)
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
    after = {profile: require_parity(root_b, profile) for profile in profiles}
    assert memo["wasi_digest"] == after["dev"][0]["digest"]
    assert memo["native_digest"] == after["native"][0]["digest"]
    assert all(after[profile][0]["digest"] != before[profile][0]["digest"] for profile in profiles)
    impact = subprocess.run([example, project, catalog, root_a, "--source-impact", root_b],
                            capture_output=True, check=True, timeout=30)
    effect = json.loads(impact.stdout)
    assert effect["annotation_tree_changed"] and not effect["executable_tree_changed"]
    policy = state("leased-llvm-grammar-policy")["result"]
    assert policy == {"new_policy_reparsed": files, "rollback_did_not_seed": True}

    # An existing ordinary ownership diagnostic must still prevent emission.
    authored.write_text(entry("reject before LLVM", "struct Item { u8 value; }\n"
        "Item(Item) take = fn(input) { next = input; next; next };\n"))
    acknowledge(state("stage"))
    for profile in profiles:
        refused = projection(profile)["answer"]
        assert refused["kind"] == "analysis-rejected" and refused["diagnostics"] and "llvm" not in refused
        ordinary_build(profile, refusal=b"build rejected by semantic diagnostics")
        path = project / ".wosy/artifacts/app" / profile / "main/partition.ll"
        assert path.read_bytes() == after[profile][0]["llvm"].encode()
    print(json.dumps({"native_and_wasi_exact": True, "ordinary_wasi_export": True,
                      "annotation_only_changes_current_ordinary_llvm": True,
                      "retained_accepted_source_without_live_entry": True, "errors_emit_no_bytes": True,
                      "projection_ms": {key: [before[key][1], after[key][1]] for key in profiles},
                      "grammar_ms": {key: [before[key][2], after[key][2]] for key in profiles},
                      "same_process_grammar_memo": {key: memo[key] for key in
                          ("files", "hot_admission_reparsed", "cold_a", "warm_a", "hot_b", "reopened_b")},
                      "policy_change_and_rollback": policy}))

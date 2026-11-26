"""A selected SQLite root uses the actual leased diagnostic dispatcher, not an emitter."""
import hashlib
import json
from pathlib import Path
import subprocess
import sys
import tempfile


example = Path(sys.argv[1]).resolve()
ordinary_cli = Path(sys.argv[2]).resolve() if len(sys.argv) > 2 else None
manifest = (Path(__file__).parent /
            "fixtures/build/shared_stdlib_local_dependency/project/wosy.toml").read_text()
manifest = manifest.replace("categories = []", 'categories = ["WHY"]')
manifest += """
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


def call(project, store, *args):
    child = subprocess.run([example, project, store, "--persistent-source", "app", *args],
                           text=True, capture_output=True, check=True)
    return json.loads(child.stdout)


def response(result):
    return result["result"]["response"]


def script(note, result):
    bindings = "".join(f"u32 fact{i} = {i};\n" for i in range(180))
    return f"""%%start
# WHY: {note}
struct Item {{ u8 value; }}
Item(Item) take = fn(input) {{ next = input; {result} }};
{bindings}%%end
"""


with tempfile.TemporaryDirectory() as temp:
    project, store = Path(temp) / "project", Path(temp) / "store"
    (project / "src").mkdir(parents=True)
    (project / "stdlib/src").mkdir(parents=True)
    (project / "wosy.toml").write_text(manifest)
    (project / "stdlib/wosy.toml").write_text('[package]\nname="std"\nsource_root="src"\n')
    entry, provider = project / "src/main.w", project / "stdlib/src/item.w"
    peers = "".join(f'peer{i} = namespace std "peer{i}.w";\n' for i in range(17))
    entry.write_text(f'%%start\nlocal = namespace std "item.w";\n{peers}%%end\n')
    for i in range(17):
        (provider.parent / f"peer{i}.w").write_text(f"%%start\nu32 value = {100 + i};\n%%end\n")
    old_text = script("origem ☕", "next; next")
    provider.write_text(old_text)

    def terminal(catalog, profile, mode, kind, streams, replay, calls=None, *options):
        out = call(project, store.with_name(catalog), "reconcile-terminal-report", profile, mode, *options)
        result = out["result"]
        assert result["answer"]["kind"] == kind
        assert (result["streams_opened"], result["recovery_payloads"]) == (streams, replay)
        assert result["adapter_calls"] == (streams if calls is None else calls)
        return result["answer"], result["elapsed_ms"]

    terminal("terminal-none", "dev", "none", "none", 0, 0)
    terminal("live-none", "dev", "live-none", "none", 0, 0)
    first = call(project, store, "stage")
    old = call(project, store, "submit-pending", "app", "src/main.w")
    assert old["selected"] == first["pending"]
    terminal("terminal-expired", "dev", "expired", "expired", 0, 0)
    terminal("terminal-late", "dev", "late-replay", "replay-timed-out", 1, 0, 2)
    terminal("terminal-unserved", "dev", "ignore-replay", "replay-not-accepted", 2, 1)

    def run(profile="dev", oracle=True, framed=True):
        options = (["--oracle"] if oracle else []) + (["--framed"] if framed else [])
        out = call(project, store, "leased-build", profile, *options)
        assert out["result"]["framed"] == framed
        answer = response(out)
        assert bytes(answer["source_root"]).hex() == out["selected"]
        assert answer["phase"] == "Analyzed" and answer["version"] == 1 and answer["profile"] == profile
        return answer, out["result"]["dispatch_ms"]

    def ordinary_report(profile):
        proc = subprocess.run([ordinary_cli, "build", "app", "--profile", profile],
                              cwd=project, capture_output=True, check=False, timeout=30)
        tail = b"wosy: build rejected by semantic diagnostics\n"
        assert proc.returncode == 1 and not proc.stdout and proc.stderr.endswith(tail), proc.stderr
        return proc.stderr[:-len(tail)].decode()

    def client_report(profile):
        return call(project, store, "leased-build-report", profile)["result"]

    orig, _ = run()
    native, _ = run("native")
    assert orig["origin"] == native["origin"] == "Derived"
    assert orig["receipt"] != native["receipt"]
    assert orig["diagnostics"] == native["diagnostics"]
    expected_print = {}
    if ordinary_cli:
        for profile in ("dev", "native"):
            expected_print[profile] = ordinary_report(profile)
            report = client_report(profile)
            assert report["response"]["profile"] == profile
            assert report["rendered"] == expected_print[profile] and report["rendered"]
        limited = subprocess.run([example, project, store, "--persistent-source", "app",
                                  "leased-build-report-limited", "dev"], capture_output=True, timeout=30)
        assert limited.returncode and not limited.stdout and b"authored byte budget" in limited.stderr

    def sqlite_bytes():
        return [(project / name).read_bytes() for name in (".wosy/client.sqlite", ".wosy/cst/nodes.sqlite")]

    before_live = sqlite_bytes()
    live_timings = {}
    for profile in ("dev", "native"):
        live_name = f"live-{profile}"
        live_first, first_ms = terminal(live_name, profile, "live-report", "report", 2, 1)
        live_warm, warm_ms = terminal(live_name, profile, "live-report", "report", 1, 0)
        assert live_first["rendered"] == live_warm["rendered"]
        assert live_first["response"]["diagnostics"] == live_warm["response"]["diagnostics"] == orig["diagnostics"]
        if ordinary_cli:
            assert live_first["rendered"] == expected_print[profile]
        live_timings[profile] = {"first_ms": round(first_ms, 3), "warm_ms": round(warm_ms, 3)}
    terminal("live-local-expired", "dev", "live-expired", "expired", 0, 0)
    terminal("live-missing-profile", "unselected", "live-missing-profile", "missing-profile", 0, 0)
    for limits, reason in ((["1", str(16 << 20)], "record-budget"), (["25000", "1"], "byte-budget")):
        result = call(project, store.with_name(f"live-{reason}"), "reconcile-terminal-report-limited",
                      *limits, "dev", f"live-{reason}")["result"]
        assert result["answer"] == {"kind": reason} and result["adapter_calls"] == result["streams_opened"] == 0
    # The actual file cap is independent from the SQLite index limit.
    # Exact aggregate succeeds, one byte less refuses across multiple files.
    sizes = [path.stat().st_size for path in project.rglob("*.w")]
    authored = sum(sizes)
    assert len(sizes) == 19 and max(sizes) < authored - 1
    exact, _ = terminal("live-dev", "dev", "live-report", "report", 1, 0, None, str(authored))
    assert exact["response"]["source_root"] == orig["source_root"]
    terminal("live-authored-total", "dev", "live-authored-budget", "authored-budget", 0, 0, None, str(authored - 1))
    # A larger new leaf cannot be observed whole even though the selected
    # SQLite index remains valid and ordinarily admitted.
    provider.write_text("not canonical base source\n" * 512)
    assert provider.stat().st_size > authored
    terminal("live-authored-large", "dev", "live-authored-budget", "authored-budget", 0, 0, None, str(authored))
    if len(sys.argv) > 3:
        import os
        passed = subprocess.run([sys.argv[3], "live_source_deadline_stops_after_one_completed_file_read",
                                 "--ignored", "--nocapture"], capture_output=True, text=True, check=True,
                                timeout=30, env={**os.environ, "WOSY_LIVE_DEADLINE_SOURCE": str(provider)})
        assert "1 passed" in passed.stdout, passed.stdout + passed.stderr
    # No parse can succeed on this new leaf, but the shared probe refuses
    # by new observed identity before attempting to parse it.
    provider.write_text("not canonical base source\n")
    terminal("live-edited-invalid", "dev", "live-changed", "changed", 0, 0)
    provider.write_text(old_text)
    config = project / "wosy.toml"
    config.write_text(manifest.replace('toolchain_identity = "llvm-18.native"', 'toolchain_identity = "new-fixture-builder"'))
    terminal("live-config", "dev", "live-changed", "changed", 0, 0)
    config.write_text(manifest)
    assert sqlite_bytes() == before_live

    direct, _ = run(framed=False)
    assert direct["origin"] == "Stored"
    assert {k: v for k, v in direct.items() if k != "origin"} == {k: v for k, v in orig.items() if k != "origin"}
    only_blob, = (store / "cas").iterdir()
    assert only_blob.stat().st_size > 64 * 1024
    digest = hashlib.sha256(only_blob.read_bytes()).digest()
    # Lost recomputable fixture CAS: the next real leased Build restores the
    # same artifact bytes and keeps the stored canonical diagnostics.
    only_blob.unlink()
    repaired, _ = run()
    assert repaired["origin"] == "Stored" and repaired["diagnostics"] == orig["diagnostics"]
    assert hashlib.sha256(only_blob.read_bytes()).digest() == digest
    old_labels = orig["diagnostics"][0]["labels"]
    assert [label["kind"] for label in old_labels] == ["Primary", "Secondary"]
    for label in old_labels:
        span = label["span"]
        assert span["source"]["path"] == "src/item.w"
        assert old_text.encode()[span["range"]["start"]:span["range"]["end"]] == b"next"

    new_text = script("origem atualizada, texto Unicode maior 🚀", "next; next")
    provider.write_text(new_text)
    staged = call(project, store, "stage")
    pending_bytes = sqlite_bytes()
    terminal("live-pending", "dev", "live-changed", "changed", 0, 0)
    provider.write_text(old_text)
    live_pending, _ = terminal("live-dev", "dev", "live-report", "report", 1, 0)
    assert live_pending["response"]["source_root"] == orig["source_root"]
    provider.write_text(new_text)
    assert sqlite_bytes() == pending_bytes
    still_old, _ = run(oracle=False)
    assert still_old["origin"] == "Stored" and still_old["diagnostics"] == orig["diagnostics"]
    assert still_old["source_root"] == orig["source_root"] and staged["pending"] != staged["selected"]
    terminal_timings = {}
    for profile in ("dev", "native"):
        name = f"terminal-{profile}"
        cold, cold_ms = terminal(name, profile, "report", "report", 2, 1)
        hot, hot_ms = terminal(name, profile, "report", "report", 1, 0)
        assert cold["response"]["source_root"] == hot["response"]["source_root"] == orig["source_root"]
        assert cold["response"]["profile"] == hot["response"]["profile"] == profile
        assert cold["response"]["diagnostics"] == hot["response"]["diagnostics"] == orig["diagnostics"]
        assert cold["rendered"] and cold["rendered"] == hot["rendered"]
        if ordinary_cli:
            assert cold["rendered"] == expected_print[profile]
        terminal_timings[profile] = {"cold_ms": round(cold_ms, 3), "warm_ms": round(hot_ms, 3)}
    latest_stage = call(project, store, "state")
    assert (latest_stage["selected"], latest_stage["pending"], latest_stage["attempt"]) == (
        staged["selected"], staged["pending"], staged["attempt"])
    if ordinary_cli:
        for profile in ("dev", "native"):
            report = client_report(profile)
            assert report["response"]["source_root"] == orig["source_root"]
            assert report["rendered"] == expected_print[profile]
    updated = call(project, store, "submit-pending", "app", "src/main.w")
    assert updated["selected"] == staged["pending"]
    remapped, warm_ms = run()
    assert remapped["origin"] == "Stored" and remapped["receipt"] == orig["receipt"]
    assert remapped["source_root"] != orig["source_root"]
    for label, previous in zip(remapped["diagnostics"][0]["labels"], old_labels):
        span = label["span"]
        assert span["source"] != previous["span"]["source"]
        assert new_text.encode()[span["range"]["start"]:span["range"]["end"]] == b"next"
    live_updated, _ = terminal("live-dev", "dev", "live-report", "report", 2, 1)
    assert live_updated["response"]["source_root"] == remapped["source_root"]
    if ordinary_cli:
        assert live_updated["rendered"] == ordinary_report("dev")
    provider.write_text(script("origem atualizada, texto Unicode maior 🚀", "next"))
    call(project, store, "stage")
    call(project, store, "submit-pending", "app", "src/main.w")
    fixed, _ = run()
    assert fixed["origin"] == "Derived" and not fixed["diagnostics"] and fixed["receipt"] != orig["receipt"]
    if ordinary_cli:
        assert client_report("dev")["rendered"] == ""
    terminal("terminal-stale", "dev", "stale", "stale", 1, 0)
    terminal("live-stale", "dev", "live-stale", "stale", 1, 0)
    after_stale = call(project, store, "state")
    assert after_stale["selected"] == bytes(fixed["source_root"]).hex() and after_stale["pending"] is None
    assert not (project / ".wosy/artifacts").exists()
    print("acknowledged SQLite root → real framed in-process leased worker → ordinary compiler diagnostics", {
        "warm_dispatch_ms": round(warm_ms, 3), "resolved_profiles": ["dev", "native"],
        "historic_UTF8": True, "newer_pending_not_selected": True, "cas_rebuilt": True,
        "rooted_files": 19, "cas_bytes": only_blob.stat().st_size,
        "ordinary_renderer_bytes_match_both_profiles": bool(ordinary_cli),
        "terminal_client_operations_ms": terminal_timings,
        "terminal_one_deadline_before_recovery_stream": True,
        "terminal_same_root_generation_rejected": True,
        "live_client_operations_ms": live_timings,
        "live_refuses_dependency_profile_config_and_distinct_file_or_index_limits_before_open": True,
        "live_exact_aggregate_authored_bytes": authored,
        "one_actual_read_completes_at_injected_deadline_cut": len(sys.argv) > 3,
        "live_does_not_mutate_selection_or_emit_artifacts": True,
    })

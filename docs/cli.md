# CLI reference

The binary is named `wosy`. The target argument defaults to `[package] default_target`. Profiles default to `dev`.

## `wosy parse [TARGET]`

Parses only the target root file. Writes `.wosy/cst/<blake3>.json` and `.wosy/cst/current.json`, prints diagnostics to stderr, and exits 1 when diagnostics are present. Recovery preserves neighboring declarations in the snapshot, but the command still fails.

## `wosy build [TARGET] --profile dev`

Compiles the full reachable source graph:

1. Parses every reachable file and rejects syntax diagnostics.
2. Derives the scalar program and rejects derivation diagnostics.
3. Validates names, types, places, calls, unsafe contexts, and initialization order.
4. Emits `partition.ll` and the backend-matched `runtime.ll`.
5. Builds `artifact-manifest.pending.json`, invokes the configured builder as `<command...> <configured_args...> <partition.ll> <output_dir> <pending_manifest> <runtime.ll>`, then publishes `artifact-manifest.json`.

Fails with `build rejected by syntax diagnostics`, `build rejected by CST derivation diagnostics`, or `build rejected by semantic diagnostics`. Missing imports fail as `M0001: missing module <package>::<path>`; cycles fail as `M0003: import cycle`.

## `wosy check [TARGET] --profile dev [--main-flow-only]`

Checks the selected main flow and checks unused functions symbolically in the
current package. The default scope needs a declared `[package].source_root`;
`--main-flow-only` selects only the reachable main flow and does not require
that field. `--declared-package` explicitly selects the default scope and
cannot be combined with `--main-flow-only`.

Check does not build, run, or generate an artifact. It reports refuted proof
obligations and publishes supported authored `core.require` and `core.narrow`
outputs. The first nonzero authored status takes precedence; otherwise a
refuted proof exits 1 and a successful check exits 0. An invalid or unsupported
analysis or rejected output preflight exits 2 without writing authored stdout
or stderr or publishing any requested file. The checker does not yet support
all language and proof constructs.

In the supported scalar calling proofs, a named `core.use` on a reached
startup call checks the cited precondition at that call's actual entry and
the cited proof against that same return. It does not evaluate a second call
or substitute a symbolic summary for the reached result. Unused functions
are checked only in the declared-package scope.

A bare literal comparison can also target a qualified, pure unary integer
function in the same package. The checker observes direct startup calls after
the imported module has initialized, or an unused symbolic root in the
declared-package scope. This profile does not support nested calls through the
qualified target or publish an ordinary call summary for it.

Check diagnostics and stream output work on native CLI hosts. Authored file
output currently requires a Linux CLI host and a flat filename under
`.wosy/outputs/check/`; other file requests are refused before authored output
is published. This is independent of the selected artifact's target platform.
File preparation can create empty output directories even if it fails. After
preparation succeeds, records are published in order; a later I/O error can
leave earlier records published.

## `wosy run [TARGET] --profile dev [-- ARGS...]`

Loads `.wosy/artifacts/<target>/<profile>/<main_artifact>/artifact-manifest.json`, rehashes every observed source and `wosy.toml`, and compares source identity, configuration identity, artifact identity, backend, output, builder identity, runner identity, compiler identity, LLVM identity, and builder toolchain identity. Any mismatch fails with `artifact manifest is stale`. A missing executable fails with `artifact is missing: ...`. Otherwise it invokes `<runner_command...> <runner_args...> <manifest_path> -- <ARGS...>` and returns the child exit code as a byte.

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

## `wosy check`

Analyze the selected target without building or running it.

- **Default (`--declared-package`):** Check the main flow and symbolically check
  unused functions in the current package. Set `[package].source_root` to a
  package-relative source directory containing the target entry and library
  sources. Use real directories and regular `.w` files, not symlinks. The
  scan skips nested packages; unimported module initializers do not run.
- **`--main-flow-only`:** Check only the reachable main flow. This scope does
  not require `source_root` and cannot be combined with `--declared-package`.

On completion, the first nonzero code from a failing `core.require` or
`core.narrow` sets the exit status. With no such code, a refuted proof exits 1;
otherwise the check exits 0. Invalid input, unsupported analysis, or output
errors exit 2. The selected authored code must fit in `0..=255`.

Directives can write to stdout, stderr, or files. File output requires a
Linux host, regardless of target. Request distinct unused filenames
directly under `.wosy/outputs/check/`.
No authored output is published before preparation succeeds, though failed
preparation can leave empty directories. Output is then published in order;
a later I/O failure can leave earlier output in place.

## `wosy run [TARGET] --profile dev [-- ARGS...]`

Loads `.wosy/artifacts/<target>/<profile>/<main_artifact>/artifact-manifest.json`, rehashes every observed source and `wosy.toml`, and compares source identity, configuration identity, artifact identity, backend, output, builder identity, runner identity, compiler identity, LLVM identity, and builder toolchain identity. Any mismatch fails with `artifact manifest is stale`. A missing executable fails with `artifact is missing: ...`. Otherwise it invokes `<runner_command...> <runner_args...> <manifest_path> -- <ARGS...>` and returns the child exit code as a byte.

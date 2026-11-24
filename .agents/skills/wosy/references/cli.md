# CLI reference

The binary is named `wosy`. The target argument defaults to `[package] default_target`. Profiles default to `dev`.

## `wosy init`

Creates a base-language package in the current directory. Requires Linux 5.6+, `/proc`, and a directory name matching `[a-z][a-z0-9_]*`. Existing scaffold paths block initialization; edits are preserved. The generated profiles are `dev` (native) and `wasm` (WASI Preview 1).

## `wosy dep add SOURCE [--path PACKAGE_PATH]`

Adds a local path or Git origin. `--path` selects a directory or `wosy.toml` within the Git repository. Keeps existing pins and locks newly reached Git packages. Run `wosy dep sync` to install them.

## `wosy dep sync`

Installs the locked Git revisions with the project's patches. Keeps the lock and local packages unchanged. Refuses to overwrite unsnapshotted edits. A verified checkout works offline.

## `wosy dep snapshot`

Saves edits to a Git dependency as project patches. Keeps the checkout and lock unchanged. Sync a changed patch stack before taking another snapshot.

## `wosy dep update`

Advances Git pins by fast-forward and reapplies patches. Conflicts or unsnapshotted edits block replacement. Removes clean checkouts no longer in the graph and keeps their patches.

Resolving or installing Git dependencies requires Linux 5.6+, `/proc`, and Git. Local-only add needs Unix but not Git.

## `wosy parse [TARGET]`

Parses only the target root file, without loading its dependency graph. Writes `.wosy/cst/<blake3>.json` and `.wosy/cst/current.json`, prints diagnostics to stderr, and exits 1 when diagnostics are present. Recovery preserves neighboring declarations in the snapshot, but the command still fails.

## `wosy build [TARGET] --profile dev`

Compiles the full reachable source graph:

1. Parses every reachable file and rejects syntax diagnostics.
2. Derives the scalar program and rejects derivation diagnostics.
3. Validates names, types, places, calls, unsafe contexts, and initialization order.
4. Emits `partition.ll` and the backend-matched `runtime.ll`.
5. Builds `artifact-manifest.pending.json`, invokes the configured builder as `<command...> <configured_args...> <partition.ll> <output_dir> <pending_manifest> <runtime.ll>`, then publishes `artifact-manifest.json`.

Fails with `build rejected by syntax diagnostics`, `build rejected by CST derivation diagnostics`, or `build rejected by semantic diagnostics`. Missing imports fail as `M0001: missing module <package>::<path>`; cycles fail as `M0003: import cycle`.

## `wosy run [TARGET] --profile dev [-- ARGS...]`

Loads `.wosy/artifacts/<target>/<profile>/<main_artifact>/artifact-manifest.json`, rehashes every observed source and `wosy.toml`, and compares source identity, configuration identity, artifact identity, backend, output, builder identity, runner identity, compiler identity, LLVM identity, and builder toolchain identity. Any mismatch fails with `artifact manifest is stale`. A missing executable fails with `artifact is missing: ...`. Otherwise it invokes `<runner_command...> <runner_args...> <manifest_path> -- <ARGS...>` and returns the child exit code as a byte.

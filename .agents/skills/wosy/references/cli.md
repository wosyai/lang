# CLI reference

The binary is named `wosy`. The target argument defaults to `[package] default_target`. Profiles default to `dev`.

## `wosy init`

Creates a base-language package in the current directory. Requires Linux 5.6+, `/proc`, and a directory name matching `[a-z][a-z0-9_]*`. Existing scaffold paths block initialization; edits are preserved. The generated profiles are `dev` (native) and `wasm` (WASI Preview 1).

## `wosy dep add SOURCE [--path PACKAGE_PATH]`

Adds a local path or Git origin. `--path` selects a directory or `wosy.toml` within the Git repository. Keeps existing pins and locks newly reached Git packages. Run `wosy dep sync` to install them.

## `wosy dep sync`

Installs the locked Git revisions with the project's patches. Keeps the lock and local packages unchanged. Refuses to overwrite unsnapshotted edits. A verified checkout works offline.

## `wosy dep snapshot`

From the consuming project, run `wosy dep sync` to install the locked Git dependencies. Edit the source in `deps/<package>/`, then run `wosy dep snapshot`. Commit the generated files under `patches/`; do not write or name them by hand. The snapshot keeps the checkout and lock unchanged. If the patch stack changes separately, run `wosy dep sync` before taking another snapshot.

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

## `wosy check`

Use `wosy check [TARGET] [--profile PROFILE]` to analyze the selected target
without building or running it.

- **Default (`--declared-package`):** Check the main flow and symbolically check
  unused functions in the current package. Set `[package].source_root` to a
  package-relative source directory containing the target entry and library
  sources. Use real directories and regular `.w` files, not symlinks. The
  scan skips nested packages; unimported module initializers do not run.
- **`--main-flow-only`:** Check only the reachable main flow. This scope does
  not require `source_root` and cannot be combined with `--declared-package`.
- **`--dependencies`:** Also check every resolved dependency, including
  transitive dependencies. Each uses its own primary target, default profile,
  and declared-package scope, even when the root uses `--main-flow-only`.
  Install locked Git dependencies with `wosy dep sync` first. Check does not
  fetch or install dependencies.

For a named proof, declare the function and its proof in a `.w` source:

```wosy
%%start
u8(u8) echo = fn(input) { input };
proof preserves_input for echo(x) -> (out) ensures (out == core.old(x)) {}
%%end
```

Run the default package check to verify it even if `echo` is unused.
Check verifies applicable proofs; a successful Build does not prove them.

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

Loads `.wosy/artifacts/<target>/<profile>/<main_artifact>/artifact-manifest.json`, rehashes every observed source and `wosy.toml`, and compares source identity, configuration identity, artifact identity, backend, output, builder identity, runner identity, compiler identity, LLVM identity, and builder toolchain identity. Any mismatch fails with `artifact manifest is stale`. A missing executable fails with `artifact is missing: ...`. Otherwise it invokes `<runner_command...> <runner_args...> <manifest_path> -- <ARGS...>` and returns the child exit code as a byte. The generated shell runners use `WOSY_ARTIFACT_EXECUTABLE`, set to the executable path from the verified manifest.

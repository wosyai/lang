# CLI reference

The binary is named `wosy`. The target argument defaults to `[package] default_target`. Profiles default to `dev`.

## Package command platforms

Linux is the validated platform for package commands. Resolving or
materializing Git packages requires Linux kernel 5.6 or newer, `/proc`,
and Git on `PATH`. A local-only `dep add` does not use Git, but requires
the Unix directory lock used for project metadata writes and is unavailable
on non-Unix platforms. Reading local dependencies does not require Git or
create package metadata. With no Git dependencies, `dep sync` only reads local
declarations and does not write a manifest, lockfile, or checkout.

## `wosy init`

Creates a base-language package in the current directory on Linux (kernel 5.6
or newer and `/proc` available). The directory name must start with a
lowercase ASCII letter and contain only lowercase ASCII letters, digits or
underscores. All scaffold paths must be absent: `src/`, `scripts/`,
`wosy.toml`, `wosy.lock`, `wosy.meta.json`, and `.gitignore`. Init does
not overwrite files or links, invoke Git, build the program, or add
dependencies. Other files in the directory are not modified. If publication
fails after some files were created, unmodified scaffold entries are moved to a
recovery directory shown in the diagnostic. External replacements and edits are
not discarded.

The scaffold has a starter `src/main.w`, an empty version 1 dependency lock,
empty mechanical metadata, and project-local build and run scripts. It selects
the native `dev` profile by default and provides a `wasm` profile for WASI
Preview 1. Building needs Python 3 and Clang 18.1.x on `PATH`; the Wasm profile
also needs `wasm-ld` from LLVM and Wasmtime to run. This base-language scaffold
does not generate compiler references or configure high-language tooling.

## `wosy dep add SOURCE [--path PACKAGE_PATH]`

Adds a local project-relative package path or a Git origin, with an optional
repository-relative package directory or `wosy.toml` path. Existing pins are
preserved; newly reached Git packages are resolved to exact commits. The
command validates the complete graph and updates the project manifest and lock
without installing checkouts. Run `wosy dep sync` to install them.

## `wosy dep sync`

Materializes the exact Git revisions already recorded in
[`wosy.lock`](project.md#wosylock). Run it after cloning a project with Git
dependencies. It never selects a revision or rewrites the lockfile. Local
dependencies stay where they are. Sync reads their declarations to find
transitive Git pins but does not load or check local source files. A verified,
unmodified dependency works offline. Sync applies the project's patches in their
recorded order; if there are unsnapshotted edits, it fails without overwriting
them.

## `wosy dep snapshot`

Records unsnapshotted edits in reached Git dependencies as ordered project
patches. It does not replace live source files or write the lock. A changed
authored patch stack must be synced first. Snapshot may need to retrieve the
exact locked Git revision to reconstruct the patch baseline.

## `wosy dep update`

Advances reached Git dependencies to same-origin fast-forward revisions and
replays their project patches. It will not follow a rewritten or unrelated tip,
overwrite unsnapshotted edits, or publish a lock for a patch conflict. An update
can resolve graph changes authored in dependency manifests. It removes clean,
verified checkouts no longer needed, without altering project-owned patches.

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

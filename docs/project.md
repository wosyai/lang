# Project layout

The project root is the nearest ancestor directory containing `wosy.toml`.

## `wosy.toml`

```toml
[package]
name = "is_even"
default_target = "app"

[dependencies.std]
path = "stdlib"

[comments]
categories = []

[builders.wasm]
command = ["python", "scripts/build-wasm"]
args = []
toolchain_identity = "llvm-18.1.4"

[runners.wasm]
command = ["python", "scripts/run-wasmtime"]
args = []

[targets.app]
root = "src/main.w"

[targets.app.artifacts.main]
artifact = "Main"

[targets.app.artifacts.main.profiles.dev]
backend = "llvm"
output = "wasm-wasip1"
builder = "wasm"
run_runner = "wasm"

[targets.app.profiles.dev]
main_artifact = "main"
```

Rules:

- `[package] name` and `default_target` are required for a buildable project.
  `source_root` is a package-relative directory used by namespace paths.
  For older root manifests without it, self-qualified namespace paths start
  at the project root. Target entry paths are always relative to the project
  root, regardless of `source_root`.
- A local `[dependencies.<name>]` table contains only `path`, normally relative to the project root. Local packages stay in place. For Git dependencies, the table instead contains `git` (the repository origin) and optionally `path` (a directory within that repository, defaulting to `"."`). Both sources must supply a `wosy.toml` with a matching `package.name` and a package-relative `source_root` directory.
- `[comments] categories` lists the `# CATEGORY:` comment categories accepted by `wosy parse` and `wosy build`.
- Each builder needs `command` (argv, first element is the executable), optional `args`, and `toolchain_identity`. The same applies to runners.
- Each target needs `root` (entry file relative to the project root), at least one artifact, one artifact profile, and one target profile naming its `main_artifact`.
- Supported backend/output pairs are `llvm/native`, `llvm/wasm-wasip1`, `llvm/wasm-wasip2`, and `llvm/wasm-web`.
- Select `output = "wasm-web"` for a browser-hosted artifact. Its builder should compile for `wasm32-unknown-unknown`. The runtime terminates on system panic with a Wasm trap and adds no WASI imports. The application can still declare host imports explicitly.

An LLVM Wasm artifact profile can opt into stable root-function labels for its builder:

```toml
[targets.app.artifacts.main.profiles.dev.exports]
demo_score = "score"
```

The optional table maps portable external labels to concrete functions in the selected target's entry file. It supports direct `i32`, `u32`, `i64`, `u64`, `f32`, and `f64` parameters and one such result or `unit`. The builder still selects labels to export (for example, `wasm-ld --export=demo_score`); its arguments do not change.

Wasm builders must link `runtime.ll` with a linker that provides `__heap_base` after static storage and the reserved stack, as `wasm-ld` does. The generated LLVM IR enables Wasm bulk memory for functions that copy fixed-size aggregates. This per-function setting overrides Clang’s `-mno-bulk-memory` for those functions. Use a Wasm runner that supports `memory.copy`.

The standard library itself is a dependency-style package (`stdlib/wosy.toml`):

```toml
[package]
name = "std"
source_root = "src"
```

## `wosy.lock`

Commit `wosy.lock` with the project when it has Git dependencies and ignore
`deps/`. Only direct declarations belong in the consuming `wosy.toml`:

```toml
[dependencies.math]
git = "https://example.org/math.git"
path = "packages/math"
```

The lock includes all reached Git packages, including transitive packages.
Each `[packages.<name>]` entry under `version = 1` records the origin (`git`),
package directory (`path`, `"."` for a repository root), exact Git `rev`, and
`source_hash` of the original tracked package bytes. Relative Git origins
are interpreted from the consuming project root. In a Git package, a `path`-only
declaration is relative to that package's directory in its Git repository. It
selects the same origin and exact revision as the declaring package, with its
own lock entry and original source hash. For example, `../support` in
`packages/math` selects `packages/support` in that commit. Paths that leave the
repository are rejected; these declarations never fall back to host paths.
An explicit Git declaration names its origin and repository path as usual.
A local manifest's `path` declarations are relative to that local package;
local packages stay in place and are not in the lock. In a wholly local graph,
an unrelated file that is not a managed Wosy lock is left untouched. Reaching
a Git dependency requires a valid lock, except when adding a new Git pin.
A valid Wosy lock must not contain unreachable pins.

The dependency graph allows one binding per package name. Multiple paths to
the same binding share one package; conflicting bindings, cycles, missing
pins, and pins outside the reached Git graph are errors. There is no automatic
version fallback. Builds and runs use the locked graph and the effective
source roots, and never fetch or resolve versions. Sources stay editable;
changing a reachable file or its routing makes an existing artifact stale.
`wosy parse` reads only the selected root source and does not need the package
graph installed.

Use `wosy dep add <local-path>` or
`wosy dep add <git-origin> --path <package-path>` to add a direct dependency.
For Git, the package path may name a directory or the `wosy.toml` inside it.
Add preserves existing pins and records the exact selected Git commit for
each newly reached package. It updates the manifest and lock after verifying
the complete graph; it does not publish a checkout. Run `wosy dep sync` to
materialize the locked packages. See the [package command platforms](cli.md#package-command-platforms) for the write and Git requirements.

Sync validates the exact original bytes of a staged package before installing
anything from the graph. The version 1 source hash is BLAKE3 of
`wosy-package-source-v1` followed by a zero byte, then, for each tracked
regular file in sorted UTF-8 path order: its package-relative path length in
bytes (unsigned 64-bit little endian), its path bytes with `/` separators,
and the 32 raw bytes of its file-content BLAKE3 hash. Symlinks and submodules are not supported inside Git checkouts.
Sync does not overwrite an edited or unverified directory. Moving the same
project with its checkouts preserves their provenance; copying them to a
different project does not.

## `wosy.meta.json`

The package owns this file for mechanical metadata such as refactor mappings.
`wosy init` starts it as an empty JSON object. The base-language build does
not need metadata entries.

## Dependency patches

`wosy dep snapshot` records edits to reached Git dependencies, including
transitive packages, in `patches/<package>/<uuid-v7>.patch`. The canonical
filename uses lowercase hyphenated UUIDv7. Other files in that directory are
not patch inputs. Each patch is a standard Git binary diff; renames, binary
changes and executable modes are preserved. Snapshot leaves the checkout and
lock untouched and writes no new patch when there is no new edit. Reconstructing
an old baseline can require access to the exact locked revision. A manifest
edit can be captured before its new dependency graph has a lock. Run
`wosy dep update` to resolve newly reached or removed dependencies when ready.

`wosy dep sync` applies the selected patch stack in UUID order. Changing a
patch invalidates the previously applied stack until sync reconciles it.
The original Git content hash remains unchanged by patches. Build and run
continue to observe editable source bytes, and the applied patch stack is
also part of their input identity. Local dependencies are not patch inputs.

`wosy dep update` selects same-origin fast-forward revisions and reapplies the
same project-owned patch files. A divergent tip, unappliable patch or edited
checkout that would be replaced leaves the old lock and worktree in place.
A successful update removes verified, clean checkouts no longer in the
resolved graph. It does not delete their authored patches; adding a package back
reapplies its existing patches. An edited or unverified checkout blocks removal.

## Source graph

The entry file plus every file named by a `namespace` declaration forms the source graph. A declaration such as `math = namespace app "src/math.w";` loads `src/math.w` from the `app` package. A declaration such as `std = namespace std "bootstrap.w";` resolves `bootstrap.w` relative to the `std` package `source_root`. Missing files fail with `M0001`; cycles fail with `M0003`.

## Build outputs

`wosy build` writes under `.wosy/`:

- `.wosy/cst/<blake3>.json` and `.wosy/cst/current.json`: parsed snapshots of each compiled source.
- `.wosy/artifacts/<target>/<profile>/<artifact>/partition.ll`: emitted LLVM for the program.
- `.wosy/artifacts/<target>/<profile>/<artifact>/runtime.ll`: core runtime for the selected backend and output.
- `.wosy/artifacts/<target>/<profile>/<artifact>/artifact-manifest.json`: identities for sources, configuration, LLVM input, compiler, LLVM library, and builder toolchain, plus the list of source observations and the executable path.

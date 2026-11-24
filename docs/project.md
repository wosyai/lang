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

- `[package] name` and `default_target` are required.
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

Commit `wosy.lock` with the project when it has Git dependencies, and ignore
`deps/`. Declare only the Git origin and package directory in `wosy.toml`:

```toml
[dependencies.math]
git = "https://example.org/math.git"
path = "packages/math"
```

The corresponding `wosy.lock` entry is `[packages.math]` under
`version = 1`. It records `git`, `path` (explicitly `"."` for the repository
root), `rev` (a full lowercase Git commit ID), and `source_hash` (the lowercase
hex BLAKE3 hash of the original package content). The version 1 hash is
BLAKE3 of the domain prefix `wosy-package-source-v1` followed by a zero byte,
then, for each tracked regular file in package-relative UTF-8 path order: its
path length in bytes (unsigned 64-bit little endian), its path bytes with
`/` separators, and the 32 raw bytes of its file-content BLAKE3 hash.

`wosy dep sync` requires those pins to be present. It checks the exact revision,
package identity and original source hash before placing files at
`deps/<name>/`, and keeps verification metadata separately under
`deps/.wosy-pins/`. Each receipt is bound to that project, pin and checkout
directory identity. It is not reusable by copying or moving a checkout to
another project. Sync does not copy sibling packages from a repository.
Normal builds use the current working source, including edits, but do not
follow symlinks in Git dependency source paths. Changing a Git package's
effective `source_root` invalidates earlier artifacts, even if the old files
still exist. Sync does not
overwrite an edited or unverified directory; move it aside before recreating
the pinned tree. The initial command supports direct Git packages only, with
regular tracked files; it rejects submodules, symlinks, transitive Git package
declarations and project patches for Git dependencies. Pin creation, updates
and patch replay are not implemented in this version.

## Source graph

The entry file plus every file named by a `namespace` declaration forms the source graph. A declaration such as `math = namespace app "src/math.w";` loads `src/math.w` from the `app` package. A declaration such as `std = namespace std "bootstrap.w";` resolves `bootstrap.w` relative to the `std` package `source_root`. Missing files fail with `M0001`; cycles fail with `M0003`.

## Build outputs

`wosy build` writes under `.wosy/`:

- `.wosy/cst/<blake3>.json` and `.wosy/cst/current.json`: parsed snapshots of each compiled source.
- `.wosy/artifacts/<target>/<profile>/<artifact>/partition.ll`: emitted LLVM for the program.
- `.wosy/artifacts/<target>/<profile>/<artifact>/runtime.ll`: core runtime for the selected backend and output.
- `.wosy/artifacts/<target>/<profile>/<artifact>/artifact-manifest.json`: identities for sources, configuration, LLVM input, compiler, LLVM library, and builder toolchain, plus the list of source observations and the executable path.

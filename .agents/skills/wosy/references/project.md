# Project layout

The project root is the nearest ancestor directory containing `wosy.toml`.

## `wosy.toml`

```toml
[package]
name = "is_even"
default_target = "app"
source_root = "src"

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

- `[package]` needs `name` and `default_target`. `source_root` sets the base for namespace paths (the project root when omitted from the root manifest). Target entries stay relative to the project root.
- `[dependencies.<name>]` uses local `path`, or `git` with an optional repository-relative `path` (default `"."`). A dependency manifest needs the matching package name and a `source_root`.
- `[comments] categories` lists the `# CATEGORY:` comment categories accepted by `wosy parse` and `wosy build`. In a new project, add `TYPE`, `REPLAY`, or `NOISE` explicitly if you need them.
- Each builder needs `command` (argv, first element is the executable), optional `args`, and `toolchain_identity`. The same applies to runners.
- Each target needs `root` (entry file relative to the project root), at least one artifact, one artifact profile, and one target profile naming its `main_artifact`.
- Supported backend/output pairs are `llvm/native`, `llvm/wasm-wasip1`, `llvm/wasm-wasip2`, and `llvm/wasm-web`.

## WebAssembly artifacts

For browser hosting, set `output = "wasm-web"` and target
`wasm32-unknown-unknown` in the builder. The runtime adds no WASI imports and
traps on system panic. Declare any host imports in the source.

Wasm builders link `runtime.ll` and need a linker that defines `__heap_base`
after static storage and the reserved stack, such as `wasm-ld`. Fixed-size
aggregate copies enable `memory.copy` per function, even with
`-mno-bulk-memory`. Use a runner with Wasm bulk-memory support.

To export a function from the target's entry file:

```toml
[targets.app.artifacts.main.profiles.dev.exports]
demo_score = "score"
```

Each label names a concrete function with `i32`, `u32`, `i64`, `u64`, `f32`,
or `f64` parameters and one such result or `unit`. The builder must still
export the label, for example with `wasm-ld --export=demo_score`.

## `wosy.lock`

Commit the lock; ignore managed `deps/` checkouts. Version 1 records the origin, package path, exact commit, and original source hash of each reachable Git package. Local packages are not locked. Build and run use the installed graph without fetching.

A path-only dependency inside a Git package selects a sibling in the same repository and commit. For example, `../support` from `packages/math` selects `packages/support`. Paths cannot leave the repository.

## Dependency patches

Store patches in `patches/<package>/<uuid-v7>.patch`. Wosy applies them in UUID order. Patches are Git binary diffs and do not change the original source hash.

## `wosy.meta.json`

Package-owned tool metadata. Base-language builds need no entries.

## Source graph

The entry file plus every file named by a `namespace` declaration forms the source graph. A declaration such as `math = namespace is_even "math.w";` loads `src/math.w` when `is_even` uses `source_root = "src"`. A declaration such as `std = namespace std "bootstrap.w";` resolves `bootstrap.w` relative to the `std` package `source_root`. Missing files fail with `M0001`; cycles fail with `M0003`.

## Build outputs

`wosy build` writes under `.wosy/`:

- `.wosy/cst/<blake3>.json` and `.wosy/cst/current.json`: parsed snapshots of each compiled source.
- `.wosy/artifacts/<target>/<profile>/<artifact>/partition.ll`: emitted LLVM for the program.
- `.wosy/artifacts/<target>/<profile>/<artifact>/runtime.ll`: core runtime for the selected backend and output.
- `.wosy/artifacts/<target>/<profile>/<artifact>/artifact-manifest.json`: identities for sources, configuration, LLVM input, compiler, LLVM library, and builder toolchain, plus the list of source observations and the executable path.

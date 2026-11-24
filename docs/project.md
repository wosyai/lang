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
- Each `[dependencies.<name>]` table takes exactly one field: `path`, a non-empty relative path to the dependency root. The directory must contain its own `wosy.toml` with a matching `package.name` and a `source_root` directory.
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

## Source graph

The entry file plus every file named by a `namespace` declaration forms the source graph. A declaration such as `math = namespace app "src/math.w";` loads `src/math.w` from the `app` package. A declaration such as `std = namespace std "bootstrap.w";` resolves `bootstrap.w` relative to the `std` package `source_root`. Missing files fail with `M0001`; cycles fail with `M0003`.

## Build outputs

`wosy build` writes under `.wosy/`:

- `.wosy/cst/<blake3>.json` and `.wosy/cst/current.json`: parsed snapshots of each compiled source.
- `.wosy/artifacts/<target>/<profile>/<artifact>/partition.ll`: emitted LLVM for the program.
- `.wosy/artifacts/<target>/<profile>/<artifact>/runtime.ll`: core runtime for the selected backend and output.
- `.wosy/artifacts/<target>/<profile>/<artifact>/artifact-manifest.json`: identities for sources, configuration, LLVM input, compiler, LLVM library, and builder toolchain, plus the list of source observations and the executable path.

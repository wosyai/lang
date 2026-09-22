# Wosy

Wosy is a statically typed, ahead-of-time compiled language for programs that must be correct, portable, and reproducible. One codebase compiles through LLVM to native binaries and to WASM (`wasm-wasip1`, `wasm-wasip2`), with no implicit conversions, no hidden control flow, and builds that verify themselves.

## Why Wosy

**Explicit safety.** Every conversion names its destination (`core.int_extend<u64>`, `core.float_trunc<f32>`). Raw addresses, allocation, and pointer arithmetic require `unsafe`. References are three distinct types — `*?T` raw, `*T` shared, `*!T` mutable — and nullable values are tested against `null` and unwrapped with `*`, never assumed.

**One language, native and WASM.** The same `std.print`, `std.read_line`, `std.parse`, and `std.exit` run on both backends. The foreign surface is deliberately narrow: `wasi/preview1.w` exposes only what `bootstrap.w` needs, so portable code stays portable by construction.

**Reproducible builds.** `wosy build` type-checks the full module graph, emits `partition.ll` plus the backend-matched `runtime.ll`, and publishes `artifact-manifest.json` with identities for every source, the configuration, the compiler, LLVM, and the builder toolchain. `wosy run` rehashes everything and refuses to run stale artifacts instead of silently running old code. Diagnostics carry stable codes (`S0001`, `B0001`–`B0012`, `M0001`/`M0003`, `P0001`/`P0002`) with labeled spans.

## Taste it

Hello, completely:

```wosy
%%start
std = namespace std "bootstrap.w";
u64 reported, bool complete = std.print("hello\n");
reported;
complete;
%%end
```

Input, parsing, and branching — nullable text, explicit status, no exceptions:

```wosy
%%start
std = namespace std "bootstrap.w";
*std.utf8 text = null;
std.ReadLineStatus status = std.ReadLineStatus::eof;
text, status = std.read_line(64);
*?u8 content = null;
u64 size = 0;
if (text != null) {
  content = (*text).data;
  size = (*text).length;
};
std.utf8 line = { .data = content; .length = size; };
*u64 value = std.parse(line);
value;
%%end
```

## Getting started

Prerequisites: the `wosy` binary on `PATH`, LLVM 18.1.x reachable through the builder scripts, and Python 3 for the reference `scripts/build-*` / `scripts/run-*` helpers.

```sh
wosy build app --profile dev
wosy run app --profile dev -- <program-args>
```

A project is a directory with `wosy.toml` and `src/main.w`. See [Project layout](docs/project.md) for the full schema.

## Documentation

- [Getting started](docs/getting-started.md) — install, create, build, and run.
- [Syntax](docs/syntax.md) — files, comments, declarations, control flow.
- [Types](docs/types.md) — scalars, structs, enums, arrays, pointers, callables.
- [Core operations](docs/core.md) — conversions, allocation, `unsafe` rules.
- [Standard library](docs/stdlib.md) — `bootstrap.w` and `wasi/preview1.w`.
- [CLI reference](docs/cli.md) — `parse`, `build`, `run`.
- [Diagnostics](docs/errors.md) — every code, what triggers it.

## Status

Version 0.1.0, Apache 2.0 licensed (see [LICENSE](LICENSE)). The language, standard library, and toolchain are under active development — expect sharp edges and precise error messages.

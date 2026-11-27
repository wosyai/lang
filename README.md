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
std.print("hello\n");
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
%%end
```

## Getting started

[Install the toolchain you need](.agents/skills/wosy/references/installation.md), then [create, build, and run a package](.agents/skills/wosy/references/getting-started.md).

## Documentation

- [Syntax](.agents/skills/wosy/references/syntax.md) — files, comments, declarations, control flow.
- [Types](.agents/skills/wosy/references/types.md) — scalars, structs, enums, arrays, pointers, callables.
- [Core operations](.agents/skills/wosy/references/core.md) — conversions, allocation, `unsafe` rules.
- [Standard library](.agents/skills/wosy/references/stdlib.md) — `bootstrap.w` and `wasi/preview1.w`.
- [CLI reference](.agents/skills/wosy/references/cli.md) — commands and options.
- [Diagnostics](.agents/skills/wosy/references/errors.md) — every code, what triggers it.

## Status

Version 0.1.0, Apache 2.0 licensed (see [LICENSE](LICENSE)). The language, standard library, and toolchain are under active development — expect sharp edges and precise error messages.

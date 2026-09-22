# Wosy documentation

Wosy is a statically typed language compiled ahead of time to LLVM. Source files are `.w`, projects are defined by `wosy.toml`, and the standard library ships `bootstrap.w` plus `wasi/preview1.w`.

## Contents

1. [Getting started](getting-started.md) — install the toolchain, create and run a project.
2. [Project layout](project.md) — `wosy.toml`, dependencies, builders, runners, artifacts.
3. [Syntax](syntax.md) — files, comments, declarations, expressions, control flow.
4. [Types](types.md) — scalars, structs, enums, arrays, pointers, callables.
5. [Core operations](core.md) — `core.*` conversions, allocation, unsafe rules.
6. [Standard library](stdlib.md) — `bootstrap.w` and `wasi/preview1.w`.
7. [CLI reference](cli.md) — `wosy parse`, `wosy build`, `wosy run`.
8. [Diagnostics](errors.md) — `S0001`, `B0001`–`B0012`, `M0001`/`M0003`, `P0001`/`P0002`.

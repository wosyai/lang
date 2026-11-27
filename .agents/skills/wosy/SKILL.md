---
name: wosy
description: Write, build, or debug Wosy programs and packages.
---

# Wosy

1. Find the nearest `wosy.toml`. Read its target, profile, builder, and runner.
2. If the project pins Git dependencies, run `wosy dep sync` before building.
3. Run `wosy build <target> --profile <profile>`, then `wosy run <target> --profile <profile>`. Finish when the build succeeds and the program produces the expected result.

- Missing `wosy` or a builder tool? Read [installation](references/installation.md).
- New Wosy project? Read [getting started](references/getting-started.md).
- Editing `.w`? Read [syntax](references/syntax.md) and [types](references/types.md). For `core.*` operations, read [core](references/core.md). For checked reads, writes, or borrows, see [checked access](references/types.md#checked-access).
- Configuring a package, namespace, or Wasm output? Read [project layout](references/project.md).
- Initializing or changing dependencies? Read the [CLI reference](references/cli.md).
- Using `std`? Read the [standard library reference](references/stdlib.md).
- Fixing a diagnostic? Read the named span and [diagnostics](references/errors.md).
- Working on recursion stack fixtures? Read [compiler fixtures](references/compiler-fixtures.md).

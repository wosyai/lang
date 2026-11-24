---
name: wosy
description: Write, build, or debug Wosy programs and packages.
---

# Wosy

1. Find the nearest `wosy.toml`. Read its target, profile, builder, and runner.
2. Run `wosy build <target> --profile <profile>`, then `wosy run <target> --profile <profile>`. Finish when the build succeeds and the program produces the expected result.

- Editing `.w`? Read [syntax](../../../docs/syntax.md) and [types](../../../docs/types.md).
- Configuring a package, namespace, or Wasm output? Read [project layout](../../../docs/project.md).
- Looking up command options? Read the [CLI reference](../../../docs/cli.md).
- Using `std`? Read the [standard library reference](../../../docs/stdlib.md).
- Fixing a diagnostic? Read the named span and [diagnostics](../../../docs/errors.md).

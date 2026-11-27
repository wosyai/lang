# Getting started

Wosy compiles statically typed `.w` programs to native and WebAssembly targets.

See the [installation requirements by workflow](installation.md).

## Create a project

On Linux:

```sh
mkdir my_app && cd my_app
wosy init
```

Use `--profile wasm` on both build and run for WASI Preview 1.

## Write source

Every `.w` file is bounded by `%%start` and `%%end`:

```wosy
%%start
i32(i32) add_one = fn(value) {
  value + 1
};
i32 result = add_one(41);
%%end
```

## Build and run

```sh
wosy build app --profile dev
wosy run app --profile dev -- <program-args>
```

To verify proofs or checker directives, see [`wosy check`](cli.md#wosy-check).

`build` type-checks every reachable module, emits `partition.ll` plus `runtime.ll`, invokes the configured builder, and writes `artifact-manifest.json`. `run` verifies the manifest is fresh and invokes the configured runner. Any source or `wosy.toml` change requires a rebuild; otherwise `run` reports `artifact manifest is stale`.

## Example

Reads one line from stdin, parses it as `u64`, and prints `even`, `odd`, or `invalid`:

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
if (status == std.ReadLineStatus::line) {
  if (value == null) {
    std.print("invalid\n");
    std.exit(3);
  };
  if (value != null) {
    u64 number = *value;
    u64 two = 2;
    u64 zero = 0;
    u64 remainder = number % two;
    if (remainder == zero) {
      std.print("even\n");
    };
    if (remainder != zero) {
      std.print("odd\n");
    };
  };
  std.exit(0);
};
%%end
```

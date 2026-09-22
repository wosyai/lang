---
name: wosy
description: Write, build, and repair Wosy programs. Use for .w files, wosy.toml projects, wosy parse/build/run, and B/M/P/S diagnostics.
---

# Wosy

Wosy is a statically typed ahead-of-time compiled language. Source files are `.w`. Projects are defined by `wosy.toml`. The toolchain emits LLVM and delegates linking and execution to configured builders and runners.

## Workflow

1. Locate the project root by searching upward for `wosy.toml`.
2. Read `wosy.toml` for the target, profile, backend, builder, and runner.
3. Write or edit `.w` sources inside `%%start` / `%%end`.
4. Run `wosy build <target> --profile dev`, then `wosy run <target> --profile dev`.
5. On failure, read the diagnostic code on stderr and fix the named file and span. Rebuild before rerunning.

Done when `wosy build` exits 0 and `wosy run` produces the expected stdout.

## Project layout

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

Rules: one table per dependency with only `path`. Builders and runners are external commands. Supported outputs are `llvm/native` and `llvm/wasm-wasip1|wasm-wasip2`. `wosy run` fails with `artifact manifest is stale` after any source or config change until the next `wosy build`.

## Source shape

Every `.w` file uses explicit boundaries:

```wosy
%%start
i32 value = 40;
%%end
```

Comments start with `#`. A `# CATEGORY: payload` comment is valid only when `CATEGORY` is listed in `[comments] categories`.

Modules and foreign functions:

```wosy
math = namespace app "src/math.w";
std = namespace std "bootstrap.w";
env = extern wasm "env" {
  unit(i32) print_i32;
};
```

Read a member as `math.value`, call as `math.add(20, 22)`. Names starting with `_` are module-private and unreachable from other modules.

## Declarations and statements

```wosy
struct Record {
  u8 value;
  u8[2] bytes;
}

i32(i32) add_one = fn(value) {
  value + 1
};

u8[4] values = [1, 2, 3, 4];
values[0] = 11;
u8 item = values[0];
Record record = { .value = 3; .bytes = [4, 5]; };
u8 record_value = record.value;
i32 result = add_one(41);
u8 first = 1;
u8 second = 2;
first, second = second, first;
```

Function types name outputs then parameters: `i32(i32)`, `unit()`, `(u64, bool)(utf8)`. The last value list without `;` is the return: `reported, complete`. `generic T;` plus `overload { }` declare generic and overloaded callables.

Control flow:

```wosy
if (ready) {
  print();
} else {
  log();
}
while (value < 3) {
  value = value + 1;
}
unsafe {
  *?u8 p = &?iovec;
}
```

Conditions are always `bool`. `&&` and `||` short-circuit.

## Types, addresses, core

Scalar types: `unit, bool, i8, i16, i32, i64, i128, u8, u16, u32, u64, u128, f32, f64, char`. Text is `std.utf8`, a struct with `data` and `length`.

Arrays and pointers:

```wosy
u8[4] fixed = [1, 2, 3, 4];
*?u8 raw = &?place;
*u8 shared = &place;
*!u8 mutable = &!place;
u8 item = fixed[0];
```

`T[N]` has fixed length. `T[n]` with a length expression inside a function allocates runtime storage. `&?`, `&`, `&!` take raw, shared, and mutable addresses. Dereference with `*p`.

Core conversions, all with explicit destination:

```wosy
u64 wide = core.int_extend<u64>(x);
u8 narrow = core.int_trunc<u8>(wide);
f64 f = core.uint_to_float<f64>(u);
f64 g = core.sint_to_float<f64>(s);
f64 h = core.float_extend<f64>(f32value);
*?u8 at = unsafe { core.offset<u8>(base, index) };
u8 v = unsafe { core.load<u8>(at) };
```

`core.alloc`, `core.free`, `core.offset`, `core.load`, and `&?` require `unsafe`.

Standard I/O through `std`:

```wosy
std = namespace std "bootstrap.w";
std.print("hello\n");
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
```

`std.print` and `std.eprint` return bytes written plus completion. `std.read_line(64)` returns nullable text plus a `ReadLineStatus` of `line, eof, io_error, invalid_utf8, limit_exceeded`. `std.parse(line)` returns a nullable number, integer or float selected by context. Compare optionals against `null` and unwrap with `*value` inside the non-null branch.

## Naming and validity rules

Use `PascalCase` for structs and enums, `snake_case` for functions, bindings, and parameters, `SCREAMING_SNAKE_CASE` for constants. A constant rejects assignment with `B0007`:

```wosy
i32 MAXIMUM = 1;
MAXIMUM = 2;
```

Every local binding must be read. An unused local fails with `B0009`. A `while` or `if` over a non-`bool` fails with `B0005`. Unknown names fail with `B0001`, duplicates with `B0002`, arity mismatches with `B0004`, out-of-range literals with `B0010`, missing modules with `M0001`, import cycles with `M0003`.

# Standard library

Add the compiler repository's `stdlib/` directory as the `std` package. Import its public module:

```wosy
std = namespace std "bootstrap.w";
```

From `stdlib/`, `wosy build` then `wosy run` runs the WASI smoke target and prints `Wosy standard library`.

## Checked access

On supported checked dereferences and array reads, writes, and borrows, the [standard-library checks](../../../../stdlib/src/checked_access.w) detect null references and compare the full `u64` index with the array's actual construction extent before address calculation or narrowing. A failed check calls `core.system_panic()` and terminates execution. The helpers and the way the compiler carries the extent are implementation details; use ordinary checked access syntax.

## Text

```wosy
struct utf8 {
  *?u8 data;
  u64 length;
}
```

`utf8` is a struct holding a raw byte pointer plus a length. String literals construct it.

## Console output

```wosy
(u64, bool)(utf8) print = fn(text) { };
(u64, bool)(utf8) eprint = fn(text) { };
```

`std.print` writes to file descriptor 1, `std.eprint` to descriptor 2. Both return bytes reported plus completion. Empty input completes immediately. Input longer than `u32` range reports zero bytes and no completion.

```wosy
u64 reported, bool complete = std.print("hello\n");
u64 ereported, bool ecomplete = std.eprint("hello stderr\n");
```

## Input

```wosy
(u64, bool)(*?u8, u64) read_into = fn(destination, capacity) { };
(*utf8, ReadLineStatus)(u64) read_line = fn(maximum) { };
```

`read_into` reads raw bytes into caller storage. `read_line` reads up to a newline or EOF and validates UTF-8, with statuses `line, eof, io_error, invalid_utf8, limit_exceeded`.

```wosy
*std.utf8 text = null;
std.ReadLineStatus status = std.ReadLineStatus::eof;
text, status = std.read_line(64);
```

## Parsing and formatting

`std.parse` is an overload set converting `utf8` to nullable numbers, integer or float selected by context:

```wosy
std.utf8 valid_input = "42";
*u64 value = std.parse(valid_input);
```

Names starting with `_` in `std` are module-private implementation details. The public conversion entry point is `std.parse`; ambiguous integer literals without a unique target type are rejected.

## Exit

```wosy
unit(u32) exit = fn(code) { };
std.exit(0);
```

## WASI preview1

`wasi/preview1.w` exposes the narrow foreign surface used by `bootstrap.w`:

```wosy
struct Iovec {
  *?u8 data;
  u32 length;
}
struct Nwritten {
  u32 value;
}
_wasi = extern wasm "wasi_snapshot_preview1" {
  unsafe i32(i32, *?_ReadIovec, i32, *?_Nread) _fd_read;
  unsafe i32(i32, *?Iovec, i32, *?Nwritten) fd_write;
  unsafe unit(i32) _proc_exit;
};
```

Direct use of these foreign functions is possible but not required for normal programs. Prefer `std.print`, `std.eprint`, `std.read_into`, `std.read_line`, and `std.exit`.

# Standard library

The standard library consists of two sources: `stdlib/src/bootstrap.w` and `stdlib/src/wasi/preview1.w`. Import them as a path dependency named `std`.

```wosy
std = namespace std "bootstrap.w";
```

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

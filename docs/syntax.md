# Syntax

## Files

Every source file starts with `%%start` and ends with `%%end`. The parser recovers from a broken item and keeps the declarations before and after it, but `wosy parse` and `wosy build` still exit non-zero.

## Whitespace and comments

Whitespace is spaces, tabs, and line endings. Comments start with `#` and run to end of line:

```wosy
# TODO: handle empty input
```

A comment shaped `# CATEGORY: payload` is a typed comment. It is accepted only when `CATEGORY` appears in `[comments] categories` in `wosy.toml`, so the example above requires `categories = ["TODO"]`. Anything inside a string literal is string content, not a comment.

## Top-level declarations

```wosy
math = namespace app "src/math.w";
std = namespace std "bootstrap.w";
env = extern wasm "env" {
  unit(i32) print_i32;
};
struct Record {
  u8 value;
  u8[2] bytes;
}
enum ReadLineStatus {
  line;
  eof;
  io_error;
  invalid_utf8;
  limit_exceeded;
}
i32(i32) add_one = fn(value) {
  value + 1
};
generic T;
T(T) identity = fn(value) {
  value
};
cast = overload {
  i32(i64) => fn(value) {
    7
  };
  generic T;
  T(T) => fn(value) {
    value
  };
};
u8[2][3] values = [[1, 2], [3, 4], [5, 6]];
Record record = { .value = 3; .bytes = [4, 5]; };
```

Namespaces bind a local name to a file in a package. `extern` blocks declare functions imported from a foreign module; only `wasm` modules are supported. Structs list fields with explicit types. Enums list variants. Functions name their signature, parameters, and block body. Generic functions are preceded by `generic T;`. Overloads group call shapes under one name. Bindings declare receivers, a type, and output values.

Top-level executable statements are also allowed:

```wosy
i32 value = 0;
value = 1;
value;
```

## Blocks and local bindings

```wosy
i32(i32) loop = fn(start) {
  i32 value = start;
  while (value < 3) {
    value = value + 1;
  }
  value
};
```

A block holds local bindings, `if` and `while` items, `unsafe` items, and expressions. Bindings ending with `;` are statements. The trailing value list without `;` is the block result. A `while` body followed by `;` is rejected.

## Control flow

```wosy
if (input == 2) {
  input
} else {
  input + 1
}
if (ready) {
  print();
}
unsafe {
  wasi.fd_write(descriptor, iovec_address, 1, byte_count_address)
}
```

`if (cond) { } else { }` returns a value; both branches must have equal types. `if (cond) { }` without `else` is a unit conditional. `unsafe { }` is required around raw addresses, `core.alloc`/`core.free`/`core.offset`/`core.load`, and unsafe extern calls. Conditions are always `bool`.

## Expressions

Precedence from loosest to tightest: `||`, `&&`, `|`, `^`, `&`, comparisons (`== != < <= > >=`), shifts (`<< >>`), addition and subtraction, multiplication/division/remainder, unary (`! ~ -`).

```wosy
i32 flags = 1 | 2 ^ 3 & 4 << 1 + 2 * 3;
bool ok = !!!ready && waiting || done;
i32 negated = -value;
```

Calls use dots and optional type arguments:

```wosy
env.print_i32(add_one(41))
math.add(20, 22)
core.int_trunc<u32>(length)
wasi.fd_write(1, iovec_address, 1, nwritten_address)
```

Places support indexing, field access, dereference, and address-taking:

```wosy
matrix[row][column]
bytes[0]
record.bytes[0] = 9;
*writer = 8;
*shared
(*mutable).value
*?u8 raw = &?iovec;
*u8 shared = &place;
*!u8 mutable = &!place;
```

Struct literals name every field: `{ .value = 3; .bytes = [4, 5]; }`. Array literals list elements: `[1, 2, 3, 4]`. Multi-target assignment binds element-wise, so `first, second = second, first;` swaps two values.

Integer literals accept decimal, `0x`, `0b`, and `0o` bases with `_` separators. Float literals must be finite. Character literals hold exactly one character. String literals support `\\ \" \' \n \r \t \0` and `\u{HEX}` escapes.

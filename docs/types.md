# Types

## Scalar types

`unit, bool, i8, i16, i32, i64, i128, u8, u16, u32, u64, u128, f32, f64, char`. Integers are fixed-width with checked literal ranges. Floats must be finite. Text is not a scalar: use `std.utf8`.

## Structs and enums

```wosy
struct Iovec {
  *?u8 data;
  u32 length;
}
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
```

A struct value names every field. An enum value names its variant: `std.ReadLineStatus::eof`. Enum variants carry no payload.

## Arrays

`u8[4]` is a fixed array of length 4. `u8[2][3]` nests fixed arrays. `u8[n]` with a length expression inside a function allocates runtime storage:

```wosy
u8[4] bytes = [1, 2, 3, 4];
u8 item = bytes[0];
bytes[1] = 6;
```

Array literals require a fixed-array context and an element count equal to the declared length. Index expressions are typed as `u64`. The base of an indexed place must be an array.

## Pointers and references

- `*?T` is a raw pointer.
- `*T` is a shared checked reference.
- `*!T` is a mutable checked reference.

```wosy
*?u8 raw = &?place;
*u8 shared = &place;
*!u8 mutable = &!place;
u8 first = *shared;
*mutable = 13;
```

`&?place` takes a raw address and requires `unsafe`. `null` is a valid pointer only where a pointer type is expected. A nullable value is tested against `null` and unwrapped with `*`:

```wosy
*std.utf8 text = null;
if (text != null) {
  content = (*text).data;
};
```

Direct foreign arrays and checked references across `extern` boundaries are rejected. Transport arrays and references through Wosy functions instead.

## Callables

A callable type names outputs followed by parameters:

```wosy
unit(i32) print_it;
i32(i32) add_one;
(u64, bool)(utf8) print;
(i32, i32)(i32, i32) pair;
```

`unit` is valid only as a function result. A call must supply every parameter and bind every output:

```wosy
u64 reported, bool complete = std.print("hello\n");
text, status = std.read_line(64);
```

Names use `PascalCase` for structs and enums, `snake_case` for functions, bindings, and parameters, and `SCREAMING_SNAKE_CASE` for constants. Constants reject assignment. Every local binding must be read at least once.

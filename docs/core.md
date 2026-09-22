# Core operations

All core operations use explicit type arguments. There are no implicit conversions.

## Integer extension and truncation

```wosy
u64 wide = core.int_extend<u64>(count);
u8 narrow = core.int_trunc<u8>(wide);
```

`int_extend` widens, `int_trunc` narrows. Source and destination must both be integer types with the appropriate width relationship.

## Float conversions

```wosy
f64 from_unsigned = core.uint_to_float<f64>(u);
f64 from_signed = core.sint_to_float<f64>(s);
i32 as_signed = core.float_to_sint_trunc<i32>(f);
u64 as_unsigned = core.float_to_uint_trunc<u64>(f);
f32 narrowed = core.float_trunc<f32>(wide);
f64 widened = core.float_extend<f64>(narrow);
```

Each conversion names its destination: `uint_to_float` and `sint_to_float` take integer sources to `f32`/`f64`; `float_to_sint_trunc` and `float_to_uint_trunc` truncate floats to integers; `float_trunc` narrows `f64` to `f32`; `float_extend` widens `f32` to `f64`.

## Bitcast

```wosy
u32 code = core.bitcast<u32>(letter);
```

Source and destination must be scalar types of equal size.

## Allocation, pointer arithmetic, and panic

```wosy
unit() release = fn {
  unsafe {
    *?u8 storage = core.alloc(16, 8);
    core.free(storage);
  };
};
unit() panic = fn {
  core.system_panic();
};
u8(*?u8, i64) read = fn(pointer, index) {
  unsafe { core.load<u8>(core.offset<u8>(pointer, index)) }
};
```

`core.alloc(size, alignment)`, `core.free(pointer)`, `core.offset`, and `core.load` require `unsafe`. Raw addresses taken with `&?` also require `unsafe`. `core.system_panic` takes no arguments.

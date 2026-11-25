%%start
probe = extern wasm "fixture_resource" {
    unit(i32, u64, *?u8) checkpoint;
};

u64(*u8, u64) descend = fn(backing, depth) {
    probe.checkpoint(0, depth, null);
    u64 length = 4;
    u8[length] scratch;
    scratch[0] = 16 + core.int_trunc<u8>(depth);
    scratch[3] = 173;
    unsafe { probe.checkpoint(1, depth, &?scratch[0]); };
    u64 subtotal = if (depth == 0) { 0 } else { descend(backing, depth - 1) };
    u64 result = subtotal + core.int_extend<u64>(scratch[0]) + core.int_extend<u64>(*backing);
    unsafe { probe.checkpoint(2, depth, &?scratch[0]); };
    result
};

unit() check = fn {
    u8 backing = 11;
    unsafe { probe.checkpoint(3, 3, &?backing); };
    *u8 view = &backing;
    u64 result = descend(view, 3);
    if (result != 114 || backing != 11) { core.system_panic(); };
    unsafe { probe.checkpoint(4, 3, &?backing); };
};

check();
probe.checkpoint(5, 3, null);
%%end

%%start
helpers = namespace checked_reference_return_default "src/helpers.w";

struct Iovec {
    *?u8 data;
    u32 length;
}

struct Nwritten {
    u32 value;
}

wasi = extern wasm "wasi_snapshot_preview1" {
    unsafe i32(i32, *?Iovec, i32, *?Nwritten) fd_write;
};

struct copy Point {
    u8 first;
    u8 second;
}

struct Payload {
    u8 value;
}

*u8(u8) f = fn(v) { unsafe { &v } };

*Point() make_point = fn {
    Point local = { .first = 3; .second = 4; };
    &local
};

*!Payload() make_payload = fn {
    Payload local = { .value = 5; };
    &!local
};

*(u8[2])() make_bytes = fn {
    u8[2] local = [6, 7];
    &local
};

*!(Payload[2])() make_payloads = fn {
    Payload[2] local = [{ .value = 7; }, { .value = 8; }];
    &!local
};

unit() run = fn {
    *u8 scalar = f(42);
    u8 churn = helpers.stack_call(1);
    u8 churn_again = helpers.stack_call(churn);
    if (*scalar != 42 || churn_again != 3) { core.system_panic(); };

    *Point point = make_point();
    u8 field = (*point).first;
    Point first_copy = *point;
    Point second_copy = *point;
    if (field != 3 || first_copy.first != 3 || second_copy.second != 4) { core.system_panic(); };

    *!Payload writer = make_payload();
    u8 prior = (*writer).value;
    Payload taken = *writer;
    if (prior != 5 || taken.value != 5) { core.system_panic(); };

    *(u8[2]) bytes = make_bytes();
    u8[2] observed_bytes = *bytes;
    u8 element = observed_bytes[0];
    u8[2] whole_bytes = *bytes;
    if (element != 6 || whole_bytes[0] != 6 || whole_bytes[1] != 7) { core.system_panic(); };

    *!(Payload[2]) items = make_payloads();
    Payload[2] whole_items = *items;
    *Payload first_item = &whole_items[0];
    *Payload second_item = &whole_items[1];
    if ((*first_item).value != 7 || (*second_item).value != 8) { core.system_panic(); };
    u8[39] output = [99, 104, 101, 99, 107, 101, 100, 32, 114, 101, 102, 101, 114, 101, 110, 99, 101, 32, 114, 101, 116, 117, 114, 110, 58, 32, 52, 50, 32, 51, 32, 52, 32, 53, 32, 54, 32, 55, 32];
    u8[2] ending = [56, 10];
    Iovec first = { .data = unsafe { &?output[0] }; .length = 39; };
    Iovec second = { .data = unsafe { &?ending[0] }; .length = 2; };
    Nwritten written = { .value = 0; };
    unsafe { wasi.fd_write(1, &?first, 1, &?written); };
    unsafe { wasi.fd_write(1, &?second, 1, &?written); };
};

run();
%%end

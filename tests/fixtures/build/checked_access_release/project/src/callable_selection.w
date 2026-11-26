%%start
u64(*!u8,u64) selector = fn(order,index) {
    if (*order != 0) { core.system_panic(); };
    *order = 1;
    index
};
u64(*!u8,u64) argument = fn(order,value) {
    if (*order != 1) { core.system_panic(); };
    *order = 2;
    value + 1
};
u64(u64) named = fn(value) { value + 10 };
u64(u64,u64) fixed = fn(index,value) {
    u64(u64)[2] operations = [named, fn(value) { value + 20 }];
    u8 order = 0;
    u64 result = operations[selector(&!order,index)](argument(&!order,value));
    if (order == 2) { result } else { 0 }
};
u64(u64) captured = fn(index) {
    u64()[3] operations = [fn(i) { fn() { i } }; 3];
    u8 order = 0;
    u64 result = operations[selector(&!order,index)]();
    if (order == 1) { result } else { 999 }
};
u64(u64,u64,u64) generated = fn(count,first,second) {
    u64()[] operations = [fn(i) { fn() { i } }; count];
    u8 first_order = 0;
    u8 second_order = 0;
    u64 first_value = operations[selector(&!first_order,first)]();
    u64 second_value = operations[selector(&!second_order,second)]();
    if (first_order == 1 && second_order == 1) { first_value * 10 + second_value } else { 999 }
};
u64(u64) empty = fn(count) {
    u8()[] operations = [fn(_) { fn() { 7 } }; count];
    core.array_len(&operations)
};
if (fixed(0,3) != 14 || fixed(1,3) != 24 || captured(2) != 2 || generated(3,2,1) != 21 || empty(0) != 0) {
    core.system_panic();
};
%%end

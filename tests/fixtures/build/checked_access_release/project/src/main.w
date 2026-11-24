%%start
u8(u64, u8) update = fn(index, replacement) {
    u8[3] cells = [7, 8, 9];
    cells[index] = replacement;
    u8 untouched = cells[1];
    if (untouched == 8) { cells[index] } else { 0 }
};
u8(*u8) checked_load = fn(pointer) { *pointer };
u8() checked_witness = fn {
    u8 byte = 42;
    checked_load(&byte)
};
u64(u64) construction_count = fn(count) { count };
u64(*!u8, u64) select_index = fn(visits, index) {
    *visits = *visits + 1;
    index
};
u8(u64, u64) runtime_update = fn(count, index) {
    u8[construction_count(count)] bytes;
    bytes[index] = 29;
    u8 visits = 0;
    u8 observed = bytes[select_index(&!visits, index)];
    if (visits == 1) { observed } else { 0 }
};
if (update(0, 14) != 14 || update(2, 21) != 21 || checked_witness() != 42
    || runtime_update(5, 3) != 29 || runtime_update(2, 1) != 29) {
    core.system_panic();
};
%%end

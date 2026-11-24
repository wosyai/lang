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
if (update(0, 14) != 14 || update(2, 21) != 21 || checked_witness() != 42) {
    core.system_panic();
};
%%end

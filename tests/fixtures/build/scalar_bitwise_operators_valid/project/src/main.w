%%start
bool(bool) logical_not = fn(value) { !value };
i32(i32) bitwise_not = fn(value) { ~value };
i32(i32, i32) bitwise = fn(left, right) { (left & right) | (left ^ right) };
i32(i32, i32) signed_shift = fn(value, count) { (value << count) >> count };
u32(u32, u32) unsigned_shift = fn(value, count) { value >> count };

bool inverted = logical_not(true);
i32 complemented = bitwise_not(1);
i32 combined = bitwise(7, 3);
i32 shifted = signed_shift(8, 1);
u32 unsigned_value = 8;
u32 unsigned_count = 1;
u32 logical_shifted = unsigned_shift(unsigned_value, unsigned_count);
%%end

%%start
struct PointerPair {
	*?u8 address;
	u32 count;
}
PointerPair pair = { .address = null; .count = 42; };
i32(i32) negate = fn(value) { -value };
i32 quotient = -7 / 3;
i32 remainder = -7 % 3;
i32 computed_quotient = negate(7) / 3;
i32 computed_remainder = negate(7) % 3;
%%end

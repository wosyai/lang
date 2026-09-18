%%start
u8[]() make = fn {
	u64 length = 1;
	u8[length] bytes;
	bytes[0] = 4;
	bytes
};

u8[](u8[]) relay = fn(values) {
	values
};

unit() run = fn {
	u64 length = 3;
	u8[length] bytes;
	bytes[0] = 7;
	bytes[1] = 8;
	bytes[2] = 9;
	u8 value = bytes[1];
	u8[] returned = make();
	u8[] alias = relay(returned);
	u8 transferred = alias[0];
	u8 transferred_expected = 4;
	u8[1] fixed = [5];
	u8[] widened = fixed;
	u8 widened_value = widened[0];
	u8 widened_expected = 5;
	if (true) {
		u64 nested_length = 1;
		u8[nested_length] nested;
		nested[0] = 6;
	};
	u8 expected = 8;
	if (value != expected || transferred != transferred_expected || widened_value != widened_expected) {
		core.system_panic();
	};
};

run();
%%end

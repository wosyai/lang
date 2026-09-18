%%start
u64 top_length = 1;
u8[top_length] top;
top[0] = 3;
u8 top_value = top[0];
u8 top_expected = 3;
if (top_value != top_expected) {
	core.system_panic();
};

u8[]() make = fn {
	u64 length = 1;
	u8[length] bytes;
	bytes[0] = 4;
	bytes
};

u8[](u8[]) relay = fn(values) {
	values
};

u8[](u8[]) replace = fn(old) {
	u64 length = 1;
	u8[length] next;
	next[0] = old[0];
	next
};

struct Packet {
	*?u8 data;
	u64 length;
}

*Packet() make_packet = fn {
	u64 length = 1;
	u8[length] bytes;
	bytes[0] = 4;
	*?u8 data = null;
	unsafe { data = &?bytes[0]; };
	Packet result = { .data = data; .length = length; };
	&result
};

unit() run = fn {
	u64 length = 3;
	u8[length] bytes;
	bytes[0] = 7;
	bytes[1] = 8;
	bytes[2] = 9;
	u8 value = bytes[1];
	u8[] returned = make();
	u8[] replaced = replace(returned);
	u8[] alias = relay(replaced);
	*Packet returned_packet = make_packet();
	u64 overwrite_length = 1;
	u8[overwrite_length] overwrite;
	overwrite[0] = 9;
	u8[] moved = bytes;
	u8 moved_expected = 7;
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
	if (value != expected || transferred != transferred_expected || widened_value != widened_expected || moved[0] != moved_expected || returned_packet == null || (*returned_packet).data == null || (*returned_packet).length != overwrite_length) {
		core.system_panic();
	};
};

run();
%%end

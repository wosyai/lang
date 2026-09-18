%%start
std = namespace std "bootstrap.w";

struct Buffer {
	u8 value;
	u8[3] bytes;
}

unit() run = fn {
	u8[3] values = [4, 5, 6];
	Buffer buffer = { .value = 7; .bytes = [8, 9, 10]; };
	*!u8 writer = &!values[1];

	values[0] = 11;
	buffer.value = 12;
	*writer = 13;
	buffer.bytes[2] = 14;
	values[0], values[2] = 15, 16;

	u8 indexed = values[0];
	u8 field = buffer.value;
	u8 dereferenced = values[1];
	u8 field_indexed = buffer.bytes[2];
	u8 indexed_expected = 15;
	u8 field_expected = 12;
	u8 dereferenced_expected = 13;
	u8 field_indexed_expected = 14;

	if (indexed == indexed_expected) {
		std.print("indexed write\n");
	};
	if (field == field_expected) {
		std.print("field write\n");
	};
	if (dereferenced == dereferenced_expected) {
		std.print("mutable dereference write\n");
	};
	if (field_indexed == field_indexed_expected) {
		std.print("field index write\n");
	};
};

run();
%%end

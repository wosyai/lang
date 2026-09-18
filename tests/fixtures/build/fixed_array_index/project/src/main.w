%%start
std = namespace std "bootstrap.w";

struct Buffer {
	u8[3] bytes;
}

unit() run = fn {
	u8[3] values = [4, 5, 6];
	u8[2][2] matrix = [[7, 8], [9, 10]];
	Buffer buffer = { .bytes = [11, 12, 13]; };

	u8 scalar = values[1];
	u8 nested = matrix[1][0];
	u8 field = buffer.bytes[2];
	*u8 indexed = &values[2];
	u8 addressed = *indexed;
	u8 scalar_expected = 5;
	u8 nested_expected = 9;
	u8 field_expected = 13;
	u8 addressed_expected = 6;

	if (scalar == scalar_expected) {
		std.print("scalar index\n");
	};
	if (nested == nested_expected) {
		std.print("nested index\n");
	};
	if (field == field_expected) {
		std.print("field index\n");
	};
	if (addressed == addressed_expected) {
		std.print("checked address index\n");
	};
};

run();
%%end

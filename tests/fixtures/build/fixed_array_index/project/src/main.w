%%start
std = namespace std "bootstrap.w";

struct Buffer {
	u8[3] bytes;
}

u8(u8) transport = fn(value) {
	value
};

unit() run = fn {
	u8[3] values = [4, 5, 6];
	u8[2][2] matrix = [[7, 8], [9, 10]];
	Buffer buffer = { .bytes = [11, 12, 13]; };

	u8 scalar = values[1];
	u8 nested = matrix[1][0];
	u8 field = buffer.bytes[2];
	*u8 indexed = &values[0];
	u8 addressed = *indexed;

	transport(scalar);
	transport(nested);
	transport(field);
	transport(addressed);
	std.print("fixed array index reads\n");
};

run();
%%end

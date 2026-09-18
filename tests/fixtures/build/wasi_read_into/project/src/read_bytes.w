%%start
std = namespace std "bootstrap.w";

u8[4] bytes = [0, 0, 0, 0];
u64 count = 0;
bool complete = false;
u64 expected_count = 2;
u8 first = 65;
u8 second = 66;
unsafe {
	count, complete = std.read_into(&?bytes[0], 4);
};
if (count == expected_count && complete && bytes[0] == first && bytes[1] == second) {
	std.print("short read\n");
};
unsafe {
	count, complete = std.read_into(&?bytes[0], 4);
};
u64 zero = 0;
if (count == zero && complete) {
	std.print("eof\n");
};
%%end

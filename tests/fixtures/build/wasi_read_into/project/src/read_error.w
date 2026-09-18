%%start
std = namespace std "bootstrap.w";

u8[4] bytes = [0, 0, 0, 0];
u64 count = 0;
bool complete = true;
u64 zero = 0;
unsafe {
	count, complete = std.read_into(&?bytes[0], 4);
};
if (count == zero && !complete) {
	std.print("error\n");
};
%%end

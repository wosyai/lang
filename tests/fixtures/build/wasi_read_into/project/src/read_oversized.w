%%start
std = namespace std "bootstrap.w";

u8[4] bytes = [0, 0, 0, 0];
u64 capacity = 4294967296;
u64 count = 1;
bool complete = true;
u64 zero = 0;
unsafe {
	count, complete = std.read_into(&?bytes[0], capacity);
};
if (count == zero && !complete) {
	std.print("oversized\n");
};
%%end

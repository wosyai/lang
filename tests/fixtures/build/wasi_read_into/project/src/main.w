%%start
std = namespace std "bootstrap.w";

u8[4] bytes = [0, 0, 0, 0];
u64 count = 0;
bool complete = false;
unsafe {
	count, complete = std.read_into(&?bytes[0], 4);
};
%%end

%%start
std = namespace std "bootstrap.w";

u8[4](u8[4]) transport = fn(bytes) {
	bytes
};

unit() run = fn {
	u8[4] bytes = [1, 2, 3, 4];
	transport(bytes);
	std.print("fixed array transport\n");
};

run();
%%end

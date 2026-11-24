%%start
std = namespace std "bootstrap.w";

u8[4] bytes = [10, 20, 30, 40];

unit() run = fn {
	*?u8 base = unsafe { core.pointer_cast<*?u8>(&?bytes) };
	u8 first = unsafe { core.load<u8>(base) };
	u8 second = unsafe { core.load<u8>(core.offset<u8>(base, 1)) };
	u8 third = unsafe { core.load<u8>(core.offset<u8>(base, 2)) };
	u8 fourth = unsafe { core.load<u8>(core.offset<u8>(base, 3)) };
	std.print(std.format(first));
	std.print("\n");
	std.print(std.format(second));
	std.print("\n");
	std.print(std.format(third));
	std.print("\n");
	std.print(std.format(fourth));
	std.print("\n");
};

run();
%%end

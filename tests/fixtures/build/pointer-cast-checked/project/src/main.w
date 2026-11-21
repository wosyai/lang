%%start
std = namespace std "bootstrap.w";

unit() run = fn {
	*?u8 storage = unsafe { core.alloc(2, 1) };
	*!u8 writer = unsafe { core.pointer_cast<*!u8>(storage) };
	*writer = 65;
	*u8 shared = unsafe { core.pointer_cast<*u8>(storage) };
	u8 first = *shared;
	std.print(std.format(first));
	std.print("\n");
	*writer = 67;
	u8 second = *writer;
	std.print(std.format(second));
	std.print("\n");
	unsafe { core.free(storage) };
};

run();
%%end

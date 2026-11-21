%%start
std = namespace std "bootstrap.w";

struct Block {
	u8 tag;
	u32 value;
}

unit() run = fn {
	*?u8 storage = unsafe { core.alloc(8, 4) };
	*!u8 w0 = unsafe { core.pointer_cast<*!u8>(storage) };
	*w0 = 7;
	at4 = unsafe { core.offset<u8>(storage, 4) };
	*!u8 w4 = unsafe { core.pointer_cast<*!u8>(at4) };
	*w4 = 42;
	viewed = unsafe { core.pointer_cast<*?Block>(storage) };
	back = unsafe { core.pointer_cast<*?u8>(viewed) };
	u8 first = unsafe { core.load<u8>(back) };
	u8 fifth = unsafe { core.load<u8>(core.offset<u8>(back, 4)) };
	std.print(std.format(first));
	std.print("\n");
	std.print(std.format(fifth));
	std.print("\n");
	unsafe { core.free(storage) };
};

run();
%%end

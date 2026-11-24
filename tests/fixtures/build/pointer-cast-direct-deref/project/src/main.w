%%start
std = namespace std "bootstrap.w";

struct Record {
	u8 tag;
}

unit() run = fn {
	*?u8 raw = unsafe { core.alloc(1, 1) };
	unsafe {
		*!Record writer = core.pointer_cast<*!Record>(raw);
		(*writer).tag = 65;
		u8 shared_scalar = *core.pointer_cast<*u8>(raw);
		std.print(std.format(shared_scalar));
		std.print("\n");
		u8 mutable_scalar = *core.pointer_cast<*!u8>(raw);
		std.print(std.format(mutable_scalar));
		std.print("\n");
		(*writer).tag = 67;
		u8 shared_field = (*core.pointer_cast<*Record>(raw)).tag;
		std.print(std.format(shared_field));
		std.print("\n");
		u8 mutable_field = (*core.pointer_cast<*!Record>(raw)).tag;
		std.print(std.format(mutable_field));
		std.print("\n");
		core.free(raw);
	}
};

run();
%%end

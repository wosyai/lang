%%start
std = namespace std "bootstrap.w";
observer = extern wasm "fixture_formatter_owner" {
	unit(i32) checkpoint;
};

unit(u64) print_view = fn(capacity) {
	unsafe {
		u8[capacity] scratch;
		scratch[0] = 75;
		*?u8 base = core.pointer_cast<*?u8>(&?scratch);
		std.utf8 view = { .data = base; .length = 1; };
		observer.checkpoint(81);
		u64 written, bool complete = std.print(view);
		if (written != 1 || !complete || scratch[0] != 75) { core.system_panic(); };
		observer.checkpoint(82);
	};
	observer.checkpoint(83);
};

observer.checkpoint(80);
print_view(3);
observer.checkpoint(84);
%%end

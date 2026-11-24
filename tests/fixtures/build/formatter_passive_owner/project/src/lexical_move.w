%%start
std = namespace std "bootstrap.w";
observer = extern wasm "fixture_formatter_owner" {
	unit(i32) checkpoint;
};

unit(u8) move_inward = fn(value) {
	std.utf8 outer = std.format(value);
	observer.checkpoint(71);
	unsafe {
		std.utf8 inner = outer;
		u8 first = core.load<u8>(inner.data);
		if (inner.length != 3 || first != 50) { core.system_panic(); };
		observer.checkpoint(72);
	};
	observer.checkpoint(73);
};

observer.checkpoint(70);
move_inward(203);
observer.checkpoint(74);
%%end

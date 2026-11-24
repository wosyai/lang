%%start
std = namespace std "bootstrap.w";

unit(u8) move_inward = fn(value) {
	std.utf8 outer = std.format(value);
	unsafe {
		std.utf8 inner = outer;
		u8 first = core.load<u8>(inner.data);
		if (inner.length != 3 || first != 50) { core.system_panic(); };
	};
	std.print(outer);
};

move_inward(203);
%%end

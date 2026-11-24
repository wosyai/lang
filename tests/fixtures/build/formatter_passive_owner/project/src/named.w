%%start
std = namespace std "bootstrap.w";
observer = extern wasm "fixture_formatter_owner" {
	unit(i32) checkpoint;
};

unit(bool) named = fn(active) {
	if (active) {
		u8 seventeen = 17;
		std.utf8 outer = std.format(seventeen);
		u8 byte_at_11 = unsafe { core.load<u8>(outer.data) };
		if (outer.length != 2 || byte_at_11 != 49) { core.system_panic(); };
		observer.checkpoint(11);
		if (active) {
			u8 two_forty_five = 245;
			std.utf8 inner = std.format(two_forty_five);
			u8 byte_at_12 = unsafe { core.load<u8>(inner.data) };
			if (inner.length != 3 || byte_at_12 != 50) { core.system_panic(); };
			observer.checkpoint(12);
			observer.checkpoint(13);
		};
		u8 byte_at_14 = unsafe { core.load<u8>(outer.data) };
		if (outer.length != 2 || byte_at_14 != 49) { core.system_panic(); };
		observer.checkpoint(14);
		observer.checkpoint(15);
	};
	observer.checkpoint(16);
};

observer.checkpoint(10);
named(true);
observer.checkpoint(17);
%%end

%%start
std = namespace std "bootstrap.w";
observer = extern wasm "fixture_formatter_owner" {
	unit(i32) checkpoint;
};

unit(std.utf8) inspect = fn(text) {
	u8 first = unsafe { core.load<u8>(text.data) };
	if (text.length != 3 || first != 50) { core.system_panic(); };
	observer.checkpoint(62);
};

unit(std.utf8) relay = fn(text) {
	inspect(text);
};

unit(u8) consume = fn(value) {
	std.utf8 text = std.format(value);
	observer.checkpoint(61);
	relay(text);
	observer.checkpoint(63);
};

observer.checkpoint(60);
consume(203);
observer.checkpoint(64);
%%end

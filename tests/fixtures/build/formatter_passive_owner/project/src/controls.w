%%start
std = namespace std "bootstrap.w";
observer = extern wasm "fixture_formatter_owner" {
	unit(i32) checkpoint;
};

unit(u64, bool) release = fn(capacity, active) {
	if (active) {
		u8[capacity] scratch;
		scratch[0] = 75;
		if (scratch[0] != 75) { core.system_panic(); };
		observer.checkpoint(23);
	};
	observer.checkpoint(24);
};

observer.checkpoint(20);
u64 static_written, bool static_complete = std.print("static");
if (static_written != 6 || !static_complete) { core.system_panic(); };
observer.checkpoint(21);
u64 empty_written, bool empty_complete = std.print("");
if (empty_written != 0 || !empty_complete) { core.system_panic(); };
observer.checkpoint(22);
release(3, true);
observer.checkpoint(25);
%%end

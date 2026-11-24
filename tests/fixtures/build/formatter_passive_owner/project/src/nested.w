%%start
std = namespace std "bootstrap.w";
observer = extern wasm "fixture_formatter_owner" {
	unit(i32) checkpoint;
};

unit(bool) nested = fn(active) {
	if (active) {
		u8 value = 203;
		std.print(std.format(value));
		observer.checkpoint(1);
	};
	observer.checkpoint(2);
};

observer.checkpoint(0);
nested(true);
observer.checkpoint(3);
%%end

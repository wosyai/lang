%%start
std = namespace std "bootstrap.w";
observer = extern wasm "fixture_formatter_owner" {
	unit(i32) checkpoint;
};

unit(bool) roundtrip = fn(active) {
	if (active) {
		f64 original = 1.5;
		std.utf8 text = std.format(original);
		observer.checkpoint(41);
		*f64 parsed = std.parse(text);
		observer.checkpoint(42);
		f64 observed = *parsed;
		if (observed != original) { core.system_panic(); };
		observer.checkpoint(43);
	};
	observer.checkpoint(44);
};

observer.checkpoint(40);
roundtrip(true);
observer.checkpoint(45);
%%end

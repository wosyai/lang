%%start
std = namespace std "bootstrap.w";
observer = extern wasm "fixture_formatter_owner" {
	unit(i32) checkpoint;
};

observer.checkpoint(30);
u128 value = 1000000;
std.utf8 text = std.format(value);
std.print(text);
observer.checkpoint(31);
std.exit(0);
%%end

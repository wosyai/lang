%%start
std = namespace std "bootstrap.w";
observer = extern wasm "fixture_formatter_owner" {
	unit(i32) checkpoint;
};

std.utf8(u8) render = fn(value) {
	std.format(value)
};

unit(u8) consume = fn(value) {
	std.utf8 text = render(value);
	observer.checkpoint(51);
	std.print(text);
	observer.checkpoint(52);
};

observer.checkpoint(50);
consume(203);
observer.checkpoint(53);
%%end

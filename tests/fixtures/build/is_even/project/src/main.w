%%start
std = namespace std "bootstrap.w";

*std.utf8 text = null;
std.ReadLineStatus status = std.ReadLineStatus::eof;
text, status = std.read_line(64);
*?u8 content = null;
u64 size = 0;
if (text != null) {
	content = (*text).data;
	size = (*text).length;
};
std.utf8 line = { .data = content; .length = size; };
*u64 value = std.parse(line);
if (status == std.ReadLineStatus::line) {
	if (value == null) {
		std.print("invalid\n");
		std.exit(3);
	};
	if (value != null) {
		u64 number = *value;
		u64 two = 2;
		u64 zero = 0;
		u64 remainder = number % two;
		if (remainder == zero) {
			std.print("even\n");
		};
		if (remainder != zero) {
			std.print("odd\n");
		};
	};
	std.exit(0);
};
if (status == std.ReadLineStatus::eof) {
	std.print("eof\n");
};
if (status == std.ReadLineStatus::io_error) {
	std.print("io_error\n");
};
if (status == std.ReadLineStatus::invalid_utf8) {
	std.print("invalid_utf8\n");
};
if (status == std.ReadLineStatus::limit_exceeded) {
	std.print("limit_exceeded\n");
};
std.exit(3);
%%end

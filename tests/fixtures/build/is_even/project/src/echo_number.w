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
	};
	if (value != null) {
		u64 number = *value;
		std.print(std.format(number));
		std.print("\n");
	};
};
if (status == std.ReadLineStatus::eof) {
	std.print("eof\n");
};
%%end

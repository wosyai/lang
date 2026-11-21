%%start
std = namespace std "bootstrap.w";

*std.utf8 text = null;
std.ReadLineStatus status = std.ReadLineStatus::eof;
u64 zero = 0;
unsafe {
	text, status = std.read_line(zero);
	if (text != null && (*text).data == null && (*text).length == zero && status == std.ReadLineStatus::line) {
		std.print("crlf\n");
	};
	if (text == null && status == std.ReadLineStatus::limit_exceeded) {
		std.print("limit-eof\n");
	};
	if (text == null && status == std.ReadLineStatus::io_error) {
		std.print("io-error\n");
	};
};
%%end

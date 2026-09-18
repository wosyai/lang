%%start
std = namespace std "bootstrap.w";

*std.utf8 text = null;
std.ReadLineStatus status = std.ReadLineStatus::eof;
u64 zero = 0;
unsafe {
	text, status = std.read_line(0);
	if (text != null && (*text).data == null && (*text).length == zero && status == std.ReadLineStatus::line) {
		std.print("empty-lf\n");
	};
};
%%end

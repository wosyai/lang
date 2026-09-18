%%start
std = namespace std "bootstrap.w";

*std.utf8 text = null;
std.ReadLineStatus status = std.ReadLineStatus::line;
unsafe {
	text, status = std.read_line(1);
	if (text == null && status == std.ReadLineStatus::io_error) {
		std.print("discard-io-error\n");
	};
};
%%end

%%start
std = namespace std "bootstrap.w";

*std.utf8 text = null;
std.ReadLineStatus status = std.ReadLineStatus::eof;
unsafe {
	text, status = std.read_line(1);
	if (text == null && status == std.ReadLineStatus::limit_exceeded) {
		std.print("limit-eof\n");
	};
};
%%end

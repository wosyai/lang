%%start
std = namespace std "bootstrap.w";

*std.utf8 text = null;
std.ReadLineStatus status = std.ReadLineStatus::line;
unsafe {
	text, status = std.read_line(4);
	if (text == null && status == std.ReadLineStatus::eof) {
		std.print("empty-eof\n");
	};
};
%%end

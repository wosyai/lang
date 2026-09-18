%%start
std = namespace std "bootstrap.w";

*std.utf8 text = null;
std.ReadLineStatus status = std.ReadLineStatus::line;
u64 max = 4294967296;
unsafe {
	text, status = std.read_line(max);
	if (text == null && status == std.ReadLineStatus::limit_exceeded) {
		std.print("oversized\n");
	};
};
%%end

%%start
std = namespace std "bootstrap.w";

*std.utf8 text = null;
*std.utf8 next = null;
std.ReadLineStatus status = std.ReadLineStatus::eof;
std.ReadLineStatus next_status = std.ReadLineStatus::eof;
unsafe {
	text, status = std.read_line(0);
	next, next_status = std.read_line(1);
	if (text == null && status == std.ReadLineStatus::limit_exceeded) {
		if (next_status == std.ReadLineStatus::line && next != null && (*next).length == 1 && (*next).data != null) {
			u8 byte = core.load<u8>((*next).data);
			if (byte == 120) { std.print("next\n"); };
		};
	};
};
%%end

%%start
std = namespace std "bootstrap.w";

*std.utf8 text = null;
std.ReadLineStatus status = std.ReadLineStatus::eof;
unsafe {
	text, status = std.read_line(2);
	if (text != null && status == std.ReadLineStatus::line) {
		*?u8 data = (*text).data;
		u64 length = (*text).length;
		if (data != null && (length == 1 || length == 2)) {
			u8 first = core.load<u8>(data);
			bool retained = length == 1 && first == 13;
			if (length == 2 && first == 65) {
				u8 second = core.load<u8>(core.offset<u8>(data, 1));
				retained = second == 13;
			};
			if (retained) { std.print("standalone-cr\n"); };
		};
	};
};
%%end

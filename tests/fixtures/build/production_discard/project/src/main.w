%%start
std = namespace std "bootstrap.w";
observer = extern wasm "fixture_discard" {
	unit(i32, i32, *?u8, *?u8) checkpoint;
};

unit(i32, std.ReadLineStatus) discard = fn(which, expected) {
	observer.checkpoint(0, which, null, null);
	*std.utf8 discarded = null;
	std.ReadLineStatus status = std.ReadLineStatus::line;
	discarded, status = std.read_line(0);
	if (discarded != null || status != expected) { core.system_panic(); };
	observer.checkpoint(1, which, null, null);
};

unit() recover = fn {
	*std.utf8 text = null;
	std.ReadLineStatus status = std.ReadLineStatus::eof;
	text, status = std.read_line(8);
	if (status != std.ReadLineStatus::line || text == null) { core.system_panic(); };
	*?u8 bytes = (*text).data;
	if ((*text).length != 2 || bytes == null) { core.system_panic(); };
	unsafe {
		*?u8 slot = core.pointer_cast<*?u8>(&?text);
		observer.checkpoint(2, 0, bytes, slot);
		core.free(bytes);
	};
	observer.checkpoint(3, 0, null, null);
};

discard(0, std.ReadLineStatus::limit_exceeded);
recover();
discard(1, std.ReadLineStatus::limit_exceeded);
discard(2, std.ReadLineStatus::limit_exceeded);
discard(3, std.ReadLineStatus::io_error);
observer.checkpoint(4, 3, null, null);
%%end

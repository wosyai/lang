%%start
std = namespace std "bootstrap.w";

struct Iovec {
	*?u8 data;
	u32 length;
}

struct Nwritten {
	u32 value;
}

wasi = extern wasm "wasi_snapshot_preview1" {
	unsafe i32(i32, *?Iovec, i32, *?Nwritten) fd_write;
};

*std.utf8 text = null;
std.ReadLineStatus status = std.ReadLineStatus::eof;
*?u8 data = null;
u64 length = 0;
Iovec iovec = { .data = null; .length = 0; };
Nwritten written = { .value = 0; };
unsafe {
	text, status = std.read_line(1);
	if (text != null && (*text).length == 1 && status == std.ReadLineStatus::line) {
		data = (*text).data;
		length = (*text).length;
		iovec.data = data;
		iovec.length = core.int_trunc<u32>(length);
		wasi.fd_write(1, &?iovec, 1, &?written);
	};
};
%%end

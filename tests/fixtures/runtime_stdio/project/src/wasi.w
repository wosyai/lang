%%start
struct WasiIovec {
	*?u8 buf;
	u32 len;
}

struct WasiNwritten {
	u32 value;
}

wasi = extern wasm "wasi_snapshot_preview1" {
	unsafe i32(i32, *?WasiIovec, i32, *?WasiNwritten) fd_write;
};

unit() write_stdout = fn {
	unsafe {
		*?u8 bytes, u64 length = core.utf8_view("runtime stdout\n");
		WasiIovec iovec = {
			.buf = bytes;
			.len = core.cast<u32>(length, "exact");
		};
		*?WasiIovec iovec_address = &?iovec;
		WasiNwritten nwritten = { .value = 0; };
		*?WasiNwritten nwritten_address = &?nwritten;
		wasi.fd_write(1, iovec_address, 1, nwritten_address);
	};
};

unit() write_stderr = fn {
	unsafe {
		*?u8 bytes, u64 length = core.utf8_view("runtime stderr\n");
		WasiIovec iovec = {
			.buf = bytes;
			.len = core.cast<u32>(length, "exact");
		};
		*?WasiIovec iovec_address = &?iovec;
		WasiNwritten nwritten = { .value = 0; };
		*?WasiNwritten nwritten_address = &?nwritten;
		wasi.fd_write(2, iovec_address, 1, nwritten_address);
	};
};
%%end

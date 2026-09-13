%%start
std = namespace std "bootstrap.w";
std.utf8 text = "direct root\n";

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

local = namespace app "src/local.w";
i32 value = local.value;
unsafe {
	*?u8 bytes = text.data;
	u64 length = text.length;
	WasiIovec iovec = {
		.buf = bytes;
		.len = core.cast<u32>(length, "exact");
	};
	*?WasiIovec iovec_address = &?iovec;
	WasiNwritten nwritten = { .value = 0; };
	*?WasiNwritten nwritten_address = &?nwritten;
	wasi.fd_write(1, iovec_address, 1, nwritten_address);
};
%%end

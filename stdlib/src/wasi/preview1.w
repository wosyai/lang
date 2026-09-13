%%start
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

i32(i32, *?Iovec, *?Nwritten) fd_write_once = fn(
	descriptor,
	iovec_address,
	byte_count_address
) {
	i32 result = unsafe {
		wasi.fd_write(descriptor, iovec_address, 1, byte_count_address)
	};
	result
};
%%end

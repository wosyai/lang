struct Iovec {
	*?u8 data;
	u32 length;
}

wasi = extern wasm "wasi_snapshot_preview1" {
	unsafe i32(i32, *?Iovec, i32, *?i32) fd_write;
};

unsafe i32(i32, *?Iovec, *?i32) fd_write_once = fn(
	descriptor,
	iovec_address,
	byte_count_address
) {
	wasi.fd_write(descriptor, iovec_address, 1, byte_count_address)
};

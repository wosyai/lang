%%start
wasi = extern wasm "wasi_snapshot_preview1" {
	unsafe i32(i32, utf8) fd_write;
};
%%end

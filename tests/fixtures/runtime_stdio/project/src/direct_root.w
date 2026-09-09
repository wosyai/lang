%%start
wasi = extern wasm "wasi_snapshot_preview1" {
	unsafe i32(i32, utf8) fd_write;
};
local = namespace app "src/local.w";
i32 value = local.value;
i32 wrote = wasi.fd_write(1, "direct root\n");
%%end

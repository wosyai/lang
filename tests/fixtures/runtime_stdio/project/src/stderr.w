%%start
wasi = namespace app "src/wasi.w";
i32 wrote = wasi.fd_write(2, "runtime stderr\n");
%%end

%%start
wasi = namespace app "src/wasi.w";
i32 wrote = wasi.fd_write(1, "runtime stdout\n");
%%end

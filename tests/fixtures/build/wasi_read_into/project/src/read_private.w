%%start
std = namespace std "bootstrap.w";
preview1 = namespace std "wasi/preview1.w";

*?u8 destination = null;
i32 raw_result = unsafe { preview1._fd_read(0, destination, 1, destination) };
u64 raw_count = preview1._fd_read_once(destination, 1);
preview1._ReadIovec iovec = { .data = destination; .length = 1; };
preview1._Nread byte_count = { .value = 0; };

u64 count = 0;
bool complete = false;
count, complete = std.read_into(destination, 0);
%%end

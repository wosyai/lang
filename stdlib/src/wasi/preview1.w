%%start
struct Iovec {
	*?u8 data;
	u32 length;
}

struct Nwritten {
	u32 value;
}

struct _ReadIovec {
	*?u8 data;
	u32 length;
}

struct _Nread {
	u32 value;
}

_wasi = extern wasm "wasi_snapshot_preview1" {
	unsafe i32(i32, *?_ReadIovec, i32, *?_Nread) _fd_read;
	unsafe i32(i32, *?Iovec, i32, *?Nwritten) fd_write;
	unsafe unit(i32) _proc_exit;
};

(u64, bool)(*?u8, u64) _fd_read_once = fn(destination, capacity) {
	_ReadIovec _iovec = {
		.data = destination;
		.length = core.int_trunc<u32>(capacity);
	};
	_Nread _byte_count = { .value = 0; };
	i32 _result = unsafe {
		_wasi._fd_read(0, &?_iovec, 1, &?_byte_count)
	};
	u64 reported = 0;
	bool complete = false;
	if (_result == 0) {
		reported = core.int_extend<u64>(_byte_count.value);
		complete = true;
	} else {
		reported = 0;
		complete = false;
	};
	reported, complete
};

i32(i32, *?Iovec, *?Nwritten) fd_write_once = fn(
	descriptor,
	iovec_address,
	byte_count_address
) {
	i32 result = unsafe {
		_wasi.fd_write(descriptor, iovec_address, 1, byte_count_address)
	};
	result
};

(u64, bool)(*?u8, u64) read_into = fn(destination, capacity) {
	u64 zero = core.int_extend<u64>(0);
	u32 maximum_u32 = 4294967295;
	u64 maximum = core.int_extend<u64>(maximum_u32);
	u64 reported = zero;
	bool complete = false;
	if (capacity == zero) {
		reported = zero;
		complete = true;
	} else {
		if (capacity <= maximum) {
			reported, complete = _fd_read_once(destination, capacity);
		} else {
			reported = zero;
			complete = false;
		};
	};
	reported, complete
};

unit(u32) exit = fn(code) {
	i32 slot = core.int_trunc<i32>(core.int_extend<u64>(code));
	unsafe {
		_wasi._proc_exit(slot);
	};
};
%%end

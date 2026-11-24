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

struct _Subscription {
	u64 userdata;
	u8 clock_tag;
	u8 _pad_1;
	u8 _pad_2;
	u8 _pad_3;
	u8 _pad_4;
	u8 _pad_5;
	u8 _pad_6;
	u8 _pad_7;
	u32 clock_id;
	u8 _pad_8;
	u8 _pad_9;
	u8 _pad_10;
	u8 _pad_11;
	u64 timeout;
	u64 precision;
	u16 flags;
	u8 _pad_12;
	u8 _pad_13;
	u8 _pad_14;
	u8 _pad_15;
	u8 _pad_16;
	u8 _pad_17;
}

struct _Event {
	u64 userdata;
	u16 error;
	u8 event_type;
	u8 _pad_1;
	u8 _pad_2;
	u8 _pad_3;
	u8 _pad_4;
	u8 _pad_5;
	u64 nbytes;
	u16 flags;
	u8 _pad_6;
	u8 _pad_7;
	u8 _pad_8;
	u8 _pad_9;
	u8 _pad_10;
	u8 _pad_11;
}

struct _Nevents {
	u32 value;
}

_wasi = extern wasm "wasi_snapshot_preview1" {
	unsafe i32(i32, *?_ReadIovec, i32, *?_Nread) _fd_read;
	unsafe i32(i32, *?Iovec, i32, *?Nwritten) fd_write;
	unsafe i32(*?_Subscription, *?_Event, i32, *?_Nevents) _poll_oneoff;
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

unit(u64) sleep_ns = fn(nanoseconds) {
	_Subscription sub = {
		.userdata = 0;
		.clock_tag = 0;
		._pad_1 = 0;
		._pad_2 = 0;
		._pad_3 = 0;
		._pad_4 = 0;
		._pad_5 = 0;
		._pad_6 = 0;
		._pad_7 = 0;
		.clock_id = 1;
		._pad_8 = 0;
		._pad_9 = 0;
		._pad_10 = 0;
		._pad_11 = 0;
		.timeout = nanoseconds;
		.precision = 0;
		.flags = 0;
		._pad_12 = 0;
		._pad_13 = 0;
		._pad_14 = 0;
		._pad_15 = 0;
		._pad_16 = 0;
		._pad_17 = 0;
	};
	_Event event = {
		.userdata = 0;
		.error = 0;
		.event_type = 0;
		._pad_1 = 0;
		._pad_2 = 0;
		._pad_3 = 0;
		._pad_4 = 0;
		._pad_5 = 0;
		.nbytes = 0;
		.flags = 0;
		._pad_6 = 0;
		._pad_7 = 0;
		._pad_8 = 0;
		._pad_9 = 0;
		._pad_10 = 0;
		._pad_11 = 0;
	};
	_Nevents fired = { .value = 0; };
	unsafe {
		_wasi._poll_oneoff(&?sub, &?event, 1, &?fired);
	};
};
%%end

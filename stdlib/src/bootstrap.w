%%start
struct utf8 {
	*?u8 data;
	u64 length;
}

preview1 = namespace std "wasi/preview1.w";

(u64, bool)(utf8) print = fn(text) {
	u64 reported = 0;
	u64 zero = core.int_extend<u64>(0);
	bool complete = text.length == zero;
	u64 maximum = core.int_extend<u64>(4294967295);
	i32 success = 0;
	preview1.Iovec iovec = {
		.data = text.data;
		.length = core.int_trunc<u32>(text.length);
	};
	preview1.Nwritten byte_count = { .value = 0; };
	i32 result = 0;
	u32 count = 0;
	if (text.length != zero && text.length <= maximum) {
		unsafe {
			result = preview1.fd_write_once(1, &?iovec, &?byte_count);
		};
		count = byte_count.value;
		if (result == success) {
			reported = core.int_extend<u64>(count);
			complete = reported == text.length;
		} else {
			reported = 0;
			complete = false;
		};
	} else {
		reported = 0;
		complete = text.length == zero;
	};
	reported, complete
};

(u64, bool)(utf8) eprint = fn(text) {
	u64 reported = 0;
	u64 zero = core.int_extend<u64>(0);
	bool complete = text.length == zero;
	u64 maximum = core.int_extend<u64>(4294967295);
	i32 success = 0;
	preview1.Iovec iovec = {
		.data = text.data;
		.length = core.int_trunc<u32>(text.length);
	};
	preview1.Nwritten byte_count = { .value = 0; };
	i32 result = 0;
	u32 count = 0;
	if (text.length != zero && text.length <= maximum) {
		unsafe {
			result = preview1.fd_write_once(2, &?iovec, &?byte_count);
		};
		count = byte_count.value;
		if (result == success) {
			reported = core.int_extend<u64>(count);
			complete = reported == text.length;
		} else {
			reported = 0;
			complete = false;
		};
	} else {
		reported = 0;
		complete = text.length == zero;
	};
	reported, complete
};

(u64, bool)(*?u8, u64) read_into = fn(destination, capacity) {
	preview1.read_into(destination, capacity)
};

enum ReadLineStatus {
	line;
	eof;
	io_error;
	invalid_utf8;
	limit_exceeded;
}

ReadLineStatus(bool, ReadLineStatus) _read_line_eof_status = fn(saw_input, status) {
	if (!saw_input && status == ReadLineStatus::line) { ReadLineStatus::eof } else { status }
};

ReadLineStatus(u64, u64, ReadLineStatus) _read_line_capacity_status = fn(length, maximum, status) {
	if (length >= maximum) { ReadLineStatus::limit_exceeded } else { status }
};

(*utf8, ReadLineStatus)(u64) read_line = fn(max_bytes) {
	u64 zero = core.int_extend<u64>(0);
	u64 one = core.int_extend<u64>(1);
	u64 maximum_u32 = core.int_extend<u64>(4294967295);
	if (max_bytes > maximum_u32) {
		null, ReadLineStatus::limit_exceeded
	} else {
		u8[1] scratch = [0];
		u8[max_bytes] bytes;
		u64 length = zero;
		bool saw_input = false;
		bool pending_cr = false;
		bool reading = true;
		ReadLineStatus status = ReadLineStatus::line;
		while (reading) {
			u64 count = zero;
			bool complete = false;
			unsafe { count, complete = read_into(&?scratch[0], one); };
			if (!complete) { status = ReadLineStatus::io_error; reading = false; };
			if (complete && count == zero) { status = _read_line_eof_status(saw_input, status); reading = false; };
			if (complete && count != zero) { saw_input = true; };
			if (complete && count != zero && scratch[0] == 10) { pending_cr = false; reading = false; };
			if (complete && count != zero && scratch[0] != 10 && pending_cr) {
				status = _read_line_capacity_status(length, max_bytes, status);
				if (length < max_bytes) { bytes[length] = 13; length = length + one; };
			};
			if (complete && count != zero && scratch[0] != 10) { pending_cr = scratch[0] == 13; };
			if (complete && count != zero && scratch[0] != 10 && scratch[0] != 13) {
				status = _read_line_capacity_status(length, max_bytes, status);
				if (length < max_bytes) { bytes[length] = scratch[0]; length = length + one; };
			};
		}
		if (status == ReadLineStatus::line) {
			*?u8 data = null;
			unsafe { data = &?bytes[0]; };
			utf8 text = { .data = data; .length = length; };
			&text, status
		} else { null, status }
	}
};

%%end

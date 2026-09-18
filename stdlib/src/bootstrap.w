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
	u32 maximum_u32 = 4294967295;
	u64 maximum = core.int_extend<u64>(maximum_u32);
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
	u32 maximum_u32 = 4294967295;
	u64 maximum = core.int_extend<u64>(maximum_u32);
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

ReadLineStatus(u8[], u64, ReadLineStatus) _read_line_utf8_status = fn(bytes, length, status) {
	u64 zero = core.int_extend<u64>(0);
	u64 one = core.int_extend<u64>(1);
	u8 continuation_minimum = 128;
	u8 continuation_maximum = 191;
	u8 two_byte_minimum = 194;
	u8 three_byte_minimum = 224;
	u8 four_byte_minimum = 240;
	u8 four_byte_limit = 244;
	u8 e0 = 224;
	u8 ed = 237;
	u8 f0 = 240;
	u8 f4 = 244;
	u64 index = zero;
	u64 remaining = zero;
	u8 next_minimum = continuation_minimum;
	u8 next_maximum = continuation_maximum;
	ReadLineStatus result = status;
	while (index < length) {
		u8 byte = bytes[index];
		if (remaining != zero) {
			if (byte < next_minimum || byte > next_maximum) { result = ReadLineStatus::invalid_utf8; };
			remaining = remaining - one;
			next_minimum = continuation_minimum;
			next_maximum = continuation_maximum;
		} else {
			if (byte >= continuation_minimum && byte < two_byte_minimum) { result = ReadLineStatus::invalid_utf8; };
			if (byte >= two_byte_minimum && byte < three_byte_minimum) { remaining = one; };
			if (byte >= three_byte_minimum && byte < four_byte_minimum) {
				remaining = core.int_extend<u64>(2);
				if (byte == e0) { next_minimum = 160; };
				if (byte == ed) { next_maximum = 159; };
			};
			if (byte >= four_byte_minimum && byte <= four_byte_limit) {
				remaining = core.int_extend<u64>(3);
				if (byte == f0) { next_minimum = 144; };
				if (byte == f4) { next_maximum = 143; };
			};
			if (byte > four_byte_limit) { result = ReadLineStatus::invalid_utf8; };
		};
		index = index + one;
	}
	if (remaining != zero) { result = ReadLineStatus::invalid_utf8; };
	result
};

ReadLineStatus() _read_line_discard = fn {
	u64 zero = core.int_extend<u64>(0);
	u64 one = core.int_extend<u64>(1);
	u8[1] scratch = [0];
	bool reading = true;
	ReadLineStatus status = ReadLineStatus::limit_exceeded;
	while (reading) {
		u64 count = zero;
		bool complete = false;
		unsafe { count, complete = read_into(&?scratch[0], one); };
		if (!complete) { status = ReadLineStatus::io_error; reading = false; };
		if (complete && count == zero) { reading = false; };
		if (complete && count != zero && scratch[0] == 10) { reading = false; };
	}
	status
};

(*utf8, ReadLineStatus)(u64, u8, bool) _read_line_allocated = fn(max_bytes, first, pending_cr) {
	u64 zero = core.int_extend<u64>(0);
	u64 one = core.int_extend<u64>(1);
	u8 lf = 10;
	u8 cr = 13;
	u8[1] scratch = [0];
	u8[max_bytes] bytes;
	u64 length = zero;
	bool reading = true;
	ReadLineStatus status = ReadLineStatus::line;
	if (!pending_cr) { bytes[length] = first; length = length + one; };
	while (reading) {
		u64 count = zero;
		bool complete = false;
		unsafe { count, complete = read_into(&?scratch[0], one); };
		if (!complete) { status = ReadLineStatus::io_error; reading = false; };
		if (complete && count == zero) { reading = false; };
		if (complete && count != zero && scratch[0] == lf) { pending_cr = false; reading = false; };
		if (complete && count != zero && scratch[0] != lf && pending_cr) {
			status = _read_line_capacity_status(length, max_bytes, status);
			if (length < max_bytes) { bytes[length] = cr; length = length + one; };
		};
		if (complete && count != zero && scratch[0] != lf) { pending_cr = scratch[0] == cr; };
		if (complete && count != zero && scratch[0] != lf && scratch[0] != cr) {
			status = _read_line_capacity_status(length, max_bytes, status);
			if (length < max_bytes) { bytes[length] = scratch[0]; length = length + one; };
		};
	}
	if (status == ReadLineStatus::line) { status = _read_line_utf8_status(bytes, length, status); };
	if (status == ReadLineStatus::line) {
		*?u8 data = null;
		if (length != zero) { unsafe { data = &?bytes[0]; }; };
		utf8 text = { .data = data; .length = length; };
		&text, status
	} else { null, status }
};

(*utf8, ReadLineStatus)(u64) read_line = fn(max_bytes) {
	u64 zero = core.int_extend<u64>(0);
	u64 one = core.int_extend<u64>(1);
	u32 maximum_u32_value = 4294967295;
	u64 maximum_u32 = core.int_extend<u64>(maximum_u32_value);
	if (max_bytes > maximum_u32) {
		null, ReadLineStatus::limit_exceeded
	} else {
		u8[1] scratch = [0];
		bool reading = true;
		*utf8 text = null;
		ReadLineStatus status = ReadLineStatus::eof;
		while (reading) {
			u64 count = zero;
			bool complete = false;
			unsafe { count, complete = read_into(&?scratch[0], one); };
			if (!complete) { status = ReadLineStatus::io_error; reading = false; };
			if (complete && count == zero) { status = ReadLineStatus::eof; reading = false; };
			if (complete && count != zero && scratch[0] == 10) {
				utf8 empty = { .data = null; .length = zero; };
				text = &empty;
				status = ReadLineStatus::line;
				reading = false;
			};
			if (complete && count != zero && scratch[0] != 10 && max_bytes == zero) { status = _read_line_discard(); reading = false; };
			if (complete && count != zero && scratch[0] != 10 && max_bytes != zero) {
				text, status = _read_line_allocated(max_bytes, scratch[0], scratch[0] == 13);
				reading = false;
			};
		}
		text, status
	}
};

%%end

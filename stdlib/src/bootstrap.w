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

u8(u8) _parse_digit_value = fn(byte) {
	u8 value = 255;
	u8 zero_digit = 48;
	u8 lower_base = 87;
	u8 upper_base = 55;
	if (byte >= 48 && byte <= 57) { value = byte - zero_digit; };
	if (byte >= 97 && byte <= 102) { value = byte - lower_base; };
	if (byte >= 65 && byte <= 70) { value = byte - upper_base; };
	value
};

bool(u8, u8) _parse_is_digit_valid = fn(byte, radix) {
	u8 value = _parse_digit_value(byte);
	value < radix
};

bool(u8) _parse_is_underscore = fn(byte) {
	byte == 95
};

bool(u8) _parse_is_sign = fn(byte) {
	byte == 45
};

bool(u8) _parse_is_plus = fn(byte) {
	byte == 43
};

bool(u8) _parse_is_zero = fn(byte) {
	byte == 48
};

bool(u8) _parse_is_x = fn(byte) {
	byte == 120 || byte == 88
};

bool(u8) _parse_is_b = fn(byte) {
	byte == 98 || byte == 66
};

bool(u8) _parse_is_o = fn(byte) {
	byte == 111 || byte == 79
};

u8(u8, u8) _parse_radix_from_prefix = fn(first, second) {
	u8 radix = 10;
	bool leading = _parse_is_zero(first);
	if (leading && _parse_is_x(second)) { radix = 16; };
	if (leading && _parse_is_b(second)) { radix = 2; };
	if (leading && _parse_is_o(second)) { radix = 8; };
	radix
};

(u128, bool)(*?u8, u64, u64, u8) _parse_accumulate_u128 = fn(data, start, end, radix) {
	u64 one = core.int_extend<u64>(1);
	u128 result = core.int_extend<u128>(0);
	u128 maximum = 340282366920938463463374607431768211455;
	u64 index = start;
	bool valid = true;
	bool saw_digit = false;
	bool last_underscore = false;
	while (index < end && valid) {
		u128 wide_index = core.int_extend<u128>(index);
		i64 position = core.int_trunc<i64>(wide_index);
		u8 byte = 0;
		unsafe { byte = core.load<u8>(core.offset<u8>(data, position)); };
		bool underscore = _parse_is_underscore(byte);
		u8 digit = _parse_digit_value(byte);
		bool digit_ok = _parse_is_digit_valid(byte, radix);
		if (digit_ok) {
			u128 digit_wide = core.int_extend<u128>(digit);
			u128 radix_wide = core.int_extend<u128>(radix);
			u128 limit = (maximum - digit_wide) / radix_wide;
			if (result > limit) { valid = false; } else { result = result * radix_wide + digit_wide; };
			last_underscore = false;
			saw_digit = true;
		} else {
			if (underscore && index == start) { valid = false; };
			if (underscore && last_underscore) { valid = false; };
			if (underscore) { last_underscore = true; } else { valid = false; };
		};
		index = index + one;
	}
	if (!saw_digit) { valid = false; };
	if (last_underscore) { valid = false; };
	result, valid
};

(u128, bool)(*?u8, u64, u64, u8) _parse_accumulate_i128_neg = fn(data, start, end, radix) {
	u64 one = core.int_extend<u64>(1);
	u128 result = core.int_extend<u128>(0);
	u128 maximum = 170141183460469231731687303715884105728;
	u64 index = start;
	bool valid = true;
	bool saw_digit = false;
	bool last_underscore = false;
	while (index < end && valid) {
		u128 wide_index = core.int_extend<u128>(index);
		i64 position = core.int_trunc<i64>(wide_index);
		u8 byte = 0;
		unsafe { byte = core.load<u8>(core.offset<u8>(data, position)); };
		bool underscore = _parse_is_underscore(byte);
		u8 digit = _parse_digit_value(byte);
		bool digit_ok = _parse_is_digit_valid(byte, radix);
		if (digit_ok) {
			u128 digit_wide = core.int_extend<u128>(digit);
			u128 radix_wide = core.int_extend<u128>(radix);
			u128 limit = (maximum - digit_wide) / radix_wide;
			if (result > limit) { valid = false; } else { result = result * radix_wide + digit_wide; };
			last_underscore = false;
			saw_digit = true;
		} else {
			if (underscore && index == start) { valid = false; };
			if (underscore && last_underscore) { valid = false; };
			if (underscore) { last_underscore = true; } else { valid = false; };
		};
		index = index + one;
	}
	if (!saw_digit) { valid = false; };
	if (last_underscore) { valid = false; };
	result, valid
};

bool(u128) _parse_check_u8 = fn(value) {
	u8 bound = 255;
	u128 limit = core.int_extend<u128>(bound);
	value <= limit
};

bool(u128) _parse_check_u16 = fn(value) {
	u16 bound = 65535;
	u128 limit = core.int_extend<u128>(bound);
	value <= limit
};

bool(u128) _parse_check_u32 = fn(value) {
	u32 bound = 4294967295;
	u128 limit = core.int_extend<u128>(bound);
	value <= limit
};

bool(u128) _parse_check_u64 = fn(value) {
	u64 bound = 18446744073709551615;
	u128 limit = core.int_extend<u128>(bound);
	value <= limit
};

bool(u128) _parse_check_u128 = fn(value) {
	u128 zero = core.int_extend<u128>(0);
	value >= zero
};

bool(u128, bool) _parse_check_i8 = fn(magnitude, negative) {
	u8 positive_bound = 127;
	u8 negative_bound = 128;
	u128 positive_limit = core.int_extend<u128>(positive_bound);
	u128 negative_limit = core.int_extend<u128>(negative_bound);
	bool fits = false;
	if (!negative && magnitude <= positive_limit) { fits = true; };
	if (negative && magnitude <= negative_limit) { fits = true; };
	fits
};

bool(u128, bool) _parse_check_i16 = fn(magnitude, negative) {
	u16 positive_bound = 32767;
	u16 negative_bound = 32768;
	u128 positive_limit = core.int_extend<u128>(positive_bound);
	u128 negative_limit = core.int_extend<u128>(negative_bound);
	bool fits = false;
	if (!negative && magnitude <= positive_limit) { fits = true; };
	if (negative && magnitude <= negative_limit) { fits = true; };
	fits
};

bool(u128, bool) _parse_check_i32 = fn(magnitude, negative) {
	u32 positive_bound = 2147483647;
	u32 negative_bound = 2147483648;
	u128 positive_limit = core.int_extend<u128>(positive_bound);
	u128 negative_limit = core.int_extend<u128>(negative_bound);
	bool fits = false;
	if (!negative && magnitude <= positive_limit) { fits = true; };
	if (negative && magnitude <= negative_limit) { fits = true; };
	fits
};

bool(u128, bool) _parse_check_i64 = fn(magnitude, negative) {
	u64 positive_bound = 9223372036854775807;
	u64 negative_bound = 9223372036854775808;
	u128 positive_limit = core.int_extend<u128>(positive_bound);
	u128 negative_limit = core.int_extend<u128>(negative_bound);
	bool fits = false;
	if (!negative && magnitude <= positive_limit) { fits = true; };
	if (negative && magnitude <= negative_limit) { fits = true; };
	fits
};

bool(u128, bool) _parse_check_i128 = fn(magnitude, negative) {
	u128 positive_limit = 170141183460469231731687303715884105727;
	u128 negative_limit = 170141183460469231731687303715884105728;
	bool fits = false;
	if (!negative && magnitude <= positive_limit) { fits = true; };
	if (negative && magnitude <= negative_limit) { fits = true; };
	fits
};

%%end

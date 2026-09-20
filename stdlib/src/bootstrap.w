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

*u8(utf8) _parse_u8 = fn(text) {
	u64 zero = core.int_extend<u64>(0);
	u64 one = core.int_extend<u64>(1);
	u64 two = core.int_extend<u64>(2);
	u8 radix = 10;
	u64 start = zero;
	u8 out = 0;
	*u8 result = null;
	if (text.data == null || text.length == zero) {
		result = null;
	} else {
		u128 wide_zero = core.int_extend<u128>(zero);
		i64 position_zero = core.int_trunc<i64>(wide_zero);
		u8 first = 0;
		unsafe { first = core.load<u8>(core.offset<u8>(text.data, position_zero)); };
		if (text.length >= two) {
			u128 wide_one = core.int_extend<u128>(one);
			i64 position_one = core.int_trunc<i64>(wide_one);
			u8 second = 0;
			unsafe { second = core.load<u8>(core.offset<u8>(text.data, position_one)); };
			u8 detected = _parse_radix_from_prefix(first, second);
			radix = detected;
			if (detected != 10) { start = two; };
		};
		u128 magnitude, bool valid = _parse_accumulate_u128(text.data, start, text.length, radix);
		bool fits = _parse_check_u8(magnitude);
		if (valid && fits) {
			out = core.int_trunc<u8>(magnitude);
			result = &out;
		};
	};
	result
};

*u16(utf8) _parse_u16 = fn(text) {
	u64 zero = core.int_extend<u64>(0);
	u64 one = core.int_extend<u64>(1);
	u64 two = core.int_extend<u64>(2);
	u8 radix = 10;
	u64 start = zero;
	u16 out = 0;
	*u16 result = null;
	if (text.data == null || text.length == zero) {
		result = null;
	} else {
		u128 wide_zero = core.int_extend<u128>(zero);
		i64 position_zero = core.int_trunc<i64>(wide_zero);
		u8 first = 0;
		unsafe { first = core.load<u8>(core.offset<u8>(text.data, position_zero)); };
		if (text.length >= two) {
			u128 wide_one = core.int_extend<u128>(one);
			i64 position_one = core.int_trunc<i64>(wide_one);
			u8 second = 0;
			unsafe { second = core.load<u8>(core.offset<u8>(text.data, position_one)); };
			u8 detected = _parse_radix_from_prefix(first, second);
			radix = detected;
			if (detected != 10) { start = two; };
		};
		u128 magnitude, bool valid = _parse_accumulate_u128(text.data, start, text.length, radix);
		bool fits = _parse_check_u16(magnitude);
		if (valid && fits) {
			out = core.int_trunc<u16>(magnitude);
			result = &out;
		};
	};
	result
};

*u32(utf8) _parse_u32 = fn(text) {
	u64 zero = core.int_extend<u64>(0);
	u64 one = core.int_extend<u64>(1);
	u64 two = core.int_extend<u64>(2);
	u8 radix = 10;
	u64 start = zero;
	u32 out = 0;
	*u32 result = null;
	if (text.data == null || text.length == zero) {
		result = null;
	} else {
		u128 wide_zero = core.int_extend<u128>(zero);
		i64 position_zero = core.int_trunc<i64>(wide_zero);
		u8 first = 0;
		unsafe { first = core.load<u8>(core.offset<u8>(text.data, position_zero)); };
		if (text.length >= two) {
			u128 wide_one = core.int_extend<u128>(one);
			i64 position_one = core.int_trunc<i64>(wide_one);
			u8 second = 0;
			unsafe { second = core.load<u8>(core.offset<u8>(text.data, position_one)); };
			u8 detected = _parse_radix_from_prefix(first, second);
			radix = detected;
			if (detected != 10) { start = two; };
		};
		u128 magnitude, bool valid = _parse_accumulate_u128(text.data, start, text.length, radix);
		bool fits = _parse_check_u32(magnitude);
		if (valid && fits) {
			out = core.int_trunc<u32>(magnitude);
			result = &out;
		};
	};
	result
};

*u64(utf8) _parse_u64 = fn(text) {
	u64 zero = core.int_extend<u64>(0);
	u64 one = core.int_extend<u64>(1);
	u64 two = core.int_extend<u64>(2);
	u8 radix = 10;
	u64 start = zero;
	u64 out = zero;
	*u64 result = null;
	if (text.data == null || text.length == zero) {
		result = null;
	} else {
		u128 wide_zero = core.int_extend<u128>(zero);
		i64 position_zero = core.int_trunc<i64>(wide_zero);
		u8 first = 0;
		unsafe { first = core.load<u8>(core.offset<u8>(text.data, position_zero)); };
		if (text.length >= two) {
			u128 wide_one = core.int_extend<u128>(one);
			i64 position_one = core.int_trunc<i64>(wide_one);
			u8 second = 0;
			unsafe { second = core.load<u8>(core.offset<u8>(text.data, position_one)); };
			u8 detected = _parse_radix_from_prefix(first, second);
			radix = detected;
			if (detected != 10) { start = two; };
		};
		u128 magnitude, bool valid = _parse_accumulate_u128(text.data, start, text.length, radix);
		bool fits = _parse_check_u64(magnitude);
		if (valid && fits) {
			out = core.int_trunc<u64>(magnitude);
			result = &out;
		};
	};
	result
};

*u128(utf8) _parse_u128 = fn(text) {
	u64 zero = core.int_extend<u64>(0);
	u64 one = core.int_extend<u64>(1);
	u64 two = core.int_extend<u64>(2);
	u8 radix = 10;
	u64 start = zero;
	u128 out = 0;
	*u128 result = null;
	if (text.data == null || text.length == zero) {
		result = null;
	} else {
		u128 wide_zero = core.int_extend<u128>(zero);
		i64 position_zero = core.int_trunc<i64>(wide_zero);
		u8 first = 0;
		unsafe { first = core.load<u8>(core.offset<u8>(text.data, position_zero)); };
		if (text.length >= two) {
			u128 wide_one = core.int_extend<u128>(one);
			i64 position_one = core.int_trunc<i64>(wide_one);
			u8 second = 0;
			unsafe { second = core.load<u8>(core.offset<u8>(text.data, position_one)); };
			u8 detected = _parse_radix_from_prefix(first, second);
			radix = detected;
			if (detected != 10) { start = two; };
		};
		u128 magnitude, bool valid = _parse_accumulate_u128(text.data, start, text.length, radix);
		bool fits = _parse_check_u128(magnitude);
		if (valid && fits) {
			out = magnitude;
			result = &out;
		};
	};
	result
};

*i8(utf8) _parse_i8 = fn(text) {
	u64 zero = core.int_extend<u64>(0);
	u64 one = core.int_extend<u64>(1);
	u64 two = core.int_extend<u64>(2);
	u8 radix = 10;
	u64 start = zero;
	bool negative = false;
	u128 zero_wide = core.int_extend<u128>(zero);
	i8 out = 0;
	*i8 result = null;
	if (text.data == null || text.length == zero) {
		result = null;
	} else {
		u128 wide_zero = core.int_extend<u128>(zero);
		i64 position_zero = core.int_trunc<i64>(wide_zero);
		u8 first = 0;
		unsafe { first = core.load<u8>(core.offset<u8>(text.data, position_zero)); };
		if (_parse_is_sign(first)) {
			negative = true;
			start = one;
		};
		u64 rest = text.length - start;
		if (rest >= two) {
			u64 next = start + one;
			u128 wide_start = core.int_extend<u128>(start);
			i64 position_start = core.int_trunc<i64>(wide_start);
			u128 wide_next = core.int_extend<u128>(next);
			i64 position_next = core.int_trunc<i64>(wide_next);
			u8 prefix_first = 0;
			unsafe { prefix_first = core.load<u8>(core.offset<u8>(text.data, position_start)); };
			u8 prefix_second = 0;
			unsafe { prefix_second = core.load<u8>(core.offset<u8>(text.data, position_next)); };
			u8 detected = _parse_radix_from_prefix(prefix_first, prefix_second);
			radix = detected;
			if (detected != 10) { start = start + two; };
		};
		u128 magnitude, bool valid = _parse_accumulate_i128_neg(text.data, start, text.length, radix);
		bool fits = _parse_check_i8(magnitude, negative);
		if (valid && fits) {
			if (negative) {
				u128 flipped = zero_wide - magnitude;
				out = core.int_trunc<i8>(flipped);
			} else {
				out = core.int_trunc<i8>(magnitude);
			};
			result = &out;
		};
	};
	result
};

*i16(utf8) _parse_i16 = fn(text) {
	u64 zero = core.int_extend<u64>(0);
	u64 one = core.int_extend<u64>(1);
	u64 two = core.int_extend<u64>(2);
	u8 radix = 10;
	u64 start = zero;
	bool negative = false;
	u128 zero_wide = core.int_extend<u128>(zero);
	i16 out = 0;
	*i16 result = null;
	if (text.data == null || text.length == zero) {
		result = null;
	} else {
		u128 wide_zero = core.int_extend<u128>(zero);
		i64 position_zero = core.int_trunc<i64>(wide_zero);
		u8 first = 0;
		unsafe { first = core.load<u8>(core.offset<u8>(text.data, position_zero)); };
		if (_parse_is_sign(first)) {
			negative = true;
			start = one;
		};
		u64 rest = text.length - start;
		if (rest >= two) {
			u64 next = start + one;
			u128 wide_start = core.int_extend<u128>(start);
			i64 position_start = core.int_trunc<i64>(wide_start);
			u128 wide_next = core.int_extend<u128>(next);
			i64 position_next = core.int_trunc<i64>(wide_next);
			u8 prefix_first = 0;
			unsafe { prefix_first = core.load<u8>(core.offset<u8>(text.data, position_start)); };
			u8 prefix_second = 0;
			unsafe { prefix_second = core.load<u8>(core.offset<u8>(text.data, position_next)); };
			u8 detected = _parse_radix_from_prefix(prefix_first, prefix_second);
			radix = detected;
			if (detected != 10) { start = start + two; };
		};
		u128 magnitude, bool valid = _parse_accumulate_i128_neg(text.data, start, text.length, radix);
		bool fits = _parse_check_i16(magnitude, negative);
		if (valid && fits) {
			if (negative) {
				u128 flipped = zero_wide - magnitude;
				out = core.int_trunc<i16>(flipped);
			} else {
				out = core.int_trunc<i16>(magnitude);
			};
			result = &out;
		};
	};
	result
};

*i32(utf8) _parse_i32 = fn(text) {
	u64 zero = core.int_extend<u64>(0);
	u64 one = core.int_extend<u64>(1);
	u64 two = core.int_extend<u64>(2);
	u8 radix = 10;
	u64 start = zero;
	bool negative = false;
	u128 zero_wide = core.int_extend<u128>(zero);
	i32 out = 0;
	*i32 result = null;
	if (text.data == null || text.length == zero) {
		result = null;
	} else {
		u128 wide_zero = core.int_extend<u128>(zero);
		i64 position_zero = core.int_trunc<i64>(wide_zero);
		u8 first = 0;
		unsafe { first = core.load<u8>(core.offset<u8>(text.data, position_zero)); };
		if (_parse_is_sign(first)) {
			negative = true;
			start = one;
		};
		u64 rest = text.length - start;
		if (rest >= two) {
			u64 next = start + one;
			u128 wide_start = core.int_extend<u128>(start);
			i64 position_start = core.int_trunc<i64>(wide_start);
			u128 wide_next = core.int_extend<u128>(next);
			i64 position_next = core.int_trunc<i64>(wide_next);
			u8 prefix_first = 0;
			unsafe { prefix_first = core.load<u8>(core.offset<u8>(text.data, position_start)); };
			u8 prefix_second = 0;
			unsafe { prefix_second = core.load<u8>(core.offset<u8>(text.data, position_next)); };
			u8 detected = _parse_radix_from_prefix(prefix_first, prefix_second);
			radix = detected;
			if (detected != 10) { start = start + two; };
		};
		u128 magnitude, bool valid = _parse_accumulate_i128_neg(text.data, start, text.length, radix);
		bool fits = _parse_check_i32(magnitude, negative);
		if (valid && fits) {
			if (negative) {
				u128 flipped = zero_wide - magnitude;
				out = core.int_trunc<i32>(flipped);
			} else {
				out = core.int_trunc<i32>(magnitude);
			};
			result = &out;
		};
	};
	result
};

*i64(utf8) _parse_i64 = fn(text) {
	u64 zero = core.int_extend<u64>(0);
	u64 one = core.int_extend<u64>(1);
	u64 two = core.int_extend<u64>(2);
	u8 radix = 10;
	u64 start = zero;
	bool negative = false;
	u128 zero_wide = core.int_extend<u128>(zero);
	i64 out = 0;
	*i64 result = null;
	if (text.data == null || text.length == zero) {
		result = null;
	} else {
		u128 wide_zero = core.int_extend<u128>(zero);
		i64 position_zero = core.int_trunc<i64>(wide_zero);
		u8 first = 0;
		unsafe { first = core.load<u8>(core.offset<u8>(text.data, position_zero)); };
		if (_parse_is_sign(first)) {
			negative = true;
			start = one;
		};
		u64 rest = text.length - start;
		if (rest >= two) {
			u64 next = start + one;
			u128 wide_start = core.int_extend<u128>(start);
			i64 position_start = core.int_trunc<i64>(wide_start);
			u128 wide_next = core.int_extend<u128>(next);
			i64 position_next = core.int_trunc<i64>(wide_next);
			u8 prefix_first = 0;
			unsafe { prefix_first = core.load<u8>(core.offset<u8>(text.data, position_start)); };
			u8 prefix_second = 0;
			unsafe { prefix_second = core.load<u8>(core.offset<u8>(text.data, position_next)); };
			u8 detected = _parse_radix_from_prefix(prefix_first, prefix_second);
			radix = detected;
			if (detected != 10) { start = start + two; };
		};
		u128 magnitude, bool valid = _parse_accumulate_i128_neg(text.data, start, text.length, radix);
		bool fits = _parse_check_i64(magnitude, negative);
		if (valid && fits) {
			if (negative) {
				u128 flipped = zero_wide - magnitude;
				out = core.int_trunc<i64>(flipped);
			} else {
				out = core.int_trunc<i64>(magnitude);
			};
			result = &out;
		};
	};
	result
};

*i128(utf8) _parse_i128 = fn(text) {
	u64 zero = core.int_extend<u64>(0);
	u64 one = core.int_extend<u64>(1);
	u64 two = core.int_extend<u64>(2);
	u8 radix = 10;
	u64 start = zero;
	bool negative = false;
	u128 two64 = 18446744073709551616;
	i128 two64_signed = 18446744073709551616;
	i128 zero_signed = core.int_extend<i128>(zero);
	i128 out = 0;
	*i128 result = null;
	if (text.data == null || text.length == zero) {
		result = null;
	} else {
		u128 wide_zero = core.int_extend<u128>(zero);
		i64 position_zero = core.int_trunc<i64>(wide_zero);
		u8 first = 0;
		unsafe { first = core.load<u8>(core.offset<u8>(text.data, position_zero)); };
		if (_parse_is_sign(first)) {
			negative = true;
			start = one;
		};
		u64 rest = text.length - start;
		if (rest >= two) {
			u64 next = start + one;
			u128 wide_start = core.int_extend<u128>(start);
			i64 position_start = core.int_trunc<i64>(wide_start);
			u128 wide_next = core.int_extend<u128>(next);
			i64 position_next = core.int_trunc<i64>(wide_next);
			u8 prefix_first = 0;
			unsafe { prefix_first = core.load<u8>(core.offset<u8>(text.data, position_start)); };
			u8 prefix_second = 0;
			unsafe { prefix_second = core.load<u8>(core.offset<u8>(text.data, position_next)); };
			u8 detected = _parse_radix_from_prefix(prefix_first, prefix_second);
			radix = detected;
			if (detected != 10) { start = start + two; };
		};
		u128 magnitude, bool valid = _parse_accumulate_i128_neg(text.data, start, text.length, radix);
		bool fits = _parse_check_i128(magnitude, negative);
		if (valid && fits) {
			u64 lo64 = core.int_trunc<u64>(magnitude);
			u128 hi_wide = magnitude / two64;
			u64 hi64 = core.int_trunc<u64>(hi_wide);
			i128 lo_ext = core.int_extend<i128>(lo64);
			i128 hi_ext = core.int_extend<i128>(hi64);
			if (negative) {
				i128 neg_hi = zero_signed - hi_ext;
				out = neg_hi * two64_signed - lo_ext;
			} else {
				out = hi_ext * two64_signed + lo_ext;
			};
			result = &out;
		};
	};
	result
};

*f32(utf8) _parse_f32 = fn(text) {
	u64 zero = core.int_extend<u64>(0);
	*f32 result = null;
	if (text.data == null || text.length == zero) {
		result = null;
	} else {
		result = null;
	};
	result
};

*f64(utf8) _parse_f64 = fn(text) {
	u64 zero = core.int_extend<u64>(0);
	*f64 result = null;
	if (text.data == null || text.length == zero) {
		result = null;
	} else {
		result = null;
	};
	result
};

parse = overload {
	*u8(utf8) => fn(text) { _parse_u8(text) };
	*u16(utf8) => fn(text) { _parse_u16(text) };
	*u32(utf8) => fn(text) { _parse_u32(text) };
	*u64(utf8) => fn(text) { _parse_u64(text) };
	*u128(utf8) => fn(text) { _parse_u128(text) };
	*i8(utf8) => fn(text) { _parse_i8(text) };
	*i16(utf8) => fn(text) { _parse_i16(text) };
	*i32(utf8) => fn(text) { _parse_i32(text) };
	*i64(utf8) => fn(text) { _parse_i64(text) };
	*i128(utf8) => fn(text) { _parse_i128(text) };
	*f32(utf8) => fn(text) { _parse_f32(text) };
	*f64(utf8) => fn(text) { _parse_f64(text) };
};

utf8(bool) _format_bool = fn(value) {
	u8[5] bytes;
	bytes[0] = 102;
	bytes[1] = 97;
	bytes[2] = 108;
	bytes[3] = 115;
	bytes[4] = 101;
	u64 length = core.int_extend<u64>(5);
	if (value) {
		bytes[0] = 116;
		bytes[1] = 114;
		bytes[2] = 117;
		bytes[3] = 101;
		length = core.int_extend<u64>(4);
	};
	*?u8 data = null;
	unsafe { data = &?bytes[0]; };
	utf8 text = { .data = data; .length = length; };
	text
};

utf8(i8) _format_i8 = fn(value) {
	u8[4] scratch;
	u64 capacity = core.int_extend<u64>(4);
	u64 one = core.int_extend<u64>(1);
	u64 zero_count = core.int_extend<u64>(0);
	i8 zero = 0;
	i8 ten = 10;
	u8 zero_digit = 48;
	u8 minus = 45;
	u64 index = capacity;
	i8 remaining = value;
	bool negative = remaining < zero;
	if (remaining == zero) {
		index = index - one;
		scratch[index] = zero_digit;
	};
	while (remaining != zero) {
		i8 digit_signed = remaining % ten;
		if (negative) { digit_signed = zero - digit_signed; };
		u16 wide_digit = core.int_extend<u16>(digit_signed);
		u8 digit = core.int_trunc<u8>(wide_digit);
		index = index - one;
		scratch[index] = zero_digit + digit;
		remaining = remaining / ten;
	}
	if (negative) {
		index = index - one;
		scratch[index] = minus;
	};
	u64 length = capacity - index;
	u64 target = zero_count;
	u64 source = index;
	while (source < capacity) {
		scratch[target] = scratch[source];
		target = target + one;
		source = source + one;
	}
	*?u8 data = null;
	unsafe { data = &?scratch[0]; };
	utf8 text = { .data = data; .length = length; };
	text
};

utf8(i16) _format_i16 = fn(value) {
	u8[6] scratch;
	u64 capacity = core.int_extend<u64>(6);
	u64 one = core.int_extend<u64>(1);
	u64 zero_count = core.int_extend<u64>(0);
	i16 zero = 0;
	i16 ten = 10;
	u8 zero_digit = 48;
	u8 minus = 45;
	u64 index = capacity;
	i16 remaining = value;
	bool negative = remaining < zero;
	if (remaining == zero) {
		index = index - one;
		scratch[index] = zero_digit;
	};
	while (remaining != zero) {
		i16 digit_signed = remaining % ten;
		if (negative) { digit_signed = zero - digit_signed; };
		u8 digit = core.int_trunc<u8>(digit_signed);
		index = index - one;
		scratch[index] = zero_digit + digit;
		remaining = remaining / ten;
	}
	if (negative) {
		index = index - one;
		scratch[index] = minus;
	};
	u64 length = capacity - index;
	u64 target = zero_count;
	u64 source = index;
	while (source < capacity) {
		scratch[target] = scratch[source];
		target = target + one;
		source = source + one;
	}
	*?u8 data = null;
	unsafe { data = &?scratch[0]; };
	utf8 text = { .data = data; .length = length; };
	text
};

utf8(i32) _format_i32 = fn(value) {
	u8[11] scratch;
	u64 capacity = core.int_extend<u64>(11);
	u64 one = core.int_extend<u64>(1);
	u64 zero_count = core.int_extend<u64>(0);
	i32 zero = 0;
	i32 ten = 10;
	u8 zero_digit = 48;
	u8 minus = 45;
	u64 index = capacity;
	i32 remaining = value;
	bool negative = remaining < zero;
	if (remaining == zero) {
		index = index - one;
		scratch[index] = zero_digit;
	};
	while (remaining != zero) {
		i32 digit_signed = remaining % ten;
		if (negative) { digit_signed = zero - digit_signed; };
		u8 digit = core.int_trunc<u8>(digit_signed);
		index = index - one;
		scratch[index] = zero_digit + digit;
		remaining = remaining / ten;
	}
	if (negative) {
		index = index - one;
		scratch[index] = minus;
	};
	u64 length = capacity - index;
	u64 target = zero_count;
	u64 source = index;
	while (source < capacity) {
		scratch[target] = scratch[source];
		target = target + one;
		source = source + one;
	}
	*?u8 data = null;
	unsafe { data = &?scratch[0]; };
	utf8 text = { .data = data; .length = length; };
	text
};

utf8(i64) _format_i64 = fn(value) {
	u8[20] scratch;
	u64 capacity = core.int_extend<u64>(20);
	u64 one = core.int_extend<u64>(1);
	u64 zero_count = core.int_extend<u64>(0);
	i64 zero = 0;
	i64 ten = 10;
	u8 zero_digit = 48;
	u8 minus = 45;
	u64 index = capacity;
	i64 remaining = value;
	bool negative = remaining < zero;
	if (remaining == zero) {
		index = index - one;
		scratch[index] = zero_digit;
	};
	while (remaining != zero) {
		i64 digit_signed = remaining % ten;
		if (negative) { digit_signed = zero - digit_signed; };
		u8 digit = core.int_trunc<u8>(digit_signed);
		index = index - one;
		scratch[index] = zero_digit + digit;
		remaining = remaining / ten;
	}
	if (negative) {
		index = index - one;
		scratch[index] = minus;
	};
	u64 length = capacity - index;
	u64 target = zero_count;
	u64 source = index;
	while (source < capacity) {
		scratch[target] = scratch[source];
		target = target + one;
		source = source + one;
	}
	*?u8 data = null;
	unsafe { data = &?scratch[0]; };
	utf8 text = { .data = data; .length = length; };
	text
};

utf8(i128) _format_i128 = fn(value) {
	u8[40] scratch;
	u64 digits = core.int_extend<u64>(39);
	u64 one = core.int_extend<u64>(1);
	u64 zero_count = core.int_extend<u64>(0);
	u64 last = core.int_extend<u64>(38);
	u8 zero_digit = 48;
	u8 one_digit = 1;
	u8 minus = 45;
	i128 zero = core.int_extend<i128>(0);
	i128 one_wide = core.int_extend<i128>(1);
	i128 minimum = -170141183460469231731687303715884105728;
	u64 emit = zero_count;
	i128 remaining = value;
	bool negative = value < zero;
	if (negative) {
		scratch[0] = minus;
		emit = one;
		if (value == minimum) { remaining = zero - (value + 1); } else { remaining = zero - value; };
	};
	bool started = false;
	u64 position = zero_count;
	while (position < digits) {
		u64 steps = last - position;
		i128 scale = one_wide;
		u64 step = zero_count;
		while (step < steps) {
			i128 twice = scale + scale;
			i128 fourth = twice + twice;
			i128 eighth = fourth + fourth;
			scale = eighth + twice;
			step = step + one;
		}
		u8 digit = zero_digit;
		while (remaining >= scale) {
			remaining = remaining - scale;
			digit = digit + one_digit;
		}
		if (started || digit != zero_digit || position == last) {
			scratch[emit] = digit;
			emit = emit + one;
			started = true;
		};
		position = position + one;
	}
	if (value == minimum) {
		scratch[emit - one] = 56;
	};
	*?u8 data = null;
	unsafe { data = &?scratch[0]; };
	utf8 text = { .data = data; .length = emit; };
	text
};

utf8(u8) _format_u8 = fn(value) {
	u8[3] scratch;
	u64 capacity = core.int_extend<u64>(3);
	u64 one = core.int_extend<u64>(1);
	u64 zero = core.int_extend<u64>(0);
	u8 ten = 10;
	u8 zero_digit = 48;
	u8 zero_value = 0;
	u64 index = capacity;
	u8 remaining = value;
	if (remaining == zero_value) {
		index = index - one;
		scratch[index] = zero_digit;
	};
	while (remaining != zero_value) {
		u8 digit = remaining % ten;
		index = index - one;
		scratch[index] = zero_digit + digit;
		remaining = remaining / ten;
	}
	u64 length = capacity - index;
	u64 target = zero;
	u64 source = index;
	while (source < capacity) {
		scratch[target] = scratch[source];
		target = target + one;
		source = source + one;
	}
	*?u8 data = null;
	unsafe { data = &?scratch[0]; };
	utf8 text = { .data = data; .length = length; };
	text
};

utf8(u16) _format_u16 = fn(value) {
	u8[5] scratch;
	u64 capacity = core.int_extend<u64>(5);
	u64 one = core.int_extend<u64>(1);
	u64 zero = core.int_extend<u64>(0);
	u16 ten = 10;
	u8 zero_digit = 48;
	u16 zero_value = 0;
	u64 index = capacity;
	u16 remaining = value;
	if (remaining == zero_value) {
		index = index - one;
		scratch[index] = zero_digit;
	};
	while (remaining != zero_value) {
		u16 digit_wide = remaining % ten;
		u8 digit = core.int_trunc<u8>(digit_wide);
		index = index - one;
		scratch[index] = zero_digit + digit;
		remaining = remaining / ten;
	}
	u64 length = capacity - index;
	u64 target = zero;
	u64 source = index;
	while (source < capacity) {
		scratch[target] = scratch[source];
		target = target + one;
		source = source + one;
	}
	*?u8 data = null;
	unsafe { data = &?scratch[0]; };
	utf8 text = { .data = data; .length = length; };
	text
};

utf8(u32) _format_u32 = fn(value) {
	u8[10] scratch;
	u64 capacity = core.int_extend<u64>(10);
	u64 one = core.int_extend<u64>(1);
	u64 zero = core.int_extend<u64>(0);
	u32 ten = 10;
	u8 zero_digit = 48;
	u32 zero_value = 0;
	u64 index = capacity;
	u32 remaining = value;
	if (remaining == zero_value) {
		index = index - one;
		scratch[index] = zero_digit;
	};
	while (remaining != zero_value) {
		u32 digit_wide = remaining % ten;
		u8 digit = core.int_trunc<u8>(digit_wide);
		index = index - one;
		scratch[index] = zero_digit + digit;
		remaining = remaining / ten;
	}
	u64 length = capacity - index;
	u64 target = zero;
	u64 source = index;
	while (source < capacity) {
		scratch[target] = scratch[source];
		target = target + one;
		source = source + one;
	}
	*?u8 data = null;
	unsafe { data = &?scratch[0]; };
	utf8 text = { .data = data; .length = length; };
	text
};

utf8(u64) _format_u64 = fn(value) {
	u8[20] scratch;
	u64 capacity = core.int_extend<u64>(20);
	u64 one = core.int_extend<u64>(1);
	u64 zero = core.int_extend<u64>(0);
	u64 ten = 10;
	u8 zero_digit = 48;
	u64 zero_value = 0;
	u64 index = capacity;
	u64 remaining = value;
	if (remaining == zero_value) {
		index = index - one;
		scratch[index] = zero_digit;
	};
	while (remaining != zero_value) {
		u64 digit_wide = remaining % ten;
		u8 digit = core.int_trunc<u8>(digit_wide);
		index = index - one;
		scratch[index] = zero_digit + digit;
		remaining = remaining / ten;
	}
	u64 length = capacity - index;
	u64 target = zero;
	u64 source = index;
	while (source < capacity) {
		scratch[target] = scratch[source];
		target = target + one;
		source = source + one;
	}
	*?u8 data = null;
	unsafe { data = &?scratch[0]; };
	utf8 text = { .data = data; .length = length; };
	text
};

utf8(u128) _format_u128 = fn(value) {
	u8[39] scratch;
	u64 capacity = core.int_extend<u64>(39);
	u64 one = core.int_extend<u64>(1);
	u64 zero = core.int_extend<u64>(0);
	u64 last = core.int_extend<u64>(38);
	u8 zero_digit = 48;
	u8 one_digit = 1;
	u128 one_wide = core.int_extend<u128>(1);
	u64 emit = zero;
	u128 remaining = value;
	bool started = false;
	u64 position = zero;
	while (position < capacity) {
		u64 steps = last - position;
		u128 scale = one_wide;
		u64 step = zero;
		while (step < steps) {
			u128 twice = scale + scale;
			u128 fourth = twice + twice;
			u128 eighth = fourth + fourth;
			scale = eighth + twice;
			step = step + one;
		}
		u8 digit = zero_digit;
		while (remaining >= scale) {
			remaining = remaining - scale;
			digit = digit + one_digit;
		}
		if (started || digit != zero_digit || position == last) {
			scratch[emit] = digit;
			emit = emit + one;
			started = true;
		};
		position = position + one;
	}
	*?u8 data = null;
	unsafe { data = &?scratch[0]; };
	utf8 text = { .data = data; .length = emit; };
	text
};

utf8(f32) _format_f32 = fn(value) {
	u8[15] scratch;
	u64 one = core.int_extend<u64>(1);
	u64 zero = core.int_extend<u64>(0);
	u64 nine = core.int_extend<u64>(9);
	f64 zero_value = 0.0;
	f64 one_value = 1.0;
	f64 ten_value = 10.0;
	f64 five_value = 5.0;
	u8 zero_digit = 48;
	u8 one_digit = 1;
	u8 nine_digit = 57;
	u8 one_char = 49;
	u8 minus = 45;
	u8 point = 46;
	u8 exponent_mark = 101;
	i32 zero_count = 0;
	i32 one_count = 1;
	f64 wide = core.float_extend<f64>(value);
	bool negative = wide < zero_value;
	f64 magnitude = wide;
	if (negative) { magnitude = zero_value - wide; };
	u64 emit = zero;
	if (negative) {
		scratch[0] = minus;
		emit = one;
	};
	if (magnitude == zero_value) {
		f64 probe = one_value / magnitude;
		if (probe < zero_value) {
			scratch[emit] = minus;
			emit = emit + one;
		};
		scratch[emit] = zero_digit;
		emit = emit + one;
	} else {
		f64 scaled = magnitude;
		i32 exponent = zero_count;
		while (scaled >= ten_value) {
			scaled = scaled / ten_value;
			exponent = exponent + one_count;
		}
		while (scaled < one_value) {
			scaled = scaled * ten_value;
			exponent = exponent - one_count;
		}
		u64 digit_start = emit;
		u64 produced = zero;
		while (produced < nine) {
			i32 digit_value = core.float_to_sint_trunc<i32>(scaled);
			u8 digit = core.int_trunc<u8>(digit_value);
			scratch[emit] = zero_digit + digit;
			emit = emit + one;
			if (produced == zero) {
				scratch[emit] = point;
				emit = emit + one;
			};
			f64 whole = core.sint_to_float<f64>(digit_value);
			scaled = scaled - whole;
			scaled = scaled * ten_value;
			produced = produced + one;
		}
		if (scaled >= five_value) {
			u64 round_position = emit - one;
			bool rounding = true;
			while (rounding) {
				u8 current = scratch[round_position];
				bool is_point = current == point;
				bool is_nine = current == nine_digit;
				bool at_start = round_position == digit_start;
				if (is_point) { round_position = round_position - one; };
				bool carry_back = !is_point && is_nine && !at_start;
				bool carry_stop = !is_point && is_nine && at_start;
				bool carry_done = !is_point && !is_nine;
				if (carry_back) {
					scratch[round_position] = zero_digit;
					round_position = round_position - one;
				};
				if (carry_stop) {
					scratch[round_position] = zero_digit;
					rounding = false;
				};
				if (carry_done) {
					scratch[round_position] = current + one_digit;
					rounding = false;
				};
			}
			u8 leading = scratch[digit_start];
			if (leading == zero_digit) {
				scratch[digit_start] = one_char;
				exponent = exponent + one_count;
			};
		};
		scratch[emit] = exponent_mark;
		emit = emit + one;
		i32 remaining_exponent = exponent;
		if (remaining_exponent < zero_count) {
			scratch[emit] = minus;
			emit = emit + one;
			remaining_exponent = zero_count - remaining_exponent;
		};
		i32 hundred_scale = 100;
		i32 ten_scale = 10;
		i32 hundreds = remaining_exponent / hundred_scale;
		i32 after_hundreds = remaining_exponent - (hundreds * hundred_scale);
		i32 tens = after_hundreds / ten_scale;
		i32 ones = after_hundreds - (tens * ten_scale);
		bool show_tens = false;
		if (hundreds != zero_count) { show_tens = true; };
		if (tens != zero_count) { show_tens = true; };
		if (hundreds != zero_count) {
			u8 hundred_digit = core.int_trunc<u8>(hundreds);
			scratch[emit] = zero_digit + hundred_digit;
			emit = emit + one;
		};
		if (show_tens) {
			u8 ten_digit = core.int_trunc<u8>(tens);
			scratch[emit] = zero_digit + ten_digit;
			emit = emit + one;
		};
		u8 one_digit_out = core.int_trunc<u8>(ones);
		scratch[emit] = zero_digit + one_digit_out;
		emit = emit + one;
	};
	*?u8 data = null;
	unsafe { data = &?scratch[0]; };
	utf8 text = { .data = data; .length = emit; };
	text
};

utf8(f64) _format_f64 = fn(value) {
	u8[24] scratch;
	u64 one = core.int_extend<u64>(1);
	u64 zero = core.int_extend<u64>(0);
	u64 seventeen = core.int_extend<u64>(17);
	f64 zero_value = 0.0;
	f64 one_value = 1.0;
	f64 ten_value = 10.0;
	f64 five_value = 5.0;
	u8 zero_digit = 48;
	u8 one_digit = 1;
	u8 nine_digit = 57;
	u8 one_char = 49;
	u8 minus = 45;
	u8 point = 46;
	u8 exponent_mark = 101;
	i32 zero_count = 0;
	i32 one_count = 1;
	bool negative = value < zero_value;
	f64 magnitude = value;
	if (negative) { magnitude = zero_value - value; };
	u64 emit = zero;
	if (negative) {
		scratch[0] = minus;
		emit = one;
	};
	if (magnitude == zero_value) {
		f64 probe = one_value / magnitude;
		if (probe < zero_value) {
			scratch[emit] = minus;
			emit = emit + one;
		};
		scratch[emit] = zero_digit;
		emit = emit + one;
	} else {
		f64 scaled = magnitude;
		i32 exponent = zero_count;
		while (scaled >= ten_value) {
			scaled = scaled / ten_value;
			exponent = exponent + one_count;
		}
		while (scaled < one_value) {
			scaled = scaled * ten_value;
			exponent = exponent - one_count;
		}
		u64 digit_start = emit;
		u64 produced = zero;
		while (produced < seventeen) {
			i32 digit_value = core.float_to_sint_trunc<i32>(scaled);
			u8 digit = core.int_trunc<u8>(digit_value);
			scratch[emit] = zero_digit + digit;
			emit = emit + one;
			if (produced == zero) {
				scratch[emit] = point;
				emit = emit + one;
			};
			f64 whole = core.sint_to_float<f64>(digit_value);
			scaled = scaled - whole;
			scaled = scaled * ten_value;
			produced = produced + one;
		}
		if (scaled >= five_value) {
			u64 round_position = emit - one;
			bool rounding = true;
			while (rounding) {
				u8 current = scratch[round_position];
				bool is_point = current == point;
				bool is_nine = current == nine_digit;
				bool at_start = round_position == digit_start;
				if (is_point) { round_position = round_position - one; };
				bool carry_back = !is_point && is_nine && !at_start;
				bool carry_stop = !is_point && is_nine && at_start;
				bool carry_done = !is_point && !is_nine;
				if (carry_back) {
					scratch[round_position] = zero_digit;
					round_position = round_position - one;
				};
				if (carry_stop) {
					scratch[round_position] = zero_digit;
					rounding = false;
				};
				if (carry_done) {
					scratch[round_position] = current + one_digit;
					rounding = false;
				};
			}
			u8 leading = scratch[digit_start];
			if (leading == zero_digit) {
				scratch[digit_start] = one_char;
				exponent = exponent + one_count;
			};
		};
		scratch[emit] = exponent_mark;
		emit = emit + one;
		i32 remaining_exponent = exponent;
		if (remaining_exponent < zero_count) {
			scratch[emit] = minus;
			emit = emit + one;
			remaining_exponent = zero_count - remaining_exponent;
		};
		i32 hundred_scale = 100;
		i32 ten_scale = 10;
		i32 hundreds = remaining_exponent / hundred_scale;
		i32 after_hundreds = remaining_exponent - (hundreds * hundred_scale);
		i32 tens = after_hundreds / ten_scale;
		i32 ones = after_hundreds - (tens * ten_scale);
		bool show_tens = false;
		if (hundreds != zero_count) { show_tens = true; };
		if (tens != zero_count) { show_tens = true; };
		if (hundreds != zero_count) {
			u8 hundred_digit = core.int_trunc<u8>(hundreds);
			scratch[emit] = zero_digit + hundred_digit;
			emit = emit + one;
		};
		if (show_tens) {
			u8 ten_digit = core.int_trunc<u8>(tens);
			scratch[emit] = zero_digit + ten_digit;
			emit = emit + one;
		};
		u8 one_digit_out = core.int_trunc<u8>(ones);
		scratch[emit] = zero_digit + one_digit_out;
		emit = emit + one;
	};
	*?u8 data = null;
	unsafe { data = &?scratch[0]; };
	utf8 text = { .data = data; .length = emit; };
	text
};

format = overload {
	utf8(bool) => fn(value) { _format_bool(value) };
	utf8(i8) => fn(value) { _format_i8(value) };
	utf8(i16) => fn(value) { _format_i16(value) };
	utf8(i32) => fn(value) { _format_i32(value) };
	utf8(i64) => fn(value) { _format_i64(value) };
	utf8(i128) => fn(value) { _format_i128(value) };
	utf8(u8) => fn(value) { _format_u8(value) };
	utf8(u16) => fn(value) { _format_u16(value) };
	utf8(u32) => fn(value) { _format_u32(value) };
	utf8(u64) => fn(value) { _format_u64(value) };
	utf8(u128) => fn(value) { _format_u128(value) };
	utf8(f32) => fn(value) { _format_f32(value) };
	utf8(f64) => fn(value) { _format_f64(value) };
};

%%end

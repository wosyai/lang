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

unit(u32) exit = fn(code) {
	preview1.exit(code);
};

unit(u64) sleep_ns = fn(nanoseconds) {
	preview1.sleep_ns(nanoseconds);
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

u8(*(u8[]), u64) _read_line_load_byte = fn(bytes, index) {
	(*bytes)[index]
};

(u64, u8, u8, ReadLineStatus)(*!(u8[]), u64, ReadLineStatus, u64, u8, u8) _read_line_utf8_step = fn(bytes, index, result, remaining, next_minimum, next_maximum) {
	u64 zero = 0;
	u64 one = 1;
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
	u8 byte = _read_line_load_byte(&(*bytes), index);
	if (remaining != zero) {
		if (byte < next_minimum || byte > next_maximum) { result = ReadLineStatus::invalid_utf8; };
		remaining = remaining - one;
		next_minimum = continuation_minimum;
		next_maximum = continuation_maximum;
	} else {
		if (byte >= continuation_minimum && byte < two_byte_minimum) { result = ReadLineStatus::invalid_utf8; };
		if (byte >= two_byte_minimum && byte < three_byte_minimum) { remaining = one; };
		if (byte >= three_byte_minimum && byte < four_byte_minimum) {
			remaining = 2;
			if (byte == e0) { next_minimum = 160; };
			if (byte == ed) { next_maximum = 159; };
		};
		if (byte >= four_byte_minimum && byte <= four_byte_limit) {
			remaining = 3;
			if (byte == f0) { next_minimum = 144; };
			if (byte == f4) { next_maximum = 143; };
		};
		if (byte > four_byte_limit) { result = ReadLineStatus::invalid_utf8; };
	};
	remaining, next_minimum, next_maximum, result
};

(u64, u8, u8, ReadLineStatus)(*!(u8[]), u64, u64, u64, u8, u8, ReadLineStatus) _read_line_utf8_range = fn(bytes, low, high, remaining, minimum, maximum, status) {
	u64 one = 1;
	if (low < high) {
		u64 count = high - low;
		if (count == one) {
			remaining, minimum, maximum, status = _read_line_utf8_step(bytes, low, status, remaining, minimum, maximum);
		} else {
			u64 two = 2;
			u64 middle = low + count / two;
			remaining, minimum, maximum, status = _read_line_utf8_range(&!(*bytes), low, middle, remaining, minimum, maximum, status);
			remaining, minimum, maximum, status = _read_line_utf8_range(bytes, middle, high, remaining, minimum, maximum, status);
		};
	};
	remaining, minimum, maximum, status
};

(u8, u64, bool)() _read_line_read_byte = fn {
	u8[1] scratch = [0];
	u64 count = 0;
	bool complete = false;
	unsafe { count, complete = read_into(&?scratch[0], 1); };
	scratch[0], count, complete
};

i32() _read_line_discard_step = fn {
	u8 byte, u64 count, bool complete = _read_line_read_byte();
	if (!complete) { 2 } else {
		if (count == 0) { 1 } else {
			if (byte == 10) { 1 } else { 0 }
		}
	}
};

i32(i32) _read_line_discard_continue = fn(outcome) {
	if (outcome == 0) {
		_read_line_discard_continue(_read_line_discard_step())
	} else { outcome }
};

ReadLineStatus(u8) _read_line_zero_capacity = fn(first) {
	ReadLineStatus status = ReadLineStatus::limit_exceeded;
	bool discard = true;
	if (first == 13) {
		u8 byte, u64 count, bool complete = _read_line_read_byte();
		if (!complete) { status = ReadLineStatus::io_error; discard = false; };
		if (complete && count == 0) { discard = false; };
		if (complete && count != 0 && byte == 10) { status = ReadLineStatus::line; discard = false; };
	};
	if (discard) {
		i32 first_outcome = _read_line_discard_step();
		i32 outcome = _read_line_discard_continue(first_outcome);
		if (outcome == 2) { status = ReadLineStatus::io_error; };
	};
	status
};

unit(*!(u8[]), u64, u8) _read_line_store_byte = fn(bytes, index, value) {
	(*bytes)[index] = value;
};

(u64, bool, bool, ReadLineStatus)(*!(u8[]), u64, u64, bool, bool, ReadLineStatus) _read_line_allocated_step = fn(bytes, max_bytes, length, pending_cr, reading, status) {
	u64 zero = 0;
	u64 one = 1;
	u8 lf = 10;
	u8 cr = 13;
	u8 byte, u64 count, bool complete = _read_line_read_byte();
	if (!complete) {
		status = ReadLineStatus::io_error;
		reading = false;
	} else {
		if (count == zero) {
			reading = false;
		} else {
			if (byte == lf) {
				pending_cr = false;
				reading = false;
			} else {
				if (pending_cr) {
					status = _read_line_capacity_status(length, max_bytes, status);
					if (length < max_bytes) { _read_line_store_byte(&!(*bytes), length, cr); length = length + one; };
				};
				pending_cr = byte == cr;
				if (!pending_cr) {
					status = _read_line_capacity_status(length, max_bytes, status);
					if (length < max_bytes) { _read_line_store_byte(&!(*bytes), length, byte); length = length + one; };
				};
			};
		};
	};
	if (status == ReadLineStatus::limit_exceeded) { reading = false; };
	length, pending_cr, reading, status
};

(u64, bool, bool, ReadLineStatus)(*!(u8[]), u64, u64, u64, bool, bool, ReadLineStatus) _read_line_allocated_range = fn(bytes, max_bytes, budget, length, pending_cr, reading, status) {
	u64 zero = 0;
	u64 one = 1;
	if (budget != zero && reading) {
		if (budget == one) {
			length, pending_cr, reading, status = _read_line_allocated_step(bytes, max_bytes, length, pending_cr, reading, status);
		} else {
			u64 two = 2;
			u64 left = budget / two;
			length, pending_cr, reading, status = _read_line_allocated_range(&!(*bytes), max_bytes, left, length, pending_cr, reading, status);
			if (reading) {
				length, pending_cr, reading, status = _read_line_allocated_range(bytes, max_bytes, budget - left, length, pending_cr, reading, status);
			};
		};
	};
	length, pending_cr, reading, status
};

(*utf8, ReadLineStatus)(u64, u8, bool) _read_line_allocated = fn(max_bytes, first, pending_cr) {
	u64 zero = 0;
	u64 one = 1;
	u8[max_bytes] bytes;
	u64 length = zero;
	bool reading = true;
	ReadLineStatus status = ReadLineStatus::line;
	if (!pending_cr) { bytes[length] = first; length = length + one; };
	u64 two = 2;
	u64 budget = max_bytes + two;
	length, pending_cr, reading, status = _read_line_allocated_range(&!bytes, max_bytes, budget, length, pending_cr, reading, status);
	if (status == ReadLineStatus::limit_exceeded) {
		i32 first_outcome = _read_line_discard_step();
		i32 outcome = _read_line_discard_continue(first_outcome);
		if (outcome == 2) { status = ReadLineStatus::io_error; };
	};
	if (status == ReadLineStatus::line && pending_cr) {
		status = _read_line_capacity_status(length, max_bytes, status);
		if (length < max_bytes) { bytes[length] = 13; length = length + one; };
	};
	if (status == ReadLineStatus::line) {
		u64 remaining = zero;
		u8 minimum = 128;
		u8 maximum = 191;
		remaining, minimum, maximum, status = _read_line_utf8_range(&!bytes, zero, length, remaining, minimum, maximum, status);
		if (remaining != zero) { status = ReadLineStatus::invalid_utf8; };
	};
	if (status == ReadLineStatus::line) {
		*?u8 data = null;
		if (length != zero) {
			unsafe { data = &?bytes[0]; };
		} else {
			unsafe { core.free(&bytes); };
		};
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
		u64 count = zero;
		bool complete = false;
		unsafe { count, complete = read_into(&?scratch[0], one); };
		if (!complete) {
			null, ReadLineStatus::io_error
		} else {
			if (count == zero) {
				null, ReadLineStatus::eof
			} else {
				u8 first = scratch[0];
				if (first == 10) {
					utf8 empty = { .data = null; .length = zero; };
					&empty, ReadLineStatus::line
				} else {
					if (max_bytes == zero) {
						ReadLineStatus status = _read_line_zero_capacity(first);
						if (status == ReadLineStatus::line) {
							utf8 empty = { .data = null; .length = zero; };
							&empty, status
						} else { null, status }
					} else {
						*utf8 text, ReadLineStatus status = _read_line_allocated(max_bytes, first, first == 13);
						text, status
					}
				}
			}
		}
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

(u128, bool, bool, bool)(*?u8, u64, u64, u64, u128, u128, u128, u128, bool, bool, bool) _parse_range_integer = fn(data, start, lo, hi, radix, limit, remainder, result, valid, saw_digit, last_underscore) {
	u64 one = core.int_extend<u64>(1);
	if (lo < hi && valid) {
		u64 length = hi - lo;
		if (length == one) {
			u128 wide_index = core.int_extend<u128>(lo);
			i64 position = core.int_trunc<i64>(wide_index);
			u8 byte = 0;
			unsafe { byte = core.load<u8>(core.offset<u8>(data, position)); };
			bool underscore = _parse_is_underscore(byte);
			u8 digit = _parse_digit_value(byte);
			u128 digit_wide = core.int_extend<u128>(digit);
			if (digit_wide < radix) {
				if (result > limit || (result == limit && digit_wide > remainder)) { valid = false; } else { result = result * radix + digit_wide; };
				last_underscore = false;
				saw_digit = true;
			} else {
				if (underscore && lo == start) { valid = false; };
				if (underscore && last_underscore) { valid = false; };
				if (underscore) { last_underscore = true; } else { valid = false; };
			};
		} else {
			u64 two = core.int_extend<u64>(2);
			u64 mid = lo + length / two;
			result, valid, saw_digit, last_underscore = _parse_range_integer(data, start, lo, mid, radix, limit, remainder, result, valid, saw_digit, last_underscore);
			if (valid) {
				result, valid, saw_digit, last_underscore = _parse_range_integer(data, start, mid, hi, radix, limit, remainder, result, valid, saw_digit, last_underscore);
			};
		};
	};
	result, valid, saw_digit, last_underscore
};

(u128, bool)(*?u8, u64, u64, u8) _parse_accumulate_u128 = fn(data, start, end, radix) {
	u128 maximum = 340282366920938463463374607431768211455;
	u128 radix_wide = core.int_extend<u128>(radix);
	u128 limit = maximum / radix_wide;
	u128 remainder = maximum - limit * radix_wide;
	u128 result = core.int_extend<u128>(0);
	bool valid = true;
	bool saw_digit = false;
	bool last_underscore = false;
	result, valid, saw_digit, last_underscore = _parse_range_integer(data, start, start, end, radix_wide, limit, remainder, result, valid, saw_digit, last_underscore);
	if (!saw_digit) { valid = false; };
	if (last_underscore) { valid = false; };
	result, valid
};

(u128, bool)(*?u8, u64, u64, u8) _parse_accumulate_i128_neg = fn(data, start, end, radix) {
	u128 maximum = 170141183460469231731687303715884105728;
	u128 radix_wide = core.int_extend<u128>(radix);
	u128 limit = maximum / radix_wide;
	u128 remainder = maximum - limit * radix_wide;
	u128 result = core.int_extend<u128>(0);
	bool valid = true;
	bool saw_digit = false;
	bool last_underscore = false;
	result, valid, saw_digit, last_underscore = _parse_range_integer(data, start, start, end, radix_wide, limit, remainder, result, valid, saw_digit, last_underscore);
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

u8(*?u8, u64) _parse_float_byte = fn(data, index) {
	u128 wide = core.int_extend<u128>(index);
	i64 position = core.int_trunc<i64>(wide);
	u8 byte = 0;
	unsafe { byte = core.load<u8>(core.offset<u8>(data, position)); };
	byte
};

bool(u8) _parse_is_dot = fn(byte) {
	byte == 46
};

bool(u8) _parse_is_e = fn(byte) {
	byte == 101 || byte == 69
};

bool(u8) _parse_is_digit10 = fn(byte) {
	byte >= 48 && byte <= 57
};

(u64, f64, i32, bool, bool, bool, bool)(*?u8, u64, u64, f64, i32, bool, bool, bool, bool) _parse_range_float_integer = fn(data, pos, end, mant, extra_scale, valid, saw_int, last_underscore, int_done) {
	u64 one = core.int_extend<u64>(1);
	u64 two = core.int_extend<u64>(2);
	f64 ten_value = 10.0;
	f64 big_limit = 1.0e300;
	u8 zero_digit = 48;
	i32 one_count = 1;
	if (pos < end && valid && !int_done) {
		u64 length = end - pos;
		if (length > one) {
			u64 mid = pos + length / two;
			pos, mant, extra_scale, valid, saw_int, last_underscore, int_done = _parse_range_float_integer(data, pos, mid, mant, extra_scale, valid, saw_int, last_underscore, int_done);
			if (valid && !int_done) {
				pos, mant, extra_scale, valid, saw_int, last_underscore, int_done = _parse_range_float_integer(data, pos, end, mant, extra_scale, valid, saw_int, last_underscore, int_done);
			};
		} else {
			u8 byte = _parse_float_byte(data, pos);
			bool digit = _parse_is_digit10(byte);
			bool underscore = _parse_is_underscore(byte);
			if (digit) {
				u8 small_int = byte - zero_digit;
				u64 wide_digit = core.int_extend<u64>(small_int);
				f64 digit_value = core.uint_to_float<f64>(wide_digit);
				mant = mant * ten_value + digit_value;
				if (mant > big_limit) {
					mant = mant / ten_value;
					extra_scale = extra_scale + one_count;
				};
				last_underscore = false;
				saw_int = true;
				pos = pos + one;
			} else {
				if (underscore) {
					if (!saw_int) { valid = false; };
					if (last_underscore) { valid = false; };
					last_underscore = true;
					pos = pos + one;
				} else {
					int_done = true;
				};
			};
		};
	};
	pos, mant, extra_scale, valid, saw_int, last_underscore, int_done
};

(u64, f64, i32, i32, bool, bool, bool, bool)(*?u8, u64, u64, f64, i32, i32, bool, bool, bool, bool) _parse_range_float_fraction = fn(data, pos, end, mant, extra_scale, frac_count, valid, frac_started, frac_trail, frac_done) {
	u64 one = core.int_extend<u64>(1);
	u64 two = core.int_extend<u64>(2);
	f64 ten_value = 10.0;
	f64 big_limit = 1.0e300;
	u8 zero_digit = 48;
	i32 one_count = 1;
	i32 cap = 1000000;
	if (pos < end && valid && !frac_done) {
		u64 length = end - pos;
		if (length > one) {
			u64 mid = pos + length / two;
			pos, mant, extra_scale, frac_count, valid, frac_started, frac_trail, frac_done = _parse_range_float_fraction(data, pos, mid, mant, extra_scale, frac_count, valid, frac_started, frac_trail, frac_done);
			if (valid && !frac_done) {
				pos, mant, extra_scale, frac_count, valid, frac_started, frac_trail, frac_done = _parse_range_float_fraction(data, pos, end, mant, extra_scale, frac_count, valid, frac_started, frac_trail, frac_done);
			};
		} else {
			u8 byte = _parse_float_byte(data, pos);
			bool digit = _parse_is_digit10(byte);
			bool underscore = _parse_is_underscore(byte);
			if (digit) {
				u8 small_frac = byte - zero_digit;
				u64 wide_digit = core.int_extend<u64>(small_frac);
				f64 digit_value = core.uint_to_float<f64>(wide_digit);
				mant = mant * ten_value + digit_value;
				if (mant > big_limit) {
					mant = mant / ten_value;
					extra_scale = extra_scale + one_count;
				};
				if (frac_count < cap) { frac_count = frac_count + one_count; };
				frac_started = true;
				frac_trail = false;
				pos = pos + one;
			} else {
				if (underscore) {
					if (!frac_started) { valid = false; };
					if (frac_trail) { valid = false; };
					frac_trail = true;
					pos = pos + one;
				} else {
					frac_done = true;
				};
			};
		};
	};
	pos, mant, extra_scale, frac_count, valid, frac_started, frac_trail, frac_done
};

(u64, i32, bool, bool, bool, bool)(*?u8, u64, u64, i32, bool, bool, bool, bool) _parse_range_float_exponent = fn(data, pos, end, exp_value, valid, exp_started, exp_trail, exp_done) {
	u64 one = core.int_extend<u64>(1);
	u64 two = core.int_extend<u64>(2);
	u8 zero_digit = 48;
	i32 ten_count = 10;
	i32 cap = 1000000;
	if (pos < end && valid && !exp_done) {
		u64 length = end - pos;
		if (length > one) {
			u64 mid = pos + length / two;
			pos, exp_value, valid, exp_started, exp_trail, exp_done = _parse_range_float_exponent(data, pos, mid, exp_value, valid, exp_started, exp_trail, exp_done);
			if (valid && !exp_done) {
				pos, exp_value, valid, exp_started, exp_trail, exp_done = _parse_range_float_exponent(data, pos, end, exp_value, valid, exp_started, exp_trail, exp_done);
			};
		} else {
			u8 byte = _parse_float_byte(data, pos);
			bool digit = _parse_is_digit10(byte);
			bool underscore = _parse_is_underscore(byte);
			if (digit) {
				u8 small = byte - zero_digit;
				i32 piece = core.int_extend<i32>(small);
				if (exp_value < cap) { exp_value = exp_value * ten_count + piece; };
				exp_started = true;
				exp_trail = false;
				pos = pos + one;
			} else {
				if (underscore) {
					if (!exp_started) { valid = false; };
					if (exp_trail) { valid = false; };
					exp_trail = true;
					pos = pos + one;
				} else {
					exp_done = true;
				};
			};
		};
	};
	pos, exp_value, valid, exp_started, exp_trail, exp_done
};

f64(f64, i32) _parse_scale_float_up = fn(scaled, steps) {
	if (steps > 0) {
		if (steps == 1) {
			f64 ten_value = 10.0;
			scaled = scaled * ten_value;
		} else {
			i32 half = steps / 2;
			scaled = _parse_scale_float_up(scaled, half);
			scaled = _parse_scale_float_up(scaled, steps - half);
		};
	};
	scaled
};

f64(f64, i32) _parse_scale_float_down = fn(scaled, steps) {
	if (steps > 0) {
		if (steps == 1) {
			f64 ten_value = 10.0;
			scaled = scaled / ten_value;
		} else {
			i32 half = steps / 2;
			scaled = _parse_scale_float_down(scaled, half);
			scaled = _parse_scale_float_down(scaled, steps - half);
		};
	};
	scaled
};

(f64, bool)(*?u8, u64, u64) _parse_float_f64 = fn(data, start, end) {
	u64 one = core.int_extend<u64>(1);
	f64 zero_value = 0.0;
	f64 f64_max = 1.7976931348623157e308;
	i32 zero_count = 0;
	i32 limit = 1000;
	i32 neg_limit = zero_count - limit;
	u64 pos = start;
	f64 mant = zero_value;
	i32 extra_scale = zero_count;
	bool valid = true;
	bool saw_int = false;
	bool last_underscore = false;
	bool int_done = false;
	pos, mant, extra_scale, valid, saw_int, last_underscore, int_done = _parse_range_float_integer(data, pos, end, mant, extra_scale, valid, saw_int, last_underscore, int_done);
	i32 frac_count = zero_count;
	i32 exp_value = zero_count;
	bool exp_negative = false;
	if (last_underscore) { valid = false; };
	if (!saw_int) { valid = false; };
	bool has_dot = false;
	if (valid && pos < end) {
		u8 byte = _parse_float_byte(data, pos);
		if (_parse_is_dot(byte)) {
			has_dot = true;
			pos = pos + one;
		};
	};
	if (valid && has_dot) {
		bool frac_started = false;
		bool frac_trail = false;
		bool frac_done = false;
		pos, mant, extra_scale, frac_count, valid, frac_started, frac_trail, frac_done = _parse_range_float_fraction(data, pos, end, mant, extra_scale, frac_count, valid, frac_started, frac_trail, frac_done);
		if (frac_trail) { valid = false; };
	};
	bool has_exp = false;
	if (valid && pos < end) {
		u8 byte = _parse_float_byte(data, pos);
		if (_parse_is_e(byte)) {
			has_exp = true;
			pos = pos + one;
		};
	};
	if (valid && has_exp) {
		if (pos < end) {
			u8 byte = _parse_float_byte(data, pos);
			if (_parse_is_sign(byte)) {
				exp_negative = true;
				pos = pos + one;
			} else {
				if (_parse_is_plus(byte)) { pos = pos + one; };
			};
		};
		bool exp_started = false;
		bool exp_trail = false;
		bool exp_done = false;
		pos, exp_value, valid, exp_started, exp_trail, exp_done = _parse_range_float_exponent(data, pos, end, exp_value, valid, exp_started, exp_trail, exp_done);
		if (!exp_started) { valid = false; };
		if (exp_trail) { valid = false; };
	};
	f64 scaled = mant;
	if (valid) {
		if (mant == zero_value) {
			scaled = zero_value;
		} else {
			i32 exp_signed = exp_value;
			if (exp_negative) { exp_signed = zero_count - exp_value; };
			i32 adjusted = exp_signed - frac_count + extra_scale;
			if (adjusted > limit) {
				valid = false;
			} else {
				if (adjusted < neg_limit) {
					scaled = zero_value;
				} else {
					i32 steps = adjusted;
					if (steps < zero_count) { steps = zero_count - steps; };
					if (adjusted > zero_count) {
						scaled = _parse_scale_float_up(scaled, steps);
					} else {
						scaled = _parse_scale_float_down(scaled, steps);
					};
				};
			};
		};
	};
	if (valid && scaled > f64_max) { valid = false; };
	if (valid && pos != end) { valid = false; };
	scaled, valid
};

*f32(utf8) _parse_f32 = fn(text) {
	u64 zero = core.int_extend<u64>(0);
	u64 one = core.int_extend<u64>(1);
	f64 f64_max = 1.7976931348623157e308;
	f32 out = 0.0;
	*f32 result = null;
	if (text.data == null || text.length == zero) {
		result = null;
	} else {
		u8 first = _parse_float_byte(text.data, zero);
		bool rejected = _parse_is_plus(first);
		u64 pos = zero;
		bool negative = false;
		if (_parse_is_sign(first)) {
			negative = true;
			pos = one;
		};
		if (!rejected) {
			f64 magnitude, bool valid = _parse_float_f64(text.data, pos, text.length);
			if (valid) {
				f32 narrowed = core.float_trunc<f32>(magnitude);
				f64 back = core.float_extend<f64>(narrowed);
				bool overflowed = back > f64_max;
				bool fits = !overflowed;
				if (fits) {
					f32 value = narrowed;
					if (negative) {
						f32 zero_single = 0.0;
						f32 one_single = 1.0;
						f32 neg_one = zero_single - one_single;
						value = narrowed * neg_one;
					};
					out = value;
					result = &out;
				};
			};
		};
	};
	result
};

*f64(utf8) _parse_f64 = fn(text) {
	u64 zero = core.int_extend<u64>(0);
	u64 one = core.int_extend<u64>(1);
	f64 zero_value = 0.0;
	f64 one_value = 1.0;
	f64 out = zero_value;
	*f64 result = null;
	if (text.data == null || text.length == zero) {
		result = null;
	} else {
		u8 first = _parse_float_byte(text.data, zero);
		bool rejected = _parse_is_plus(first);
		u64 pos = zero;
		bool negative = false;
		if (_parse_is_sign(first)) {
			negative = true;
			pos = one;
		};
		if (!rejected) {
			f64 magnitude, bool valid = _parse_float_f64(text.data, pos, text.length);
			if (valid) {
				f64 value = magnitude;
				if (negative) {
					f64 neg_one = zero_value - one_value;
					value = magnitude * neg_one;
				};
				out = value;
				result = &out;
			};
		};
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

utf8(char) _format_char = fn(value) {
	u32 scalar = core.bitcast<u32>(value);
	u8[4] scratch;
	u64 one = core.int_extend<u64>(1);
	u64 two = core.int_extend<u64>(2);
	u64 three = core.int_extend<u64>(3);
	u64 four = core.int_extend<u64>(4);
	u32 ascii_limit = 128;
	u32 two_byte_limit = 2048;
	u32 three_byte_limit = 65536;
	u32 continuation_prefix = 128;
	u32 two_byte_prefix = 192;
	u32 three_byte_prefix = 224;
	u32 four_byte_prefix = 240;
	u32 payload_mask = 63;
	u32 shift_six = 6;
	u32 shift_twelve = 12;
	u32 shift_eighteen = 18;
	u64 length = one;
	if (scalar >= ascii_limit) { length = two; };
	if (scalar >= two_byte_limit) { length = three; };
	if (scalar >= three_byte_limit) { length = four; };
	if (length == one) { scratch[0] = core.int_trunc<u8>(scalar); };
	if (length == two) {
		scratch[0] = core.int_trunc<u8>(two_byte_prefix | (scalar >> shift_six));
		scratch[one] = core.int_trunc<u8>(continuation_prefix | (scalar & payload_mask));
	};
	if (length == three) {
		scratch[0] = core.int_trunc<u8>(three_byte_prefix | (scalar >> shift_twelve));
		scratch[one] = core.int_trunc<u8>(continuation_prefix | ((scalar >> shift_six) & payload_mask));
		scratch[two] = core.int_trunc<u8>(continuation_prefix | (scalar & payload_mask));
	};
	if (length == four) {
		scratch[0] = core.int_trunc<u8>(four_byte_prefix | (scalar >> shift_eighteen));
		scratch[one] = core.int_trunc<u8>(continuation_prefix | ((scalar >> shift_twelve) & payload_mask));
		scratch[two] = core.int_trunc<u8>(continuation_prefix | ((scalar >> shift_six) & payload_mask));
		scratch[three] = core.int_trunc<u8>(continuation_prefix | (scalar & payload_mask));
	};
	*?u8 data = null;
	unsafe { data = &?scratch[0]; };
	utf8 text = { .data = data; .length = length; };
	text
};

unit(*!(u8[]), u64, u8) _format_store_byte = fn(bytes, index, value) {
	(*bytes)[index] = value;
};

u8(*(u8[]), u64) _format_load_byte = fn(bytes, index) {
	(*bytes)[index]
};

unit(*!(u8[]), u64, u64, u64) _format_compact_checked_bytes = fn(bytes, target, source, capacity) {
	if (source < capacity) {
		u8 value = _format_load_byte(&(*bytes), source);
		_format_store_byte(&!(*bytes), target, value);
		_format_compact_checked_bytes(bytes, target + 1, source + 1, capacity);
	};
};

u64(*!(u8[]), u64, i8, bool, u64) _format_digits_i8 = fn(scratch, index, remaining, negative, capacity) {
	i8 zero = 0;
	if (remaining == zero) {
		if (negative) {
			u64 one = core.int_extend<u64>(1);
			u8 minus = 45;
			index = index - one;
			_format_store_byte(&!(*scratch), index, minus);
		};
		u64 length = capacity - index;
		_format_compact_checked_bytes(scratch, 0, index, capacity);
		length
	} else {
		i8 ten = 10;
		i8 digit_signed = remaining % ten;
		if (negative) { digit_signed = zero - digit_signed; };
		u16 wide_digit = core.int_extend<u16>(digit_signed);
		u8 digit = core.int_trunc<u8>(wide_digit);
		u64 one = core.int_extend<u64>(1);
		u8 zero_digit = 48;
		index = index - one;
		u8 rendered_byte = zero_digit + digit;
		_format_store_byte(&!(*scratch), index, rendered_byte);
		_format_digits_i8(scratch, index, remaining / ten, negative, capacity)
	}
};

u64(*!(u8[]), u64, i16, bool, u64) _format_digits_i16 = fn(scratch, index, remaining, negative, capacity) {
	i16 zero = 0;
	if (remaining == zero) {
		if (negative) {
			u64 one = core.int_extend<u64>(1);
			u8 minus = 45;
			index = index - one;
			_format_store_byte(&!(*scratch), index, minus);
		};
		u64 length = capacity - index;
		_format_compact_checked_bytes(scratch, 0, index, capacity);
		length
	} else {
		i16 ten = 10;
		i16 digit_signed = remaining % ten;
		if (negative) { digit_signed = zero - digit_signed; };
		u8 digit = core.int_trunc<u8>(digit_signed);
		u64 one = core.int_extend<u64>(1);
		u8 zero_digit = 48;
		index = index - one;
		u8 rendered_byte = zero_digit + digit;
		_format_store_byte(&!(*scratch), index, rendered_byte);
		_format_digits_i16(scratch, index, remaining / ten, negative, capacity)
	}
};

u64(*!(u8[]), u64, i32, bool, u64) _format_digits_i32 = fn(scratch, index, remaining, negative, capacity) {
	i32 zero = 0;
	if (remaining == zero) {
		if (negative) {
			u64 one = core.int_extend<u64>(1);
			u8 minus = 45;
			index = index - one;
			_format_store_byte(&!(*scratch), index, minus);
		};
		u64 length = capacity - index;
		_format_compact_checked_bytes(scratch, 0, index, capacity);
		length
	} else {
		i32 ten = 10;
		i32 digit_signed = remaining % ten;
		if (negative) { digit_signed = zero - digit_signed; };
		u8 digit = core.int_trunc<u8>(digit_signed);
		u64 one = core.int_extend<u64>(1);
		u8 zero_digit = 48;
		index = index - one;
		u8 rendered_byte = zero_digit + digit;
		_format_store_byte(&!(*scratch), index, rendered_byte);
		_format_digits_i32(scratch, index, remaining / ten, negative, capacity)
	}
};

u64(*!(u8[]), u64, i64, bool, u64) _format_digits_i64 = fn(scratch, index, remaining, negative, capacity) {
	i64 zero = 0;
	if (remaining == zero) {
		if (negative) {
			u64 one = core.int_extend<u64>(1);
			u8 minus = 45;
			index = index - one;
			_format_store_byte(&!(*scratch), index, minus);
		};
		u64 length = capacity - index;
		_format_compact_checked_bytes(scratch, 0, index, capacity);
		length
	} else {
		i64 ten = 10;
		i64 digit_signed = remaining % ten;
		if (negative) { digit_signed = zero - digit_signed; };
		u8 digit = core.int_trunc<u8>(digit_signed);
		u64 one = core.int_extend<u64>(1);
		u8 zero_digit = 48;
		index = index - one;
		u8 rendered_byte = zero_digit + digit;
		_format_store_byte(&!(*scratch), index, rendered_byte);
		_format_digits_i64(scratch, index, remaining / ten, negative, capacity)
	}
};

u64(*!(u8[]), u64, u8, u64) _format_digits_u8 = fn(scratch, index, remaining, capacity) {
	u8 zero = 0;
	if (remaining == zero) {
		u64 length = capacity - index;
		_format_compact_checked_bytes(scratch, 0, index, capacity);
		length
	} else {
		u8 ten = 10;
		u8 digit = remaining % ten;
		u64 one = core.int_extend<u64>(1);
		u8 zero_digit = 48;
		index = index - one;
		u8 rendered_byte = zero_digit + digit;
		_format_store_byte(&!(*scratch), index, rendered_byte);
		_format_digits_u8(scratch, index, remaining / ten, capacity)
	}
};

u64(*!(u8[]), u64, u16, u64) _format_digits_u16 = fn(scratch, index, remaining, capacity) {
	u16 zero = 0;
	if (remaining == zero) {
		u64 length = capacity - index;
		_format_compact_checked_bytes(scratch, 0, index, capacity);
		length
	} else {
		u16 ten = 10;
		u16 digit_wide = remaining % ten;
		u8 digit = core.int_trunc<u8>(digit_wide);
		u64 one = core.int_extend<u64>(1);
		u8 zero_digit = 48;
		index = index - one;
		u8 rendered_byte = zero_digit + digit;
		_format_store_byte(&!(*scratch), index, rendered_byte);
		_format_digits_u16(scratch, index, remaining / ten, capacity)
	}
};

u64(*!(u8[]), u64, u32, u64) _format_digits_u32 = fn(scratch, index, remaining, capacity) {
	u32 zero = 0;
	if (remaining == zero) {
		u64 length = capacity - index;
		_format_compact_checked_bytes(scratch, 0, index, capacity);
		length
	} else {
		u32 ten = 10;
		u32 digit_wide = remaining % ten;
		u8 digit = core.int_trunc<u8>(digit_wide);
		u64 one = core.int_extend<u64>(1);
		u8 zero_digit = 48;
		index = index - one;
		u8 rendered_byte = zero_digit + digit;
		_format_store_byte(&!(*scratch), index, rendered_byte);
		_format_digits_u32(scratch, index, remaining / ten, capacity)
	}
};

u64(*!(u8[]), u64, u64, u64) _format_digits_u64 = fn(scratch, index, remaining, capacity) {
	u64 zero = 0;
	if (remaining == zero) {
		u64 length = capacity - index;
		_format_compact_checked_bytes(scratch, 0, index, capacity);
		length
	} else {
		u64 ten = 10;
		u64 digit_wide = remaining % ten;
		u8 digit = core.int_trunc<u8>(digit_wide);
		u64 one = core.int_extend<u64>(1);
		u8 zero_digit = 48;
		index = index - one;
		u8 rendered_byte = zero_digit + digit;
		_format_store_byte(&!(*scratch), index, rendered_byte);
		_format_digits_u64(scratch, index, remaining / ten, capacity)
	}
};

utf8(i8) _format_i8 = fn(value) {
	u8[4] scratch;
	u64 capacity = core.int_extend<u64>(4);
	u64 one = core.int_extend<u64>(1);
	i8 zero = 0;
	u8 zero_digit = 48;
	u64 index = capacity;
	i8 remaining = value;
	bool negative = remaining < zero;
	if (remaining == zero) {
		index = index - one;
		scratch[index] = zero_digit;
	};
	u64 length = _format_digits_i8(&!scratch, index, remaining, negative, capacity);
	*?u8 data = null;
	unsafe { data = &?scratch[0]; };
	utf8 text = { .data = data; .length = length; };
	text
};

utf8(i16) _format_i16 = fn(value) {
	u8[6] scratch;
	u64 capacity = core.int_extend<u64>(6);
	u64 one = core.int_extend<u64>(1);
	i16 zero = 0;
	u8 zero_digit = 48;
	u64 index = capacity;
	i16 remaining = value;
	bool negative = remaining < zero;
	if (remaining == zero) {
		index = index - one;
		scratch[index] = zero_digit;
	};
	u64 length = _format_digits_i16(&!scratch, index, remaining, negative, capacity);
	*?u8 data = null;
	unsafe { data = &?scratch[0]; };
	utf8 text = { .data = data; .length = length; };
	text
};

utf8(i32) _format_i32 = fn(value) {
	u8[11] scratch;
	u64 capacity = core.int_extend<u64>(11);
	u64 one = core.int_extend<u64>(1);
	i32 zero = 0;
	u8 zero_digit = 48;
	u64 index = capacity;
	i32 remaining = value;
	bool negative = remaining < zero;
	if (remaining == zero) {
		index = index - one;
		scratch[index] = zero_digit;
	};
	u64 length = _format_digits_i32(&!scratch, index, remaining, negative, capacity);
	*?u8 data = null;
	unsafe { data = &?scratch[0]; };
	utf8 text = { .data = data; .length = length; };
	text
};

utf8(i64) _format_i64 = fn(value) {
	u8[20] scratch;
	u64 capacity = core.int_extend<u64>(20);
	u64 one = core.int_extend<u64>(1);
	i64 zero = 0;
	u8 zero_digit = 48;
	u64 index = capacity;
	i64 remaining = value;
	bool negative = remaining < zero;
	if (remaining == zero) {
		index = index - one;
		scratch[index] = zero_digit;
	};
	u64 length = _format_digits_i64(&!scratch, index, remaining, negative, capacity);
	*?u8 data = null;
	unsafe { data = &?scratch[0]; };
	utf8 text = { .data = data; .length = length; };
	text
};

utf8(i128) _format_i128 = fn(value) {
	u8[40] scratch;
	u64 capacity = core.int_extend<u64>(40);
	i128 zero = core.int_extend<i128>(0);
	bool negative = value < zero;
	u128 magnitude = if (negative) {
		i128 before_last = zero - (value + 1);
		core.bitcast<u128>(before_last) + 1
	} else {
		core.bitcast<u128>(value)
	};
	u64 length = _format_wide_decimal(&!scratch, magnitude, negative, capacity);
	*?u8 data = null;
	unsafe { data = &?scratch[0]; };
	utf8 text = { .data = data; .length = length; };
	text
};

utf8(u8) _format_u8 = fn(value) {
	u8[3] scratch;
	u64 capacity = core.int_extend<u64>(3);
	u64 one = core.int_extend<u64>(1);
	u8 zero_digit = 48;
	u8 zero_value = 0;
	u64 index = capacity;
	u8 remaining = value;
	if (remaining == zero_value) {
		index = index - one;
		scratch[index] = zero_digit;
	};
	u64 length = _format_digits_u8(&!scratch, index, remaining, capacity);
	*?u8 data = null;
	unsafe { data = &?scratch[0]; };
	utf8 text = { .data = data; .length = length; };
	text
};

utf8(u16) _format_u16 = fn(value) {
	u8[5] scratch;
	u64 capacity = core.int_extend<u64>(5);
	u64 one = core.int_extend<u64>(1);
	u8 zero_digit = 48;
	u16 zero_value = 0;
	u64 index = capacity;
	u16 remaining = value;
	if (remaining == zero_value) {
		index = index - one;
		scratch[index] = zero_digit;
	};
	u64 length = _format_digits_u16(&!scratch, index, remaining, capacity);
	*?u8 data = null;
	unsafe { data = &?scratch[0]; };
	utf8 text = { .data = data; .length = length; };
	text
};

utf8(u32) _format_u32 = fn(value) {
	u8[10] scratch;
	u64 capacity = core.int_extend<u64>(10);
	u64 one = core.int_extend<u64>(1);
	u8 zero_digit = 48;
	u32 zero_value = 0;
	u64 index = capacity;
	u32 remaining = value;
	if (remaining == zero_value) {
		index = index - one;
		scratch[index] = zero_digit;
	};
	u64 length = _format_digits_u32(&!scratch, index, remaining, capacity);
	*?u8 data = null;
	unsafe { data = &?scratch[0]; };
	utf8 text = { .data = data; .length = length; };
	text
};

utf8(u64) _format_u64 = fn(value) {
	u8[20] scratch;
	u64 capacity = core.int_extend<u64>(20);
	u64 one = core.int_extend<u64>(1);
	u8 zero_digit = 48;
	u64 zero_value = 0;
	u64 index = capacity;
	u64 remaining = value;
	if (remaining == zero_value) {
		index = index - one;
		scratch[index] = zero_digit;
	};
	u64 length = _format_digits_u64(&!scratch, index, remaining, capacity);
	*?u8 data = null;
	unsafe { data = &?scratch[0]; };
	utf8 text = { .data = data; .length = length; };
	text
};

(u128, u8)(u128, u128, u8) _format_subtract_u128 = fn(remaining, scale, digit) {
	if (remaining >= scale) {
		u8 one_digit = 1;
		u128 rest, u8 next_digit = _format_subtract_u128(remaining - scale, scale, digit + one_digit);
		rest, next_digit
	} else {
		remaining, digit
	}
};

u64(*!(u8[]), u64, u64, u64, u64, u8, bool, bool, u64) _format_decimal_chunks = fn(scratch, index, remaining, upper, width, leading, has_upper, negative, capacity) {
	u64 zero = 0;
	u64 one = core.int_extend<u64>(1);
	u8 zero_digit = 48;
	if (remaining != zero || width != zero) {
		u64 ten = 10;
		u64 quotient = remaining / ten;
		u64 remainder = remaining - quotient * ten;
		u8 digit = core.int_trunc<u8>(remainder);
		index = index - one;
		u8 rendered_byte = zero_digit + digit;
		_format_store_byte(&!(*scratch), index, rendered_byte);
		if (width != zero) { width = width - one; };
		_format_decimal_chunks(scratch, index, quotient, upper, width, leading, has_upper, negative, capacity)
	} else {
		if (has_upper) {
			u64 padded = if (leading != 0) { core.int_extend<u64>(19) } else { zero };
			_format_decimal_chunks(scratch, index, upper, zero, padded, leading, false, negative, capacity)
		} else {
			if (leading != 0) {
				index = index - one;
				u8 rendered_byte = zero_digit + leading;
				_format_store_byte(&!(*scratch), index, rendered_byte);
			};
			if (negative) {
				index = index - one;
				_format_store_byte(&!(*scratch), index, 45);
			};
			u64 length = capacity - index;
			_format_compact_checked_bytes(scratch, zero, index, capacity);
			length
		}
	}
};

u64(*!(u8[]), u128, bool, u64) _format_wide_decimal = fn(scratch, value, negative, capacity) {
	u128 chunk_base = 10000000000000000000;
	u64 remaining = 0;
	u64 upper = 0;
	u64 width = core.int_extend<u64>(1);
	u8 leading = 0;
	bool has_upper = value >= chunk_base;
	if (has_upper) {
		u128 quotient = value / chunk_base;
		u128 remainder = value - quotient * chunk_base;
		remaining = core.int_trunc<u64>(remainder);
		quotient, leading = _format_subtract_u128(quotient, chunk_base, leading);
		upper = core.int_trunc<u64>(quotient);
		width = core.int_extend<u64>(19);
	} else {
		remaining = core.int_trunc<u64>(value);
	};
	_format_decimal_chunks(scratch, capacity, remaining, upper, width, leading, has_upper, negative, capacity)
};

utf8(u128) _format_u128 = fn(value) {
	u8[39] scratch;
	u64 capacity = core.int_extend<u64>(39);
	u64 length = _format_wide_decimal(&!scratch, value, false, capacity);
	*?u8 data = null;
	unsafe { data = &?scratch[0]; };
	utf8 text = { .data = data; .length = length; };
	text
};

(f64, i32)(f64, i32, u64) _format_normalize_float_down = fn(scaled, exponent, budget) {
	f64 ten_value = 10.0;
	u64 zero = 0;
	u64 one = 1;
	if (budget == zero || !(scaled >= ten_value)) {
		scaled, exponent
	} else {
		if (budget == one) {
			i32 one_count = 1;
			scaled / ten_value, exponent + one_count
		} else {
			u64 two = 2;
			u64 half = budget / two;
			f64 left, i32 left_exponent = _format_normalize_float_down(scaled, exponent, half);
			if (left >= ten_value) {
				f64 right, i32 right_exponent = _format_normalize_float_down(left, left_exponent, budget - half);
				right, right_exponent
			} else {
				left, left_exponent
			}
		}
	}
};

(f64, i32)(f64, i32, u64) _format_normalize_float_up = fn(scaled, exponent, budget) {
	f64 one_value = 1.0;
	u64 zero = 0;
	u64 one = 1;
	if (budget == zero || !(scaled < one_value)) {
		scaled, exponent
	} else {
		if (budget == one) {
			f64 ten_value = 10.0;
			i32 one_count = 1;
			scaled * ten_value, exponent - one_count
		} else {
			u64 two = 2;
			u64 half = budget / two;
			f64 left, i32 left_exponent = _format_normalize_float_up(scaled, exponent, half);
			if (left < one_value) {
				f64 right, i32 right_exponent = _format_normalize_float_up(left, left_exponent, budget - half);
				right, right_exponent
			} else {
				left, left_exponent
			}
		}
	}
};

i32(u8[], u64, u64, i32, bool) _format_carry_f32 = fn(scratch, round_position, digit_start, exponent, rounding) {
	u8 zero_digit = 48;
	if (rounding) {
		u64 one = core.int_extend<u64>(1);
		u8 point = 46;
		u8 nine_digit = 57;
		u8 one_digit = 1;
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
		_format_carry_f32(scratch, round_position, digit_start, exponent, rounding)
	} else {
		u8 leading = scratch[digit_start];
		if (leading == zero_digit) {
			u8 one_char = 49;
			i32 one_count = 1;
			scratch[digit_start] = one_char;
			exponent = exponent + one_count;
		};
		exponent
	}
};

(u64, i32)(u8[], u64, u64, u64, f64, i32) _format_digits_f32 = fn(scratch, digit_start, emit, produced, scaled, exponent) {
	u64 nine = core.int_extend<u64>(9);
	u64 one = core.int_extend<u64>(1);
	if (produced < nine) {
		u64 zero = core.int_extend<u64>(0);
		u8 zero_digit = 48;
		u8 point = 46;
		f64 ten_value = 10.0;
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
		u64 result_emit, i32 result_exponent = _format_digits_f32(scratch, digit_start, emit, produced + one, scaled, exponent);
		result_emit, result_exponent
	} else {
		f64 five_value = 5.0;
		if (scaled >= five_value) {
			exponent = _format_carry_f32(scratch, emit - one, digit_start, exponent, true);
		};
		emit, exponent
	}
};

utf8(f32) _format_f32 = fn(value) {
	u8[15] scratch;
	u64 one = core.int_extend<u64>(1);
	u64 zero = core.int_extend<u64>(0);
	f64 zero_value = 0.0;
	f64 one_value = 1.0;
	u8 zero_digit = 48;
	u8 minus = 45;
	u8 exponent_mark = 101;
	i32 zero_count = 0;
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
		u64 budget = 50;
		f64 down, i32 down_exponent = _format_normalize_float_down(magnitude, zero_count, budget);
		f64 scaled, i32 normalized_exponent = _format_normalize_float_up(down, down_exponent, budget);
		u64 digit_emit, i32 exponent = _format_digits_f32(scratch, emit, emit, zero, scaled, normalized_exponent);
		emit = digit_emit;
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

i32(u8[], u64, u64, i32, bool) _format_carry_f64 = fn(scratch, round_position, digit_start, exponent, rounding) {
	u8 zero_digit = 48;
	if (rounding) {
		u64 one = core.int_extend<u64>(1);
		u8 point = 46;
		u8 nine_digit = 57;
		u8 one_digit = 1;
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
		_format_carry_f64(scratch, round_position, digit_start, exponent, rounding)
	} else {
		u8 leading = scratch[digit_start];
		if (leading == zero_digit) {
			u8 one_char = 49;
			i32 one_count = 1;
			scratch[digit_start] = one_char;
			exponent = exponent + one_count;
		};
		exponent
	}
};

(u64, i32)(u8[], u64, u64, u64, f64, i32) _format_digits_f64 = fn(scratch, digit_start, emit, produced, scaled, exponent) {
	u64 seventeen = core.int_extend<u64>(17);
	u64 one = core.int_extend<u64>(1);
	if (produced < seventeen) {
		u64 zero = core.int_extend<u64>(0);
		u8 zero_digit = 48;
		u8 point = 46;
		f64 ten_value = 10.0;
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
		u64 result_emit, i32 result_exponent = _format_digits_f64(scratch, digit_start, emit, produced + one, scaled, exponent);
		result_emit, result_exponent
	} else {
		f64 five_value = 5.0;
		if (scaled >= five_value) {
			exponent = _format_carry_f64(scratch, emit - one, digit_start, exponent, true);
		};
		emit, exponent
	}
};

utf8(f64) _format_f64 = fn(value) {
	u8[24] scratch;
	u64 one = core.int_extend<u64>(1);
	u64 zero = core.int_extend<u64>(0);
	f64 zero_value = 0.0;
	f64 one_value = 1.0;
	u8 zero_digit = 48;
	u8 minus = 45;
	u8 exponent_mark = 101;
	i32 zero_count = 0;
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
		u64 budget = 400;
		f64 down, i32 down_exponent = _format_normalize_float_down(magnitude, zero_count, budget);
		f64 scaled, i32 normalized_exponent = _format_normalize_float_up(down, down_exponent, budget);
		u64 digit_emit, i32 exponent = _format_digits_f64(scratch, emit, emit, zero, scaled, normalized_exponent);
		emit = digit_emit;
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
	utf8(char) => fn(value) { _format_char(value) };
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

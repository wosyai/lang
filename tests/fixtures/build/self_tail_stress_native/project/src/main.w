%%start
host = extern wasm "fixture_stdin" {
	i32() getchar;
};

i32() read_workload = fn { host.getchar() };

i32 phase = 0;
i32 first_count = 0;
i32 second_count = 0;

i32(i32) first_arg = fn(value) {
	if (phase != 0) { core.system_panic(); };
	phase = 1;
	first_count = first_count + 1;
	value
};

i32(i32) second_arg = fn(value) {
	if (phase != 1) { core.system_panic(); };
	phase = 0;
	second_count = second_count + 1;
	value
};

i32(i32, i32, i32) swap = fn(left, right, remaining) {
	if (remaining == 0) {
		left * 100 + right
	} else {
		swap(first_arg(right), second_arg(left), remaining - 1)
	}
};

unit() check = fn {
	i32 input = read_workload();
	if (input != 0 && input != 1) { core.system_panic(); };
	i32 iterations = 1000000 + input;
	i32 encoded = swap(17, 39, iterations);
	i32 expected = if (input == 0) { 1739 } else { 3917 };
	if (phase != 0 || first_count != iterations || second_count != iterations
		|| encoded != expected) {
		core.system_panic();
	};
};

u64(u64, bool) replace_wide = fn(value, done) {
	if (done) { value } else { replace_wide(0 + 1, true) }
};

check();
u64 wide_result = replace_wide(4294967296, false);
if (wide_result != 1) { core.system_panic(); };
%%end

%%start
i32(i32, i32) add = fn(left, right) {
	left + right
};

i32 seed = 20;
i32 increment = 22;
bool choose_sum = true;
i32 result = if (choose_sum) {
	add(seed, increment)
} else {
	0
};
%%end

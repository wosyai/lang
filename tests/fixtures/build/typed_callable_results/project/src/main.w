%%start
bool(i32) flag = fn(flag_value) {
	flag_value == 42
};

i32(i32) value = fn(input) {
	input
};

unit(i32) touch = fn(input) {
	i32 ignored = value(input);
};

bool observed = flag(42);
unit finished = touch(42);
i32 result = value(42);
%%end

%%start
std = namespace std "bootstrap.w";

(i32, i32)(i32, i32) pair = fn(first_value, second_value) {
	first_value, second_value
};

(i32, i32)() initial = fn {
	3, 5
};

unit() run = fn {
	first, second = initial();
	first, second = second, first;
	first, second = pair(second, first);
	std.utf8 message = "ordered assignments failed\n";
	if (first == 3 && second == 5) {
		message = "ordered assignments\n";
	};
	u64 reported, bool complete = std.print(message);
	reported;
	complete;
};
run();
%%end

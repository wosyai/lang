%%start
i32(i32) loop = fn(start) {
	i32 value = start;
	while (value < 3) {
		value = value + 1;
	}
	value
};

i32 result = loop(0);
%%end

%%start
i32(i32) loop = fn(start) {
	i32 value = start;
	if (value < 3) {
		value = true;
		loop(value)
	} else {
		value
	}
};

i32 result = loop(0);
%%end

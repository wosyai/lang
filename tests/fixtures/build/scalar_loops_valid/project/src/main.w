%%start
i32(i32) loop = fn(start) {
	if (start < 3) {
		loop(start + 1)
	} else {
		start
	}
};

i32 result = loop(0);
%%end

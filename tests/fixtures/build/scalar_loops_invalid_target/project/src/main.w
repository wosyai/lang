%%start
i32(i32) loop = fn(start) {
	if (start < 3) {
		missing = start + 1;
		loop(start)
	} else {
		start
	}
};

i32 result = loop(0);
%%end

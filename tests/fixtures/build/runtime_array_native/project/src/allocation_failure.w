%%start
unit() run = fn {
	u64 length = 18446744073709551615;
	u8[length] bytes;
	bytes[0] = 1;
};

run();
%%end

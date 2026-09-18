%%start
unit() run = fn {
	u64 length = 3;
	u8[length] bytes;
	bytes[0] = 7;
	bytes[1] = 8;
	bytes[2] = 9;
	u8 value = bytes[1];
	u8 expected = 8;
	unsafe {
		*?u8 storage = &?bytes[0];
		core.free(storage);
	};
	if (value != expected) {
		core.system_panic();
	};
};

run();
%%end

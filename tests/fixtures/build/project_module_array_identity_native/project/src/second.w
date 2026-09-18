%%start
u64 length = 1;
u8[length] bytes;
bytes[0] = 11;

u8() read = fn {
	bytes[0]
};
%%end

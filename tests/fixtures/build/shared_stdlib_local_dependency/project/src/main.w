%%start
std = namespace std "bootstrap.w";
std.utf8 text = "hé";
unsafe {
	*?u8 data = text.data;
	u64 length = text.length;
};
%%end

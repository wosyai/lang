%%start
struct Record {
	u8 tag;
}

unit() run = fn {
	*?u8 raw = null;
	u8 value = (*core.pointer_cast<*Record>(raw)).tag;
	value;
};

run();
%%end

%%start
std = namespace std "bootstrap.w";

struct Record {
	u32 field;
}

u32(u32) transport_shared = fn(value) {
	value
};

u32(u32) transport_mutable = fn(value) {
	value
};

unit() run = fn {
	u32 value = 4;
	Record record = { .field = 9; };

	*u32 shared = &value;
	*!u32 mutable = &!value;
	*Record record_shared = &record;
	*!Record record_mutable = &!record;
	transport_shared(*shared);
	transport_mutable(*mutable);
	transport_shared((*record_shared).field);
	transport_mutable((*record_mutable).field);
	std.print("checked reference dereference\n");
};

run();
%%end

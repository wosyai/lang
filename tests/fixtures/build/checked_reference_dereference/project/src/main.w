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
	transport_shared(*shared);
	*Record record_shared = &record;
	transport_shared((*record_shared).field);

	*!u32 mutable = &!value;
	transport_mutable(*mutable);
	*!Record record_mutable = &!record;
	transport_mutable((*record_mutable).field);
	std.print("checked reference dereference\n");
};

run();
%%end

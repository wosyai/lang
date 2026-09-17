%%start
std = namespace std "bootstrap.w";

struct Record {
	u8 field;
}

*(u8[4])(*(u8[4])) transport_shared_array = fn(reference) {
	reference
};

*!(u8[4])(*!(u8[4])) transport_mutable_array = fn(reference) {
	reference
};

*u8(*u8) transport_shared_field = fn(reference) {
	reference
};

*!u8(*!u8) transport_mutable_field = fn(reference) {
	reference
};

unit() run = fn {
	u8[4] bytes = [1, 2, 3, 4];
	Record record = { .field = 5; };

	transport_shared_array(&bytes);
	transport_mutable_array(&!bytes);
	transport_shared_field(&record.field);
	transport_mutable_field(&!record.field);
	std.print("checked references\n");
};

run();
%%end

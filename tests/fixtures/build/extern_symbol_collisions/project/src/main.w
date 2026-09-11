%%start
left = extern wasm "env" {
	i32() ping;
};
right = extern wasm "env" {
	i32() ping;
};

i32() ping = fn {
	7
};

i32 first = left.ping();
i32 second = right.ping();
%%end

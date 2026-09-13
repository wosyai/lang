%%start
env = extern wasm "helper" {
	(i32, u64)() read;
};

(i32, u64)() make_pair = fn {
	env.read()
};

i32(i32) identity = fn(value) {
	value
};

(i32, u64)() external_pair = fn {
	env.read()
};

child = namespace multiple_outputs "src/child.w";

i32 first, u64 second = 7, 99;
(i32, bool)() use_child = fn {
	child.pair()
};
i32 direct = identity(42);
%%end

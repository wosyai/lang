%%start
env = extern wasm "child_helper" {
	(i32, bool)() read;
};

(i32, bool)() pair = fn {
	env.read()
};
%%end

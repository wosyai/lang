%%start
env = extern wasm "env" {
	unit(i32) print_i32;
};
# INTENT:  exact payload  
i32(i32) add_one = fn(value) {
	value + 1
};
unit() main = fn {
	env.print_i32(add_one(41));
};
%%end

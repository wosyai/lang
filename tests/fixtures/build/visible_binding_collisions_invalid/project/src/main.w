%%start
i32 value = 1;
math = namespace visible_binding_collisions_invalid "src/math.w";
i32(i32) parameter_collision = fn(value) { value };
i32(i32, i32) repeated_parameter = fn(first, first) { first };
i32(i32) nested_collision = fn(outer) {
	if (true) {
		i32 outer = 1;
		outer
	} else {
		outer
	}
};
i32(i32) namespace_collision = fn(math) { 0 };
%%end

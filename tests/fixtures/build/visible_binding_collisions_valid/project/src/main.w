%%start
i32 module_value = 1;
i32(i32) distinct = fn(parameter) {
	if (true) {
		i32 inner = parameter;
		inner
	} else {
		parameter
	}
};
%%end

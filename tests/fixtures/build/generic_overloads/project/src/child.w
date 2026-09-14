%%start
identity = overload {
	i32(i64) => fn(value) {
		7
	};

	generic T;
	T(T) => fn(value) {
		value
	};
};
%%end

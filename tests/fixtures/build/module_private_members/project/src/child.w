%%start
struct _Private {
	i32 value;
}

i32 _binding = 1;

i32() _function = fn {
	_binding
};

_ordinary = overload {
	i32(i64) => fn(value) {
		value
	};
};

_identity = overload {
	i32(i64) => fn(value) {
		7
	};

	generic T;
	T(T) => fn(value) {
		value
	};
};

ffi = extern wasm "env" {
	i32() _extern;
};

struct Public {
	i32 value;
}

i32 public_binding = _function();

i32() public_function = fn {
	public_binding
};
%%end

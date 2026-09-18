%%start
struct _Private {
	i32 value;
}

i32 _binding = 1;

i32() _function = fn {
	_binding
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

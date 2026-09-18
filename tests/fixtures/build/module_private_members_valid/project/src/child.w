%%start
i32() _helper = fn {
	42
};

i32() public_value = fn {
	_helper()
};
%%end

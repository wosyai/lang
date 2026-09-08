%%start
i32 count = 1;
i32() read = fn {
	count
};
count = 42;
i32 observed = read();
second = namespace top_level_assignment "src/second.w";
%%end

%%start
i32 count = 42;
i32() read_count = fn {
	count
};
i32 observed = read_count();
storage = namespace top_level_storage "src/storage.w";
%%end

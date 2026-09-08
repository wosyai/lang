%%start
i32 calls = 0;
bool() right = fn {
	calls = calls + 1;
	true
};
%%end

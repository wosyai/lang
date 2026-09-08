%%start
i32 calls = 0;
bool() right = fn {
	calls = calls + 1;
	true
};

bool skipped_and = false && right();
bool called_and = true && right();
bool skipped_or = true || right();
bool called_or = false || right();
%%end

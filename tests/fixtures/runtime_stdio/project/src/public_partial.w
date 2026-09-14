%%start
std = namespace std "bootstrap.w";
u64 reported, bool complete = std.print("partial\n");
u64 expected = core.int_extend<u64>(3);

if (reported == expected && complete == false) {
	u64 result_reported, bool result_complete = std.eprint("partial result\n");
} else {
	u64 mismatch_reported, bool mismatch_complete = std.eprint("partial result mismatch\n");
};
%%end

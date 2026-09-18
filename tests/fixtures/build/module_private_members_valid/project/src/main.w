%%start
std = namespace std "bootstrap.w";
child = namespace module_private_members_valid "src/child.w";

unit(bool) verify = fn(value) {
	value;
};

i32 result = child.public_value();
verify(result == 42);
u64 reported, bool complete = std.print("module-private helper\n");
%%end

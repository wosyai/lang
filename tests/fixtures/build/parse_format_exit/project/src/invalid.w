%%start
std = namespace std "bootstrap.w";

std.utf8 invalid_input = "12x";
*u64 rejected = std.parse(invalid_input);
if (rejected == null) {
	std.print("invalid\n");
};
std.exit(3);
%%end

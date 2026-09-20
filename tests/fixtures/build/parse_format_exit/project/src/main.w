%%start
std = namespace std "bootstrap.w";

std.utf8 valid_input = "42";
std.utf8 invalid_input = "12x";
*u64 rejected = std.parse(invalid_input);
*u64 parsed = std.parse(valid_input);
u64 number = *parsed;
u64 expected = 42;
if (rejected == null && parsed != null && number == expected) {
	std.print("parsed\n");
};
std.exit(7);
%%end

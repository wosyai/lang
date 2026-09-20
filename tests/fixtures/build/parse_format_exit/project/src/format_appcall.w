%%start
std = namespace std "bootstrap.w";

u64 zero = 0;
std.print(std.format(zero));
std.print("\n");
u64 answer = 42;
std.utf8 text = std.format(answer);
std.print(text);
std.print("\n");
std.exit(0);
%%end

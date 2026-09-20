%%start
std = namespace std "bootstrap.w";

i128 zero = 0;
std.utf8 out0 = std.format(zero);
std.print(out0);
std.print("\n");
i128 small = 42;
std.utf8 out1 = std.format(small);
std.print(out1);
std.print("\n");
i128 big = 123456789;
std.utf8 out2 = std.format(big);
std.print(out2);
std.print("\n");
std.exit(0);
%%end

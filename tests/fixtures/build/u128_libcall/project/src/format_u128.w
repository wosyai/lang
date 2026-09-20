%%start
std = namespace std "bootstrap.w";

u128 zero = 0;
std.utf8 out0 = std.format(zero);
std.print(out0);
std.print("\n");
u128 one = 1;
std.utf8 out1 = std.format(one);
std.print(out1);
std.print("\n");
u128 big = 1000000;
std.utf8 out2 = std.format(big);
std.print(out2);
std.print("\n");
std.exit(0);
%%end

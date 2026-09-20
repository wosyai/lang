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
i128 min = -170141183460469231731687303715884105728;
std.utf8 out3 = std.format(min);
std.print(out3);
std.print("\n");
i128 max = 170141183460469231731687303715884105727;
std.utf8 out4 = std.format(max);
std.print(out4);
std.print("\n");
std.exit(0);
%%end

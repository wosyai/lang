%%start
std = namespace std "bootstrap.w";

u64 delay = 1000000;
std.sleep_ns(delay);
std.print("slept small\n");
%%end

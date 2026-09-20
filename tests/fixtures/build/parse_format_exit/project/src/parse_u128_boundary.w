%%start
std = namespace std "bootstrap.w";

std.utf8 max_text = "340282366920938463463374607431768211455";
*u128 max_value = std.parse(max_text);
u128 max_number = *max_value;
std.utf8 max_out = std.format(max_number);
std.print(max_out);
std.print("\n");
std.utf8 over_text = "340282366920938463463374607431768211456";
*u128 over_value = std.parse(over_text);
if (over_value == null) {
	std.print("null\n");
};
if (over_value != null) {
	std.print("UNEXPECTED\n");
};
std.utf8 min_text = "-170141183460469231731687303715884105728";
*i128 min_value = std.parse(min_text);
i128 min_number = *min_value;
std.utf8 min_out = std.format(min_number);
std.print(min_out);
std.print("\n");
std.utf8 imax_text = "170141183460469231731687303715884105727";
*i128 imax_value = std.parse(imax_text);
i128 imax_number = *imax_value;
std.utf8 imax_out = std.format(imax_number);
std.print(imax_out);
std.print("\n");
std.utf8 hex_text = "0xFF";
*u128 hex_value = std.parse(hex_text);
u128 hex_number = *hex_value;
std.utf8 hex_out = std.format(hex_number);
std.print(hex_out);
std.print("\n");
u128 max_lit = 340282366920938463463374607431768211455;
std.utf8 max_lit_out = std.format(max_lit);
std.print(max_lit_out);
std.print("\n");
i128 min_lit = -170141183460469231731687303715884105728;
std.utf8 min_lit_out = std.format(min_lit);
std.print(min_lit_out);
std.print("\n");
std.exit(0);
%%end

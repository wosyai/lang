%%start
std = namespace std "bootstrap.w";

unit() check_u8 = fn {
	std.utf8 t_zero = "0";
	*u8 v_zero = std.parse(t_zero);
	u8 n_zero = *v_zero;
	std.utf8 o_zero = std.format(n_zero);
	std.print(o_zero);
	std.print("\n");
	std.utf8 t_max = "255";
	*u8 v_max = std.parse(t_max);
	u8 n_max = *v_max;
	std.utf8 o_max = std.format(n_max);
	std.print(o_max);
	std.print("\n");
	std.utf8 t_over = "256";
	*u8 v_over = std.parse(t_over);
	if (v_over == null) {
		std.print("null\n");
	};
	if (v_over != null) {
		std.print("UNEXPECTED\n");
	};
	std.utf8 t_plus = "+1";
	*u8 v_plus = std.parse(t_plus);
	if (v_plus == null) {
		std.print("null\n");
	};
	if (v_plus != null) {
		std.print("UNEXPECTED\n");
	};
	std.utf8 t_minus = "-1";
	*u8 v_minus = std.parse(t_minus);
	if (v_minus == null) {
		std.print("null\n");
	};
	if (v_minus != null) {
		std.print("UNEXPECTED\n");
	};
};
unit() check_i8 = fn {
	std.utf8 t_min = "-128";
	*i8 v_min = std.parse(t_min);
	i8 n_min = *v_min;
	std.utf8 o_min = std.format(n_min);
	std.print(o_min);
	std.print("\n");
	std.utf8 t_max = "127";
	*i8 v_max = std.parse(t_max);
	i8 n_max = *v_max;
	std.utf8 o_max = std.format(n_max);
	std.print(o_max);
	std.print("\n");
	std.utf8 t_over = "128";
	*i8 v_over = std.parse(t_over);
	if (v_over == null) {
		std.print("null\n");
	};
	if (v_over != null) {
		std.print("UNEXPECTED\n");
	};
	std.utf8 t_under = "-129";
	*i8 v_under = std.parse(t_under);
	if (v_under == null) {
		std.print("null\n");
	};
	if (v_under != null) {
		std.print("UNEXPECTED\n");
	};
};
unit() check_u16 = fn {
	std.utf8 t_max = "65535";
	*u16 v_max = std.parse(t_max);
	u16 n_max = *v_max;
	std.utf8 o_max = std.format(n_max);
	std.print(o_max);
	std.print("\n");
	std.utf8 t_over = "65536";
	*u16 v_over = std.parse(t_over);
	if (v_over == null) {
		std.print("null\n");
	};
	if (v_over != null) {
		std.print("UNEXPECTED\n");
	};
};
unit() check_i16 = fn {
	std.utf8 t_min = "-32768";
	*i16 v_min = std.parse(t_min);
	i16 n_min = *v_min;
	std.utf8 o_min = std.format(n_min);
	std.print(o_min);
	std.print("\n");
	std.utf8 t_max = "32767";
	*i16 v_max = std.parse(t_max);
	i16 n_max = *v_max;
	std.utf8 o_max = std.format(n_max);
	std.print(o_max);
	std.print("\n");
	std.utf8 t_over = "32768";
	*i16 v_over = std.parse(t_over);
	if (v_over == null) {
		std.print("null\n");
	};
	if (v_over != null) {
		std.print("UNEXPECTED\n");
	};
	std.utf8 t_under = "-32769";
	*i16 v_under = std.parse(t_under);
	if (v_under == null) {
		std.print("null\n");
	};
	if (v_under != null) {
		std.print("UNEXPECTED\n");
	};
};
unit() check_u32 = fn {
	std.utf8 t_max = "4294967295";
	*u32 v_max = std.parse(t_max);
	u32 n_max = *v_max;
	std.utf8 o_max = std.format(n_max);
	std.print(o_max);
	std.print("\n");
	std.utf8 t_over = "4294967296";
	*u32 v_over = std.parse(t_over);
	if (v_over == null) {
		std.print("null\n");
	};
	if (v_over != null) {
		std.print("UNEXPECTED\n");
	};
};
unit() check_i32 = fn {
	std.utf8 t_min = "-2147483648";
	*i32 v_min = std.parse(t_min);
	i32 n_min = *v_min;
	std.utf8 o_min = std.format(n_min);
	std.print(o_min);
	std.print("\n");
	std.utf8 t_max = "2147483647";
	*i32 v_max = std.parse(t_max);
	i32 n_max = *v_max;
	std.utf8 o_max = std.format(n_max);
	std.print(o_max);
	std.print("\n");
	std.utf8 t_over = "2147483648";
	*i32 v_over = std.parse(t_over);
	if (v_over == null) {
		std.print("null\n");
	};
	if (v_over != null) {
		std.print("UNEXPECTED\n");
	};
	std.utf8 t_under = "-2147483649";
	*i32 v_under = std.parse(t_under);
	if (v_under == null) {
		std.print("null\n");
	};
	if (v_under != null) {
		std.print("UNEXPECTED\n");
	};
};
unit() check_misc = fn {
	std.utf8 t_empty = "";
	*u64 v_empty = std.parse(t_empty);
	if (v_empty == null) {
		std.print("null\n");
	};
	if (v_empty != null) {
		std.print("UNEXPECTED\n");
	};
	std.utf8 t_bad_digit = "12x";
	*u64 v_bad_digit = std.parse(t_bad_digit);
	if (v_bad_digit == null) {
		std.print("null\n");
	};
	if (v_bad_digit != null) {
		std.print("UNEXPECTED\n");
	};
	std.utf8 t_inf = "inf";
	*f64 v_inf = std.parse(t_inf);
	if (v_inf == null) {
		std.print("null\n");
	};
	if (v_inf != null) {
		std.print("UNEXPECTED\n");
	};
	std.utf8 t_nan = "nan";
	*f64 v_nan = std.parse(t_nan);
	if (v_nan == null) {
		std.print("null\n");
	};
	if (v_nan != null) {
		std.print("UNEXPECTED\n");
	};
	std.utf8 t_plus = "+1";
	*f64 v_plus = std.parse(t_plus);
	if (v_plus == null) {
		std.print("null\n");
	};
	if (v_plus != null) {
		std.print("UNEXPECTED\n");
	};
};
unit() check_bad_c3 = fn {
	u8[1] raw = [195];
	*?u8 data = null;
	unsafe {
		data = &?raw[0];
	};
	std.utf8 text = { .data = data; .length = 1; };
	*u8 value = std.parse(text);
	if (value == null) {
		std.print("null\n");
	};
	if (value != null) {
		std.print("UNEXPECTED\n");
	};
};
unit() check_bad_ff = fn {
	u8[1] raw = [255];
	*?u8 data = null;
	unsafe {
		data = &?raw[0];
	};
	std.utf8 text = { .data = data; .length = 1; };
	*u8 value = std.parse(text);
	if (value == null) {
		std.print("null\n");
	};
	if (value != null) {
		std.print("UNEXPECTED\n");
	};
};
unit() check_bool = fn {
	std.utf8 s_true = std.format(true);
	std.print(s_true);
	std.print("\n");
	std.utf8 s_false = std.format(false);
	std.print(s_false);
	std.print("\n");
};
check_u8();
check_i8();
check_u16();
check_i16();
check_u32();
check_i32();
check_misc();
check_bad_c3();
check_bad_ff();
check_bool();
std.exit(0);
%%end

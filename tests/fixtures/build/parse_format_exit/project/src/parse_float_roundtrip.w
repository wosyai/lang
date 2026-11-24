%%start
std = namespace std "bootstrap.w";

unit() check_f64a = fn {
	std.utf8 t_a = "1.5";
	*f64 v_a = std.parse(t_a);
	f64 n_a = *v_a;
	std.utf8 o_a = std.format(n_a);
	std.print(o_a);
	std.print("\n");
	std.utf8 t_b = "-2.25";
	*f64 v_b = std.parse(t_b);
	f64 n_b = *v_b;
	std.utf8 o_b = std.format(n_b);
	std.print(o_b);
	std.print("\n");
};
unit() check_f64b = fn {
	std.utf8 t_exp = "1e3";
	*f64 v_exp = std.parse(t_exp);
	f64 n_exp = *v_exp;
	std.utf8 o_exp = std.format(n_exp);
	std.print(o_exp);
	std.print("\n");
	std.utf8 t_tiny = "1.25e-4";
	*f64 v_tiny = std.parse(t_tiny);
	f64 n_tiny = *v_tiny;
	std.utf8 o_tiny = std.format(n_tiny);
	std.print(o_tiny);
	std.print("\n");
};
unit() check_f32 = fn {
	std.utf8 t_a = "1.5";
	*f32 v_a = std.parse(t_a);
	f32 n_a = *v_a;
	std.utf8 o_a = std.format(n_a);
	std.print(o_a);
	std.print("\n");
	std.utf8 t_b = "-2.25";
	*f32 v_b = std.parse(t_b);
	f32 n_b = *v_b;
	std.utf8 o_b = std.format(n_b);
	std.print(o_b);
	std.print("\n");
};
unit() check_roundtrip_f64 = fn {
	std.utf8 t_base = "1.5";
	*f64 v_base = std.parse(t_base);
	f64 original = *v_base;
	std.utf8 text = std.format(original);
	std.print(std.format(original));
	std.print("\n");
	*f64 parsed = std.parse(text);
	f64 value = *parsed;
	if (value == original) {
		std.print("match\n");
	};
	if (value != original) {
		std.print("MISMATCH\n");
	};
};
unit() check_roundtrip_f64_neg = fn {
	std.utf8 t_base = "-2.25";
	*f64 v_base = std.parse(t_base);
	f64 original = *v_base;
	std.utf8 text = std.format(original);
	std.print(std.format(original));
	std.print("\n");
	*f64 parsed = std.parse(text);
	f64 value = *parsed;
	if (value == original) {
		std.print("match\n");
	};
	if (value != original) {
		std.print("MISMATCH\n");
	};
};
unit() check_roundtrip_f32 = fn {
	std.utf8 t_base = "1.5";
	*f32 v_base = std.parse(t_base);
	f32 original = *v_base;
	std.utf8 text = std.format(original);
	std.print(std.format(original));
	std.print("\n");
	*f32 parsed = std.parse(text);
	f32 value = *parsed;
	if (value == original) {
		std.print("match\n");
	};
	if (value != original) {
		std.print("MISMATCH\n");
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
check_f64a();
check_f64b();
check_f32();
check_roundtrip_f64();
check_roundtrip_f64_neg();
check_roundtrip_f32();
check_bool();
std.exit(0);
%%end

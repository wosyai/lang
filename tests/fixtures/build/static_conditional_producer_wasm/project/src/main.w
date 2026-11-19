%%start
u64 condition_count = 0;
u64 then_count = 0;
u64 else_count = 0;

bool(bool) test_condition = fn(polarity) {
	condition_count = condition_count + 1;
	polarity
};

(i32, bool)(bool) choose = fn(polarity) {
	if (test_condition(polarity)) {
		then_count = then_count + 1;
		17, true
	} else {
		else_count = else_count + 1;
		42, false
	}
};

unit() check = fn {
	i32 then_value, bool then_flag = choose(true);
	if (then_value != 17 || then_flag != true
		|| condition_count != 1 || then_count != 1 || else_count != 0) {
		core.system_panic();
	};

	i32 else_value, bool else_flag = choose(false);
	if (else_value != 42 || else_flag != false
		|| condition_count != 2 || then_count != 1 || else_count != 1) {
		core.system_panic();
	};
};

check();
%%end

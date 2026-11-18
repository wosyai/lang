%%start
unit() check = fn {
	u8 first = 1;
	u8 second = 2;
	*!u8 selected = &!first;
	selected, *selected = &!second, 9;
	u8 observed = *selected;
	u8 first_expected = 9;
	u8 observed_expected = 2;
	if (first != first_expected || observed != observed_expected) {
		core.system_panic();
	};
};

bool rhs_ready = false;
bool selector_saw_rhs = false;
u64 selector_count = 0;

u8() replacement = fn {
	rhs_ready = true;
	9
};

u64(u64) select_index = fn(current) {
	selector_count = selector_count + 1;
	selector_saw_rhs = rhs_ready;
	current
};

*!u8(*!u8) retain_item = fn(item) {
	item
};

unit() check_selector_effects = fn {
	u64 index = 0;
	u8[2] items = [3, 4];
	index, *retain_item(&!items[select_index(index)]) = 1, replacement();
	if (selector_count != 1 || selector_saw_rhs == false
		|| index != 1 || items[0] != 9 || items[1] != 4) {
		core.system_panic();
	};
};

check();
check_selector_effects();
%%end

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
check();
%%end

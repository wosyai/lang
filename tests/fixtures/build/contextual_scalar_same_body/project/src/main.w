%%start
i8(i8) counter = fn(selector) {
	i8 core.context.count = 1;
	if (selector < 0) {
		i8 core.context.count = core.context.count;
		if (core.context.count != 1) { core.system_panic(); };
		core.context.count = 2;
		if (core.context.count != 2) { core.system_panic(); };
	} else {
		core.context.count = 3;
	};
	core.context.count
};

unit() verify = fn {
	i8 first = counter(-1);
	i8 second = counter(2);
	if (first != 1 || second != 3) { core.system_panic(); };
};

verify();
%%end

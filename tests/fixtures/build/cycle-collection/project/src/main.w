%%start
(unit())() make_scalar = fn {
	unit() inner = fn { 1; };
	inner = fn { inner(); };
	inner
};
(unit())() make_installed = fn {
	(unit())[1] slots;
	unit() inner = fn { unit() picked = slots[0]; picked(); };
	slots[0] = inner;
	inner
};
unit() run_scalar = fn {
	unit() first = make_scalar();
	unit() copied = first;
	unit() second = make_scalar();
	copied;
	second;
};
unit() run_installed = fn {
	unit() first = make_installed();
	unit() copied = first;
	unit() second = make_installed();
	copied;
	second;
};
run_scalar();
run_installed();
%%end

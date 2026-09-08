%%start
unit() touch = fn {
	touch();
};
unit done = touch();

unit() use = fn {
	unit local = touch();
	local;
	local = touch();
	done;
	done = touch();
};

use();
%%end

%%start
child = namespace module_unit_effects "src/child.w";

unit() use = fn {
	unit local = child.marker;
	local;
	child.marker = child.touch();
};

use();
unit observed = child.marker;
child.marker = child.touch();
%%end

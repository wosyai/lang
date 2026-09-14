%%start
std = namespace std "bootstrap.w";
child = namespace generic_overloads "src/child.w";

generic T;
T(T) local_identity = fn(value) {
	value
};

select = overload {
	i32(i64) => fn(value) {
		7
	};

	generic T;
	T(T) => fn(value) {
		value
	};
};

pair = overload {
	generic T;
	(T, T)(T) => fn(value) {
		value
	};
};

unit(bool) verify = fn(value) {
	value;
};

i64 wide = 41;
i64 local = local_identity<i64>(wide);
char namespaced = child.identity('g');
i32 ordinary = select(wide);
bool generic = select(true);
i64 first, i64 second = pair(wide);
i64 expected_wide = 41;
bool selected = local == expected_wide && namespaced == 'g' && ordinary == 7 && generic && first == expected_wide && second == expected_wide;
verify(selected);
u64 reported, bool complete = std.print("generic overloads selected\n");
%%end

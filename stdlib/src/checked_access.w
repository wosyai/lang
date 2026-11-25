%%start
unit(*?u8) check_non_null_address = fn(address) {
	if (address == null) { core.system_panic(); };
};

unit(u64, u64) check_index_in_extent = fn(index, count) {
	if (index >= count) { core.system_panic(); };
};
%%end

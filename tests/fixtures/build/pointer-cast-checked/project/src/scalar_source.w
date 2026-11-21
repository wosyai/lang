%%start
*?u8(u8) cast = fn(value) {
	unsafe { core.pointer_cast<*?u8>(value) }
};
%%end

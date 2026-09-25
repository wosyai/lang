%%start
*?u8(*?u8) cast = fn(pointer) {
	unsafe { core.pointer_cast<*?u8, *?u8>(pointer) }
};
%%end

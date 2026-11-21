%%start
u32(*?u8) cast = fn(pointer) {
	unsafe { core.pointer_cast<u32>(pointer) }
};
%%end

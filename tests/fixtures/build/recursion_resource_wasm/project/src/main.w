%%start
probe = extern wasm "fixture_resource" {
	unit(i32, u64, *?u8) checkpoint;
};

struct Backing {
	u8[3] bytes;
}

*!Backing(*!Backing, u64, u64, u64) step = fn(view, index, iteration, length) {
	u8[length] scratch;
	scratch[0] = core.int_trunc<u8>(iteration) + 19;
	scratch[length - 1] = 173;
	(*view).bytes[index] = scratch[0];
	unsafe { probe.checkpoint(1, iteration, &?scratch[0]); };
	view
};

unit(*!Backing, u64, u64) walk = fn(view, iteration, length) {
	if (iteration == 3) {
		unsafe { probe.checkpoint(3, iteration, core.pointer_cast<*?u8>(&?*view)); };
	} else {
		probe.checkpoint(0, iteration, null);
		*!Backing seen = step(view, iteration % 2, iteration, length);
		unsafe { probe.checkpoint(2, iteration, core.pointer_cast<*?u8>(&?*seen)); };
		walk(seen, iteration + 1, length);
	};
};

unit() check = fn {
	Backing backing = { .bytes = [5, 7, 91]; };
	unsafe { probe.checkpoint(5, 0, core.pointer_cast<*?u8>(&?backing)); };
	*!Backing original = &!backing;
	walk(original, 0, 4);
	if (backing.bytes[0] != 21
		|| backing.bytes[1] != 20 || backing.bytes[2] != 91) {
		core.system_panic();
	};
	unsafe { probe.checkpoint(6, 3, core.pointer_cast<*?u8>(&?backing)); };
};

check();
probe.checkpoint(4, 3, null);
%%end

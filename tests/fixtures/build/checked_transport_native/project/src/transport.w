%%start
probe = extern wasm "fixture_transport" {
	unsafe unit(i32, *?u8) checkpoint;
	unsafe unit(*?u8) write_byte;
};

unit(i32, *?u8) checkpoint = fn(event, address) {
	unsafe { probe.checkpoint(event, address); };
};

unit(*!u8) step = fn(view) {
	unsafe { probe.write_byte(&?*view); };
};
%%end

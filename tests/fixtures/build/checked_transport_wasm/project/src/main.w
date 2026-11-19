%%start
transport = namespace checked_transport_wasm "src/transport.w";

unit() check = fn {
	u8[2] scratch = [5, 91];
	unsafe { transport.checkpoint(0, &?scratch[0]); };
	*!u8 view = &!scratch[0];
	transport.step(view);
	if (scratch[0] != 173 || scratch[1] != 91) { core.system_panic(); };
	unsafe { transport.checkpoint(1, &?scratch[0]); };
};

check();
%%end

%%start
transport = namespace checked_transport_wasm "src/transport.w";

unit() check = fn {
	transport.checkpoint(2, null);
	*!u8 missing = null;
	transport.step(missing);
};

check();
%%end

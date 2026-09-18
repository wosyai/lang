%%start
first = namespace project_module_array_identity_wasm "src/first.w";
second = namespace project_module_array_identity_wasm "src/second.w";

u8 first_value = first.read();
u8 second_value = second.read();
u8 first_expected = 7;
u8 second_expected = 11;
if (first_value != first_expected || second_value != second_expected) {
	core.system_panic();
};
%%end

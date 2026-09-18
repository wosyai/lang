%%start
first = namespace project_module_array_identity_native "src/first.w";
second = namespace project_module_array_identity_native "src/second.w";

u8 first_value = first.read();
u8 second_value = second.read();
if (first_value != second_value) {
	core.system_panic();
};
%%end

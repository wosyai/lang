%%start
std = namespace std "bootstrap.w";

u8[39] bytes0 = [49, 55, 48, 49, 52, 49, 49, 56, 51, 52, 54, 48, 52, 54, 57, 50, 51, 49, 55, 51, 49, 54, 56, 55, 51, 48, 51, 55, 49, 53, 56, 56, 52, 49, 48, 53, 55, 50, 55];
*?u8 data0 = null;
unsafe { data0 = &?bytes0[0]; };
std.utf8 text0 = { .data = data0; .length = 39; };
*i128 value0 = std.parse(text0);
i128 number0 = *value0;
i128 expected0 = 170141183460469231731687303715884105727;
if (number0 == expected0) {
	std.print("match\n");
};
if (number0 != expected0) {
	std.print("MISMATCH\n");
};
u8[40] bytes1 = [45, 49, 55, 48, 49, 52, 49, 49, 56, 51, 52, 54, 48, 52, 54, 57, 50, 51, 49, 55, 51, 49, 54, 56, 55, 51, 48, 51, 55, 49, 53, 56, 56, 52, 49, 48, 53, 55, 50, 56];
*?u8 data1 = null;
unsafe { data1 = &?bytes1[0]; };
std.utf8 text1 = { .data = data1; .length = 40; };
*i128 value1 = std.parse(text1);
i128 number1 = *value1;
i128 expected1 = -170141183460469231731687303715884105728;
if (number1 == expected1) {
	std.print("match\n");
};
if (number1 != expected1) {
	std.print("MISMATCH\n");
};
u8[39] bytes2 = [49, 55, 48, 49, 52, 49, 49, 56, 51, 52, 54, 48, 52, 54, 57, 50, 51, 49, 55, 51, 49, 54, 56, 55, 51, 48, 51, 55, 49, 53, 56, 56, 52, 49, 48, 53, 55, 50, 56];
*?u8 data2 = null;
unsafe { data2 = &?bytes2[0]; };
std.utf8 text2 = { .data = data2; .length = 39; };
*i128 value2 = std.parse(text2);
if (value2 == null) {
	std.print("null\n");
};
if (value2 != null) {
	std.print("UNEXPECTED\n");
};
u8[40] bytes3 = [45, 49, 55, 48, 49, 52, 49, 49, 56, 51, 52, 54, 48, 52, 54, 57, 50, 51, 49, 55, 51, 49, 54, 56, 55, 51, 48, 51, 55, 49, 53, 56, 56, 52, 49, 48, 53, 55, 50, 57];
*?u8 data3 = null;
unsafe { data3 = &?bytes3[0]; };
std.utf8 text3 = { .data = data3; .length = 40; };
*i128 value3 = std.parse(text3);
if (value3 == null) {
	std.print("null\n");
};
if (value3 != null) {
	std.print("UNEXPECTED\n");
};
u8[35] bytes4 = [45, 48, 120, 56, 48, 48, 48, 48, 48, 48, 48, 48, 48, 48, 48, 48, 48, 48, 48, 48, 48, 48, 48, 48, 48, 48, 48, 48, 48, 48, 48, 48, 48, 48, 48];
*?u8 data4 = null;
unsafe { data4 = &?bytes4[0]; };
std.utf8 text4 = { .data = data4; .length = 35; };
*i128 value4 = std.parse(text4);
i128 number4 = *value4;
i128 expected4 = -170141183460469231731687303715884105728;
if (number4 == expected4) {
	std.print("match\n");
};
if (number4 != expected4) {
	std.print("MISMATCH\n");
};
std.exit(0);
%%end

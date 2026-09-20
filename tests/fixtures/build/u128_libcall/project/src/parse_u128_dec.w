%%start
std = namespace std "bootstrap.w";

u8[1] bytes0 = [48];
*?u8 data0 = null;
unsafe { data0 = &?bytes0[0]; };
std.utf8 text0 = { .data = data0; .length = 1; };
*u128 value0 = std.parse(text0);
u128 number0 = *value0;
std.utf8 out0 = std.format(number0);
std.print(out0);
std.print("\n");
u8[39] bytes1 = [51, 52, 48, 50, 56, 50, 51, 54, 54, 57, 50, 48, 57, 51, 56, 52, 54, 51, 52, 54, 51, 51, 55, 52, 54, 48, 55, 52, 51, 49, 55, 54, 56, 50, 49, 49, 52, 53, 53];
*?u8 data1 = null;
unsafe { data1 = &?bytes1[0]; };
std.utf8 text1 = { .data = data1; .length = 39; };
*u128 value1 = std.parse(text1);
u128 number1 = *value1;
u128 expected1 = 340282366920938463463374607431768211455;
if (number1 == expected1) {
	std.print("match\n");
};
if (number1 != expected1) {
	std.print("MISMATCH\n");
};
u8[39] bytes2 = [51, 52, 48, 50, 56, 50, 51, 54, 54, 57, 50, 48, 57, 51, 56, 52, 54, 51, 52, 54, 51, 51, 55, 52, 54, 48, 55, 52, 51, 49, 55, 54, 56, 50, 49, 49, 52, 53, 54];
*?u8 data2 = null;
unsafe { data2 = &?bytes2[0]; };
std.utf8 text2 = { .data = data2; .length = 39; };
*u128 value2 = std.parse(text2);
if (value2 == null) {
	std.print("null\n");
};
if (value2 != null) {
	std.print("UNEXPECTED\n");
};
u8[34] bytes3 = [48, 120, 70, 70, 70, 70, 70, 70, 70, 70, 70, 70, 70, 70, 70, 70, 70, 70, 70, 70, 70, 70, 70, 70, 70, 70, 70, 70, 70, 70, 70, 70, 70, 70];
*?u8 data3 = null;
unsafe { data3 = &?bytes3[0]; };
std.utf8 text3 = { .data = data3; .length = 34; };
*u128 value3 = std.parse(text3);
u128 number3 = *value3;
u128 expected3 = 340282366920938463463374607431768211455;
if (number3 == expected3) {
	std.print("match\n");
};
if (number3 != expected3) {
	std.print("MISMATCH\n");
};
u8[130] bytes4 = [48, 98, 49, 49, 49, 49, 49, 49, 49, 49, 49, 49, 49, 49, 49, 49, 49, 49, 49, 49, 49, 49, 49, 49, 49, 49, 49, 49, 49, 49, 49, 49, 49, 49, 49, 49, 49, 49, 49, 49, 49, 49, 49, 49, 49, 49, 49, 49, 49, 49, 49, 49, 49, 49, 49, 49, 49, 49, 49, 49, 49, 49, 49, 49, 49, 49, 49, 49, 49, 49, 49, 49, 49, 49, 49, 49, 49, 49, 49, 49, 49, 49, 49, 49, 49, 49, 49, 49, 49, 49, 49, 49, 49, 49, 49, 49, 49, 49, 49, 49, 49, 49, 49, 49, 49, 49, 49, 49, 49, 49, 49, 49, 49, 49, 49, 49, 49, 49, 49, 49, 49, 49, 49, 49, 49, 49, 49, 49, 49, 49];
*?u8 data4 = null;
unsafe { data4 = &?bytes4[0]; };
std.utf8 text4 = { .data = data4; .length = 130; };
*u128 value4 = std.parse(text4);
u128 number4 = *value4;
u128 expected4 = 340282366920938463463374607431768211455;
if (number4 == expected4) {
	std.print("match\n");
};
if (number4 != expected4) {
	std.print("MISMATCH\n");
};
u8[45] bytes5 = [48, 111, 51, 55, 55, 55, 55, 55, 55, 55, 55, 55, 55, 55, 55, 55, 55, 55, 55, 55, 55, 55, 55, 55, 55, 55, 55, 55, 55, 55, 55, 55, 55, 55, 55, 55, 55, 55, 55, 55, 55, 55, 55, 55, 55];
*?u8 data5 = null;
unsafe { data5 = &?bytes5[0]; };
std.utf8 text5 = { .data = data5; .length = 45; };
*u128 value5 = std.parse(text5);
u128 number5 = *value5;
u128 expected5 = 340282366920938463463374607431768211455;
if (number5 == expected5) {
	std.print("match\n");
};
if (number5 != expected5) {
	std.print("MISMATCH\n");
};
u8[35] bytes6 = [48, 120, 49, 48, 48, 48, 48, 48, 48, 48, 48, 48, 48, 48, 48, 48, 48, 48, 48, 48, 48, 48, 48, 48, 48, 48, 48, 48, 48, 48, 48, 48, 48, 48, 48];
*?u8 data6 = null;
unsafe { data6 = &?bytes6[0]; };
std.utf8 text6 = { .data = data6; .length = 35; };
*u128 value6 = std.parse(text6);
if (value6 == null) {
	std.print("null\n");
};
if (value6 != null) {
	std.print("UNEXPECTED\n");
};
u8[5] bytes7 = [49, 95, 48, 48, 48];
*?u8 data7 = null;
unsafe { data7 = &?bytes7[0]; };
std.utf8 text7 = { .data = data7; .length = 5; };
*u128 value7 = std.parse(text7);
u128 number7 = *value7;
std.utf8 out7 = std.format(number7);
std.print(out7);
std.print("\n");
u8[2] bytes8 = [49, 95];
*?u8 data8 = null;
unsafe { data8 = &?bytes8[0]; };
std.utf8 text8 = { .data = data8; .length = 2; };
*u128 value8 = std.parse(text8);
if (value8 == null) {
	std.print("null\n");
};
if (value8 != null) {
	std.print("UNEXPECTED\n");
};
u8[2] bytes9 = [95, 49];
*?u8 data9 = null;
unsafe { data9 = &?bytes9[0]; };
std.utf8 text9 = { .data = data9; .length = 2; };
*u128 value9 = std.parse(text9);
if (value9 == null) {
	std.print("null\n");
};
if (value9 != null) {
	std.print("UNEXPECTED\n");
};
u8[4] bytes10 = [49, 95, 95, 48];
*?u8 data10 = null;
unsafe { data10 = &?bytes10[0]; };
std.utf8 text10 = { .data = data10; .length = 4; };
*u128 value10 = std.parse(text10);
if (value10 == null) {
	std.print("null\n");
};
if (value10 != null) {
	std.print("UNEXPECTED\n");
};
std.exit(0);
%%end

%%start
child = namespace app "src/child.w";
i32 MAX_VALUE = 1;
MAX_VALUE = 2;

i32(i32) F = fn(PARAMETER) {
	i32 LOCAL_VALUE = PARAMETER;
	LOCAL_VALUE = PARAMETER;
	LOCAL_VALUE
};

child.MAX_VALUE = MAX_VALUE;
%%end

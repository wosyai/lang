%%start
math = namespace app "src/math.w";
i32 result = math.value;
math.value = result + 2;
%%end

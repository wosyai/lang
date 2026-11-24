%%start
i32(i32) add_one = fn(input) { input + 1 };
i32(i32) add_nine = fn(input) { input + 9 };
i32(i32, bool) apply = fn(value, choose) {
    i32(i32) operation = add_one;
    i32 first = operation(value);
    if (choose) { operation = add_nine; };
    first * 10 + operation(value)
};
i32() keep = fn { apply(5, false) };
i32() replace = fn { apply(5, true) };
keep();
replace();
%%end

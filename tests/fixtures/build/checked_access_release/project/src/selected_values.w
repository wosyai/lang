%%start
u64(u64) checked_selector = fn(index) { index };
u64(u64) named_operation = fn(value) { value + 10 };

u64(u64,u64) fixed_selected = fn(index,value) {
    u64(u64)[2] operations = [named_operation, fn(value) { value + 20 }];
    operations[checked_selector(index)](value)
};

u64(u64,u64,u64) generated_selected = fn(count,first,second) {
    u64()[] operations = [fn(i) { fn() { i } }; count];
    u64() saved = operations[checked_selector(first)];
    saved() * 10 + operations[checked_selector(second)]()
};

if (fixed_selected(0,3) != 13 || fixed_selected(1,3) != 23
    || generated_selected(3,2,1) != 21 || generated_selected(2,1,0) != 10) {
    core.system_panic();
};
%%end

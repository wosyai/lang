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

struct copy Cell { u8 tag; u64 value; }
u64(*(Cell[]),u64) _read_field = fn(view,index) {
    *u64 child = &(*view)[index].value;
    *child
};
u64(*!(Cell[]),u64,u64) _update_field = fn(view,index,value) {
    *!u64 child = &!(*view)[index].value;
    *child = value;
    *child
};

u64(u64,u64,u64) combined_field = fn(count,index,value) {
    Cell[count] rows;
    rows[index].tag = 7;
    rows[index].value = 11;
    u64 before = _read_field(&rows,index);
    u64 after = _update_field(&!rows,index,value);
    u64 readback = rows[index].value;
    if (before == 11 && after == readback && rows[index].tag == 7) { readback } else { 0 }
};

if (fixed_selected(0,3) != 13 || fixed_selected(1,3) != 23
    || generated_selected(3,2,1) != 21 || generated_selected(2,1,0) != 10
    || combined_field(2,1,202) != 202 || combined_field(3,2,17) != 17) {
    core.system_panic();
};
%%end

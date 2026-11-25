%%start
struct copy Cell { u8 tag; u64 value; }
u64(*!(Cell[]),u64,u64) _update_field = fn(view,index,value) {
    *!u64 child = &!(*view)[index].value;
    *child = value;
    *child
};
u64(u64,u64) observe = fn(count,index) {
    Cell[count] rows;
    rows[index].tag = 7;
    _update_field(&!rows,index,202)
};
if (observe(2,1) != 202 || observe(3,2) != 202) { core.system_panic(); };
%%end

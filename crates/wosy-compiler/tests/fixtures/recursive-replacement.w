%%start
(u8[], u64)(u8[], u64, u64) replace_owner = fn(previous, replacement_length, count) {
    u8[replacement_length] replacement;
    replacement[0] = previous[0] + 1;
    replacement, count
};

(u8[], u64)(u8[], u64, u64, u64) recursive_return = fn(bytes, depth, replacement_length, count) {
    if (depth == 0) {
        bytes, count = replace_owner(bytes, replacement_length, count);
    } else {
        u64 one = 1;
        bytes, count = recursive_return(bytes, depth - one, replacement_length, count);
    };
    bytes, count
};

u8(u64, u64, u64) recursive_replacement = fn(depth, original_length, replacement_length) {
    u8[original_length] bytes;
    bytes[0] = 17;
    u64 count = 1;
    bytes, count = recursive_return(bytes, depth, replacement_length, count);
    u8 value = bytes[0];
    if (count == 1) { value } else { 0 }
};

u8 observed = recursive_replacement(3, 3, 5);
%%end

%%start
struct Payload { u8 value; }

*Payload() invalid = fn {
    Payload local = { .value = 1; };
    *Payload held = &local;
    Payload moved = local;
    moved;
    held
};
%%end

%%start
struct Payload { u8 value; }

unit() invalid = fn {
    Payload specimen = { .value = 1; };
    *!Payload writer = &!specimen;
    Payload taken = *writer;
    Payload again = *writer;
    taken.value;
    again.value;
};
%%end

%%start
struct Payload { u8 value; }

unit() invalid = fn {
    Payload specimen = { .value = 1; };
    *Payload retained = &specimen;
    Payload moved = specimen;
    retained;
    moved.value;
};
%%end

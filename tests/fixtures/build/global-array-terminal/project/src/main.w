%%start
struct copy Reader { u8() read; }
u64 length = 2;
u8[length] bytes;
bytes[0] = 5;
Reader result = { .read = fn { bytes[0] }; };
u8 observed = result.read();
if (observed != 5) { core.system_panic(); };
observed;
%%end

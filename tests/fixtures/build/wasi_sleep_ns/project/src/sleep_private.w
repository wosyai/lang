%%start
std = namespace std "bootstrap.w";
preview1 = namespace std "wasi/preview1.w";

*?u8 destination = null;
i32 raw_result = unsafe { preview1._poll_oneoff(destination, destination, 1, destination) };
preview1._Subscription subscription = {
	.userdata = 0;
	.clock_tag = 0;
	._pad_1 = 0;
	._pad_2 = 0;
	._pad_3 = 0;
	._pad_4 = 0;
	._pad_5 = 0;
	._pad_6 = 0;
	._pad_7 = 0;
	.clock_id = 1;
	._pad_8 = 0;
	._pad_9 = 0;
	._pad_10 = 0;
	._pad_11 = 0;
	.timeout = 0;
	.precision = 0;
	.flags = 0;
	._pad_12 = 0;
	._pad_13 = 0;
	._pad_14 = 0;
	._pad_15 = 0;
	._pad_16 = 0;
	._pad_17 = 0;
};
preview1._Event event = {
	.userdata = 0;
	.error = 0;
	.event_type = 0;
	._pad_1 = 0;
	._pad_2 = 0;
	._pad_3 = 0;
	._pad_4 = 0;
	._pad_5 = 0;
	.nbytes = 0;
	.flags = 0;
	._pad_6 = 0;
	._pad_7 = 0;
	._pad_8 = 0;
	._pad_9 = 0;
	._pad_10 = 0;
	._pad_11 = 0;
};

u64 delay = 0;
std.sleep_ns(delay);
%%end

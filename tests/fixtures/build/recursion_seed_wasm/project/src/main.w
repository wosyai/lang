%%start
struct ReadIovec {
	*?u8 data;
	u32 length;
}

struct ReadCount {
	u32 value;
}

wasi = extern wasm "wasi_snapshot_preview1" {
	unsafe i32(i32, *?ReadIovec, i32, *?ReadCount) fd_read;
};

i32() read_workload = fn {
	u8 byte = 0;
	ReadCount count = { .value = 0; };
	unsafe {
		ReadIovec iovec = { .data = &?byte; .length = 1; };
		i32 status = wasi.fd_read(0, &?iovec, 1, &?count);
		if (status != 0 || count.value != 1) { core.system_panic(); };
	};
	core.int_extend<i32>(byte)
};

i32 workload = 0;
i32 trace_position = 0;
i32 logical_peak = 0;

enum TraceMode { tail; non_tail; mutual; }
enum Event { tail_enter; non_tail_enter; non_tail_return; mutual_a; mutual_b; }
TraceMode trace_mode = TraceMode::tail;

unit(Event, i32) record_event = fn(event, remaining) {
	Event expected = Event::tail_enter;
	i32 expected_remaining = workload - trace_position;
	if (trace_mode == TraceMode::non_tail) {
		if (trace_position <= workload) {
			expected = Event::non_tail_enter;
		} else {
			expected = Event::non_tail_return;
			expected_remaining = trace_position - workload;
		};
	};
	if (trace_mode == TraceMode::mutual) {
		expected = if (trace_position % 2 == 0) { Event::mutual_a } else { Event::mutual_b };
	};
	if (event != expected || remaining != expected_remaining) { core.system_panic(); };
	trace_position = trace_position + 1;
};

unit(i32) observe_depth = fn(logical_depth) {
	if (logical_depth > logical_peak) { logical_peak = logical_depth; };
};

i32(i32, i32, i32) tail_sum = fn(remaining, total, logical_depth) {
	record_event(Event::tail_enter, remaining);
	observe_depth(logical_depth);
	if (remaining == 0) {
		total
	} else {
		tail_sum(remaining - 1, total + remaining, logical_depth + 1)
	}
};

(i32, i32)(i32, i32) non_tail_pair = fn(remaining, logical_depth) {
	record_event(Event::non_tail_enter, remaining);
	observe_depth(logical_depth);
	if (remaining == 0) {
		5, 11
	} else {
		i32 total, i32 weight = non_tail_pair(remaining - 1, logical_depth + 1);
		record_event(Event::non_tail_return, remaining);
		total + remaining, weight + 3
	}
};

(i32, i32)(i32, i32) mutual_a = fn(remaining, logical_depth) {
	record_event(Event::mutual_a, remaining);
	observe_depth(logical_depth);
	if (remaining == 0) {
		101, 201
	} else {
		i32 first, i32 second = mutual_b(remaining - 1, logical_depth + 1);
		first + 2, second + 5
	}
};

(i32, i32)(i32, i32) mutual_b = fn(remaining, logical_depth) {
	record_event(Event::mutual_b, remaining);
	observe_depth(logical_depth);
	if (remaining == 0) {
		103, 203
	} else {
		i32 first, i32 second = mutual_a(remaining - 1, logical_depth + 1);
		first + 3, second + 7
	}
};

unit() check = fn {
	workload = read_workload();
	if (workload != 3 && workload != 64) { core.system_panic(); };
	i32 sum = workload * (workload + 1) / 2;
	i32 total = tail_sum(workload, 0, 1);
	if (total != sum
		|| trace_position != workload + 1 || logical_peak != workload + 1) {
		core.system_panic();
	};

	trace_mode = TraceMode::non_tail;
	trace_position = 0;
	logical_peak = 0;
	i32 non_tail_total, i32 non_tail_weight = non_tail_pair(workload, 1);
	if (non_tail_total != 5 + sum || non_tail_weight != 11 + 3 * workload
		|| trace_position != 2 * workload + 1 || logical_peak != workload + 1) {
		core.system_panic();
	};

	trace_mode = TraceMode::mutual;
	trace_position = 0;
	logical_peak = 0;
	i32 first, i32 second = mutual_a(workload, 1);
	if ((workload == 3 && (first != 110 || second != 220))
		|| (workload == 64 && (first != 261 || second != 585))
		|| trace_position != workload + 1 || logical_peak != workload + 1) {
		core.system_panic();
	};
};

check();
%%end

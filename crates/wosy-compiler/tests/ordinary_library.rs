use inkwell::context::Context;
use inkwell::memory_buffer::MemoryBuffer;
use inkwell::module::Module;
use inkwell::values::{CallSiteValue, InstructionOpcode};
use wosy_compiler::{
    analyze_scalar_build, derive_scalar_declarations_from_cst, emit_scalar_build_project_llvm,
    parse_source, prepare_scalar_project, ScalarNamespaceBinding, ScalarPreparationInputs,
    ScalarSourceDeclarations, ScalarTargetLayout,
};
use wosy_syntax::{ByteSpan, SourceIdentity};

const LIBRARY: &str = r#"%%start
struct Reading { u32 meter; }
Reading(u32) capture = fn(value) {
    Reading item = { .meter = value; };
    item
};
u32(Reading) measure = fn(item) { item.meter };
%%end"#;

const USER: &str = r#"%%start
sensor = namespace meterkit "course.w";
sensor.Reading() relay = fn { sensor.capture(112) };
u32() trial = fn { sensor.measure(relay()) };
%%end"#;

const INVALID_USER: &str = r#"%%start
sensor = namespace meterkit "course.w";
bool() relay = fn { true };
u32() trial = fn { sensor.measure(relay()) };
%%end"#;

const CAPTURE: &str =
    "wosy_fn__6d657465726b6974__7372632f636f757273652e77__73616d706c652d7231__63617074757265";
const MEASURE: &str =
    "wosy_fn__6d657465726b6974__7372632f636f757273652e77__73616d706c652d7231__6d656173757265";
const RELAY: &str =
    "wosy_fn__776f726b62656e6368__7372632f73657373696f6e2e77__73616d706c652d7231__72656c6179";
const TRIAL: &str =
    "wosy_fn__776f726b62656e6368__7372632f73657373696f6e2e77__73616d706c652d7231__747269616c";
const STD_PRINT: &str =
    "wosy_fn__737464__7372632f626f6f7473747261702e77__73616d706c652d7231__7072696e74";

fn declarations(source: &SourceIdentity, text: &str) -> ScalarSourceDeclarations {
    let parsed = parse_source(source.clone(), text.into(), &[]);
    assert!(parsed.diagnostics.is_empty(), "{:?}", parsed.diagnostics);
    derive_scalar_declarations_from_cst(parsed.result.canonical_cst())
}

fn source(package: &str, path: &str) -> SourceIdentity {
    SourceIdentity::new(
        "lab".into(),
        package.into(),
        path.into(),
        "sample-r1".into(),
    )
}

fn namespace(binding: &str, target: &SourceIdentity) -> ScalarNamespaceBinding {
    ScalarNamespaceBinding {
        binding: binding.into(),
        target: target.clone(),
        span: ByteSpan::new(0, 0),
    }
}

fn direct_calls(module: &Module<'_>, caller: &str) -> Vec<String> {
    module
        .get_function(caller)
        .unwrap()
        .get_basic_blocks()
        .into_iter()
        .flat_map(|block| block.get_instructions())
        .filter(|instruction| instruction.get_opcode() == InstructionOpcode::Call)
        .map(|instruction| {
            CallSiteValue::try_from(instruction)
                .unwrap()
                .get_called_fn_value()
                .expect("ordinary direct call")
                .get_name()
                .to_str()
                .unwrap()
                .to_owned()
        })
        .collect()
}

#[test]
fn ordinary_library_calls_keep_their_typed_routes_without_test_models() {
    let user = source("workbench", "src/session.w");
    let library = source("meterkit", "src/course.w");

    for target_layout in [ScalarTargetLayout::NATIVE64, ScalarTargetLayout::WASM32] {
        let prepare = |user_text| {
            prepare_scalar_project(
                vec![
                    (
                        declarations(&user, user_text),
                        vec![namespace("sensor", &library)],
                    ),
                    (declarations(&library, LIBRARY), Vec::new()),
                ],
                vec![user.clone(), library.clone()],
                ScalarPreparationInputs { target_layout },
            )
        };
        let prepared =
            prepare(USER).unwrap_or_else(|e| panic!("{target_layout:?}: {:?}", e.diagnostics));
        let build = analyze_scalar_build(&prepared);
        assert!(build.diagnostics().is_empty(), "{:?}", build.diagnostics());
        let partition = emit_scalar_build_project_llvm(&build).unwrap();
        for expected in [CAPTURE, MEASURE, RELAY, TRIAL] {
            assert_eq!(
                partition
                    .functions
                    .iter()
                    .filter(|fun| fun.name == expected)
                    .count(),
                1
            );
        }

        let context = Context::create();
        let ir = partition.to_text();
        let buffer = MemoryBuffer::create_from_memory_range_copy(ir.as_bytes(), "ordinary-library");
        let module = context.create_module_from_ir(buffer).unwrap();
        assert_eq!(direct_calls(&module, RELAY), [CAPTURE]);
        assert_eq!(direct_calls(&module, TRIAL), [RELAY, MEASURE]);

        let Err(invalid) = prepare(INVALID_USER) else {
            panic!("{target_layout:?}: bool unexpectedly used as Reading");
        };
        assert!(
            invalid.diagnostics.iter().any(|diagnostic| {
                diagnostic.code == "B0003"
                    && diagnostic.message == "expression type does not match expected type"
            }),
            "{target_layout:?}: {:?}",
            invalid.diagnostics
        );
    }
}

#[test]
fn ordinary_stdlib_print_still_uses_its_declared_call() {
    assert_stdlib_print(
        "%%start\nstd = namespace std \"bootstrap.w\";\nstd.print(\"ordinary\");\n%%end",
        false,
    );
}

#[test]
fn stdlib_print_releases_formatted_owner_not_its_passive_iovec() {
    assert_stdlib_print(
        "%%start\nstd = namespace std \"bootstrap.w\";\nu64 n = 42;\nstd.print(std.format(n));\n%%end",
        true,
    );
}

fn assert_stdlib_print(text: &str, owning: bool) {
    let app = source("workbench", "src/run.w");
    let std = source("std", "src/bootstrap.w");
    let preview = source("std", "src/wasi/preview1.w");
    for target_layout in [ScalarTargetLayout::NATIVE64, ScalarTargetLayout::WASM32] {
        let prepared = prepare_scalar_project(
            vec![
                (declarations(&app, text), vec![namespace("std", &std)]),
                (
                    declarations(&std, include_str!("../../../stdlib/src/bootstrap.w")),
                    vec![namespace("preview1", &preview)],
                ),
                (
                    declarations(
                        &preview,
                        include_str!("../../../stdlib/src/wasi/preview1.w"),
                    ),
                    Vec::new(),
                ),
            ],
            vec![app.clone(), std.clone(), preview.clone()],
            ScalarPreparationInputs { target_layout },
        )
        .unwrap_or_else(|e| panic!("{target_layout:?}: {:?}", e.diagnostics));
        let build = analyze_scalar_build(&prepared);
        assert!(build.diagnostics().is_empty(), "{:?}", build.diagnostics());
        let partition = emit_scalar_build_project_llvm(&build).unwrap();
        assert!(partition
            .functions
            .iter()
            .any(|function| function.name == STD_PRINT));
        let context = Context::create();
        let ir = partition.to_text();
        let buffer = MemoryBuffer::create_from_memory_range_copy(ir.as_bytes(), "ordinary-stdlib");
        let module = context.create_module_from_ir(buffer).unwrap();
        module.verify().unwrap();
        assert!(direct_calls(&module, "main")
            .iter()
            .any(|target| target == STD_PRINT));
        let print = module.get_function(STD_PRINT).unwrap();
        let frees: Vec<_> = print.get_basic_blocks().into_iter()
            .flat_map(|block| block.get_instructions())
            .filter(|instruction| instruction.get_opcode() == InstructionOpcode::Call
                && CallSiteValue::try_from(*instruction).unwrap().get_called_fn_value()
                    .is_some_and(|callee| callee.get_name().to_bytes() == b"__wosy_core_free"))
            .collect();
        if owning {
            let [free] = frees.as_slice() else {
                panic!("{target_layout:?}: expected one formal release");
            };
            assert_eq!(free.get_operand(0).unwrap().value(), print.get_last_param());
        } else {
            assert!(frees.is_empty());
        }
    }
}

fn function_with_suffix(module: &Module<'_>, name: &str) -> String {
    let suffix = format!(
        "__{}",
        name.bytes()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>()
    );
    let mut matches = module.get_functions().filter_map(|function| {
        let symbol = function.get_name().to_str().unwrap();
        symbol.ends_with(&suffix).then(|| symbol.to_owned())
    });
    let symbol = matches
        .next()
        .unwrap_or_else(|| panic!("missing function {name}"));
    assert!(matches.next().is_none(), "ambiguous function {name}");
    symbol
}

#[test]
fn same_scope_releases_two_independent_aggregate_receipts() {
    let app = source("workbench", "src/two_receipts.w");
    let text = r#"%%start
struct Reading { *?u8 data; u64 length; }
Reading(u64) make_reading = fn(length) {
    u8[length] backing;
    backing[0] = 19;
    *?u8 data = null;
    unsafe { data = &?backing[0]; };
    Reading reading_packet = { .data = data; .length = length; };
    reading_packet
};
u64(u64) drive = fn(length) {
    Reading left = make_reading(length);
    Reading right = make_reading(length);
    *Reading left_view = if (length == 0) { &right } else { &left };
    (*left_view).length + right.length
};
u64 result = drive(2);
%%end"#;
    for target_layout in [ScalarTargetLayout::NATIVE64, ScalarTargetLayout::WASM32] {
        let prepared = prepare_scalar_project(
            vec![(declarations(&app, text), Vec::new())],
            vec![app.clone()],
            ScalarPreparationInputs { target_layout },
        )
        .unwrap_or_else(|error| panic!("{target_layout:?}: {:?}", error.diagnostics));
        let build = analyze_scalar_build(&prepared);
        assert!(build.diagnostics().is_empty(), "{:?}", build.diagnostics());
        let ir = emit_scalar_build_project_llvm(&build).unwrap().to_text();
        let context = Context::create();
        let buffer = MemoryBuffer::create_from_memory_range_copy(ir.as_bytes(), "two-receipts");
        let module = context.create_module_from_ir(buffer).unwrap();
        module.verify().unwrap();

        let drive = module.get_function(&function_with_suffix(&module, "drive")).unwrap();
        let producer = module.get_function(&function_with_suffix(&module, "make_reading")).unwrap();
        let release = module.get_function("__wosy_core_free").unwrap();
        let mut received = Vec::new();
        let mut released = Vec::new();
        for instruction in drive.get_basic_blocks().into_iter().flat_map(|block| block.get_instructions()) {
            if instruction.get_opcode() != InstructionOpcode::Call {
                continue;
            }
            let call = CallSiteValue::try_from(instruction).unwrap();
            match call.get_called_fn_value() {
                Some(callee) if callee == producer => received.push(call.try_as_basic_value().basic().unwrap()),
                Some(callee) if callee == release => released.push(instruction.get_operand(0).unwrap().value().unwrap()),
                _ => {}
            }
        }
        assert_eq!(received.len(), 2);
        assert_ne!(received[0], received[1]);
        received.reverse();
        assert_eq!(released, received, "{target_layout:?}");
    }
}

#[test]
fn runtime_array_recommit_keeps_the_owner_until_its_real_end() {
    let recursive = r#"
(u8[], u64)(u8[], u64, u64) carry = fn(bytes, depth, count) {
    if (depth != 0) {
        u64 one = 1;
        bytes, count = carry(bytes, depth - one, count);
    };
    bytes, count
};
"#;
    let changed = r#"
(u8[], u64)(u8[], u64, u64) carry = fn(previous, depth, count) {
    u8[depth] replacement;
    replacement[0] = previous[0];
    replacement, count
};
"#;
    let owning_struct = r#"
struct Reading { *?u8 data; u64 length; }
Reading(u64) make_reading = fn(length) {
    u8[length] backing;
    backing[0] = 19;
    *?u8 data = null;
    unsafe { data = &?backing[0]; };
    Reading reading_packet = { .data = data; .length = length; };
    reading_packet
};
"#;
    for target_layout in [ScalarTargetLayout::NATIVE64, ScalarTargetLayout::WASM32] {
        for (name, carry, companion, expected_before) in [
            ("array_only", recursive, false, 0),
            ("beside_struct", recursive, true, 0),
            ("changed_base", changed, false, 1),
        ] {
            let user = source("workbench", "src/recommit.w");
            let text = format!(
                r#"%%start
{carry}
{}
u8(u64) drive = fn(length) {{
    u8[length] bytes;
    bytes[0] = 17;
    u64 count = 1;
    bytes, count = carry(bytes, length, count);
    u8 observed = bytes[0];
    {}
    observed
}};
u8 result = drive(2);
%%end"#,
                if companion { owning_struct } else { "" },
                if companion {
                    "Reading reading = make_reading(length); if (reading.length == count) { observed = 1; };"
                } else {
                    "if (count == length) { observed = 1; };"
                },
            );
            let prepared = prepare_scalar_project(
                vec![(declarations(&user, &text), Vec::new())],
                vec![user.clone()],
                ScalarPreparationInputs { target_layout },
            )
            .unwrap_or_else(|error| panic!("{name} {target_layout:?}: {:?}", error.diagnostics));
            let build = analyze_scalar_build(&prepared);
            assert!(
                build.diagnostics().is_empty(),
                "{name} {target_layout:?}: {:?}",
                build.diagnostics()
            );
            let ir = emit_scalar_build_project_llvm(&build).unwrap().to_text();
            let context = Context::create();
            let buffer = MemoryBuffer::create_from_memory_range_copy(ir.as_bytes(), name);
            let module = context.create_module_from_ir(buffer).unwrap();
            module.verify().unwrap();
            let drive_symbol = function_with_suffix(&module, "drive");
            let carry_symbol = function_with_suffix(&module, "carry");
            let drive = ir.split_once(&format!("define i8 @{drive_symbol}(")).unwrap().1;
            let drive = drive.split_once("
}").unwrap().0;
            let (prior, post) = drive.split_once(&format!("@{carry_symbol}(")).unwrap();
            assert!(prior.contains("call ptr @__wosy_core_alloc"), "{name}: {drive}");
            let (before_use, after_use) = post.split_once("load i8, ptr %array_element").unwrap();
            let free = "call void @__wosy_core_free";
            assert!(!prior.contains(free), "{name}: {drive}");
            assert_eq!(
                before_use.matches(free).count(),
                expected_before,
                "{name}: {drive}"
            );
            assert_eq!(
                after_use.matches(free).count(),
                if companion { 2 } else { 1 },
                "{name}: {drive}"
            );
        }
    }
}

#[test]
fn runtime_array_read_line_graph_emits_the_actual_stdlib_ranges() {
    let app = source("wasi_read_into", "src/main.w");
    let std = source("std", "src/bootstrap.w");
    let preview = source("std", "src/wasi/preview1.w");
    let text = include_str!("../../../tests/fixtures/build/wasi_read_into/project/src/read_line_lf.w");
    for target_layout in [ScalarTargetLayout::NATIVE64, ScalarTargetLayout::WASM32] {
        let prepared = prepare_scalar_project(
            vec![
                (declarations(&app, text), vec![namespace("std", &std)]),
                (
                    declarations(&std, include_str!("../../../stdlib/src/bootstrap.w")),
                    vec![namespace("preview1", &preview)],
                ),
                (
                    declarations(&preview, include_str!("../../../stdlib/src/wasi/preview1.w")),
                    Vec::new(),
                ),
            ],
            vec![app.clone(), std.clone(), preview.clone()],
            ScalarPreparationInputs { target_layout },
        )
        .unwrap_or_else(|error| panic!("{target_layout:?}: {:?}", error.diagnostics));
        let build = analyze_scalar_build(&prepared);
        assert!(
            build.diagnostics().is_empty(),
            "{target_layout:?}: {:?}",
            build.diagnostics()
        );
        let ir = emit_scalar_build_project_llvm(&build).unwrap().to_text();
        let context = Context::create();
        let buffer = MemoryBuffer::create_from_memory_range_copy(ir.as_bytes(), "read-line");
        let module = context.create_module_from_ir(buffer).unwrap();
        module.verify().unwrap();
        let lookup = |name| function_with_suffix(&module, name);
        let read = lookup("read_line");
        let allocated = lookup("_read_line_allocated");
        let discard = lookup("_read_line_discard_continue");
        let calls = direct_calls(&module, &allocated);
        assert!(direct_calls(&module, "main").contains(&read));
        assert!(direct_calls(&module, &read).contains(&allocated));
        assert!(calls.contains(&discard));
        for (range, step) in [
            ("_read_line_allocated_range", "_read_line_allocated_step"),
            ("_read_line_utf8_range", "_read_line_utf8_step"),
        ] {
            let range = lookup(range);
            let step = lookup(step);
            assert!(calls.contains(&range));
            let range_calls = direct_calls(&module, &range);
            assert_eq!(range_calls.iter().filter(|call| *call == &range).count(), 2);
            assert_eq!(range_calls.iter().filter(|call| *call == &step).count(), 1);
            for symbol in [range, step] {
                let calls = direct_calls(&module, &symbol);
                assert!(!calls
                    .iter()
                    .any(|call| matches!(call.as_str(), "__wosy_core_alloc" | "__wosy_core_free")));
            }
        }
    }
}

#[test]
fn closed_initializer_owner_stays_in_its_conditional_branch() {
    let app = source("workbench", "src/conditional_receipt.w");
    let text = r#"%%start
struct Packet { *?u8 data; u64 length; }
Packet(u64) make_packet = fn(length) {
    u8[length] backing;
    backing[0] = 19;
    *?u8 data = null;
    unsafe { data = &?backing[0]; };
    Packet packet = { .data = data; .length = length; };
    packet
};
u64(Packet) accept_packet = fn(incoming) { incoming.length };
bool enabled = true;
if (enabled) {
    if (enabled) { accept_packet(make_packet(2)); };
};
if (false) { accept_packet(make_packet(3)); };
%%end"#;
    for target_layout in [ScalarTargetLayout::NATIVE64, ScalarTargetLayout::WASM32] {
        let prepare = |text: &str| {
            prepare_scalar_project(
                vec![(declarations(&app, text), Vec::new())],
                vec![app.clone()],
                ScalarPreparationInputs { target_layout },
            )
            .unwrap_or_else(|error| panic!("{target_layout:?}: {:?}", error.diagnostics))
        };
        let prepared = prepare(text);
        let build = analyze_scalar_build(&prepared);
        assert!(build.diagnostics().is_empty(), "{:?}", build.diagnostics());
        let ir = emit_scalar_build_project_llvm(&build).unwrap().to_text();
        let context = Context::create();
        let buffer = MemoryBuffer::create_from_memory_range_copy(ir.as_bytes(), "conditional-receipt");
        let module = context.create_module_from_ir(buffer).unwrap();
        module.verify().unwrap();

        let producer = module.get_function(&function_with_suffix(&module, "make_packet")).unwrap();
        let consumer = module.get_function(&function_with_suffix(&module, "accept_packet")).unwrap();
        let release = module.get_function("__wosy_core_free").unwrap();
        let mut pairs = 0;
        for block in module.get_function("main").unwrap().get_basic_blocks() {
            let calls: Vec<_> = block.get_instructions()
                .filter(|instruction| instruction.get_opcode() == InstructionOpcode::Call)
                .map(|instruction| (instruction, CallSiteValue::try_from(instruction).unwrap()))
                .filter(|(_, call)| matches!(call.get_called_fn_value(), Some(fun)
                    if fun == producer || fun == consumer || fun == release))
                .collect();
            if calls.is_empty() {
                continue;
            }
            assert!(block.get_name().to_str().unwrap().starts_with("if.then"));
            let [(_, make), (accept_instruction, accept)] = calls.as_slice() else {
                panic!("{target_layout:?}: unpaired conditional transfer");
            };
            assert_eq!(make.get_called_fn_value(), Some(producer));
            assert_eq!(accept.get_called_fn_value(), Some(consumer));
            let returned_base = make.try_as_basic_value().basic().unwrap();
            let passed_base = accept_instruction.get_operand(accept.count_arguments() - 1)
                .unwrap().value().unwrap();
            assert_eq!(returned_base, passed_base);
            pairs += 1;
        }
        assert_eq!(pairs, 2);
        let releases: Vec<_> = consumer.get_basic_blocks().into_iter()
            .flat_map(|block| block.get_instructions())
            .filter(|instruction| instruction.get_opcode() == InstructionOpcode::Call)
            .filter(|instruction| CallSiteValue::try_from(*instruction).unwrap().get_called_fn_value()
                == Some(release))
            .collect();
        let [free] = releases.as_slice() else {
            panic!("{target_layout:?}: expected one terminal release");
        };
        assert_eq!(free.get_operand(0).unwrap().value(), consumer.get_last_param());

        let unproved = text.replace(
            "if (false) { accept_packet(make_packet(3)); };",
            "if (accept_packet(make_packet(3)) == 3) { accept_packet(make_packet(4)); };",
        );
        let prepared = prepare(&unproved);
        let build = analyze_scalar_build(&prepared);
        assert!(build.diagnostics().is_empty(), "{:?}", build.diagnostics());
        let failure = emit_scalar_build_project_llvm(&build).err().expect("unproved condition emitted");
        assert!(failure.contains("aggregate owning initializer receiver is not supported"), "{failure}");
    }
}

#[test]
fn grouped_scalar_descriptor_assignment_preserves_checked_owner_routes() {
    let text = r#"%%start
struct copy Packet { i32 value; }

(*Packet, i32)(i32) manufacture = fn(input) {
	Packet packet = { .value = input; };
	&packet, 3
};
(*Packet, i32)() absent = fn { null, 5 };
(*Packet, i32)(*Packet, bool) choose_packet = fn(other, fresh) {
	Packet packet = { .value = 9; };
	if (fresh) { &packet, 5 } else { other, 6 }
};
i32(i32) use_fresh = fn(input) {
	*Packet received = null;
	i32 marker = 0;
	received, marker = manufacture(input);
	if (received != null) { (*received).value + marker } else { 0 }
};
i32() use_null = fn {
	*Packet received = null;
	i32 marker = 0;
	received, marker = absent();
	marker
};
i32(*Packet, bool) use_mixed = fn(other, fresh) {
	*Packet received = null;
	i32 marker = 0;
	received, marker = choose_packet(other, fresh);
	if (received != null) { (*received).value + marker } else { marker }
};
i32(i32) use_declared = fn(input) {
	*Packet received, i32 marker = manufacture(input);
	if (received != null) { (*received).value + marker } else { 0 }
};
i32(i32) use_reassigned = fn(input) {
	*Packet received = null;
	i32 marker = 0;
	received, marker = manufacture(input);
	received, marker = manufacture(input);
	marker
};
i32(i32) use_address = fn(input) {
	*Packet received = null;
	i32 marker = 0;
	received, marker = manufacture(input);
	if (received != null) { unsafe { &?*received; }; };
	marker
};
i32(*Packet) look = fn(value) { if (value != null) { (*value).value } else { 0 } };
i32(i32) use_opaque = fn(input) {
	*Packet received = null;
	i32 marker = 0;
	received, marker = manufacture(input);
	look(received) + marker
};
%%end"#;

    let app = source("grouped_descriptor", "src/main.w");
    for target_layout in [ScalarTargetLayout::NATIVE64, ScalarTargetLayout::WASM32] {
        let prepared = prepare_scalar_project(
            vec![(declarations(&app, text), Vec::new())],
            vec![app.clone()],
            ScalarPreparationInputs { target_layout },
        )
        .unwrap_or_else(|error| panic!("{:?}", error.diagnostics));
        let build = analyze_scalar_build(&prepared);
        assert!(build.diagnostics().is_empty(), "{:#?}", build.diagnostics());
        let llvm = emit_scalar_build_project_llvm(&build).unwrap().to_text();
        let context = Context::create();
        let buffer = MemoryBuffer::create_from_memory_range_copy(llvm.as_bytes(), "grouped-descriptor");
        let module = context.create_module_from_ir(buffer).unwrap();
        module.verify().unwrap();

        let render = |name| {
            let symbol = function_with_suffix(&module, name);
            use inkwell::values::AnyValue;
            module.get_function(&symbol).unwrap().print_to_string().to_string()
        };
        let producer = function_with_suffix(&module, "manufacture");
        let allocate = direct_calls(&module, &producer);
        assert_eq!(allocate.iter().filter(|name| *name == "__wosy_core_alloc").count(), 1);
        for caller in ["use_fresh", "use_mixed", "use_declared"] {
            let ir = render(caller);
            let free = ir.find("call void @__wosy_core_free").expect("missing lexical descriptor release");
            assert_eq!(ir.matches("call void @__wosy_core_free").count(), 1, "{ir}");
            assert!(ir[..free].contains("load i32"), "{ir}");
            assert!(ir[free..].contains("ret i32"), "{ir}");
            assert!(ir[..free].contains("return_result_owner"), "{ir}");
            if caller == "use_mixed" {
                assert!(ir[..free].contains("checked_return_owned_0"), "{ir}");
                assert!(ir[..free].contains("return_result_owned"), "{ir}");
            }
        }
        for caller in ["use_null", "use_reassigned", "use_address", "use_opaque"] {
            let ir = render(caller);
            assert!(!ir.contains("call void @__wosy_core_free"), "{caller}: {ir}");
        }
    }
}

#[test]
fn private_checked_pair_tracks_an_original_backing_without_loading_the_descriptor_field() {
    let positive = r#"%%start
struct Packet { *?u8 data; u64 length; }
(*Packet, bool)(u64, bool) make_checked = fn(count, flag) {
	u8[count] bytes;
	if (flag) {
		*?u8 data = null;
		unsafe { data = &?bytes[0]; };
		Packet packet = { .data = data; .length = count; };
		&packet, true
	} else {
		*?u8 data = null;
		unsafe { data = &?bytes[0]; };
		Packet packet = { .data = data; .length = count; };
		&packet, false
	}
};
i32(u64, bool) inspect_checked = fn(count, flag) {
	*Packet received = null;
	bool status = false;
	received, status = make_checked(count, flag);
	if (received != null && status) { 1 } else { 0 }
};
%%end"#;
    let unproved = positive.replace("if (received != null && status) { 1 } else { 0 }",
        "if (received != null && status) { if ((*received).data != null) { 1 } else { 2 } } else { 0 }");
    let passive = positive.replace("i32(u64, bool) inspect_checked", r#"u8(Packet) read_payload = fn(input) {
        u8 byte = 0;
        if (input.data != null) { unsafe { byte = core.load<u8>(input.data); }; };
        byte
    };
    reader = overload { u8(Packet) => fn(input) { read_payload(input) }; };
    i32(u64, bool) inspect_checked"#)
        .replace("if (received != null && status) { 1 } else { 0 }", r#"*?u8 content = null;
        if (received != null) { content = (*received).data; };
        Packet carrier = { .data = content; .length = count; };
        u8 byte = reader(carrier);
        if (status && byte == 0) { 1 } else { 0 }"#);
    let projecting_length = passive.replace(
        "if (received != null) { content = (*received).data; };",
        "u64 size = 0; if (received != null) { content = (*received).data; size = (*received).length; };")
        .replace(".data = content; .length = count;", ".data = content; .length = size;");
    let repeated_projection = projecting_length.replace("size = (*received).length;",
        "size = (*received).length; size = (*received).length;");
    let checked_reader = projecting_length
        .replace("u8(Packet) read_payload", "*u8(Packet) read_payload")
        .replace("        byte\n    };", "        if (byte == 0) { &byte } else { null }\n    };")
        .replace("u8(Packet) => fn(input)", "*u8(Packet) => fn(input)")
        .replace("u8 byte = reader(carrier);", r#"*u8 value = reader(carrier);
        u8 byte = 0;
        if (value != null) { byte = *value; };"#);
    let unsupported_guard = checked_reader.replace("if (value != null) { byte = *value; };",
        "if (!(value == null)) { byte = *value; };");
    let overwritten_reader = checked_reader.replace("u8 byte = 0;\n        if (value != null)",
        "value = null; u8 byte = 0;\n        if (value != null)");
    let consuming = passive.replace("byte = core.load<u8>(input.data);",
        "byte = core.load<u8>(input.data); core.free(input.data);");
    let nonreturning = passive.replace("byte = core.load<u8>(input.data);",
        "byte = core.load<u8>(input.data); if (byte == 9) { core.system_panic(); };");
    let length_call = passive
        .replace("u8(Packet) read_payload", "u64() scratch_length = fn { core.int_extend<u64>(1) };\nu8(Packet) read_payload")
        .replace("u8 byte = 0;", "u8[scratch_length()] scratch; if (false) { scratch[0]; }; u8 byte = 0;");
    let ambiguous_alias = passive.replace("Packet carrier =",
        "content = null; Packet carrier =");
    let module_sink = positive.replace("i32(u64, bool) inspect_checked", "*Packet escaped = null;\ni32(u64, bool) inspect_checked")
        .replace("if (received != null && status)", "escaped = received;\n\tif (received != null && status)");
    let qualified_sink = positive.replace("struct Packet", "imported = namespace package \"values.w\";\nstruct Packet")
        .replace("if (received != null && status)", "imported.value = 4;\n\tif (received != null && status)");
    let pure_recommit = positive.replace("(*Packet, bool)(u64, bool) make_checked",
        "u8[](u8[], bool) preserve = fn(buffer, flag) { flag; buffer };\n(*Packet, bool)(u64, bool) make_checked")
        .replace("u8[count] bytes;", "u8[count] bytes;\n\tbytes = preserve(bytes, flag);");
    let observable_recommit = pure_recommit.replace("u8[](u8[], bool) preserve = fn(buffer, flag) { flag; buffer };",
        "bool escaped = false;\nu8[](u8[], bool) preserve = fn(buffer, flag) { escaped = flag; buffer };");
    let recursive_recommit = positive.replace("(*Packet, bool)(u64, bool) make_checked", r#"enum Validation { valid; invalid; }
(u8[], Validation)(u8[], u64, Validation) check_byte = fn(buffer, index, status) {
    u8 byte = buffer[index];
    if (byte == 255) { status = Validation::invalid; };
    buffer, status
};
(u8[], Validation)(u8[], u64, u64, Validation) check_range = fn(buffer, low, high, status) {
    u64 one = core.int_extend<u64>(1);
    if (low < high) {
        u64 count = high - low;
        if (count == one) {
            buffer, status = check_byte(buffer, low, status);
        } else {
            u64 two = core.int_extend<u64>(2);
            u64 middle = low + count / two;
            buffer, status = check_range(buffer, low, middle, status);
            buffer, status = check_range(buffer, middle, high, status);
        };
    };
    buffer, status
};
(*Packet, bool)(u64, bool) make_checked"#)
        .replace("u8[count] bytes;", "u8[count] bytes; u64 zero = 0; Validation result = Validation::valid; bytes, result = check_range(bytes, zero, count, result);");
    let recursive_nonlocal = recursive_recommit
        .replace("(u8[], Validation)(u8[], u64, Validation) check_byte", "u8 observed = 0;\n(u8[], Validation)(u8[], u64, Validation) check_byte")
        .replace("u8 byte = buffer[index];", "u8 byte = buffer[index]; observed = byte;");
    let nullable_backing = positive.replace(
        "unsafe { data = &?bytes[0]; };",
        "if (count != 0) { unsafe { data = &?bytes[0]; }; };",
    );
    let null_field_live_base = positive.replace("unsafe { data = &?bytes[0]; };", "")
        .replace("u8[count] bytes;", "u8[count] bytes; if (count != 0) { bytes[0]; };");
    let aliased_nullable = nullable_backing.replace(
        "Packet packet = { .data = data; .length = count; };",
        "*?u8 alias = data; Packet packet = { .data = alias; .length = count; };",
    );
    let nullable_descriptor = nullable_backing.replace(
        "else {\n\t\t*?u8 data = null;\n\t\tif (count != 0) { unsafe { data = &?bytes[0]; }; };\n\t\tPacket packet = { .data = data; .length = count; };\n\t\t&packet, false\n\t}",
        "else { null, false }");
    let mixed_descriptor = pure_recommit.replace("&packet, false", "null, false")
        .replace(
            "else {\n\t\t*?u8 data = null;\n\t\tunsafe { data = &?bytes[0]; };\n\t\tPacket packet = { .data = data; .length = count; };\n\t\tnull, false\n\t}",
            "else { null, false }",
        );
    let competing_mixed = mixed_descriptor.replace("u8[count] bytes;", "u8[count] bytes; u8[count] other; if (count != 0) { other[0]; };");
    let missing_failure_receipt = mixed_descriptor.replace("bytes = preserve(bytes, flag);", "");
    let competing_formal = mixed_descriptor
        .replace("(*Packet, bool)(u64, bool) make_checked = fn(count, flag)",
            "(*Packet, bool)(u64, bool, u8[]) make_checked = fn(count, flag, unused)")
        .replace("make_checked = fn(count, flag, unused) {", "make_checked = fn(count, flag, unused) { if (false) { unused[0]; };")
        .replace("i32(u64, bool) inspect_checked = fn(count, flag)",
            "i32(u64, bool, u8[]) inspect_checked = fn(count, flag, buffer)")
        .replace("make_checked(count, flag)", "make_checked(count, flag, buffer)");
    let wrap = |text: &str| text.replace("received, status = make_checked(count, flag);",
        "received, status = pass_checked(count, flag);")
        .replace("i32(u64, bool) inspect_checked", "(*Packet, bool)(u64, bool) pass_checked = fn(count, flag) {\n\t*Packet forwarded, bool marker = make_checked(count, flag);\n\tforwarded, marker\n};\ni32(u64, bool) inspect_checked");
    let wrapper = wrap(&mixed_descriptor);
    let wrapper_passive = wrap(&projecting_length);
    let wrapper_checked_reader = wrap(&checked_reader);
    let wrapper_consuming = wrap(&consuming);
    let wrapper_aliased = wrapper.replace("forwarded, marker\n};", "*Packet moved = forwarded; moved, marker\n};");
    let wrapper_overwritten = wrapper.replace("forwarded, marker\n};", "forwarded = null; forwarded, marker\n};");
    let wrapper_two_callers = wrapper.replace("%%end", "i32(u64, bool) other = fn(count, flag) { *Packet received, bool status = pass_checked(count, flag); if (received != null && status) { 3 } else { 0 } };\n%%end");
    let app = source("private_checked_backing", "src/main.w");
    let values = source("package", "src/values.w");
    for target_layout in [ScalarTargetLayout::NATIVE64, ScalarTargetLayout::WASM32] {
        for (text, expected_transport, qualified) in [
            (positive, true, false), (unproved.as_str(), false, false),
            (passive.as_str(), true, false), (projecting_length.as_str(), true, false),
            (repeated_projection.as_str(), false, false), (consuming.as_str(), false, false),
            (checked_reader.as_str(), true, false), (unsupported_guard.as_str(), false, false),
            (overwritten_reader.as_str(), false, false),
            (ambiguous_alias.as_str(), false, false), (nonreturning.as_str(), false, false),
            (length_call.as_str(), false, false),
            (module_sink.as_str(), false, false), (qualified_sink.as_str(), false, true),
            (pure_recommit.as_str(), true, false), (observable_recommit.as_str(), false, false),
            (recursive_recommit.as_str(), true, false), (recursive_nonlocal.as_str(), false, false),
            (nullable_backing.as_str(), true, false), (null_field_live_base.as_str(), true, false),
            (aliased_nullable.as_str(), false, false), (nullable_descriptor.as_str(), false, false),
            (mixed_descriptor.as_str(), true, false), (competing_mixed.as_str(), false, false),
            (missing_failure_receipt.as_str(), false, false), (competing_formal.as_str(), false, false),
            (wrapper.as_str(), true, false), (wrapper_aliased.as_str(), false, false),
            (wrapper_overwritten.as_str(), false, false), (wrapper_two_callers.as_str(), false, false),
            (wrapper_passive.as_str(), true, false), (wrapper_checked_reader.as_str(), true, false),
            (wrapper_consuming.as_str(), false, false),
        ] {
            let (inputs, order) = if qualified {
                (vec![(declarations(&app, text), vec![namespace("imported", &values)]),
                    (declarations(&values, "%%start\nu16 value = 0;\n%%end"), Vec::new())],
                    vec![app.clone(), values.clone()])
            } else {
                (vec![(declarations(&app, text), Vec::new())], vec![app.clone()])
            };
            let prepared = prepare_scalar_project(
                inputs,
                order,
                ScalarPreparationInputs { target_layout },
            ).unwrap_or_else(|error| panic!("{:?}", error.diagnostics));
            let build = analyze_scalar_build(&prepared);
            assert!(build.diagnostics().is_empty(), "{:?}", build.diagnostics());
            let llvm = emit_scalar_build_project_llvm(&build).unwrap().to_text();
            let context = Context::create();
            let buffer = MemoryBuffer::create_from_memory_range_copy(llvm.as_bytes(), "private-checked-backing");
            let module = context.create_module_from_ir(buffer).unwrap();
            module.verify().unwrap();
            use inkwell::values::AnyValue;
            let producer = module.get_function(&function_with_suffix(&module, "make_checked")).unwrap();
            let consumer = module.get_function(&function_with_suffix(&module, "inspect_checked")).unwrap();
            let consumer_ir = consumer.print_to_string().to_string();
            if text.contains("pass_checked") {
                let wrapper = module.get_function(&function_with_suffix(&module, "pass_checked")).unwrap();
                let wrapper_ir = wrapper.print_to_string().to_string();
                if expected_transport {
                    assert_eq!(wrapper.get_linkage(), inkwell::module::Linkage::Internal);
                    let original_slot = wrapper.get_nth_param(wrapper.count_params() - 1).unwrap().print_to_string().to_string();
                    let calls = wrapper_ir.lines().filter(|line| line.contains("checked_backing_call = call")).collect::<Vec<_>>();
                    assert_eq!(calls.len(), 1, "{wrapper_ir}");
                    assert!(calls[0].contains(&format!("{})", original_slot.trim())), "{wrapper_ir}");
                    assert!(!wrapper_ir.contains("checked_backing_out"), "{wrapper_ir}");
                    assert!(!wrapper_ir.contains("checked_backing_base"), "{wrapper_ir}");
                    assert!(!wrapper_ir.contains("@__wosy_core_free"), "{wrapper_ir}");
                } else {
                    assert_ne!(wrapper.get_linkage(), inkwell::module::Linkage::Internal);
                    assert!(!wrapper_ir.contains("checked_backing_call"), "{wrapper_ir}");
                }
            }
            if expected_transport {
                let producer_ir = producer.print_to_string().to_string();
                assert_eq!(producer.count_params(), 3);
                assert_eq!(producer.get_linkage(), inkwell::module::Linkage::Internal);
                let mixed = text == mixed_descriptor || text == wrapper.as_str();
                let allocation = module.get_function("__wosy_core_alloc").unwrap();
                let instructions = || producer.get_basic_blocks().into_iter()
                    .flat_map(|block| block.get_instructions());
                let mut allocations = instructions().filter_map(|instruction| {
                    if instruction.get_opcode() != InstructionOpcode::Call {
                        return None;
                    }
                    let call = CallSiteValue::try_from(instruction).unwrap();
                    if call.get_called_fn_value() != Some(allocation)
                        || instruction.get_operand(1)?.value()?.into_int_value()
                            .get_zero_extended_constant() != Some(1)
                    {
                        return None;
                    }
                    call.try_as_basic_value().basic()
                });
                let original_base = allocations.next().expect("original byte array allocation");
                assert!(allocations.next().is_none(), "{producer_ir}");
                let output_slot = producer.get_last_param().expect("hidden backing result");
                let mut originals = 0;
                let mut nulls = 0;
                for store in instructions().filter(|instruction| {
                    instruction.get_opcode() == InstructionOpcode::Store
                        && instruction.get_operand(1).unwrap().value() == Some(output_slot)
                }) {
                    let value = store.get_operand(0).unwrap().value().expect("published backing");
                    if value == original_base {
                        originals += 1;
                    } else {
                        assert!(value.into_pointer_value().is_null(), "{producer_ir}");
                        nulls += 1;
                    }
                }
                assert_eq!((originals, nulls), if mixed { (1, 1) } else { (2, 0) }, "{producer_ir}");
                assert_eq!(producer_ir.matches("call void @__wosy_core_free").count(), usize::from(mixed), "{producer_ir}");
                if mixed {
                    assert!(producer_ir.contains("call void @__wosy_core_free(ptr %allocation)"), "{producer_ir}");
                    assert!(producer_ir.contains("store ptr null, ptr"), "{producer_ir}");
                    assert!(consumer_ir.contains("icmp ne ptr %checked_backing_base, null"), "{consumer_ir}");
                }
                let expected_releases = if text == checked_reader || text == wrapper_checked_reader { 3 } else { 2 };
                assert_eq!(consumer_ir.matches("call void @__wosy_core_free").count(), expected_releases, "{consumer_ir}");
                assert_eq!(consumer_ir.matches("call void @__wosy_core_free(ptr %checked_backing_base)").count(), 1, "{consumer_ir}");
            } else {
                assert_eq!(producer.count_params(), if text == competing_formal.as_str() { 3 } else { 2 });
                assert!(!consumer_ir.contains("checked_backing_call"), "{consumer_ir}");
            }
        }
    }
}

#[test]
fn ordinary_build_erases_named_proofs_before_runtime_diagnostics_and_emits_the_same_llvm() {
    use wosy_compiler::emit_scalar_build_single_module_llvm_text;
    let source = SourceIdentity::new("local".into(), "app".into(), "main.w".into(), "pinned-source".into());
    let compile = |proofs: &str, target_layout| {
        let text = format!("%%start\nu8(u8) identity = fn(value) {{ value }};\n{proofs}\n%%end");
        let parsed = parse_source(source.clone(), text, &[]);
        assert!(parsed.diagnostics.is_empty(), "{:?}", parsed.diagnostics);
        let declarations = derive_scalar_declarations_from_cst(parsed.result.canonical_cst());
        assert!(declarations.diagnostics().is_empty(), "{:?}", declarations.diagnostics());
        let prepared = prepare_scalar_project(
            vec![(declarations, Vec::new())],
            vec![source.clone()],
            ScalarPreparationInputs { target_layout },
        ).expect("ordinary source preparation");
        let validated = analyze_scalar_build(&prepared);
        assert!(validated.diagnostics().is_empty(), "{:?}", validated.diagnostics());
        emit_scalar_build_single_module_llvm_text(&validated).expect("real LLVM emission")
    };
    let proofs = r#"
        proof false_claim for identity(v) -> (out) ensures (out <= 10) {}
        proof ghost_quantifier for identity(v) -> (out) forall (u8 i where i < 4) ensures (out <= i) {}
        proof bad_where for identity(v) -> (out) where (unknown <= 10) ensures (out <= 1) {}
        proof unknown_target for missing(v) -> (out) ensures (out <= 255) {}
        proof unchecked_measure for identity(v) -> (out) ensures (out <= 255) decreases (unknown, v) {
            core.use(unchecked_measure(v - 1));
        }
        proof bad_literal for identity(v) -> (out) ensures (out <= 255) {
            core.show(out == 1__0);
        }
    "#;
    for target_layout in [ScalarTargetLayout::NATIVE64, ScalarTargetLayout::WASM32] {
        assert_eq!(compile(proofs, target_layout), compile("", target_layout));
    }
    let bad_runtime = parse_source(
        source.clone(),
        format!("%%start\nu8() value = fn {{ 1__0 }};\n{proofs}\n%%end"),
        &[],
    );
    assert!(bad_runtime.diagnostics.is_empty());
    let diagnostics = wosy_compiler::derive_scalar_diagnostics_from_cst(bad_runtime.result.canonical_cst());
    assert_eq!(diagnostics.len(), 1, "{diagnostics:?}");
    assert_eq!(diagnostics[0].code, "B0010");
    let malformed = parse_source(source, "%%start\nproof bad for identity(v) -> (out) ensures (out == 7) {\n%%end".into(), &[]);
    assert!(!malformed.diagnostics.is_empty());
}

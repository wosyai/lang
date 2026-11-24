use inkwell::context::Context;
use inkwell::memory_buffer::MemoryBuffer;
use inkwell::module::Module;
use inkwell::values::{BasicValue, CallSiteValue, InstructionOpcode};
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
fn conditional_aggregate_input_transfer_leaves_the_other_arm_owning() {
    let app = source("workbench", "src/conditional_aggregate_transfer.w");
    let text = include_str!("fixtures/conditional-aggregate-transfer.w");
    for (target, target_layout) in [
        ("native64", ScalarTargetLayout::NATIVE64),
        ("wasm32", ScalarTargetLayout::WASM32),
    ] {
        let prepared = prepare_scalar_project(
            vec![(declarations(&app, text), Vec::new())],
            vec![app.clone()],
            ScalarPreparationInputs { target_layout },
        )
        .unwrap_or_else(|error| panic!("{target}: {:?}", error.diagnostics));
        let build = analyze_scalar_build(&prepared);
        assert!(build.diagnostics().is_empty(), "{target}: {:?}", build.diagnostics());
        let ir = emit_scalar_build_project_llvm(&build).unwrap().to_text();
        let context = Context::create();
        let buffer = MemoryBuffer::create_from_memory_range_copy(ir.as_bytes(), target);
        let module = context.create_module_from_ir(buffer).unwrap();
        module.verify().unwrap();

        let drive = module.get_function(&function_with_suffix(&module, "drive")).unwrap();
        let producer = module.get_function(&function_with_suffix(&module, "make_packet")).unwrap();
        let consumer = module.get_function(&function_with_suffix(&module, "accept_packet")).unwrap();
        let release = module.get_function("__wosy_core_free").unwrap();
        let mut produced = None;
        let mut passed = None;
        let mut freed = None;
        let mut receipt_phi = None;
        let mut consumer_block = None;
        for block in drive.get_basic_blocks() {
            for instruction in block.get_instructions() {
                let printed = instruction.to_string();
                assert!(!printed.contains("aggregate_receipt_live"), "{target}: {printed}");
                if instruction.get_opcode() == InstructionOpcode::Phi
                    && printed.contains("aggregate_receipt_active")
                {
                    assert!(block.get_name().to_str().unwrap().starts_with("if.merge"));
                    assert!(receipt_phi.replace((instruction, block)).is_none());
                }
                if instruction.get_opcode() != InstructionOpcode::Call {
                    continue;
                }
                let call = CallSiteValue::try_from(instruction).unwrap();
                match call.get_called_fn_value() {
                    Some(callee) if callee == producer => {
                        assert!(produced.replace(call.try_as_basic_value().basic().unwrap()).is_none());
                    }
                    Some(callee) if callee == consumer => {
                        assert!(block.get_name().to_str().unwrap().starts_with("if.then"));
                        consumer_block = Some(block);
                        let base = instruction.get_operand(call.count_arguments() - 1).unwrap().value().unwrap();
                        assert!(passed.replace(base).is_none());
                    }
                    Some(callee) if callee == release => {
                        assert!(block.get_name().to_str().unwrap().starts_with("aggregate.receipt.release"));
                        let base = instruction.get_operand(0).unwrap().value().unwrap();
                        assert!(freed.replace(base).is_none());
                    }
                    _ => {}
                }
            }
        }
        let (instruction, merge) = receipt_phi.unwrap();
        let phi = inkwell::values::PhiValue::try_from(instruction).unwrap();
        assert_eq!(phi.count_incoming(), 2);
        let (keep, bypass) = phi.get_incoming(0).unwrap();
        let (moved, then_end) = phi.get_incoming(1).unwrap();
        assert_eq!(keep.into_int_value().get_zero_extended_constant(), Some(1));
        assert_eq!(moved.into_int_value().get_zero_extended_constant(), Some(0));
        assert_ne!(bypass, then_end);
        assert_eq!(Some(then_end), consumer_block);
        assert_eq!(bypass.get_terminator().unwrap().get_operand(1).unwrap().block(), Some(merge));
        assert_eq!(then_end.get_terminator().unwrap().get_operand(0).unwrap().block(), Some(merge));
        assert!(produced.is_some());
        assert_eq!(passed, produced, "{target}");
        assert_eq!(freed, produced, "{target}");
        let consumer_releases: Vec<_> = consumer.get_basic_blocks().into_iter()
            .flat_map(|block| block.get_instructions())
            .filter(|instruction| instruction.get_opcode() == InstructionOpcode::Call
                && CallSiteValue::try_from(*instruction).unwrap().get_called_fn_value() == Some(release))
            .collect();
        let [callee_release] = consumer_releases.as_slice() else {
            panic!("{target}: expected one terminal consumer release");
        };
        assert_eq!(callee_release.get_operand(0).unwrap().value(), consumer.get_last_param());

        if let Some(out) = std::env::var_os("WOSY_CONDITIONAL_AGGREGATE_IR_DIR") {
            let retain = function_with_suffix(&module, "retain_witness");
            let transfer = function_with_suffix(&module, "transfer_witness");
            let mut witness = ir.replace("@main(", "@wosy_authored_main(");
            for (name, target) in [("retain", retain), ("transfer", transfer)] {
                witness.push_str(&format!(
                    "\ndefine i64 @wosy_conditional_{name}() {{\nentry:\n  %result = call i64 @{target}()\n  ret i64 %result\n}}\n"
                ));
            }
            std::fs::write(std::path::PathBuf::from(out).join(format!("{target}.ll")), witness).unwrap();
        }
    }
}

#[test]
fn conditional_aggregate_value_shares_arm_predicates_and_preserves_receipt_identity() {
    let app = source("workbench", "src/conditional_aggregate_value.w");
    let text = include_str!("fixtures/conditional-aggregate-value.w");
    for (target, target_layout) in [
        ("native64", ScalarTargetLayout::NATIVE64),
        ("wasm32", ScalarTargetLayout::WASM32),
    ] {
        let emit = |text: &str| {
            let prepared = prepare_scalar_project(
                vec![(declarations(&app, text), Vec::new())], vec![app.clone()],
                ScalarPreparationInputs { target_layout },
            ).unwrap_or_else(|error| panic!("{target}: {:?}", error.diagnostics));
            let build = analyze_scalar_build(&prepared);
            assert!(build.diagnostics().is_empty(), "{target}: {:?}", build.diagnostics());
            emit_scalar_build_project_llvm(&build).map(|module| module.to_text())
        };
        let ir = emit(text).unwrap();
        let context = Context::create();
        let buffer = MemoryBuffer::create_from_memory_range_copy(ir.as_bytes(), target);
        let module = context.create_module_from_ir(buffer).unwrap();
        module.verify().unwrap();
        let drive = module.get_function(&function_with_suffix(&module, "drive")).unwrap();
        let mut guarded = 0;
        let mut branches = 0;
        for block in drive.get_basic_blocks() {
            for instruction in block.get_instructions() {
                let row = instruction.to_string();
                if instruction.get_opcode() == InstructionOpcode::Phi && row.contains("aggregate_receipt_active") {
                    guarded += 1;
                    assert!(block.get_name().to_str().unwrap().starts_with("if.merge"));
                }
                if instruction.get_opcode() == InstructionOpcode::Br && row.contains("aggregate_receipt_active") {
                    branches += 1;
                }
                assert!(!row.contains("aggregate_receipt_live"));
            }
        }
        assert_eq!((guarded, branches), (2, 3), "{target}: {ir}");
        if let Some(out) = std::env::var_os("WOSY_VALUE_RECEIPT_IR_DIR") {
            let mut witness = ir.replace("@main(", "@wosy_authored_main(");
            for (wrapper, source) in [("retain", "retain_witness"), ("transfer", "transfer_witness")] {
                let symbol = function_with_suffix(&module, source);
                witness.push_str(&format!(
                    "\ndefine i64 @wosy_conditional_{wrapper}() {{\nentry:\n  %result = call i64 @{symbol}()\n  ret i64 %result\n}}\n"
                ));
            }
            std::fs::write(std::path::PathBuf::from(out).join(format!("{target}.ll")), witness).unwrap();
        }
        // Exercise both contextual numeric emission and the canonical ordered tail,
        // which use different output paths than a plain scalar expression.
        let contextual = text.replace("    if (take)", "    u64 result = if (take)")
            .replace("    }\n};\nu64() retain_witness", "    };\n    result\n};\nu64() retain_witness");
        let ordered = text.replace("u64(u64, bool) drive", "(u64, bool)(u64, bool) drive")
            .replace("left + right\n", "left + right, true\n")
            .replace("        accept_packet(third)\n", "        accept_packet(third), false\n")
            .replace("fn { drive(2, false) }", "fn { u64 value, bool marker = drive(2, false); value }")
            .replace("fn { drive(2, true) }", "fn { u64 value, bool marker = drive(2, true); value }");
        for variant in [contextual, ordered] {
            let ir = emit(&variant).unwrap_or_else(|error| panic!("{target}: {error}\n{variant}"));
            let buffer = MemoryBuffer::create_from_memory_range_copy(ir.as_bytes(), target);
            context.create_module_from_ir(buffer).unwrap().verify().unwrap();
            assert_eq!(ir.lines().filter(|row| row.contains("aggregate_receipt_active")
                && row.contains(" = phi i1 ")).count(), 2, "{target}: {ir}");
        }
        // Either arm may enclose a nested receipt; upstream owners sharing only
        // the outer condition still have a different predicate from the nested move.
        for (needle, replacement, expected) in [
            ("u64 left = accept_packet(first);",
             "u64 left = if (take) { accept_packet(first) } else { length };", 4),
            ("accept_packet(third)", "if (take) { length } else { accept_packet(third) }", 3),
        ] {
            let variant = text.replace(needle, replacement);
            let ir = emit(&variant).unwrap_or_else(|error| panic!("{target}: {error}\n{variant}"));
            let buffer = MemoryBuffer::create_from_memory_range_copy(ir.as_bytes(), target);
            context.create_module_from_ir(buffer).unwrap().verify().unwrap();
            assert_eq!(ir.lines().filter(|row| row.contains("aggregate_receipt_active")
                && row.contains(" = phi i1 ")).count(), expected, "{target}: {ir}");
        }
    }
}

#[test]
fn sequential_aggregate_transfer_retires_exhaustive_receipts_without_a_guard() {
    let app = source("workbench", "src/sequential_aggregate_transfer.w");
    let text = include_str!("fixtures/sequential-aggregate-transfer.w");
    for (target, target_layout) in [
        ("native64", ScalarTargetLayout::NATIVE64),
        ("wasm32", ScalarTargetLayout::WASM32),
    ] {
        let prepared = prepare_scalar_project(
            vec![(declarations(&app, text), Vec::new())], vec![app.clone()],
            ScalarPreparationInputs { target_layout },
        ).unwrap_or_else(|error| panic!("{target}: {:?}", error.diagnostics));
        let build = analyze_scalar_build(&prepared);
        assert!(build.diagnostics().is_empty(), "{target}: {:?}", build.diagnostics());
        let ir = emit_scalar_build_project_llvm(&build).unwrap().to_text();
        let context = Context::create();
        let buffer = MemoryBuffer::create_from_memory_range_copy(ir.as_bytes(), target);
        let module = context.create_module_from_ir(buffer).unwrap();
        module.verify().unwrap();

        let drive = module.get_function(&function_with_suffix(&module, "drive")).unwrap();
        let producer = module.get_function(&function_with_suffix(&module, "make_packet")).unwrap();
        let consumer = module.get_function(&function_with_suffix(&module, "accept_packet")).unwrap();
        let release = module.get_function("__wosy_core_free").unwrap();
        let mut produced = Vec::new();
        let mut transferred = Vec::new();
        let mut released = Vec::new();
        let mut receipt_merges = Vec::new();
        let mut guarded_branches = 0;
        for block in drive.get_basic_blocks() {
            for instruction in block.get_instructions() {
                let row = instruction.to_string();
                assert!(!row.contains("aggregate_receipt_live"));
                if instruction.get_opcode() == InstructionOpcode::Phi && row.contains("aggregate_receipt_active") {
                    receipt_merges.push(block);
                }
                if instruction.get_opcode() == InstructionOpcode::Br && row.contains("aggregate_receipt_active") {
                    guarded_branches += 1;
                }
                if instruction.get_opcode() != InstructionOpcode::Call { continue; }
                let call = CallSiteValue::try_from(instruction).unwrap();
                let pointer = |slot| instruction.get_operand(slot).unwrap().value().unwrap();
                match call.get_called_fn_value() {
                    Some(callee) if callee == producer => produced.push(call.try_as_basic_value().basic().unwrap()),
                    Some(callee) if callee == consumer => transferred.push(pointer(call.count_arguments() - 1)),
                    Some(callee) if callee == release => released.push(pointer(0)),
                    _ => {}
                }
            }
        }
        let [first, preserved, second, exhaustive] = produced.as_slice() else {
            panic!("{target}: {ir}");
        };
        assert_eq!(transferred.len(), 4, "{target}: {ir}");
        for (receipt, uses) in [(first, 1), (preserved, 0), (second, 1), (exhaustive, 2)] {
            assert_eq!(transferred.iter().filter(|other| *other == receipt).count(), uses, "{target}: {ir}");
        }
        assert_eq!(released, [*second, *preserved, *first], "{target}: {ir}");
        assert_eq!(receipt_merges.len(), 2, "{target}: {ir}");
        assert_ne!(receipt_merges[0], receipt_merges[1], "{target}: {ir}");
        assert_eq!(guarded_branches, 2, "{target}: {ir}");
        if let Some(out) = std::env::var_os("WOSY_SEQUENTIAL_RECEIPT_IR_DIR") {
            let mut witness = ir.replace("@main(", "@wosy_authored_main(");
            for name in ["case_ff", "case_ft", "case_tf", "case_tt"] {
                let symbol = function_with_suffix(&module, name);
                witness.push_str(&format!(
                    "\ndefine i64 @wosy_{name}() {{\nentry:\n  %result = call i64 @{symbol}()\n  ret i64 %result\n}}\n"
                ));
            }
            std::fs::write(std::path::PathBuf::from(out).join(format!("{target}.ll")), witness).unwrap();
        }
    }
}

#[test]
fn nested_aggregate_transfer_shares_and_merges_path_receipts() {
    let app = source("workbench", "src/nested_aggregate_transfer.w");
    let text = include_str!("fixtures/nested-aggregate-transfer.w");
    for (target, target_layout) in [
        ("native64", ScalarTargetLayout::NATIVE64),
        ("wasm32", ScalarTargetLayout::WASM32),
    ] {
        let prepared = prepare_scalar_project(
            vec![(declarations(&app, text), Vec::new())], vec![app.clone()],
            ScalarPreparationInputs { target_layout },
        ).unwrap_or_else(|error| panic!("{target}: {:?}", error.diagnostics));
        let build = analyze_scalar_build(&prepared);
        assert!(build.diagnostics().is_empty(), "{target}: {:?}", build.diagnostics());
        let ir = emit_scalar_build_project_llvm(&build).unwrap().to_text();
        let context = Context::create();
        let buffer = MemoryBuffer::create_from_memory_range_copy(ir.as_bytes(), target);
        let module = context.create_module_from_ir(buffer).unwrap();
        module.verify().unwrap();

        let drive = module.get_function(&function_with_suffix(&module, "drive")).unwrap();
        let producer = module.get_function(&function_with_suffix(&module, "make_packet")).unwrap();
        let consumer = module.get_function(&function_with_suffix(&module, "accept_packet")).unwrap();
        let release = module.get_function("__wosy_core_free").unwrap();
        let mut produced = Vec::new();
        let mut transferred = Vec::new();
        let mut released = Vec::new();
        let mut receipt_merges = Vec::new();
        let mut guarded_branches = 0;
        for block in drive.get_basic_blocks() {
            for instruction in block.get_instructions() {
                let row = instruction.to_string();
                assert!(!row.contains("aggregate_receipt_live"));
                if instruction.get_opcode() == InstructionOpcode::Phi && row.contains("aggregate_receipt_active") {
                    receipt_merges.push(block);
                }
                if instruction.get_opcode() == InstructionOpcode::Br && row.contains("aggregate_receipt_active") {
                    guarded_branches += 1;
                }
                if instruction.get_opcode() != InstructionOpcode::Call { continue; }
                let call = CallSiteValue::try_from(instruction).unwrap();
                let pointer = |slot| instruction.get_operand(slot).unwrap().value().unwrap();
                match call.get_called_fn_value() {
                    Some(callee) if callee == producer => produced.push(call.try_as_basic_value().basic().unwrap()),
                    Some(callee) if callee == consumer => transferred.push(pointer(call.count_arguments() - 1)),
                    Some(callee) if callee == release => released.push(pointer(0)),
                    _ => {}
                }
            }
        }
        let [first, preserved, exhaustive, right, shared] = produced.as_slice() else {
            panic!("{target}: {ir}");
        };
        assert_eq!(transferred.len(), 6, "{target}: {ir}");
        for (receipt, uses) in [(first, 1), (preserved, 0), (exhaustive, 2), (right, 1), (shared, 2)] {
            assert_eq!(transferred.iter().filter(|other| *other == receipt).count(), uses, "{target}: {ir}");
        }
        assert_eq!(released, [*shared, *right, *exhaustive, *preserved, *first], "{target}: {ir}");
        assert_eq!(receipt_merges.len(), 7, "{target}: {ir}");
        assert_eq!(guarded_branches, 4, "{target}: {ir}");
        if let Some(out) = std::env::var_os("WOSY_NESTED_RECEIPT_IR_DIR") {
            let mut witness = ir.replace("@main(", "@wosy_authored_main(");
            for name in ["case_ff", "case_ft", "case_tf", "case_tt"] {
                let symbol = function_with_suffix(&module, name);
                witness.push_str(&format!(
                    "\ndefine i64 @wosy_{name}() {{\nentry:\n  %result = call i64 @{symbol}()\n  ret i64 %result\n}}\n"
                ));
            }
            std::fs::write(std::path::PathBuf::from(out).join(format!("{target}.ll")), witness).unwrap();
        }
    }
}

#[test]
fn lexical_aggregate_transfer_releases_at_the_exact_owner_scope() {
    let app = source("workbench", "src/lexical_aggregate_transfer.w");
    let text = include_str!("fixtures/lexical-aggregate-transfer.w");
    for (target, target_layout) in [
        ("native64", ScalarTargetLayout::NATIVE64),
        ("wasm32", ScalarTargetLayout::WASM32),
    ] {
        let prepared = prepare_scalar_project(
            vec![(declarations(&app, text), Vec::new())], vec![app.clone()],
            ScalarPreparationInputs { target_layout },
        ).unwrap_or_else(|error| panic!("{target}: {:?}", error.diagnostics));
        let build = analyze_scalar_build(&prepared);
        assert!(build.diagnostics().is_empty(), "{target}: {:?}", build.diagnostics());
        let ir = emit_scalar_build_project_llvm(&build).unwrap().to_text();
        let context = Context::create();
        let buffer = MemoryBuffer::create_from_memory_range_copy(ir.as_bytes(), target);
        let module = context.create_module_from_ir(buffer).unwrap();
        module.verify().unwrap();

        let drive = module.get_function(&function_with_suffix(&module, "drive")).unwrap();
        let blocks = drive.get_basic_blocks();
        let rows: Vec<_> = blocks.iter().flat_map(|block| block.get_instructions()).collect();
        let named_guards = |opcode| rows.iter().filter(|row|
            row.get_opcode() == opcode && row.to_string().contains("aggregate_receipt_active")
        ).count();
        // One plain-block guard, one left-child guard, one independent parent guard.
        // The right child's exhaustive merge requires no guard of its own.
        assert_eq!(named_guards(InstructionOpcode::Phi), 3, "{target}: {ir}");
        assert_eq!(named_guards(InstructionOpcode::Br), 3, "{target}: {ir}");
        assert!(!ir.contains("aggregate_receipt_live"));

        if let Some(out) = std::env::var_os("WOSY_LEXICAL_RECEIPT_IR_DIR") {
            let mut witness = ir.replace("@main(", "@wosy_authored_main(");
            for name in ["case_ff", "case_ft", "case_tf", "case_tt"] {
                let symbol = function_with_suffix(&module, name);
                witness.push_str(&format!(
                    "\ndefine i64 @wosy_{name}() {{\nentry:\n  %result = call i64 @{symbol}()\n  ret i64 %result\n}}\n"
                ));
            }
            std::fs::write(std::path::PathBuf::from(out).join(format!("{target}.ll")), witness).unwrap();
        }
    }
}

#[test]
fn conditional_formal_transfer_uses_original_hidden_base_in_recursive_frames() {
    let app = source("workbench", "src/conditional_formal_transfer.w");
    let text = include_str!("fixtures/conditional-formal-transfer.w");
    for (target, target_layout) in [
        ("native64", ScalarTargetLayout::NATIVE64),
        ("wasm32", ScalarTargetLayout::WASM32),
    ] {
        let emit = |text: &str| {
            let prepared = prepare_scalar_project(
                vec![(declarations(&app, text), Vec::new())], vec![app.clone()],
                ScalarPreparationInputs { target_layout },
            ).unwrap_or_else(|error| panic!("{target}: {:?}", error.diagnostics));
            let build = analyze_scalar_build(&prepared);
            assert!(build.diagnostics().is_empty(), "{target}: {:?}", build.diagnostics());
            emit_scalar_build_project_llvm(&build).map(|module| module.to_text())
        };
        let ir = emit(text).unwrap_or_else(|error| panic!("{target}: {error}"));
        let context = Context::create();
        let buffer = MemoryBuffer::create_from_memory_range_copy(ir.as_bytes(), target);
        let module = context.create_module_from_ir(buffer).unwrap();
        module.verify().unwrap();
        let relay = module.get_function(&function_with_suffix(&module, "relay")).unwrap();
        let consumer = module.get_function(&function_with_suffix(&module, "accept_packet")).unwrap();
        let release = module.get_function("__wosy_core_free").unwrap();
        let rows: Vec<_> = relay.get_basic_blocks().into_iter()
            .flat_map(|block| block.get_instructions()).collect();
        let matching = |opcode, name: &str| rows.iter().filter(|row|
            row.get_opcode() == opcode && row.to_string().contains(name)
        ).count();
        assert_eq!(matching(InstructionOpcode::Phi, "aggregate_receipt_active"), 1, "{target}: {ir}");
        assert_eq!(matching(InstructionOpcode::Br, "aggregate_receipt_active"), 1, "{target}: {ir}");
        let uses = |callee, last_arg: bool| rows.iter().filter_map(|&row| {
            if row.get_opcode() != InstructionOpcode::Call { return None; }
            let call = CallSiteValue::try_from(row).unwrap();
            (call.get_called_fn_value() == Some(callee)).then(|| {
                let slot = if last_arg { call.count_arguments() - 1 } else { 0 };
                row.get_operand(slot).unwrap().value().unwrap()
            })
        }).collect::<Vec<_>>();
        assert_eq!(uses(consumer, true), [relay.get_last_param().unwrap()], "{target}: {ir}");
        assert_eq!(uses(release, false), [relay.get_last_param().unwrap()], "{target}: {ir}");
        assert_eq!(matching(InstructionOpcode::Alloca, "aggregate_receipt_active"), 0, "{target}: {ir}");

        if let Some(out) = std::env::var_os("WOSY_FORMAL_RECEIPT_IR_DIR") {
            let mut witness = ir.replace("@main(", "@wosy_authored_main(");
            for name in ["case_zero", "case_two", "case_three"] {
                let symbol = function_with_suffix(&module, name);
                witness.push_str(&format!(
                    "\ndefine i64 @wosy_{name}() {{\nentry:\n  %result = call i64 @{symbol}()\n  ret i64 %result\n}}\n"
                ));
            }
            std::fs::write(std::path::PathBuf::from(out).join(format!("{target}.ll")), witness).unwrap();
        }

        let exhaustive = text.replace(
            "if (take) { accept_packet(packet) } else { packet.length }",
            "if (take) { accept_packet(packet) } else { accept_packet(packet) }",
        );
        assert_ne!(exhaustive, text);
        let two_sinks = exhaustive.replace("u64(Packet, bool) relay",
            "u64(u64, Packet) alternative = fn(delta, packet) { packet.length + delta };\nu64(Packet, bool) relay")
            .replace("else { accept_packet(packet) }", "else { alternative(9, packet) }");
        let through_forwarder = exhaustive.replace("u64(Packet, bool) relay",
            "u64(Packet) forward = fn(packet) { accept_packet(packet) };\nu64(Packet, bool) relay")
            .replace("else { accept_packet(packet) }", "else { forward(packet) }");
        for (route, candidate) in [("exhaustive", &exhaustive), ("two-sinks", &two_sinks),
            ("through-forwarder", &through_forwarder)]
        {
            let ir = emit(candidate).unwrap_or_else(|error| panic!("{target}: {error}"));
            let module = context.create_module_from_ir(MemoryBuffer::create_from_memory_range_copy(ir.as_bytes(), target)).unwrap();
            module.verify().unwrap();
            let relay = module.get_function(&function_with_suffix(&module, "relay")).unwrap();
            let consumer = module.get_function(&function_with_suffix(&module, "accept_packet")).unwrap();
            let second = match route {
                "two-sinks" => module.get_function(&function_with_suffix(&module, "alternative")).unwrap(),
                "through-forwarder" => module.get_function(&function_with_suffix(&module, "forward")).unwrap(),
                _ => consumer,
            };
            if route == "through-forwarder" {
                let mut forwarded = Vec::new();
                for row in second.get_basic_blocks().into_iter().flat_map(|block| block.get_instructions()) {
                    if row.get_opcode() != InstructionOpcode::Call { continue; }
                    let call = CallSiteValue::try_from(row).unwrap();
                    let callee = call.get_called_fn_value().unwrap();
                    if callee == consumer {
                        forwarded.push(row.get_operand(call.count_arguments() - 1).unwrap().value().unwrap());
                    } else {
                        assert!(callee.get_name().to_str().unwrap().starts_with("llvm.memmove."));
                    }
                }
                assert_eq!(forwarded, [second.get_last_param().unwrap()], "{target}: {ir}");
            }
            let rows: Vec<_> = relay.get_basic_blocks().into_iter().flat_map(|block| block.get_instructions()).collect();
            let mut transfers = Vec::new();
            let mut sinks = Vec::new();
            for row in rows {
                assert!(!row.to_string().contains("aggregate_receipt_active"), "{target}: {ir}");
                if row.get_opcode() != InstructionOpcode::Call { continue; }
                let call = CallSiteValue::try_from(row).unwrap();
                let callee = call.get_called_fn_value().expect("an exhaustive relay has no indirect calls");
                if callee != consumer && callee != second {
                    let name = callee.get_name().to_str().unwrap();
                    assert!(matches!(name, "llvm.memmove.p0.p0.i32" | "llvm.memmove.p0.p0.i64"),
                        "{target}: unexpected relay effect {name}");
                    assert_eq!(call.count_arguments(), 4);
                    let argument = |slot| row.get_operand(slot).unwrap().value().unwrap();
                    assert_eq!(argument(1), relay.get_first_param().unwrap(), "{target}: copy must read the descriptor");
                    assert_ne!(argument(0), relay.get_last_param().unwrap(), "{target}: copy must not write the original backing");
                    assert!(argument(2).into_int_value().get_zero_extended_constant().is_some_and(|size| size > 0));
                    assert_eq!(argument(3).into_int_value().get_zero_extended_constant(), Some(0));
                    continue;
                }
                let base = row.get_operand(call.count_arguments() - 1).unwrap().value().unwrap();
                assert_eq!(base, relay.get_last_param().unwrap(), "{target}: {ir}");
                if route == "two-sinks" && callee == second {
                    assert_eq!(call.count_arguments(), 3);
                    assert_eq!(row.get_operand(0).unwrap().value().unwrap().into_int_value()
                        .get_zero_extended_constant(), Some(9));
                }
                sinks.push(callee);
                transfers.push(row.get_parent().unwrap());
            }
            assert_eq!(transfers.len(), 2);
            assert_eq!(sinks.iter().filter(|&&value| value == consumer).count(), if second == consumer { 2 } else { 1 });
            assert!(sinks.contains(&second));
            assert_ne!(transfers[0], transfers[1], "{target}: transfers must be in opposite arms");
            if let Some(out) = std::env::var_os("WOSY_FORMAL_RECEIPT_IR_DIR") {
                let out = std::path::PathBuf::from(out).join(route);
                std::fs::create_dir_all(&out).unwrap();
                let mut witness = ir.replace("@main(", "@wosy_authored_main(");
                for name in ["case_zero", "case_two", "case_three"] {
                    let symbol = function_with_suffix(&module, name);
                    witness.push_str(&format!("\ndefine i64 @wosy_{name}() {{\nentry:\n  %v = call i64 @{symbol}()\n  ret i64 %v\n}}\n"));
                }
                std::fs::write(out.join(format!("{target}.ll")), witness).unwrap();
            }
        }
        let cycle = through_forwarder.replace("forward = fn(packet) { accept_packet(packet) }",
            "forward = fn(packet) { forward(packet) }");
        assert_ne!(cycle, through_forwarder);
        assert!(emit(&cycle).expect_err("one grounded sibling cannot authorize the other sibling's cycle")
            .contains("aggregate owning formal transfer does not reach a proved release"));

        let overflow = exhaustive.replace("else { accept_packet(packet) }",
            "else { if (packet.length == 0) { accept_packet(packet) } else { accept_packet(packet) } }");
        assert_ne!(overflow, exhaustive);
        assert!(emit(&overflow).expect_err("three distinct formal transfers exceed the supported exact envelope")
            .contains("aggregate owning input has no closed formal"));

        // A shallow full tree exercises the semantic candidate limit rather
        // than a nesting-depth limit. Every leaf moves the original formal.
        fn consuming_tree(low: u64, high: u64) -> String {
            if high - low == 1 {
                return "accept_packet(packet)".to_owned();
            }
            let split = (low + high) / 2;
            let left = consuming_tree(low, split);
            let right = consuming_tree(split, high);
            format!("if (packet.length < {split}) {{ {left} }} else {{ {right} }}")
        }
        let left = consuming_tree(0, 32);
        let right = consuming_tree(32, 64);
        let many = format!("if (take) {{ {left} }} else {{ {right} }}");
        assert_eq!(many.matches("accept_packet(packet)").count(), 64);
        let many = text.replace("if (take) { accept_packet(packet) } else { packet.length }", &many);
        assert_ne!(many, text);
        assert!(emit(&many).expect_err("many sites must fail the bounded formal proof")
            .contains("aggregate owning input has no closed formal"));

        // A local receipt taking the identical fork in this relay should
        // share the formal plan's certified CFG and emitted guard.
        let combined = text.replace(
            "if (take) { accept_packet(packet) } else { packet.length }",
            "Packet local = make_packet(2); if (take) { u64 added = accept_packet(local); u64 moved = accept_packet(packet); moved + added } else { packet.length }",
        );
        let combined = emit(&combined).unwrap_or_else(|error| panic!("{target}: {error}"));
        let buffer = MemoryBuffer::create_from_memory_range_copy(combined.as_bytes(), target);
        context.create_module_from_ir(buffer).unwrap().verify().unwrap();
        assert_eq!(combined.lines().filter(|row| row.contains("aggregate_receipt_active")
            && row.contains(" = phi i1 ")).count(), 1, "{target}: {combined}");
        assert_eq!(combined.lines().filter(|row| row.contains("br i1 %aggregate_receipt_active"))
            .count(), 2, "{target}: {combined}");

        let nonterminal = text
            .replace("u64(Packet, bool) relay", "u64(Packet) forward = fn(packet) { accept_packet(packet) };\nu64(Packet, bool) relay")
            .replace("if (take) { accept_packet(packet) }", "if (take) { forward(packet) }");
        let error = emit(&nonterminal).expect_err("a conditional formal relay requires a terminal destination");
        assert!(error.contains("aggregate owning formal conditional transfer"), "{target}: {error}");

        let unseeded = text.replace("relay(packet, take)", "accept_packet(packet)");
        let error = emit(&unseeded).expect_err("a relay without an owned incoming mode must fail closed");
        assert!(error.contains("aggregate owning formal conditional transfer"), "{target}: {error}");
    }
}

#[test]
fn aggregate_owner_recommit_preserves_the_original_base_across_ordinary_identity_calls() {
    let app = source("workbench", "src/aggregate_owner_recommit.w");
    let unary = include_str!("fixtures/aggregate-owner-recommit.w");
    let companions = unary
        .replace("Packet(Packet) first_identity = fn(packet) { packet };",
            "Packet(u64, Packet, bool) first_identity = fn(prefix, packet, toggle) { prefix; toggle; packet };")
        .replace("Packet(Packet) second_identity = fn(packet) { packet };",
            "Packet(bool, u64, Packet) second_identity = fn(toggle, suffix, packet) { toggle; suffix; packet };")
        .replace("first_identity(original)", "first_identity(length, original, true)")
        .replace("second_identity(middle)", "second_identity(take, 7, middle)");
    let wrappers = companions
        .replace("Packet(u64, Packet, bool) first_identity = fn(prefix, packet, toggle) { prefix; toggle; packet };",
            concat!(
                "Packet(bool, u64, Packet) identity_leaf = fn(toggle, suffix, packet) { toggle; suffix; packet };\n",
                "Packet(u64, Packet, bool) bridge = fn(prefix, packet, toggle) { identity_leaf(toggle, prefix, packet) };\n",
                "Packet(u64, Packet, bool) first_identity = fn(prefix, packet, toggle) { bridge(prefix, packet, toggle) };",
            ))
        .replace("Packet(bool, u64, Packet) second_identity = fn(toggle, suffix, packet) { toggle; suffix; packet };",
            "Packet(bool, u64, Packet) second_identity = fn(toggle, suffix, packet) { bridge(suffix, packet, toggle) };");
    assert_ne!(companions, wrappers);
    for (target, target_layout) in [
        ("native64", ScalarTargetLayout::NATIVE64),
        ("wasm32", ScalarTargetLayout::WASM32),
    ] {
        let emit = |text: &str| {
            let prepared = prepare_scalar_project(
                vec![(declarations(&app, text), Vec::new())], vec![app.clone()],
                ScalarPreparationInputs { target_layout },
            ).unwrap_or_else(|error| panic!("{target}: {:?}", error.diagnostics));
            let build = analyze_scalar_build(&prepared);
            assert!(build.diagnostics().is_empty(), "{target}: {:?}", build.diagnostics());
            emit_scalar_build_project_llvm(&build).map(|module| module.to_text())
        };
        for (variant, text, slots, arities) in [
            ("unary", unary, [0, 0], [1, 1]),
            ("companions", companions.as_str(), [1, 2], [3, 3]),
            ("wrappers", wrappers.as_str(), [1, 2], [3, 3]),
        ] {
            let ir = emit(text).unwrap_or_else(|error| panic!("{variant}/{target}: {error}"));
            let context = Context::create();
            let buffer = MemoryBuffer::create_from_memory_range_copy(ir.as_bytes(), target);
            let module = context.create_module_from_ir(buffer).unwrap();
            module.verify().unwrap();
            let find = |name| module.get_function(&function_with_suffix(&module, name)).unwrap();
            let (drive, producer, consumer) = (find("drive"), find("make_packet"), find("accept_packet"));
            let (first, second) = (find("first_identity"), find("second_identity"));
            let release = module.get_function("__wosy_core_free").unwrap();
            for ((relay, slot), arity) in [first, second].into_iter().zip(slots).zip(arities) {
                assert_eq!(relay.count_params(), arity, "{target}: {ir}");
                assert!(matches!(relay.get_type().get_return_type(), Some(inkwell::types::BasicTypeEnum::PointerType(_))));
                let rows: Vec<_> = relay.get_basic_blocks().into_iter()
                    .flat_map(|block| block.get_instructions()).collect();
                let [return_row] = rows.as_slice() else { panic!("{target}: {ir}"); };
                assert_eq!(return_row.get_opcode(), InstructionOpcode::Return);
                assert_eq!(return_row.get_operand(0).unwrap().value(), relay.get_nth_param(slot));
            }
            let mut produced = Vec::new();
            let mut passed = Vec::new();
            let mut released = Vec::new();
            let mut relays = Vec::new();
            for row in drive.get_basic_blocks().into_iter().flat_map(|block| block.get_instructions()) {
                if row.get_opcode() != InstructionOpcode::Call { continue; }
                let call = CallSiteValue::try_from(row).unwrap();
                let argument = |slot| row.get_operand(slot).unwrap().value().unwrap();
                match call.get_called_fn_value() {
                    Some(callee) if callee == producer => produced.push(call.try_as_basic_value().basic().unwrap()),
                    Some(callee) if callee == consumer => passed.push(argument(call.count_arguments() - 1)),
                    Some(callee) if callee == release => released.push(argument(0)),
                    Some(callee) if callee == first || callee == second => {
                        assert_eq!(call.count_arguments(), arities[usize::from(callee == second)], "{target}: {ir}");
                        relays.push(callee);
                    }
                    _ => {}
                }
            }
            let [original, keeper, trailing] = produced.as_slice() else { panic!("{target}: {ir}"); };
            assert_eq!(relays, [first, second]);
            assert_eq!(passed, [*original]);
            assert_eq!(released, [*trailing, *keeper, *original]);

            if let Some(out) = std::env::var_os("WOSY_RECOMMIT_RECEIPT_IR_DIR") {
                let mut witness = ir.replace("@main(", "@wosy_authored_main(");
                for (wrapper, name) in [("retain", "retain_witness"), ("transfer", "transfer_witness")] {
                    let symbol = function_with_suffix(&module, name);
                    witness.push_str(&format!(
                        "\ndefine i64 @wosy_{wrapper}() {{\nentry:\n  %result = call i64 @{symbol}()\n  ret i64 %result\n}}\n"
                    ));
                }
                let filename = if variant == "unary" { format!("{target}.ll") }
                    else { format!("{variant}-{target}.ll") };
                std::fs::write(std::path::PathBuf::from(out).join(filename), witness).unwrap();
            }

        }

        let replacement = unary.replace(
            "Packet(Packet) second_identity = fn(packet) { packet };",
            "Packet(Packet) second_identity = fn(packet) { Packet different = { .data = null; .length = packet.length; }; different };",
        );
        let error = emit(&replacement).expect_err("a different output cannot recommit the old owner");
        assert!(error.contains("aggregate owning input has no closed formal"), "{target}: {error}");
    }
}

#[test]
fn independent_conditioned_owners_keep_separate_receipts_at_one_exit() {
    let app = source("workbench", "src/independent_conditioned_owners.w");
    let factory_fixture = include_str!("fixtures/structured-local-choice.w");
    let (factory_source, _) = factory_fixture.split_once("u64(u64, bool) drive").unwrap();
    for (variant, primary_transfer, secondary_transfer, guarded, primary_return) in [
        ("release", false, false, false, false), ("transfer", true, false, false, false),
        ("transfer-secondary", false, true, false, false), ("transfer-both", true, true, false, false),
        ("guard-secondary", false, true, true, false), ("guard-secondary-transfer-primary", true, true, true, false),
        ("guard-secondary-return-primary", false, true, true, true),
        ("guard-primary-after-read", false, false, true, false),
        ("guard-primary-after-recommit", false, false, true, false),
        ("exhaustive-secondary", false, true, true, false),
    ] {
        let guard_primary = variant.starts_with("guard-primary-");
        let exhaustive = variant == "exhaustive-secondary";
        let migrate_primary = variant == "guard-primary-after-recommit";
        let mut text = factory_source.to_owned();
        if primary_transfer || secondary_transfer || guard_primary {
            text.push_str("u64(Packet) sink = fn(packet) { packet.length };\n");
        }
        if exhaustive {
            text.push_str("u64(u64, Packet) alternative_sink = fn(extra, packet) { packet.length + extra };\n");
        }
        if migrate_primary {
            text.push_str("Packet(Packet) relay = fn(packet) { packet };\n");
        }
        if guard_primary {
            text.push_str("u64(*Packet) read_amount = fn(view) { (*view).length };\n");
        }
        text.push_str(if primary_return { concat!(
            "Packet(Packet) relay = fn(packet) { packet };\n",
            "Packet(bool, bool, bool) shipment = fn(left, right, take) {\n",
        ) } else if guarded { "u64(bool, bool, bool) drive = fn(left, right, take) {\n" }
            else { "u64(bool, bool) drive = fn(left, right) {\n" });
        text.push_str(concat!(
            "    Packet first = if (left) { make_short(2) } else { make_long(3) };\n",
            "    Packet second = if (right) { make_short(7) } else { make_long(9) };\n",
            "    second.length;\n",
        ));
        if guard_primary {
            if migrate_primary {
                text.push_str("    Packet current = relay(first);\n");
            }
            let holder = if migrate_primary { "current" } else { "first" };
            text.push_str(&format!("    u64 saved = read_amount(&{holder});\n    if (take) {{ sink({holder}); }};\n"));
        } else if exhaustive {
            text.push_str("    if (take) { sink(second) } else { alternative_sink(1, second) };\n");
        } else if guarded { text.push_str("    if (take) { sink(second); };\n"); }
        else if secondary_transfer { text.push_str("    sink(second);\n"); }
        text.push_str(if primary_return { concat!(
            "    Packet outgoing = relay(first);\n",
            "    outgoing\n};\n",
            "u64(bool, bool, bool) drive = fn(left, right, take) {\n",
            "    Packet received = shipment(left, right, take);\n",
            "    received.length\n};\n",
        ) } else if guard_primary { "    saved\n};\n" }
            else if primary_transfer { "    sink(first)\n};\n" } else { "    first.length\n};\n" });
        let mut cases = Vec::new();
        for (case, left, right) in [("ff", "false", "false"), ("ft", "false", "true"), ("tf", "true", "false"), ("tt", "true", "true")] {
            if guarded {
                for (suffix, take) in [("keep", "false"), ("send", "true")] {
                    let case = format!("{case}_{suffix}");
                    text.push_str(&format!("u64() case_{case} = fn {{ drive({left}, {right}, {take}) }};\ncase_{case}();\n"));
                    cases.push(case);
                }
            } else {
                text.push_str(&format!("u64() case_{case} = fn {{ drive({left}, {right}) }};\ncase_{case}();\n"));
                cases.push(case.to_owned());
            }
        }
        text.push_str("%%end\n");
        for (target, target_layout) in [("native64", ScalarTargetLayout::NATIVE64), ("wasm32", ScalarTargetLayout::WASM32)] {
            for (route, project) in [("single", false), ("project", true)] {
                let prepared = prepare_scalar_project(vec![(declarations(&app, &text), vec![])],
                    vec![app.clone()], ScalarPreparationInputs { target_layout }).unwrap();
                let build = analyze_scalar_build(&prepared);
                assert!(build.diagnostics().is_empty(), "{variant}/{target}/{route}: {:?}", build.diagnostics());
                let ir = if project { emit_scalar_build_project_llvm(&build) }
                    else { wosy_compiler::emit_scalar_build_single_module_llvm(&build) }
                    .unwrap_or_else(|error| panic!("{variant}/{target}/{route}: {error}")).to_text();
                let context = Context::create();
                let module = context.create_module_from_ir(
                    MemoryBuffer::create_from_memory_range_copy(ir.as_bytes(), target)).unwrap();
                module.verify().unwrap();
                let symbol = |name: &str| if project { function_with_suffix(&module, name) } else { name.to_owned() };
                let find = |name: &str| module.get_function(&symbol(name)).unwrap();
                let primary = find(if primary_return { "shipment" } else { "drive" });
                let relay = (primary_return || migrate_primary).then(|| find("relay"));
                let reader = guard_primary.then(|| find("read_amount"));
                let mut relay_calls = 0;
                let mut read_amounts = 0;
                let mut returned = None;
                let factories = [find("make_short"), find("make_long")];
                let sink = (primary_transfer || secondary_transfer || guard_primary).then(|| find("sink"));
                let alternative_sink = exhaustive.then(|| find("alternative_sink"));
                let mut alternative_passed = None;
                let free = module.get_function("__wosy_core_free").unwrap();
                let mut produced = [Vec::new(), Vec::new()];
                let mut merges = Vec::new();
                let mut released = Vec::new();
                let mut passed = Vec::new();
                let mut guards = Vec::new();
                for row in primary.get_basic_blocks().into_iter().flat_map(|block| block.get_instructions()) {
                    let arg = |slot| row.get_operand(slot).unwrap().value().unwrap();
                    if row.get_opcode() == InstructionOpcode::Phi && row.to_string().contains("phi ptr") {
                        let incoming = (0..row.get_num_operands()).filter_map(|n| row.get_operand(n)?.value()).collect::<Vec<_>>();
                        merges.push((inkwell::values::PhiValue::try_from(row).unwrap().as_basic_value(), incoming));
                    }
                    if row.get_opcode() == InstructionOpcode::Phi && row.to_string().contains("aggregate_receipt_active") {
                        let mut flags = (0..row.get_num_operands()).filter_map(|n| row.get_operand(n)?.value())
                            .map(|value| value.into_int_value().get_zero_extended_constant().unwrap()).collect::<Vec<_>>();
                        flags.sort_unstable();
                        guards.push(flags);
                    }
                    if primary_return && row.get_opcode() == InstructionOpcode::Return {
                        assert!(returned.replace(arg(0)).is_none());
                    }
                    let Ok(call) = CallSiteValue::try_from(row) else { continue };
                    let callee = call.get_called_fn_value().unwrap();
                    if factories.contains(&callee) {
                        let choice = usize::from(arg(0).into_int_value().get_zero_extended_constant().unwrap() >= 7);
                        produced[choice].push((call.try_as_basic_value().basic().unwrap(), arg(1)));
                    } else if Some(callee) == alternative_sink {
                        assert_eq!(call.count_arguments(), 3);
                        assert_eq!(arg(0).into_int_value().get_zero_extended_constant(), Some(1));
                        assert!(alternative_passed.replace(arg(2)).is_none());
                    } else if Some(callee) == sink {
                        assert_eq!(call.count_arguments(), 2);
                        assert!(!guard_primary || read_amounts == 1, "guarded handoff follows the author's certified read");
                        passed.push(arg(1));
                    } else if Some(callee) == reader {
                        assert_eq!(relay_calls, usize::from(migrate_primary));
                        assert_eq!(call.count_arguments(), 1);
                        read_amounts += 1;
                    } else if Some(callee) == relay {
                        assert_eq!(call.count_arguments(), 1);
                        relay_calls += 1;
                    } else if callee == free {
                        released.push(arg(0));
                    }
                }
                assert_eq!(merges.len(), 2);
                let chosen = produced.map(|arms| {
                    let [first, second] = arms.as_slice() else { panic!("{ir}") };
                    assert_ne!(first.0, second.0);
                    assert_eq!(first.1, second.1, "each choice uses one dedicated descriptor");
                    let [(phi, incoming)] = merges.iter().filter(|(_, incoming)|
                        incoming.contains(&first.0) && incoming.contains(&second.0)).collect::<Vec<_>>()[..]
                    else { panic!("{ir}") };
                    assert_eq!(incoming.len(), 2);
                    (*phi, first.1)
                });
                assert_ne!(chosen[0].0, chosen[1].0);
                assert_ne!(chosen[0].1, chosen[1].1);
                let expected_transfers = secondary_transfer.then_some(chosen[1].0).into_iter()
                    .chain((primary_transfer || guard_primary).then_some(chosen[0].0));
                assert!(passed.iter().copied().eq(expected_transfers), "only the named receipts transfer in source order");
                assert_eq!(guards, if guarded && !exhaustive { vec![vec![0, 1]] } else { vec![] });
                assert_eq!(alternative_passed, exhaustive.then_some(chosen[1].0));
                let expected_local = (!secondary_transfer || (guarded && !exhaustive)).then_some(chosen[1].0).into_iter()
                    .chain((!primary_transfer && !primary_return).then_some(chosen[0].0));
                assert!(released.iter().copied().eq(expected_local), "only retained original backings release locally");
                assert_eq!(relay_calls, usize::from(primary_return || migrate_primary));
                assert_eq!(read_amounts, usize::from(guard_primary));
                assert_eq!(returned, primary_return.then_some(chosen[0].0), "only the primary backing may be returned");
                if primary_return {
                    assert_eq!(primary.count_params(), 4, "one ordinary hidden result destination");
                    let mut received = None;
                    let mut caller_releases = Vec::new();
                    for row in find("drive").get_basic_blocks().into_iter().flat_map(|block| block.get_instructions()) {
                        let Ok(call) = CallSiteValue::try_from(row) else { continue };
                        match call.get_called_fn_value() {
                            Some(callee) if callee == primary => {
                                assert!(received.replace(call.try_as_basic_value().basic().unwrap()).is_none());
                            }
                            Some(callee) if callee == free => caller_releases.push(row.get_operand(0).unwrap().value().unwrap()),
                            _ => {}
                        }
                    }
                    assert_eq!(caller_releases, [received.expect("one returned backing")]);
                }
                for sink in [sink, alternative_sink].into_iter().flatten() {
                    let releases: Vec<_> = sink.get_basic_blocks().into_iter().flat_map(|block| block.get_instructions())
                        .filter(|row| CallSiteValue::try_from(*row).ok().and_then(|call| call.get_called_fn_value()) == Some(free))
                        .map(|row| row.get_operand(0).unwrap().value().unwrap()).collect();
                    assert_eq!(releases, [sink.get_last_param().unwrap()]);
                }
                if let Some(out) = std::env::var_os("WOSY_INDEPENDENT_CHOICES_IR_DIR") {
                    let out = std::path::PathBuf::from(out).join(format!("{variant}-{route}"));
                    std::fs::create_dir_all(&out).unwrap();
                    let mut witness = ir.replace("@main(", "@wosy_authored_main(");
                    for case in &cases {
                        witness.push_str(&format!("\ndefine i64 @wosy_case_{case}() {{\nentry:\n  %v = call i64 @{}()\n  ret i64 %v\n}}\n", symbol(&format!("case_{case}"))));
                    }
                    std::fs::write(out.join(format!("{target}.ll")), witness).unwrap();
                }
            }
        }
    }
}

#[test]
fn selected_recommitted_owner_returns_its_backing_to_the_caller() {
    let app = source("workbench", "src/selected_recommitted_return.w");
    let text = include_str!("fixtures/structured-local-choice.w")
        .replace("u64(u64, bool) drive = fn(depth, choose) {",
            "u64(*Packet) read_amount = fn(view) { (*view).length };\nPacket(Packet) relay = fn(packet) { packet };\nPacket(u64, bool) shipment = fn(depth, choose) {")
        .replace("    chosen.length\n};", concat!(
            "    read_amount(&chosen);\n",
            "    Packet returned = relay(chosen);\n",
            "    read_amount(&returned);\n",
            "    returned\n",
            "};\n",
            "u64(u64, bool) drive = fn(depth, choose) { Packet received = shipment(depth, choose); received.length };"));
    let with_siblings = text
        .replace("Packet(u64, bool) shipment", concat!(
            "Packet(u64) neighbor = fn(length) { u8[length] bytes; bytes[0] = 39; *?u8 data = null; ",
            "unsafe { data = &?bytes[0]; }; Packet packet = { .data = data; .length = length; }; packet };\n",
            "Packet(u64, bool) shipment"))
        .replace("    Packet returned = relay(chosen);",
            "    Packet older = neighbor(5);\n    older.length;\n    Packet returned = relay(chosen);")
        .replace("    returned\n};",
            "    Packet newer = neighbor(7);\n    newer.length;\n    returned\n};");
    assert_ne!(text, with_siblings);
    let recursive = with_siblings
        .replace("if (choose != (depth == 3)) { make_short(2) } else { make_long(2) }",
            "if (depth == 0) { if (choose) { make_short(2) } else { make_long(2) } } else { shipment(depth - 1, choose) }")
        .replace("drive(3, true)", "drive(3, false)");
    assert_ne!(recursive, with_siblings);
    let (_, bodies_and_caller) = recursive.split_once("Packet(u64, bool) shipment").unwrap();
    let (shipment_body, _) = bodies_and_caller.split_once("u64(u64, bool) drive").unwrap();
    let seed_choice = "if (depth == 0) { if (choose) { make_short(2) } else { make_long(2) } } else { shipment(depth - 1, choose) }";
    let alternate = shipment_body.replace(seed_choice,
        "if (depth == 0) { shipment(0, choose) } else { shipment(depth - 1, choose) }");
    assert_ne!(shipment_body, alternate);
    let mutual = recursive.replace("shipment(depth - 1, choose) }", "alternate(depth - 1, choose) }")
        .replace("u64(u64, bool) drive", &format!("Packet(u64, bool) alternate{alternate}u64(u64, bool) drive"));
    assert_ne!(recursive, mutual);
    for (variant, text) in [
        ("baseline", text.as_str()), ("siblings", with_siblings.as_str()),
        ("recursive", recursive.as_str()), ("mutual", mutual.as_str()),
    ] {
        for (target, target_layout) in [("native64", ScalarTargetLayout::NATIVE64), ("wasm32", ScalarTargetLayout::WASM32)] {
            for (route, project) in [("single", false), ("project", true)] {
                let prepared = prepare_scalar_project(
                    vec![(declarations(&app, text), Vec::new())], vec![app.clone()],
                    ScalarPreparationInputs { target_layout },
                ).unwrap_or_else(|error| panic!("{variant}/{target}/{route}: {:?}", error.diagnostics));
                let build = analyze_scalar_build(&prepared);
                assert!(build.diagnostics().is_empty(), "{target}/{route}: {:?}", build.diagnostics());
                let ir = if project { emit_scalar_build_project_llvm(&build) }
                    else { wosy_compiler::emit_scalar_build_single_module_llvm(&build) }
                    .unwrap_or_else(|error| panic!("{variant}/{target}/{route}: {error}")).to_text();
                let context = Context::create();
                let module = context.create_module_from_ir(
                    MemoryBuffer::create_from_memory_range_copy(ir.as_bytes(), target)).unwrap();
                module.verify().unwrap();
                let symbol = |name: &str| if project { function_with_suffix(&module, name) } else { name.to_owned() };
                let find = |name| module.get_function(&symbol(name)).unwrap();
                let (ship, drive, relay, reader, free) = (
                    find("shipment"), find("drive"), find("relay"), find("read_amount"),
                    module.get_function("__wosy_core_free").unwrap(),
                );
                let factories = [find("make_short"), find("make_long")];
                let sibling_factory = (variant != "baseline").then(|| find("neighbor"));
                let frames = match variant {
                    "mutual" => {
                        let alternate = find("alternate");
                        vec![(ship, Some(alternate), 2, 1), (alternate, Some(ship), 0, 2)]
                    }
                    "recursive" => vec![(ship, Some(ship), 2, 1)],
                    _ => vec![(ship, None, 2, 0)],
                };
                for (frame, recursive_target, local_leaves, recursive_leaves) in frames {
                    let mut recursive_returns = Vec::new();
                    let mut siblings = Vec::new();
                    let mut local_releases = Vec::new();
                    assert_eq!(frame.count_params(), 3, "{ir}");
                    assert_eq!(relay.count_params(), 1, "{ir}");
                    assert_eq!(drive.count_params(), 2, "{ir}");
                    let mut original = Vec::new();
                    let mut reads_and_handoff = Vec::new();
                    let mut returned = None;
                    let mut bases = Vec::new();
                    for row in frame.get_basic_blocks().into_iter().flat_map(|block| block.get_instructions()) {
                        if row.get_opcode() == InstructionOpcode::Phi && row.to_string().contains("phi ptr") {
                            let incoming = (0..row.get_num_operands()).filter_map(|n| row.get_operand(n)?.value()).collect::<Vec<_>>();
                            bases.push((inkwell::values::PhiValue::try_from(row).unwrap().as_basic_value(), incoming));
                        }
                        if row.get_opcode() == InstructionOpcode::Return {
                            assert!(returned.replace(row.get_operand(0).unwrap().value().unwrap()).is_none());
                            assert!(local_releases.iter().copied().eq(siblings.iter().rev().copied()),
                                "release only local siblings, in reverse birth order before returning");
                        }
                        let Ok(call) = CallSiteValue::try_from(row) else { continue };
                        let callee = call.get_called_fn_value().unwrap();
                        if callee == free {
                            local_releases.push(row.get_operand(0).unwrap().value().unwrap());
                        } else if Some(callee) == sibling_factory {
                            assert_eq!(reads_and_handoff.len(), if siblings.is_empty() { 1 } else { 3 });
                            siblings.push(call.try_as_basic_value().basic().unwrap());
                        } else if Some(callee) == recursive_target {
                            assert_eq!(call.count_arguments(), callee.count_params());
                            recursive_returns.push(call.try_as_basic_value().basic().unwrap());
                        } else if factories.contains(&callee) {
                            original.push(call.try_as_basic_value().basic().unwrap());
                        } else if callee == reader || callee == relay {
                            assert_eq!(call.count_arguments(), 1);
                            reads_and_handoff.push(callee);
                        }
                    }
                    assert_eq!(reads_and_handoff, [reader, relay, reader]);
                    assert_eq!(siblings.len(), if sibling_factory.is_some() { 2 } else { 0 });
                    assert_eq!(recursive_returns.len(), recursive_leaves);
                    assert_eq!(bases.len(), local_leaves + recursive_leaves - 1);
                    let selected = returned.expect("return the actual original-base phi");
                    assert!(!local_releases.contains(&selected), "the selected backing survives into the caller");
                    let mut pending = vec![selected];
                    let mut expanded = Vec::new();
                    let mut leaves = Vec::new();
                    while let Some(value) = pending.pop() {
                        assert!(!expanded.contains(&value), "same-frame receipt phis are acyclic");
                        expanded.push(value);
                        if let Some((_, incoming)) = bases.iter().find(|(phi, _)| *phi == value) {
                            assert_eq!(incoming.len(), 2);
                            pending.extend(incoming.iter().copied());
                        } else {
                            leaves.push(value);
                        }
                    }
                    assert_eq!(original.len(), local_leaves);
                    assert_eq!(leaves.len(), original.len() + recursive_returns.len());
                    assert!(original.iter().chain(&recursive_returns).all(|value| leaves.contains(value)));
                }
                let mut received = None;
                let mut released = Vec::new();
                for row in drive.get_basic_blocks().into_iter().flat_map(|block| block.get_instructions()) {
                    let Ok(call) = CallSiteValue::try_from(row) else { continue };
                    match call.get_called_fn_value() {
                        Some(target) if target == ship => assert!(received.replace(call.try_as_basic_value().basic().unwrap()).is_none()),
                        Some(target) if target == free => released.push(row.get_operand(0).unwrap().value().unwrap()),
                        _ => {}
                    }
                }
                assert_eq!(released, [received.expect("one received backing")], "caller releases the transferred original once");
                if let Some(out) = std::env::var_os("WOSY_SELECTED_RECOMMITTED_RETURN_IR_DIR") {
                    let out = std::path::PathBuf::from(out).join(route);
                    let out = if variant == "baseline" { out } else { out.join(variant) };
                    std::fs::create_dir_all(&out).unwrap();
                    let mut witness = ir.replace("@main(", "@wosy_authored_main(");
                    for name in ["case_zero", "case_two", "case_three"] {
                        witness.push_str(&format!("\ndefine i64 @wosy_{name}() {{\nentry:\n  %v = call i64 @{}()\n  ret i64 %v\n}}\n", symbol(name)));
                    }
                    std::fs::write(out.join(format!("{target}.ll")), witness).unwrap();
                }
            }
        }
    }
}

#[test]
fn recommitted_wrapper_return_preserves_one_base_across_two_owning_returns() {
    let app = source("workbench", "src/recommitted_wrapper_return.w");
    let text = include_str!("fixtures/recommitted-wrapper-return.w");
    for (target, target_layout) in [
        ("native64", ScalarTargetLayout::NATIVE64),
        ("wasm32", ScalarTargetLayout::WASM32),
    ] {
        let emit = |text: &str| {
            let prepared = prepare_scalar_project(
                vec![(declarations(&app, text), Vec::new())], vec![app.clone()],
                ScalarPreparationInputs { target_layout },
            ).unwrap_or_else(|error| panic!("{target}: {:?}", error.diagnostics));
            let build = analyze_scalar_build(&prepared);
            assert!(build.diagnostics().is_empty(), "{target}: {:?}", build.diagnostics());
            emit_scalar_build_project_llvm(&build).map(|module| module.to_text())
        };
        let ir = emit(text).unwrap_or_else(|error| panic!("{target}: {error}"));
        let context = Context::create();
        let buffer = MemoryBuffer::create_from_memory_range_copy(ir.as_bytes(), target);
        let module = context.create_module_from_ir(buffer).unwrap();
        module.verify().unwrap();
        let find = |name| module.get_function(&function_with_suffix(&module, name)).unwrap();
        let (make, inner, outer, drive) = (find("make_packet"), find("inner_owner"), find("outer_owner"), find("drive"));
        let (first, second, sink) = (find("first_identity"), find("second_identity"), find("accept_packet"));
        let free = module.get_function("__wosy_core_free").unwrap();
        let verify_wrapper = |wrapper: inkwell::values::FunctionValue<'_>, source, relay: inkwell::values::FunctionValue<'_>, independent: bool| {
            assert_eq!(wrapper.count_params(), 2, "{target}: {ir}");
            assert_eq!(relay.count_params(), 1, "{target}: {ir}");
            let mut produced = Vec::new();
            let mut released = Vec::new();
            let mut returned = None;
            for row in wrapper.get_basic_blocks().into_iter().flat_map(|block| block.get_instructions()) {
                if row.get_opcode() == InstructionOpcode::Return {
                    assert!(returned.replace(row.get_operand(0).unwrap().value().unwrap()).is_none());
                }
                if row.get_opcode() != InstructionOpcode::Call { continue; }
                let call = CallSiteValue::try_from(row).unwrap();
                match call.get_called_fn_value() {
                    Some(callee) if callee == source => produced.push(call.try_as_basic_value().basic().unwrap()),
                    Some(callee) if callee == free => released.push(row.get_operand(0).unwrap().value().unwrap()),
                    Some(callee) if callee == relay => assert_eq!(call.count_arguments(), 1),
                    _ => {}
                }
            }
            assert_eq!(produced.len(), if independent { 2 } else { 1 }, "{target}: {ir}");
            assert_eq!(returned, Some(produced[0]), "{target}: {ir}");
            assert_eq!(released, produced[1..], "{target}: {ir}");
        };
        verify_wrapper(inner, make, first, true);
        verify_wrapper(outer, inner, second, false);
        let mut owned = None;
        let mut passed = None;
        let mut releases = Vec::new();
        for row in drive.get_basic_blocks().into_iter().flat_map(|block| block.get_instructions()) {
            if row.get_opcode() != InstructionOpcode::Call { continue; }
            let call = CallSiteValue::try_from(row).unwrap();
            let argument = |slot| row.get_operand(slot).unwrap().value().unwrap();
            match call.get_called_fn_value() {
                Some(callee) if callee == outer => owned = call.try_as_basic_value().basic(),
                Some(callee) if callee == sink => passed = Some(argument(call.count_arguments() - 1)),
                Some(callee) if callee == free => releases.push(argument(0)),
                _ => {}
            }
        }
        assert!(owned.is_some(), "{target}: {ir}");
        assert_eq!(passed, owned, "{target}: {ir}");
        assert_eq!(releases.last().copied(), owned, "{target}: {ir}");

        if let Some(out) = std::env::var_os("WOSY_RECOMMITTED_RETURN_IR_DIR") {
            let mut witness = ir.replace("@main(", "@wosy_authored_main(");
            for (wrapper, name) in [("retain", "retain_witness"), ("transfer", "transfer_witness")] {
                let symbol = function_with_suffix(&module, name);
                witness.push_str(&format!(
                    "\ndefine i64 @wosy_{wrapper}() {{\nentry:\n  %result = call i64 @{symbol}()\n  ret i64 %result\n}}\n"
                ));
            }
            std::fs::write(std::path::PathBuf::from(out).join(format!("{target}.ll")), witness).unwrap();
        }

        let interrupted = text.replace("    read_amount(&moved);", "    length;");
        let error = emit(&interrupted).expect_err("an interrupted chain lacks a root handoff certificate");
        assert!(error.contains("recommitted aggregate return has no exact root binding"), "{target}: {error}");
    }
}

#[test]
fn structured_owner_factory_joins_grounded_recursive_results_in_caller_storage() {
    let app = source("workbench", "src/structured_owner_factory.w");
    let text = include_str!("fixtures/structured-owner-factory.w");
    for (target, target_layout) in [
        ("native64", ScalarTargetLayout::NATIVE64),
        ("wasm32", ScalarTargetLayout::WASM32),
    ] {
        let emit = |text: &str| {
            let prepared = prepare_scalar_project(
                vec![(declarations(&app, text), Vec::new())], vec![app.clone()],
                ScalarPreparationInputs { target_layout },
            ).unwrap_or_else(|error| panic!("{target}: {:?}", error.diagnostics));
            let build = analyze_scalar_build(&prepared);
            assert!(build.diagnostics().is_empty(), "{target}: {:?}", build.diagnostics());
            emit_scalar_build_project_llvm(&build).map(|module| module.to_text())
        };
        let ir = emit(text).unwrap_or_else(|error| panic!("{target}: {error}"));
        let context = Context::create();
        let buffer = MemoryBuffer::create_from_memory_range_copy(ir.as_bytes(), target);
        let module = context.create_module_from_ir(buffer).unwrap();
        module.verify().unwrap();

        let find = |name| module.get_function(&function_with_suffix(&module, name)).unwrap();
        let (factory, make, alternate) = (find("factory"), find("make_packet"), find("make_alt"));
        let free = module.get_function("__wosy_core_free").unwrap();
        assert_eq!(factory.count_params(), 4, "{target}: {ir}");
        let dest = factory.get_last_param().unwrap();
        let mut arms = Vec::new();
        let mut frame = None;
        let mut released = None;
        let mut result_phis = 0;
        for row in factory.get_basic_blocks().into_iter().flat_map(|block| block.get_instructions()) {
            if row.get_opcode() == InstructionOpcode::Phi && row.to_string().contains(" = phi ptr ") {
                result_phis += 1;
            }
            if row.get_opcode() != InstructionOpcode::Call { continue; }
            let call = CallSiteValue::try_from(row).unwrap();
            let argument = |slot| row.get_operand(slot).unwrap().value().unwrap();
            match call.get_called_fn_value() {
                Some(callee) if [factory, make, alternate].contains(&callee) => {
                    if argument(call.count_arguments() - 1) == dest {
                        arms.push(callee);
                    } else {
                        assert_eq!(callee, make, "{target}: {ir}");
                        assert!(frame.replace(call.try_as_basic_value().basic().unwrap()).is_none());
                    }
                }
                Some(callee) if callee == free => assert!(released.replace(argument(0)).is_none()),
                _ => {}
            }
        }
        assert_eq!(arms.len(), 3, "{target}: {ir}");
        assert!([factory, make, alternate].into_iter().all(|callee| arms.contains(&callee)));
        assert_eq!(result_phis, 2, "{target}: {ir}");
        assert!(frame.is_some());
        assert_eq!(frame, released, "{target}: {ir}");

        if let Some(out) = std::env::var_os("WOSY_STRUCTURED_FACTORY_IR_DIR") {
            let mut witness = ir.replace("@main(", "@wosy_authored_main(");
            for name in ["case_zero", "case_two", "case_three"] {
                let symbol = function_with_suffix(&module, name);
                witness.push_str(&format!(
                    "\ndefine i64 @wosy_{name}() {{\nentry:\n  %result = call i64 @{symbol}()\n  ret i64 %result\n}}\n"
                ));
            }
            std::fs::write(std::path::PathBuf::from(out).join(format!("{target}.ll")), witness).unwrap();
        }

        let ungrounded = text.replace("Packet(u64, u64, bool) factory",
            "Packet(u64) no_seed = fn(length) { no_seed(length) };\nPacket(u64, u64, bool) factory")
            .replace("else { make_alt(length) }", "else { no_seed(length) }");
        let error = emit(&ungrounded).expect_err("an ungrounded leaf cannot discharge a total owner result");
        assert!(error.contains("structured aggregate return is not closed on every result path"), "{target}: {error}");
    }
}

#[test]
fn structured_named_terminal_moves_descriptor_and_keeps_original_backing() {
    let app = source("workbench", "src/structured_named_factory.w");
    let text = include_str!("fixtures/structured-named-factory.w");
    let mutual = text.replace("Packet incoming = factory(depth - 1, length, !choose);",
        "Packet incoming = peer(depth - 1, length, !choose);")
        .replace("u64(u64, bool) drive", "Packet(u64, u64, bool) peer = fn(depth, length, choose) {\n    Packet incoming = factory(depth, length, choose);\n    read_amount(&incoming.length);\n    Packet intermediate = incoming;\n    intermediate.length;\n    Packet selected = intermediate;\n    selected\n};\nu64(u64, bool) drive");
    let updated = text.replace("        selected\n", "        selected.length = 21;\n        selected\n");
    assert_ne!(updated, text);
    for (text, mutual, metadata) in [(text, false, false), (mutual.as_str(), true, false),
        (updated.as_str(), false, true)]
    {
        for (target, target_layout) in [("native64", ScalarTargetLayout::NATIVE64), ("wasm32", ScalarTargetLayout::WASM32)] {
            for project in [false, true] {
                let emit = |text: &str| {
                    let prepared = prepare_scalar_project(vec![(declarations(&app, text), vec![])],
                        vec![app.clone()], ScalarPreparationInputs { target_layout }).unwrap();
                    let build = analyze_scalar_build(&prepared);
                    assert!(build.diagnostics().is_empty(), "{target}: {:?}", build.diagnostics());
                    if project { emit_scalar_build_project_llvm(&build) }
                    else { wosy_compiler::emit_scalar_build_single_module_llvm(&build) }
                        .map(|module| module.to_text())
                };
                let ir = emit(text).unwrap_or_else(|e| panic!("{target}/{project}: {e}"));
                let context = Context::create();
                let module = context.create_module_from_ir(MemoryBuffer::create_from_memory_range_copy(ir.as_bytes(), target)).unwrap();
                module.verify().unwrap();
                let symbol = |name: &str| if project { function_with_suffix(&module, name) } else { name.to_owned() };
                let factory = module.get_function(&symbol("factory")).unwrap();
                let recursive = module.get_function(&symbol(if mutual { "peer" } else { "factory" })).unwrap();
                if mutual {
                    let root_dest = recursive.get_last_param().unwrap();
                    let mut original_backing = None;
                    let mut copies = 0;
                    let mut returned = None;
                    for row in recursive.get_basic_blocks().into_iter().flat_map(|b| b.get_instructions()) {
                        let arg = |slot| row.get_operand(slot).unwrap().value().unwrap();
                        if row.get_opcode() == InstructionOpcode::Return {
                            returned = Some(arg(0));
                        }
                        if row.get_opcode() != InstructionOpcode::Call { continue; }
                        let call = CallSiteValue::try_from(row).unwrap();
                        let Some(callee) = call.get_called_fn_value() else { continue };
                        let name = callee.get_name().to_str().unwrap();
                        assert_ne!(name, "__wosy_core_free", "{target}/{project}: {ir}");
                        if callee == factory {
                            assert_ne!(arg(call.count_arguments()-1), root_dest);
                            assert!(original_backing.replace(call.try_as_basic_value().basic().unwrap()).is_none());
                        } else if name.starts_with("llvm.memmove.") && arg(0) == root_dest {
                            copies += 1;
                        }
                    }
                    assert!(original_backing.is_some() && original_backing == returned && copies == 1, "{target}/{project}: {ir}");
                }
                let alternate = module.get_function(&symbol("make_alt")).unwrap();
                let dest = factory.get_last_param().unwrap();
                let mut received = Vec::new();
                let mut copied = 0;
                let mut metadata_stores = 0;
                let mut released = Vec::new();
                let mut incoming = Vec::new();
                for row in factory.get_basic_blocks().into_iter().flat_map(|b| b.get_instructions()) {
                    if row.get_opcode() == InstructionOpcode::Store
                        && row.get_operand(0).and_then(|operand| operand.value()).filter(|value| value.is_int_value())
                            .and_then(|value| value.into_int_value().get_zero_extended_constant()) == Some(21)
                    {
                        metadata_stores += 1;
                    }
                    if row.get_opcode() == InstructionOpcode::Phi {
                        incoming.extend((0..row.get_num_operands()).filter_map(|n| row.get_operand(n)?.value()));
                    }
                    if row.get_opcode() != InstructionOpcode::Call { continue; }
                    let call = CallSiteValue::try_from(row).unwrap();
                    let Some(callee) = call.get_called_fn_value() else { continue };
                    let arg = |slot| row.get_operand(slot).unwrap().value().unwrap();
                    let name = callee.get_name().to_str().unwrap();
                    if callee == recursive || callee == alternate {
                        assert_ne!(arg(call.count_arguments()-1), dest);
                        received.push(call.try_as_basic_value().basic().unwrap());
                    } else if name.starts_with("llvm.memmove.") && arg(0) == dest {
                        copied += 1;
                    } else if name == "__wosy_core_free" {
                        released.push(arg(0));
                    }
                }
                assert_eq!(metadata_stores, usize::from(metadata), "{target}/{project}: {ir}");
                assert_eq!((received.len(), copied, released.len()), (2, 2, 1), "{target}/{project}: {ir}");
                for base in received {
                    assert!(incoming.contains(&base) && !released.contains(&base), "{target}/{project}: {ir}");
                }
                if let Some(out) = std::env::var_os("WOSY_NAMED_FACTORY_IR_DIR") {
                    let route = match (mutual, metadata, project) {
                        (_, true, true) => "named-metadata-project", (_, true, false) => "named-metadata-single",
                        (true, false, true) => "mutual-project", (true, false, false) => "mutual-single",
                        (false, false, true) => "project", (false, false, false) => "single",
                    };
                    let out = std::path::PathBuf::from(out).join(route);
                    std::fs::create_dir_all(&out).unwrap();
                    let mut witness = ir.replace("@main(", "@wosy_authored_main(");
                    for name in ["case_zero", "case_two", "case_three"] {
                        witness.push_str(&format!("\ndefine i64 @wosy_{name}() {{\nentry:\n  %v = call i64 @{}()\n  ret i64 %v\n}}\n", symbol(name)));
                    }
                    std::fs::write(out.join(format!("{target}.ll")), witness).unwrap();
                }
                if mutual {
                    let indirect_root = text.replace("Packet intermediate = incoming;\n    intermediate.length;",
                        "Packet intermediate = incoming;\n    length;");
                    assert!(emit(&indirect_root).expect_err("alias is not the exact root binding")
                        .contains("aggregate forwarding has no exact root binding"));
                }
                if metadata {
                    let compound = text.replace("selected.length = 21;", "selected.length = length + 1;");
                    assert_ne!(compound, text);
                    assert!(emit(&compound).expect_err("a compound RHS is outside this exact store corridor")
                        .contains("structured aggregate return is not closed on every result path"));
                }
                let unrelated = text.replace("        intermediate.length;", "        frame.length;");
                assert_ne!(unrelated, text);
                assert!(emit(&unrelated).expect_err("another live descriptor cannot certify this observation corridor")
                    .contains("structured aggregate return is not closed on every result path"));
                let uncertified = text.replace("fn(value) { *value }", "fn(value) { u64 copy = *value; copy }");
                assert_ne!(uncertified, text);
                assert!(emit(&uncertified).expect_err("an uncertified call cannot borrow and forward the receipt")
                    .contains("aggregate owning receipt has an unproved checked loan"));
            }
        }
    }
}

#[test]
fn structured_recursive_base_can_transfer_its_branch_local_allocation() {
    let app = source("workbench", "src/structured_local_base.w");
    let original = include_str!("fixtures/structured-named-factory.w");
    let (_, rest) = original.split_once("Packet(u64) make_alt = fn(length) {").unwrap();
    let (local, _) = rest.split_once("\n};").unwrap();
    let before = original.split_once("Packet(u64) make_alt = fn(length) {").unwrap().0;
    let after = rest.split_once("\n};").unwrap().1;
    let text = format!("{before}{after}").replace("Packet selected = make_alt(length); selected", local);
    assert!(!text.contains("make_alt"));
    let all_local = include_str!("fixtures/structured-local-factory.w");
    let bind_result = |source: &str, first_condition: &str| {
        let bound = source.replacen(first_condition, &first_condition.replacen("if", "Packet chosen = if", 1), 1)
            .replacen("\n    }\n};\nu64(u64, bool) drive", "\n    };\n    chosen\n};\nu64(u64, bool) drive", 1);
        assert_eq!(bound.matches("Packet chosen = if").count(), 1);
        assert_eq!(bound.matches("\n    chosen\n").count(), 1);
        bound
    };
    let bound_recursive = bind_result(&text, "    if (depth == 0)");
    let bound_all_local = bind_result(all_local, "    if (choose");
    for (text, all_local, bound) in [(text.as_str(), false, false), (all_local, true, false),
        (bound_recursive.as_str(), false, true), (bound_all_local.as_str(), true, true)]
    {
        for (target, target_layout) in [("native64", ScalarTargetLayout::NATIVE64), ("wasm32", ScalarTargetLayout::WASM32)] {
            for project in [false, true] {
                let emit = |text: &str| {
                    let prepared = prepare_scalar_project(vec![(declarations(&app, text), vec![])],
                        vec![app.clone()], ScalarPreparationInputs { target_layout }).unwrap();
                    let build = analyze_scalar_build(&prepared);
                    assert!(build.diagnostics().is_empty(), "{target}: {:?}", build.diagnostics());
                    if project { emit_scalar_build_project_llvm(&build) }
                    else { wosy_compiler::emit_scalar_build_single_module_llvm(&build) }.map(|module| module.to_text())
                };
                let ir = emit(text).unwrap_or_else(|error| panic!("{target}/{project}: {error}"));
                let context = Context::create();
                let module = context.create_module_from_ir(MemoryBuffer::create_from_memory_range_copy(ir.as_bytes(), target)).unwrap();
                module.verify().unwrap();
                let symbol = |name: &str| if project { function_with_suffix(&module, name) } else { name.to_owned() };
                let factory = module.get_function(&symbol("factory")).unwrap();
                let mut bases = Vec::new();
                let mut released = Vec::new();
                let mut merged = Vec::new();
                let mut committed = 0;
                for row in factory.get_basic_blocks().into_iter().flat_map(|block| block.get_instructions()) {
                    let arg = |slot| row.get_operand(slot).unwrap().value().unwrap();
                    if row.get_opcode() == InstructionOpcode::Phi {
                        merged.extend((0..row.get_num_operands()).filter_map(|n| row.get_operand(n)?.value()));
                    }
                    if row.get_opcode() != InstructionOpcode::Call { continue; }
                    let call = CallSiteValue::try_from(row).unwrap();
                    let Some(callee) = call.get_called_fn_value() else { continue };
                    assert!(!all_local || callee != factory, "all-local case must not use a recursive call");
                    match callee.get_name().to_str().unwrap() {
                        "__wosy_core_alloc" => bases.push(call.try_as_basic_value().basic().unwrap()),
                        "__wosy_core_free" => released.push(arg(0)),
                        name if name.starts_with("llvm.memmove.") && arg(0) == factory.get_last_param().unwrap() => committed += 1,
                        _ => {}
                    }
                }
                assert_eq!(bases.len(), if all_local { 2 } else { 1 }, "{target}/{project}: {ir}");
                assert_eq!(released.len(), if all_local { 0 } else { 1 }, "{target}/{project}: {ir}");
                assert!(bases.iter().all(|base| merged.contains(base) && !released.contains(base)));
                assert_eq!(committed, 2, "{target}/{project}: both named arms write the destination");
                let drive = module.get_function(&symbol("drive")).unwrap();
                let mut received = None;
                let mut caller_frees = Vec::new();
                for row in drive.get_basic_blocks().into_iter().flat_map(|block| block.get_instructions()) {
                    if row.get_opcode() != InstructionOpcode::Call { continue; }
                    let call = CallSiteValue::try_from(row).unwrap();
                    let Some(callee) = call.get_called_fn_value() else { continue };
                    if callee == factory {
                        assert!(received.replace(call.try_as_basic_value().basic().unwrap()).is_none());
                    } else if callee.get_name().to_str().unwrap() == "__wosy_core_free" {
                        caller_frees.push(row.get_operand(0).unwrap().value().unwrap());
                    }
                }
                assert_eq!(caller_frees, [received.expect("caller receives factory result")], "{target}/{project}: {ir}");
                if let Some(out) = std::env::var_os("WOSY_NAMED_FACTORY_IR_DIR") {
                    let route = match (all_local, project) {
                        (true, true) => "all-local-project", (true, false) => "all-local-single",
                        (false, true) => "local-project", (false, false) => "local-single",
                    };
                    let route = if bound { format!("bound-{route}") } else { route.to_owned() };
                    let out = std::path::PathBuf::from(out).join(route);
                    std::fs::create_dir_all(&out).unwrap();
                    let mut witness = ir.replace("@main(", "@wosy_authored_main(");
                    for name in ["case_zero", "case_two", "case_three"] {
                        witness.push_str(&format!("\ndefine i64 @wosy_{name}() {{\nentry:\n  %v = call i64 @{}()\n  ret i64 %v\n}}\n", symbol(name)));
                    }
                    std::fs::write(out.join(format!("{target}.ll")), witness).unwrap();
                }
                if bound {
                    let interrupted = text.replace("\n    chosen\n", "\n    chosen.length;\n    chosen\n");
                    assert_ne!(interrupted, text);
                    assert!(emit(&interrupted).expect_err("the conditional binding must be immediately returned")
                        .contains("structured aggregate return is not closed on every result path"));
                }
                let plain = if all_local {
                    text.replacen("\n        packet\n",
                        "\n        Packet plain = { .data = null; .length = packet.length; };\n        plain\n", 1)
                } else {
                    text.replace(local, &local.replace("    packet",
                        "    Packet plain = { .data = null; .length = packet.length; };\n    plain"))
                };
                assert_ne!(plain, text);
                assert!(emit(&plain).expect_err("plain descriptor cannot claim the branch allocation")
                    .contains("structured aggregate return is not closed on every result path"));
            }
        }
    }
}

#[test]
fn lexical_conditional_owner_choice_releases_selected_original_after_read() {
    let app = source("workbench", "src/local_receipt_choice.w");
    let text = include_str!("fixtures/structured-local-choice.w");
    let consumed = text.replace("u64(u64, bool) drive", "u64(Packet) consume = fn(packet) { packet.length };\nu64(u64, bool) drive")
        .replace("    chosen.length", "    consume(chosen)");
    assert_ne!(consumed, text);
    let recommit = |source: &str, endpoint: &str| source
        .replace("u64(u64, bool) drive",
            "Packet(u64, Packet, bool) first_relay = fn(prefix, packet, toggle) { prefix; toggle; packet };\nPacket(Packet) last_relay = fn(packet) { packet };\nu64(u64, bool) drive")
        .replace(endpoint, &format!("Packet moved = first_relay(depth, chosen, choose);\n    Packet returned = last_relay(moved);\n    {}", endpoint.replace("chosen", "returned")));
    let recommitted_release = recommit(text, "chosen.length");
    let recommitted_consume = recommit(&consumed, "consume(chosen)");
    assert_ne!(recommitted_release, text);
    assert_ne!(recommitted_consume, consumed);
    let coexisting = |source: &str| source
        .replace("u64(u64, bool) drive",
            "Packet(u64) make_neighbor = fn(length) { u8[length] backing; backing[0] = 39; *?u8 data = null; unsafe { data = &?backing[0]; }; Packet packet = { .data = data; .length = length; }; packet };\nu64(u64, bool) drive")
        .replace("    Packet moved = first_relay",
            "    Packet neighbor = make_neighbor(7);\n    neighbor.length;\n    Packet moved = first_relay");
    let sibling_released = coexisting(&recommitted_release);
    let sibling_and_transferred = coexisting(&recommitted_consume);
    assert_ne!(sibling_released, recommitted_release);
    assert_ne!(sibling_and_transferred, recommitted_consume);
    let borrowed_then_consumed = consumed.replace("u64(Packet) consume",
        "u64(*u64) read_selected = fn(view) { *view };\nu64(Packet) consume")
        .replace("    consume(chosen)", "    read_selected(&chosen.length);\n    consume(chosen)");
    assert_ne!(borrowed_then_consumed, consumed);
    let saved_then_consumed = borrowed_then_consumed
        .replace("    read_selected(&chosen.length);\n    consume(chosen)",
            "    u64 observed = read_selected(&chosen.length);\n    consume(chosen) + observed")
        .replace("drive(0, false) }", "if (drive(0, false) == 24) { 12 } else { 0 } }")
        .replace("drive(2, true) }", "if (drive(2, true) == 4) { 2 } else { 0 } }")
        .replace("drive(3, true) }", "if (drive(3, true) == 24) { 12 } else { 0 } }");
    assert_ne!(saved_then_consumed, borrowed_then_consumed);
    let read_recommitted_release = recommitted_release
        .replace("u64(u64, bool) drive", "u64(*Packet) read_selected = fn(view) { (*view).length };\nu64(u64, bool) drive")
        .replace("    returned.length", "    read_selected(&returned)");
    let read_recommitted_consume = recommit(&saved_then_consumed, "u64 observed = read_selected(&chosen.length)")
        .replace("consume(chosen) + observed", "consume(returned) + observed");
    assert_ne!(read_recommitted_release, recommitted_release);
    assert_ne!(read_recommitted_consume, saved_then_consumed);
    let interleaved_release = read_recommitted_release
        .replace("    Packet moved = first_relay", "    u64 prior = read_selected(&chosen);\n    Packet moved = first_relay")
        .replace("    read_selected(&returned)", "    u64 subsequent = read_selected(&returned);\n    prior + subsequent")
        .replace("drive(0, false) }", "if (drive(0, false) == 24) { 12 } else { 0 } }")
        .replace("drive(2, true) }", "if (drive(2, true) == 4) { 2 } else { 0 } }")
        .replace("drive(3, true) }", "if (drive(3, true) == 24) { 12 } else { 0 } }");
    let interleaved_consume = read_recommitted_consume
        .replace("    Packet moved = first_relay", "    u64 prior = read_selected(&chosen.length);\n    Packet moved = first_relay")
        .replace("consume(returned) + observed", "consume(returned) + (prior + observed)")
        .replace("== 24", "== 36").replace("== 4) { 2", "== 6) { 2");
    assert_ne!(interleaved_release, read_recommitted_release);
    assert_ne!(interleaved_consume, read_recommitted_consume);
    let forwarding = |consume: &str| saved_then_consumed.replace(
        "u64(Packet) consume = fn(packet) { packet.length };",
        &format!("u64(Packet) terminal = fn(packet) {{ packet.length }};\n\
                  u64(Packet) forward = fn(packet) {{ terminal(packet) }};\n\
                  u64(Packet) consume = fn(packet) {{ {consume} }};"),
    );
    let through_calls = forwarding("forward(packet)");
    let through_arms = forwarding("if (packet.length == 2) { terminal(packet) } else { forward(packet) }");
    assert_ne!(through_calls, saved_then_consumed);
    assert_ne!(through_arms, saved_then_consumed);
    let middle_slot = saved_then_consumed.replace(
        "u64(Packet) consume = fn(packet) { packet.length };",
        "u64(u64, Packet, u64) terminal = fn(left, packet, right) { packet.length + left + right };\n\
         u64(u64, Packet, u64) forward = fn(left, packet, right) { terminal(left, packet, right) };\n\
         u64(u64, Packet, u64) consume = fn(left, packet, right) { forward(left, packet, right) };",
    ).replace("consume(chosen) + observed", "consume(1, chosen, 2) + observed")
        .replace("== 24", "== 27").replace("== 4) { 2", "== 7) { 2");
    assert_ne!(middle_slot, saved_then_consumed);
    let nested_relays = middle_slot.replace(
        "u64(u64, Packet, u64) consume = fn(left, packet, right) { forward(left, packet, right) };",
        "u64(u64, Packet, u64) dispatch = fn(left, packet, right) {\n\
            if (left == 1) { terminal(left, packet, right) } else { forward(left, packet, right) }\n\
         };\n\
         u64(Packet) consume = fn(packet) {\n\
            if (packet.length == 2) { dispatch(1, packet, 2) } else { dispatch(2, packet, 1) }\n\
         };",
    ).replace("consume(1, chosen, 2)", "consume(chosen)");
    assert_ne!(nested_relays, middle_slot);
    let multiple_then_consumed = saved_then_consumed
        .replace("u64(*u64) read_selected", "u64(*Packet) read_whole = fn(view) { (*view).length };\nu64(*u64) read_selected")
        .replace("    u64 observed = read_selected", "    read_whole(&chosen);\n    u64 observed = read_selected");
    assert_ne!(multiple_then_consumed, saved_then_consumed);
    let saved_then_released = multiple_then_consumed
        .replace("u64(Packet) consume = fn(packet) { packet.length };\n", "")
        .replace("consume(chosen) + observed", "observed")
        .replace("if (drive(0, false) == 24) { 12 } else { 0 }", "drive(0, false)")
        .replace("if (drive(2, true) == 4) { 2 } else { 0 }", "drive(2, true)")
        .replace("if (drive(3, true) == 24) { 12 } else { 0 }", "drive(3, true)");
    assert_ne!(saved_then_released, multiple_then_consumed);
    let computed_then_released = saved_then_released
        .replace("    observed\n", "    (observed + 3) * 2\n")
        .replace("drive(0, false) }", "if (drive(0, false) == 30) { 12 } else { 0 } }")
        .replace("drive(2, true) }", "if (drive(2, true) == 10) { 2 } else { 0 } }")
        .replace("drive(3, true) }", "if (drive(3, true) == 30) { 12 } else { 0 } }");
    assert_ne!(computed_then_released, saved_then_released);
    let updated_then_consumed = saved_then_consumed
        .replace("    u64 observed = read_selected", "    chosen.length = 5;\n    u64 observed = read_selected")
        .replace("== 24", "== 10").replace("== 4) { 2", "== 10) { 2");
    assert_ne!(updated_then_consumed, saved_then_consumed);
    let temporary = consumed.replace("    Packet chosen = if", "    consume(if")
        .replace("else { make_long(2) };\n    consume(chosen)", "else { make_long(2) })");
    assert_ne!(temporary, consumed);
    assert!(!temporary.contains("chosen"));
    let borrowed = text.replace("u64(u64, bool) drive", "u64(*u64) read_selected = fn(view) { *view };\nu64(u64, bool) drive")
        .replace("    chosen.length", "    read_selected(&chosen.length)");
    let named_borrow = text.replace("u64(u64, bool) drive", "u64(*Packet) read_selected = fn(view) { (*view).length };\nu64(u64, bool) drive")
        .replace("    chosen.length", "    *Packet view = &chosen;\n    read_selected(view)");
    assert_ne!(borrowed, text);
    assert_ne!(named_borrow, text);
    let nested = |source: &str| {
        let (_, long) = source.split_once("Packet(u64) make_long = fn(length) {").unwrap();
        let (body, _) = long.split_once("\n};").unwrap();
        let additional = format!("Packet(u64) make_later = fn(length) {{{body}\n}};\nu64(u64, bool) drive");
        let changed = source.replacen("u64(u64, bool) drive", &additional, 1).replacen(
            "else { make_long(2) }", "else { if (depth == 0) { make_long(2) } else { make_later(2) } }", 1);
        assert_eq!(changed.matches("make_later").count(), 2);
        changed
    };
    let nested_temporary = nested(&temporary);
    let nested_borrow = nested(&named_borrow);
    for (text, use_site) in [(text, "choice"), (consumed.as_str(), "consume"),
        (temporary.as_str(), "consume-temporary"), (borrowed.as_str(), "borrow"),
        (recommitted_release.as_str(), "release-after-recommit"),
        (recommitted_consume.as_str(), "consume-after-recommit"),
        (sibling_released.as_str(), "release-with-sibling-recommit"),
        (sibling_and_transferred.as_str(), "consume-with-sibling-recommit"),
        (read_recommitted_release.as_str(), "release-after-read-recommit"),
        (read_recommitted_consume.as_str(), "consume-after-read-recommit"),
        (interleaved_release.as_str(), "release-interleaved-read-recommit"),
        (interleaved_consume.as_str(), "consume-interleaved-read-recommit"),
        (borrowed_then_consumed.as_str(), "consume-after-borrow"),
        (saved_then_consumed.as_str(), "consume-after-saved-borrow"),
        (through_calls.as_str(), "consume-after-forwarded-borrow"),
        (through_arms.as_str(), "consume-after-exhaustive-borrow"),
        (middle_slot.as_str(), "consume-through-middle-slot"),
        (nested_relays.as_str(), "consume-through-exhaustive-relays"),
        (multiple_then_consumed.as_str(), "consume-after-multiple-borrows"),
        (saved_then_released.as_str(), "release-after-saved-borrows"),
        (computed_then_released.as_str(), "release-after-scalar-continuation"),
        (updated_then_consumed.as_str(), "consume-after-metadata-write"),
        (named_borrow.as_str(), "named-borrow"), (nested_temporary.as_str(), "nested-consume-temporary"),
        (nested_borrow.as_str(), "nested-named-borrow")]
    {
        let nested = use_site.starts_with("nested-");
        let base_use = use_site.strip_prefix("nested-").unwrap_or(use_site);
        let consuming = base_use.starts_with("consume");
        let recommitting = base_use.ends_with("-recommit");
        let sibling = base_use.contains("-with-sibling-");
        let reading_recommit = base_use.ends_with("-read-recommit");
        let interleaved = base_use.contains("-interleaved-");
        let releases_saved = matches!(base_use, "release-after-saved-borrows" | "release-after-scalar-continuation");
        let multiple = releases_saved || base_use == "consume-after-multiple-borrows";
        let updates_metadata = base_use == "consume-after-metadata-write";
        let exhaustive = base_use == "consume-after-exhaustive-borrow";
        let middle_slot = base_use == "consume-through-middle-slot";
        let nested_relays = base_use == "consume-through-exhaustive-relays";
        let forwarding = exhaustive || middle_slot || nested_relays || base_use == "consume-after-forwarded-borrow";
        let borrowing = reading_recommit || multiple || updates_metadata || forwarding
            || matches!(base_use, "borrow" | "named-borrow" | "consume-after-borrow" | "consume-after-saved-borrow");
        for (target, target_layout) in [("native64", ScalarTargetLayout::NATIVE64), ("wasm32", ScalarTargetLayout::WASM32)] {
            for project in [false, true] {
                let emit = |text: &str| {
                    let prepared = prepare_scalar_project(vec![(declarations(&app, text), vec![])],
                        vec![app.clone()], ScalarPreparationInputs { target_layout }).unwrap();
                    let build = analyze_scalar_build(&prepared);
                    assert!(build.diagnostics().is_empty(), "{target}: {:?}", build.diagnostics());
                    if project { emit_scalar_build_project_llvm(&build) }
                    else { wosy_compiler::emit_scalar_build_single_module_llvm(&build) }.map(|module| module.to_text())
                };
                let ir = emit(text).unwrap_or_else(|error| panic!("{use_site}/{target}/{project}: {error}"));
                let context = Context::create();
                let module = context.create_module_from_ir(MemoryBuffer::create_from_memory_range_copy(ir.as_bytes(), target)).unwrap();
                module.verify().unwrap();
                let symbol = |name: &str| if project { function_with_suffix(&module, name) } else { name.to_owned() };
                let find = |name| module.get_function(&symbol(name)).unwrap();
                let drive = find("drive");
                assert_eq!(drive.count_params(), 2, "lexical choice introduces no hidden result ABI");
                let mut producers = vec![find("make_short"), find("make_long")];
                if nested { producers.push(find("make_later")); }
                let consumer = consuming.then(|| find("consume"));
                let sibling_factory = sibling.then(|| find("make_neighbor"));
                let mut sibling_receipt = None;
                let relays = recommitting.then(|| [find("first_relay"), find("last_relay")]);
                if let Some([first, last]) = relays {
                    for (relay, slot) in [(first, 1), (last, 0)] {
                        let rows: Vec<_> = relay.get_basic_blocks().into_iter()
                            .flat_map(|block| block.get_instructions()).collect();
                        let [returned] = rows.as_slice() else { panic!("{ir}") };
                        assert_eq!(returned.get_opcode(), InstructionOpcode::Return);
                        assert_eq!(returned.get_operand(0).unwrap().value(), relay.get_nth_param(slot));
                    }
                }
                let mut recommitted = Vec::new();
                let reader = borrowing.then(|| find("read_selected"));
                let whole_reader = multiple.then(|| find("read_whole"));
                let mut expected_readers = whole_reader.into_iter()
                    .chain(reader.filter(|_| interleaved)).chain(reader);
                let mut reader_seen = false;
                let mut transfers = Vec::new();
                let mut writes = Vec::new();
                let mut merges = Vec::new();
                let mut reads = Vec::new();
                let mut released = Vec::new();
                for row in drive.get_basic_blocks().into_iter().flat_map(|block| block.get_instructions()) {
                    let arg = |slot| row.get_operand(slot).unwrap().value().unwrap();
                    if row.get_opcode() == InstructionOpcode::Phi && row.to_string().contains("phi ptr") {
                        let incoming = (0..row.get_num_operands()).filter_map(|n| row.get_operand(n)?.value()).collect::<Vec<_>>();
                        merges.push((inkwell::values::PhiValue::try_from(row).unwrap().as_basic_value(), incoming));
                    }
                    if row.get_opcode() == InstructionOpcode::GetElementPtr {
                        reads.push(arg(0));
                    }
                    if row.get_opcode() != InstructionOpcode::Call { continue; }
                    let call = CallSiteValue::try_from(row).unwrap();
                    let Some(callee) = call.get_called_fn_value() else { continue };
                    if producers.contains(&callee) {
                        writes.push((callee, arg(call.count_arguments() - 1), call.try_as_basic_value().basic().unwrap()));
                    } else if Some(callee) == sibling_factory {
                        let base = call.try_as_basic_value().basic().unwrap();
                        assert!(sibling_receipt.replace(base).is_none());
                        assert!(recommitted.is_empty(), "sibling is born before receipt migration");
                    } else if relays.is_some_and(|relays| relays.contains(&callee)) {
                        assert_eq!(call.count_arguments(), if Some(callee) == relays.map(|[first, _]| first) { 3 } else { 1 });
                        recommitted.push(callee);
                    } else if Some(callee) == consumer {
                        assert!(!recommitting || recommitted.len() == 2, "both identity returns precede terminal transfer");
                        assert!(!borrowing || reader_seen, "the certified read precedes sole owner transfer");
                        if middle_slot {
                            assert_eq!(call.count_arguments(), 4);
                            assert_eq!(arg(0).into_int_value().get_zero_extended_constant(), Some(1));
                            assert_eq!(arg(2).into_int_value().get_zero_extended_constant(), Some(2));
                            transfers.push((arg(1), arg(3)));
                        } else {
                            assert_eq!(call.count_arguments(), 2);
                            transfers.push((arg(0), arg(1)));
                        }
                    } else if Some(callee) == reader || Some(callee) == whole_reader {
                        assert_eq!(expected_readers.next(), Some(callee), "every authored loan is observed in order");
                        if recommitting {
                            let expected_relays = if interleaved && !reader_seen { 0 } else { 2 };
                            assert_eq!(recommitted.len(), expected_relays, "each read must be on its exact side of the relay chain");
                        }
                        assert_eq!(call.count_arguments(), 1, "a reader never receives the owning sidecar");
                        assert_eq!(arg(0).into_int_value().get_type().get_bit_width(), target_layout.pointer_size as u32 * 8);
                        reader_seen = Some(callee) == reader;
                    } else if callee.get_name().to_str().unwrap() == "__wosy_core_free" {
                        assert!(!recommitting || recommitted.len() == 2, "both identity returns precede lexical release");
                        assert!(!borrowing || reader_seen, "the shared loan must end before release");
                        released.push(arg(0));
                    }
                }
                assert!(recommitted.iter().copied().eq(relays.into_iter().flatten()), "emit the exact relay chain once");
                assert!(expected_readers.next().is_none(), "all authored observations must be emitted");
                assert_eq!(writes.len(), producers.len(), "one call per exact leaf");
                let descriptor = writes[0].1;
                assert!(writes.iter().all(|&(_, slot, _)| slot == descriptor), "all arms share descriptor storage");
                assert!(producers.iter().all(|producer| writes.iter().any(|(callee, _, _)| callee == producer)));
                let selected = if consuming { transfers.first().expect("owner transfer").1 }
                    else { *released.last().expect("lexical release") };
                assert_ne!(selected, descriptor, "descriptor and backing must remain distinct");
                assert_eq!(sibling_receipt.is_some(), sibling);
                assert_ne!(sibling_receipt, Some(selected), "sibling and selected receipts are independent");
                assert_eq!(merges.len(), producers.len() - 1);
                let mut pending = vec![selected];
                let mut leaf_bases = Vec::new();
                let mut seen = Vec::new();
                while let Some(value) = pending.pop() {
                    assert!(!seen.contains(&value), "backing phi tree must be acyclic");
                    seen.push(value);
                    if let Some((_, incoming)) = merges.iter().find(|(phi, _)| *phi == value) {
                        assert_eq!(incoming.len(), 2);
                        pending.extend(incoming.iter().copied());
                    } else {
                        leaf_bases.push(value);
                    }
                }
                assert_eq!(leaf_bases.len(), writes.len());
                assert!(writes.iter().all(|&(_, _, base)| leaf_bases.contains(&base)));
                if let Some(consumer) = consumer {
                    if borrowing {
                        assert!(reader_seen);
                        if !recommitting {
                            assert!(reads.contains(&descriptor), "the read must borrow the descriptor handed to consumer");
                        }
                    }
                    assert!(released.iter().copied().eq(sibling_receipt),
                        "the caller releases only the coexisting receipt when the selected owner transfers");
                    if recommitting {
                        assert_eq!(transfers.len(), 1, "only the final receipt may transfer");
                        assert_ne!(transfers[0].0, selected, "the returned descriptor remains distinct from its backing");
                    } else {
                        assert_eq!(transfers, [(descriptor, selected)], "pass the descriptor and its independent backing");
                    }
                    let frees = |function: inkwell::values::FunctionValue<'_>| function.get_basic_blocks()
                        .into_iter().flat_map(|block| block.get_instructions()).filter(|row| {
                            let Ok(call) = CallSiteValue::try_from(*row) else { return false };
                            let Some(target) = call.get_called_fn_value() else { return false };
                            if target.get_name().to_str().unwrap() != "__wosy_core_free" { return false; }
                            assert_eq!(row.get_operand(0).unwrap().value().unwrap(), function.get_last_param().unwrap());
                            true
                        }).count();
                    if forwarding {
                        let middle = find("forward");
                        let terminal = find("terminal");
                        let dispatch = nested_relays.then(|| find("dispatch"));
                        let forwarded = |function: inkwell::values::FunctionValue<'_>| {
                            let mut targets = Vec::new();
                            for row in function.get_basic_blocks().into_iter().flat_map(|block| block.get_instructions()) {
                                if row.get_opcode() != InstructionOpcode::Call { continue; }
                                let call = CallSiteValue::try_from(row).unwrap();
                                let target = call.get_called_fn_value().unwrap();
                                if target == middle || target == terminal || Some(target) == dispatch {
                                    assert_eq!(call.count_arguments(), if middle_slot || nested_relays { 4 } else { 2 });
                                    assert_eq!(row.get_operand(call.count_arguments() - 1).unwrap().value().unwrap(),
                                        function.get_last_param().unwrap());
                                    targets.push(if target == middle { "forward" }
                                        else if target == terminal { "terminal" } else { "dispatch" });
                                } else {
                                    assert!(matches!(target.get_name().to_str().unwrap(),
                                        "llvm.memmove.p0.p0.i32" | "llvm.memmove.p0.p0.i64"));
                                }
                            }
                            targets
                        };
                        assert_eq!(frees(consumer), 0);
                        assert_eq!(frees(middle), 0);
                        assert_eq!(frees(terminal), 1);
                        assert_eq!(forwarded(middle), ["terminal"]);
                        if let Some(dispatch) = dispatch {
                            assert_eq!(frees(dispatch), 0);
                            assert_eq!(forwarded(consumer), ["dispatch", "dispatch"]);
                            assert_eq!(forwarded(dispatch), ["terminal", "forward"]);
                        } else {
                            assert_eq!(forwarded(consumer), if exhaustive { vec!["terminal", "forward"] } else { vec!["forward"] });
                        }
                    } else {
                        assert_eq!(frees(consumer), 1);
                    }
                } else {
                    if borrowing {
                        assert!(reader_seen);
                        if base_use == "borrow" || releases_saved {
                            assert!(reads.contains(&descriptor), "field loan must borrow from the joined descriptor slot");
                        }
                    } else if recommitting {
                        assert!(!reads.is_empty(), "observe the recommitted descriptor");
                    } else {
                        assert!(reads.contains(&descriptor), "final field observation must use that slot");
                    }
                    let ordered = sibling_receipt.into_iter().chain([selected]);
                    assert!(released.iter().copied().eq(ordered), "release original backings in reverse birth order");
                }
                if let Some(out) = std::env::var_os("WOSY_NAMED_FACTORY_IR_DIR") {
                    let route = if project { "project" } else { "single" };
                    let out = std::path::PathBuf::from(out).join(format!("lexical-{use_site}-{route}"));
                    std::fs::create_dir_all(&out).unwrap();
                    let mut witness = ir.replace("@main(", "@wosy_authored_main(");
                    for name in ["case_zero", "case_two", "case_three"] {
                        witness.push_str(&format!("\ndefine i64 @wosy_{name}() {{\nentry:\n  %v = call i64 @{}()\n  ret i64 %v\n}}\n", symbol(name)));
                    }
                    std::fs::write(out.join(format!("{target}.ll")), witness).unwrap();
                }
                if consuming {
                    if borrowing {
                        let uncertified = text.replace("{ *view }", "{ u64 observed = *view; observed }");
                        assert_ne!(uncertified, text);
                        assert!(emit(&uncertified).expect_err("the pre-transfer loan needs exact nonescape authority")
                            .contains("aggregate owning call has no proved endpoint"));
                    }
                    if middle_slot {
                        let opaque = text.replace("u64(u64, Packet, u64) terminal",
                            "u64(u64) companion = fn(value) { value };\nu64(u64, Packet, u64) terminal")
                            .replace("consume(1, chosen, 2)", "consume(companion(1), chosen, 2)");
                        assert_ne!(opaque, text);
                        assert!(emit(&opaque).expect_err("nontrivial companion effects are outside the transfer corridor")
                            .contains("aggregate owning call has no proved endpoint"));
                    }
                    if updates_metadata {
                        let wrong_target = text.replace("chosen.length = 5;", "chosen.data = null;");
                        assert_ne!(wrong_target, text);
                        assert!(emit(&wrong_target).expect_err("backing address rewrites cannot preserve the owning corridor")
                            .contains("aggregate owning call has no proved endpoint"));
                    }
                    if base_use == "consume-after-saved-borrow" {
                        let wrong_suffix = text.replace("consume(chosen) + observed", "consume(chosen) - observed");
                        assert_ne!(wrong_suffix, text);
                        assert!(emit(&wrong_suffix).expect_err("only the exact ordinary scalar suffix is in this corridor")
                            .contains("aggregate owning call has no proved endpoint"));
                    }
                    if multiple {
                        let invalid_first = text.replace("{ (*view).length }", "{ u64 local = (*view).length; local }");
                        assert_ne!(invalid_first, text);
                        assert!(emit(&invalid_first).expect_err("all pre-transfer loans require exact authority")
                            .contains("aggregate owning call has no proved endpoint"));
                        let excess = text.replace("    read_whole(&chosen);",
                            &"    read_whole(&chosen);\n".repeat(3));
                        assert_ne!(excess, text);
                        assert!(emit(&excess).expect_err("four observations exceed the lexical corridor budget")
                            .contains("aggregate owning call has no proved endpoint"));
                    }
                    if use_site == "consume" {
                        let cycle = text.replace("u64(Packet) consume = fn(packet) { packet.length };",
                            "u64(Packet) terminal = fn(packet) { consume(packet) };\nu64(Packet) consume = fn(packet) { terminal(packet) };");
                        assert_ne!(cycle, text);
                        assert!(emit(&cycle).expect_err("the joined owner must ground in a terminal release")
                            .contains("aggregate owning formal transfer does not reach a proved release"));
                    }
                } else if borrowing {
                    if releases_saved {
                        let result = if base_use == "release-after-scalar-continuation" {
                            "    (observed + 3) * 2\n"
                        } else { "    observed\n" };
                        let wrong = text.replace(result, "    chosen.length + observed\n");
                        assert_ne!(wrong, text);
                        assert!(emit(&wrong).expect_err("only the exact saved scalar survives the selected owner")
                            .contains("aggregate owning call has no proved endpoint"));
                    }
                    let uncertified = text.replace("{ *view }", "{ u64 seen = *view; seen }")
                        .replace("{ (*view).length }", "{ u64 seen = (*view).length; seen }");
                    assert_ne!(uncertified, text);
                    assert!(emit(&uncertified).expect_err("only a certified noescape reader may borrow the joined owner")
                        .contains("aggregate owning call has no proved endpoint"));
                } else {
                    let holder = if recommitting { "returned" } else { "chosen" };
                    let extra = text.replace(&format!("    {holder}.length\n"),
                        &format!("    u64 observed = {holder}.length;\n    observed\n"));
                    assert_ne!(extra, text);
                    assert!(emit(&extra).expect_err("the extra binding is outside the exact observation-and-release corridor")
                        .contains("aggregate owning call has no proved endpoint"));
                    fn choice_tree(low: u64, high: u64) -> String {
                        if high - low == 1 { return "make_long(2)".to_owned(); }
                        let split = (low + high) / 2;
                        let low = choice_tree(low, split);
                        let high = choice_tree(split, high);
                        format!("if (depth < {split}) {{ {low} }} else {{ {high} }}")
                    }
                    if !recommitting {
                        let many = text.replace("else { make_long(2) }", &format!("else {{ {} }}", choice_tree(0, 8)));
                        assert_eq!(many.matches("make_long(2)").count(), 8);
                        assert!(emit(&many).expect_err("nine owning leaves exceed the closed proof bound")
                            .contains("aggregate owning call has no proved endpoint"));
                    }
                }
            }
        }
    }
}

#[test]
fn d0024_recursive_ownership_preserves_live_holders_and_carried_owner() {
    let app = source("workbench", "src/d0024_recursive_ownership.w");
    let text = include_str!("fixtures/d0024-recursive-ownership.w");
    for (target, target_layout) in [
        ("native64", ScalarTargetLayout::NATIVE64),
        ("wasm32", ScalarTargetLayout::WASM32),
    ] {
        let prepared = prepare_scalar_project(
            vec![(declarations(&app, text), Vec::new())], vec![app.clone()],
            ScalarPreparationInputs { target_layout },
        ).unwrap_or_else(|error| panic!("{target}: {:?}", error.diagnostics));
        let build = analyze_scalar_build(&prepared);
        assert!(build.diagnostics().is_empty(), "{target}: {:?}", build.diagnostics());
        let ir = emit_scalar_build_project_llvm(&build).unwrap_or_else(|error| panic!("{target}: {error}")).to_text();
        let context = Context::create();
        let buffer = MemoryBuffer::create_from_memory_range_copy(ir.as_bytes(), target);
        let module = context.create_module_from_ir(buffer).unwrap();
        module.verify().unwrap();
        let frame = function_with_suffix(&module, "frame");
        let reader = function_with_suffix(&module, "read_selected");
        let carry = function_with_suffix(&module, "carry");
        let witness = function_with_suffix(&module, "carry_witness");
        let frame_calls = direct_calls(&module, &frame);
        assert!(frame_calls.contains(&frame) && frame_calls.contains(&reader), "{target}: {ir}");
        let frame_fn = module.get_function(&frame).unwrap();
        let allocation_call = frame_fn.get_basic_blocks().into_iter()
            .flat_map(|block| block.get_instructions())
            .filter(|row| row.get_opcode() == InstructionOpcode::Call)
            .map(|row| CallSiteValue::try_from(row).unwrap())
            .find(|call| call.get_called_fn_value().unwrap().get_name().to_bytes() == b"__wosy_core_alloc")
            .expect("frame allocation");
        let original_frame = allocation_call.try_as_basic_value().basic().unwrap();
        let mut checked_order = false;
        for block in frame_fn.get_basic_blocks() {
            let mut stage = 0;
            for row in block.get_instructions().filter(|row| row.get_opcode() == InstructionOpcode::Call) {
                let call = CallSiteValue::try_from(row).unwrap();
                let callee = call.get_called_fn_value().unwrap();
                match callee.get_name().to_str().unwrap() {
                    name if name == frame => { assert_eq!(stage, 0, "{target}: {ir}"); stage = 1; }
                    name if name == reader => { assert_eq!(stage, 1, "{target}: {ir}"); stage = 2; }
                    "__wosy_core_free" => {
                        assert_eq!(stage, 2, "{target}: {ir}");
                        assert_eq!(row.get_operand(0).unwrap().value(), Some(original_frame), "{target}: {ir}");
                        checked_order = true;
                    }
                    _ => {}
                }
            }
        }
        assert!(checked_order, "{target}: {ir}");
        let owned_calls = |name: &str, runtime: &str| direct_calls(&module, name).iter()
            .filter(|call| call.as_str() == runtime).count();
        for runtime in ["__wosy_core_alloc", "__wosy_core_free"] {
            assert_eq!(owned_calls(&frame, runtime), 1, "{target}: {ir}");
            assert_eq!(owned_calls(&carry, runtime), 0, "{target}: {ir}");
            assert_eq!(owned_calls(&witness, runtime), 1, "{target}: {ir}");
        }
        assert!(direct_calls(&module, &carry).contains(&carry), "{target}: {ir}");
        assert!(direct_calls(&module, &witness).contains(&carry), "{target}: {ir}");

        if let Some(out) = std::env::var_os("WOSY_D0024_OWNERSHIP_IR_DIR") {
            let mut ir = ir.replace("@main(", "@wosy_authored_main(");
            for name in ["frame_zero", "frame_two", "frame_three", "carry_zero", "carry_two", "carry_three"] {
                let symbol = function_with_suffix(&module, name);
                ir.push_str(&format!(
                    "\ndefine i8 @wosy_{name}() {{\nentry:\n  %result = call i8 @{symbol}()\n  ret i8 %result\n}}\n"
                ));
            }
            std::fs::write(std::path::PathBuf::from(out).join(format!("{target}.ll")), ir).unwrap();
        }
    }
}

#[test]
fn selected_holder_ends_before_two_independent_owner_recommits() {
    let app = source("workbench", "src/paired_array_handoff.w");
    let text = include_str!("fixtures/paired-array-handoff.w");
    for (target, target_layout) in [
        ("native64", ScalarTargetLayout::NATIVE64),
        ("wasm32", ScalarTargetLayout::WASM32),
    ] {
        let prepared = prepare_scalar_project(
            vec![(declarations(&app, text), Vec::new())], vec![app.clone()],
            ScalarPreparationInputs { target_layout },
        ).unwrap_or_else(|error| panic!("{target}: {:?}", error.diagnostics));
        let build = analyze_scalar_build(&prepared);
        assert!(build.diagnostics().is_empty(), "{target}: {:?}", build.diagnostics());
        let ir = emit_scalar_build_project_llvm(&build).unwrap_or_else(|error| panic!("{target}: {error}")).to_text();
        let context = Context::create();
        let buffer = MemoryBuffer::create_from_memory_range_copy(ir.as_bytes(), target);
        let module = context.create_module_from_ir(buffer).unwrap();
        module.verify().unwrap();

        let drive = function_with_suffix(&module, "drive");
        let descend = function_with_suffix(&module, "descend");
        let reader = function_with_suffix(&module, "read_selected");
        let carry = function_with_suffix(&module, "carry");
        let calls = direct_calls(&module, &drive);
        let selected: Vec<_> = calls.iter().filter(|name| [descend.as_str(), reader.as_str(), carry.as_str()]
            .contains(&name.as_str())).map(String::as_str).collect();
        assert_eq!(selected, [descend.as_str(), reader.as_str(), carry.as_str(), carry.as_str()], "{target}: {ir}");
        for runtime in ["__wosy_core_alloc", "__wosy_core_free"] {
            assert_eq!(calls.iter().filter(|name| name.as_str() == runtime).count(), 2, "{target}: {ir}");
            assert!(!direct_calls(&module, &carry).iter().any(|name| name == runtime));
        }
        assert!(direct_calls(&module, &descend).contains(&descend), "{target}: {ir}");
        assert!(direct_calls(&module, &carry).contains(&carry), "{target}: {ir}");

        if let Some(out) = std::env::var_os("WOSY_PAIRED_HANDOFF_IR_DIR") {
            let mut ir = ir.replace("@main(", "@wosy_authored_main(");
            for name in ["choose_left", "choose_right"] {
                let symbol = function_with_suffix(&module, name);
                ir.push_str(&format!(
                    "\ndefine i8 @wosy_{name}() {{\nentry:\n  %result = call i8 @{symbol}()\n  ret i8 %result\n}}\n"
                ));
            }
            std::fs::write(std::path::PathBuf::from(out).join(format!("{target}.ll")), ir).unwrap();
        }
    }
}

#[test]
fn projected_recursive_holder_retires_only_the_replaced_lane() {
    let app = source("workbench", "src/projected_recursive_holder.w");
    let text = include_str!("fixtures/projected-recursive-holder.w");
    for (target, target_layout) in [
        ("native64", ScalarTargetLayout::NATIVE64),
        ("wasm32", ScalarTargetLayout::WASM32),
    ] {
        let prepared = prepare_scalar_project(
            vec![(declarations(&app, text), Vec::new())], vec![app.clone()],
            ScalarPreparationInputs { target_layout },
        ).unwrap_or_else(|error| panic!("{target}: {:?}", error.diagnostics));
        let build = analyze_scalar_build(&prepared);
        assert!(build.diagnostics().is_empty(), "{target}: {:?}", build.diagnostics());
        let ir = emit_scalar_build_project_llvm(&build).unwrap_or_else(|error| panic!("{target}: {error}")).to_text();
        let context = Context::create();
        let buffer = MemoryBuffer::create_from_memory_range_copy(ir.as_bytes(), target);
        let module = context.create_module_from_ir(buffer).unwrap();
        module.verify().unwrap();
        let frame = function_with_suffix(&module, "frame");
        let reader = function_with_suffix(&module, "read_selected");
        let calls = direct_calls(&module, &frame);
        let selected: Vec<_> = calls.iter().map(String::as_str).filter(|name|
            [frame.as_str(), reader.as_str(), "__wosy_core_alloc", "__wosy_core_free"].contains(name)
        ).collect();
        assert_eq!(selected, ["__wosy_core_alloc", frame.as_str(), reader.as_str(),
            reader.as_str(), "__wosy_core_free"], "{target}: {ir}");

        if let Some(out) = std::env::var_os("WOSY_PROJECTED_HOLDER_IR_DIR") {
            let mut ir = ir.replace("@main(", "@wosy_authored_main(");
            for name in ["frame_zero", "frame_two", "frame_three"] {
                let symbol = function_with_suffix(&module, name);
                ir.push_str(&format!(
                    "\ndefine i8 @wosy_{name}() {{\nentry:\n  %result = call i8 @{symbol}()\n  ret i8 %result\n}}\n"
                ));
            }
            std::fs::write(std::path::PathBuf::from(out).join(format!("{target}.ll")), ir).unwrap();
        }
    }
}

#[test]
fn recursive_fallible_owner_keeps_successful_ancestors_until_their_actual_exit() {
    let app = source("workbench", "src/recursive_fallible_owner.w");
    let text = include_str!("fixtures/recursive-fallible-owner.w");
    for (target, target_layout) in [
        ("native64", ScalarTargetLayout::NATIVE64),
        ("wasm32", ScalarTargetLayout::WASM32),
    ] {
        let prepared = prepare_scalar_project(
            vec![(declarations(&app, text), Vec::new())], vec![app.clone()],
            ScalarPreparationInputs { target_layout },
        ).unwrap_or_else(|error| panic!("{target}: {:?}", error.diagnostics));
        let build = analyze_scalar_build(&prepared);
        assert!(build.diagnostics().is_empty(), "{target}: {:?}", build.diagnostics());
        let ir = emit_scalar_build_project_llvm(&build).unwrap_or_else(|error| panic!("{target}: {error}")).to_text();
        let context = Context::create();
        let buffer = MemoryBuffer::create_from_memory_range_copy(ir.as_bytes(), target);
        let module = context.create_module_from_ir(buffer).unwrap();
        module.verify().unwrap();
        let frame = module.get_function(&function_with_suffix(&module, "frame")).unwrap();
        let alloc = module.get_function("__wosy_core_alloc").unwrap();
        let free = module.get_function("__wosy_core_free").unwrap();
        let mut allocation = None;
        let mut release = None;
        let mut recurses = false;
        for block in frame.get_basic_blocks() {
            for row in block.get_instructions().filter(|row| row.get_opcode() == InstructionOpcode::Call) {
                let call = CallSiteValue::try_from(row).unwrap();
                match call.get_called_fn_value() {
                    Some(callee) if callee == alloc => assert!(allocation.replace(call.try_as_basic_value().basic().unwrap()).is_none()),
                    Some(callee) if callee == free => {
                        assert!(block.get_name().to_str().unwrap().starts_with("fallible.release"));
                        assert!(release.replace(row.get_operand(0).unwrap().value().unwrap()).is_none());
                    }
                    Some(callee) if callee == frame => recurses = true,
                    _ => panic!("{target}: unexpected call in fallible frame: {row}"),
                }
            }
        }
        assert!(allocation.is_some() && recurses, "{target}: {ir}");
        assert_eq!(allocation, release, "{target}: {ir}");

        if let Some(out) = std::env::var_os("WOSY_RECURSIVE_FALLIBLE_IR_DIR") {
            let mut ir = ir.replace("@main(", "@wosy_authored_main(");
            for name in ["no_allocation", "three_frames"] {
                let symbol = function_with_suffix(&module, name);
                ir.push_str(&format!(
                    "\ndefine i64 @wosy_{name}() {{\nentry:\n  %result = call i64 @{symbol}()\n  ret i64 %result\n}}\n"
                ));
            }
            std::fs::write(std::path::PathBuf::from(out).join(format!("{target}.ll")), ir).unwrap();
        }
    }
}

#[test]
fn local_callable_replacement_selects_only_proved_ordinary_scalar_targets() {
    let app = source("workbench", "src/local_callable_replacement.w");
    let text = include_str!("fixtures/local-callable-replacement.w");
    for (target, target_layout) in [
        ("native64", ScalarTargetLayout::NATIVE64),
        ("wasm32", ScalarTargetLayout::WASM32),
    ] {
        let emit = |text: &str| {
            let prepared = prepare_scalar_project(
                vec![(declarations(&app, text), Vec::new())], vec![app.clone()],
                ScalarPreparationInputs { target_layout },
            ).unwrap_or_else(|error| panic!("{target}: {:?}", error.diagnostics));
            let build = analyze_scalar_build(&prepared);
            assert!(build.diagnostics().is_empty(), "{target}: {:?}", build.diagnostics());
            emit_scalar_build_project_llvm(&build).map(|module| module.to_text())
        };
        let ir = emit(text).unwrap_or_else(|error| panic!("{target}: {error}"));
        let context = Context::create();
        let buffer = MemoryBuffer::create_from_memory_range_copy(ir.as_bytes(), target);
        let module = context.create_module_from_ir(buffer).unwrap();
        module.verify().unwrap();
        let apply = module.get_function(&function_with_suffix(&module, "apply")).unwrap();
        let mut calls = 0;
        let mut writes = 0;
        for row in apply.get_basic_blocks().into_iter().flat_map(|block| block.get_instructions()) {
            match row.get_opcode() {
                InstructionOpcode::Call => {
                    let call = CallSiteValue::try_from(row).unwrap();
                    assert!(call.get_called_fn_value().is_none(), "{target}: {ir}");
                    assert_eq!(call.count_arguments(), 1);
                    calls += 1;
                }
                InstructionOpcode::Store if row.to_string().contains("store ptr @wosy_fn__") => writes += 1,
                _ => {}
            }
        }
        assert_eq!((calls, writes), (2, 2), "{target}: {ir}");
        if let Some(out) = std::env::var_os("WOSY_LOCAL_CALLABLE_IR_DIR") {
            let mut ir = ir.replace("@main(", "@wosy_authored_main(");
            for name in ["keep", "replace"] {
                let symbol = function_with_suffix(&module, name);
                ir.push_str(&format!(
                    "\ndefine i32 @wosy_{name}() {{\nentry:\n  %result = call i32 @{symbol}()\n  ret i32 %result\n}}\n"
                ));
            }
            std::fs::write(std::path::PathBuf::from(out).join(format!("{target}.ll")), ir).unwrap();
        }

        let effectful = text.replace("i32(i32) add_nine", "i32 observed_delta = 9;\ni32(i32) add_nine")
            .replace("input + 9", "input + observed_delta");
        let error = emit(&effectful).expect_err("an unclosed call target may not be stored");
        assert!(error.contains("local callable slot requires a proved direct scalar target"), "{target}: {error}");
    }
}

#[test]
fn checked_scalar_reader_keeps_received_owner_live() {
    let app = source("workbench", "src/receipt_field_reader.w");
    let text = include_str!("fixtures/receipt-field-reader.w");
    for (target, target_layout) in [("native64", ScalarTargetLayout::NATIVE64), ("wasm32", ScalarTargetLayout::WASM32)] {
        for project in [false, true] {
            let emit = |text: &str| {
                let prepared = prepare_scalar_project(vec![(declarations(&app, text), vec![])],
                    vec![app.clone()], ScalarPreparationInputs { target_layout }).unwrap();
                let build = analyze_scalar_build(&prepared);
                assert!(build.diagnostics().is_empty(), "{target}: {:?}", build.diagnostics());
                if project { emit_scalar_build_project_llvm(&build) }
                else { wosy_compiler::emit_scalar_build_single_module_llvm(&build) }
                    .map(|module| module.to_text())
            };
            let context = Context::create();
            let immediate = text.replace("    *u64 view = &packet.length;\n    read_length(view)", "    read_length(&packet.length)");
            assert_ne!(immediate, text);
            let symbol = |module: &Module<'_>, name: &str| {
                if project { function_with_suffix(module, name) } else { name.to_owned() }
            };
            let inline_named = text.replace("    read_length(view)", "    *view + 1");
            assert_ne!(inline_named, text);
            let metadata = text.replace("    *u64 view = &packet.length;",
                "    packet.length = 9;\n    *u64 view = &packet.length;");
            assert_ne!(metadata, text);
            for (text, route) in [(text, "named"), (&immediate, "immediate"),
                (&inline_named, "inline-named"), (&metadata, "metadata")]
            {
                let inline = route.starts_with("inline-");
                let ir = emit(text).unwrap_or_else(|error| panic!("{target}/{project}: {error}"));
                let module = context.create_module_from_ir(MemoryBuffer::create_from_memory_range_copy(ir.as_bytes(), target)).unwrap();
                module.verify().unwrap();
                let drive = module.get_function(&symbol(&module, "drive")).unwrap();
                let mut seen = Vec::new();
                let mut owner = None;
                for row in drive.get_basic_blocks().into_iter().flat_map(|b| b.get_instructions()) {
                    if inline && row.get_opcode() == InstructionOpcode::Load
                        && seen == ["allocate"] && row.to_string().contains("load i64,")
                    {
                        seen.push("read");
                    }
                    if route == "metadata" && row.get_opcode() == InstructionOpcode::Store
                        && row.get_operand(0).and_then(|operand| operand.value())
                            .filter(|value| value.is_int_value())
                            .and_then(|value| value.into_int_value().get_zero_extended_constant()) == Some(9)
                    {
                        seen.push("write");
                    }
                    if row.get_opcode() != InstructionOpcode::Call { continue; }
                    let call = CallSiteValue::try_from(row).unwrap();
                    let Some(callee) = call.get_called_fn_value() else { continue };
                    let name = callee.get_name().to_str().unwrap();
                    if name == symbol(&module, "make_packet") {
                        owner = call.try_as_basic_value().basic();
                        seen.push("allocate");
                    } else if name == symbol(&module, "read_length") {
                        assert!(!inline);
                        assert_eq!(call.count_arguments(), 1);
                        seen.push("read");
                    } else if name == "__wosy_core_free" {
                        assert!(owner.is_some());
                        assert_eq!(row.get_operand(0).unwrap().value(), owner);
                        seen.push("release");
                    }
                }
                let expected = if route == "metadata" { vec!["allocate", "write", "read", "release"] }
                    else { vec!["allocate", "read", "release"] };
                assert_eq!(seen, expected, "{target}/{project}: {ir}");
                if let Some(out) = std::env::var_os("WOSY_RECEIPT_FIELD_IR_DIR") {
                    let base = if project { "project" } else { "single" };
                    let out = std::path::PathBuf::from(out).join(
                        if route == "named" { base.to_owned() } else { format!("{route}-{base}") });
                    std::fs::create_dir_all(&out).unwrap();
                    let mut witness = ir.replace("@main(", "@wosy_authored_main(");
                    witness.push_str(&format!("\ndefine i64 @wosy_field_witness() {{\nentry:\n  %v = call i64 @{}()\n  ret i64 %v\n}}\n", symbol(&module, "field_witness")));
                    std::fs::write(out.join(format!("{target}.ll")), witness).unwrap();
                }
                if !inline {
                    let uncertified = text.replace("{ *view + 1 }", "{ u64 observed = *view; observed + 1 }");
                    assert!(emit(&uncertified).expect_err("signature alone cannot certify a checked reader")
                        .contains("aggregate owning receipt has an unproved checked loan"));
                }
                if route == "metadata" {
                    let overwrites_backing_view = text.replace("    packet.length = 9;", "    packet.data = null;");
                    assert!(emit(&overwrites_backing_view).expect_err("a store to the backing field is not scalar metadata")
                        .contains("aggregate transfer has unsupported use"));
                    let overlap = text.replace("    packet.length = 9;\n    *u64 view = &packet.length;",
                        "    *u64 view = &packet.length;\n    packet.length = 9;");
                    assert_ne!(overlap, text);
                    let prepared = prepare_scalar_project(vec![(declarations(&app, &overlap), vec![])],
                        vec![app.clone()], ScalarPreparationInputs { target_layout }).unwrap();
                    let checked = analyze_scalar_build(&prepared);
                    assert!(checked.diagnostics().iter().any(|error| error.labels.iter()
                        .any(|label| label.message == "origin of conflicting place or loan")), "{target}: {:?}", checked.diagnostics());
                }
            }
            for width in [8, 64] {
                let unrelated: String = (0..width).map(|i| format!("    Packet other_{i} = make_packet(length);\n")).collect();
                let reads: String = (0..width).map(|i| format!("    total = total + other_{i}.length;\n")).collect();
                let wide = text.replace("    *u64 view = &packet.length;\n    read_length(view)", &format!(
                    "{unrelated}    *u64 view = &packet.length;\n    u64 total = read_length(view);\n    *Packet whole = &packet;\n    total = total + (*whole).length;\n{reads}    total"));
                assert_ne!(wide, text);
                let wide = emit(&wide).unwrap_or_else(|error| panic!("{target}/{project}/{width}: {error}"));
                let verified = context.create_module_from_ir(MemoryBuffer::create_from_memory_range_copy(wide.as_bytes(), target)).unwrap();
                verified.verify().unwrap();
                let drive = verified.get_function(&symbol(&verified, "drive")).unwrap();
                let (mut owned, mut released) = (Vec::new(), Vec::new());
                let mut called = false;
                let mut post_call_loads = 0;
                for row in drive.get_basic_blocks().into_iter().flat_map(|b| b.get_instructions()) {
                    if called && released.is_empty() && row.get_opcode() == InstructionOpcode::Load {
                        post_call_loads += 1;
                    }
                    if row.get_opcode() != InstructionOpcode::Call { continue; }
                    let call = CallSiteValue::try_from(row).unwrap();
                    let Some(callee) = call.get_called_fn_value() else { continue };
                    let name = callee.get_name().to_str().unwrap();
                    if name == symbol(&verified, "make_packet") {
                        assert!(!called);
                        owned.push(call.try_as_basic_value().basic().unwrap());
                    } else if name == symbol(&verified, "read_length") {
                        assert!(!called);
                        called = true;
                    } else if name == "__wosy_core_free" {
                        assert!(called);
                        released.push(row.get_operand(0).unwrap().value().unwrap());
                    }
                }
                assert_eq!(owned.len(), width + 1);
                assert!(post_call_loads > width, "{target}/{project}/{width}: missing field observations after reader");
                owned.reverse();
                assert_eq!(released, owned, "{target}/{project}/{width}: released wrong backing");
            }

        }
    }
}

#[test]
fn imported_checked_reader_requires_exact_source_and_declaration() {
    let app = source("workbench", "src/receipt_field_import.w");
    let imported = source("workbench", "src/reader.w");
    let decoy = source("workbench", "src/decoy.w");
    let base = include_str!("fixtures/receipt-field-reader.w");
    let direct = "u64(*u64) read_length = fn(view) { *view + 1 };";
    let approved = format!("%%start\n{direct}\n%%end");
    // Same declaration name, byte offset and signature as the real target.
    let uncertified = approved.replace("{ *view + 1 }", "{ u64 seen = *view; seen + 1 }");
    let base = base.replace("%%start", "%%start\nreader = namespace workbench \"src/reader.w\";")
        .replace(direct, "").replace("read_length(view)", "reader.read_length(view)");
    let immediate = base.replace("    *u64 view = &packet.length;\n    reader.read_length(view)",
        "    reader.read_length(&packet.length)");
    assert_ne!(immediate, base);
    for (target, target_layout) in [("native64", ScalarTargetLayout::NATIVE64), ("wasm32", ScalarTargetLayout::WASM32)] {
        for (shape, main) in [("named", &base), ("immediate", &immediate)] {
            let emit = |reader_text: &str| {
                let root = declarations(&app, main);
                let namespace = root.namespaces().next().unwrap();
                let bindings = vec![ScalarNamespaceBinding { binding: namespace.binding.clone(),
                    target: imported.clone(), span: namespace.span }];
                let prepared = prepare_scalar_project(vec![(root, bindings),
                    (declarations(&imported, reader_text), vec![]), (declarations(&decoy, &approved), vec![])],
                    vec![app.clone(), imported.clone(), decoy.clone()], ScalarPreparationInputs { target_layout }).unwrap();
                let build = analyze_scalar_build(&prepared);
                assert!(build.diagnostics().is_empty(), "{target}/{shape}: {:?}", build.diagnostics());
                emit_scalar_build_project_llvm(&build).map(|module| module.to_text())
            };
            let ir = emit(&approved).unwrap_or_else(|error| panic!("{target}/{shape}: {error}"));
            let context = Context::create();
            let module = context.create_module_from_ir(MemoryBuffer::create_from_memory_range_copy(ir.as_bytes(), target)).unwrap();
            module.verify().unwrap();
            let drive = module.get_function(&function_with_suffix(&module, "drive")).unwrap();
            let mut seen = Vec::new();
            let mut backing = None;
            for row in drive.get_basic_blocks().into_iter().flat_map(|b| b.get_instructions()) {
                if row.get_opcode() != InstructionOpcode::Call { continue; }
                let call = CallSiteValue::try_from(row).unwrap();
                let Some(callee) = call.get_called_fn_value() else { continue };
                let name = callee.get_name().to_str().unwrap();
                if name.ends_with("__6d616b655f7061636b6574") {
                    backing = call.try_as_basic_value().basic();
                    seen.push("allocate");
                } else if name.ends_with("__726561645f6c656e677468") {
                    assert!(name.contains("__7372632f7265616465722e77__"), "{name}");
                    seen.push("read");
                } else if name == "__wosy_core_free" {
                    assert!(backing.is_some());
                    assert_eq!(row.get_operand(0).unwrap().value(), backing);
                    seen.push("release");
                }
            }
            assert_eq!(seen, ["allocate", "read", "release"], "{target}/{shape}: {ir}");
            if let Some(out) = std::env::var_os("WOSY_RECEIPT_FIELD_IR_DIR") {
                let out = std::path::PathBuf::from(out).join(format!("imported-{shape}"));
                std::fs::create_dir_all(&out).unwrap();
                let mut witness = ir.replace("@main(", "@wosy_authored_main(");
                witness.push_str(&format!("\ndefine i64 @wosy_field_witness() {{\nentry:\n  %v = call i64 @{}()\n  ret i64 %v\n}}\n", function_with_suffix(&module, "field_witness")));
                std::fs::write(out.join(format!("{target}.ll")), witness).unwrap();
            }
            assert!(emit(&uncertified).expect_err("another source's reader cannot seal this call")
                .contains("aggregate owning receipt has an unproved checked loan"));
        }
    }
}

#[test]
fn direct_scalar_descriptor_reader_preserves_backing_without_slot_authority() {
    let app = source("workbench", "src/receipt_descriptor.w");
    let imported = source("workbench", "src/reader.w");
    let base = include_str!("fixtures/receipt-field-reader.w");
    let record = "struct Packet { *?u8 data; u64 length; }";
    let direct = "u64(*Packet) read_length = fn(view) { (*view).length };";
    let fallback = "u64(*Packet) read_length = fn(view) { u64 seen = (*view).length; seen };";
    let base = base.replace("u64(*u64) read_length = fn(view) { *view + 1 };", direct)
        .replace("*u64 view = &packet.length;", "*Packet view = &packet;")
        .replace("    read_length(view)", "    read_length(view) + 1");
    for (target, target_layout) in [("native64", ScalarTargetLayout::NATIVE64), ("wasm32", ScalarTargetLayout::WASM32)] {
        for (route, immediate) in ["single", "project", "import"].into_iter()
            .flat_map(|route| [(route, false), (route, true)])
        {
            let external = route == "import";
            let emit = |reader: &str, slot: bool| {
                let reader_text = format!("%%start\n{record}\n{reader}\n%%end");
                let main = if external {
                    base.replace("%%start", "%%start\nreader = namespace workbench \"src/reader.w\";")
                        .replace(record, "").replace(direct, "")
                        .replace("Packet", "reader.Packet").replace("read_length(view)", "reader.read_length(view)")
                } else if slot {
                    base.replace(direct, reader).replace("    read_length(view)",
                        "    u64(*Packet) slot = read_length;\n    slot(view)")
                } else {
                    base.replace(direct, reader)
                };
                let main = if immediate {
                    let named = if external { "*reader.Packet" } else { "*Packet" };
                    let rewritten = main.replace(&format!("    {named} view = &packet;\n"), "")
                        .replace("read_length(view)", "read_length(&packet)");
                    assert_ne!(rewritten, main);
                    rewritten
                } else {
                    main
                };
                let root = declarations(&app, &main);
                let (rows, sources) = if external {
                    let namespace = root.namespaces().next().unwrap();
                    let bindings = vec![ScalarNamespaceBinding { binding: namespace.binding.clone(),
                        target: imported.clone(), span: namespace.span }];
                    (vec![(root, bindings), (declarations(&imported, &reader_text), vec![])],
                        vec![app.clone(), imported.clone()])
                } else {
                    (vec![(root, vec![])], vec![app.clone()])
                };
                let prepared = prepare_scalar_project(rows, sources, ScalarPreparationInputs { target_layout }).unwrap();
                let build = analyze_scalar_build(&prepared);
                if !build.diagnostics().is_empty() { return Err(format!("{:?}", build.diagnostics())); }
                if route == "single" { wosy_compiler::emit_scalar_build_single_module_llvm(&build) }
                else { emit_scalar_build_project_llvm(&build) }.map(|module| module.to_text())
            };
            let ir = emit(direct, false).unwrap_or_else(|error| panic!("{target}/{route}: {error}"));
            let context = Context::create();
            let module = context.create_module_from_ir(MemoryBuffer::create_from_memory_range_copy(ir.as_bytes(), target)).unwrap();
            module.verify().unwrap();
            let symbol = |name: &str| if route == "single" { name.to_owned() } else { function_with_suffix(&module, name) };
            let drive = module.get_function(&symbol("drive")).unwrap();
            let mut seen = Vec::new();
            let mut original = None;
            for row in drive.get_basic_blocks().into_iter().flat_map(|b| b.get_instructions()) {
                if row.get_opcode() != InstructionOpcode::Call { continue; }
                let call = CallSiteValue::try_from(row).unwrap();
                let Some(callee) = call.get_called_fn_value() else { continue };
                let name = callee.get_name().to_str().unwrap();
                if name == symbol("make_packet") {
                    original = call.try_as_basic_value().basic();
                    seen.push("allocate");
                } else if name == symbol("read_length") {
                    assert_eq!(call.count_arguments(), 1);
                    seen.push("read");
                } else if name == "__wosy_core_free" {
                    assert!(original.is_some());
                    assert_eq!(row.get_operand(0).unwrap().value(), original);
                    seen.push("release");
                }
            }
            assert_eq!(seen, ["allocate", "read", "release"], "{target}/{route}: {ir}");
            assert!(emit(fallback, false).expect_err("signature alone cannot authorize a descriptor loan")
                .contains("aggregate owning receipt has an unproved checked loan"));
            if !external && !immediate {
                let error = emit(direct, true).expect_err("direct descriptor reader certificate cannot authorize a slot");
                assert!(error.contains("aggregate owning receipt has an unproved checked loan")
                    || error.contains("local callable slot requires a proved direct scalar target"), "{error}");
            }
            if let Some(out) = std::env::var_os("WOSY_RECEIPT_FIELD_IR_DIR") {
                let shape = if immediate { "-immediate" } else { "" };
                let out = std::path::PathBuf::from(out).join(format!("descriptor-{route}{shape}"));
                std::fs::create_dir_all(&out).unwrap();
                let mut witness = ir.replace("@main(", "@wosy_authored_main(");
                witness.push_str(&format!("\ndefine i64 @wosy_field_witness() {{\nentry:\n  %v = call i64 @{}()\n  ret i64 %v\n}}\n", symbol("field_witness")));
                std::fs::write(out.join(format!("{target}.ll")), witness).unwrap();
            }
        }
    }
}

#[test]
fn local_checked_reader_preserves_the_selected_loan_through_recursion() {
    let app = source("workbench", "src/local_checked_reader.w");
    let text = include_str!("fixtures/local-checked-reader.w");
    for (target, target_layout) in [
        ("native64", ScalarTargetLayout::NATIVE64),
        ("wasm32", ScalarTargetLayout::WASM32),
    ] {
        let prepare = |text: &str| prepare_scalar_project(
            vec![(declarations(&app, text), Vec::new())], vec![app.clone()],
            ScalarPreparationInputs { target_layout },
        ).unwrap_or_else(|error| panic!("{target}: {:?}", error.diagnostics));
        let emit = |text: &str| {
            let prepared = prepare(text);
            let build = analyze_scalar_build(&prepared);
            assert!(build.diagnostics().is_empty(), "{target}: {:?}", build.diagnostics());
            emit_scalar_build_project_llvm(&build).map(|module| module.to_text())
        };
        let ir = emit(text).unwrap_or_else(|error| panic!("{target}: {error}"));
        let context = Context::create();
        let buffer = MemoryBuffer::create_from_memory_range_copy(ir.as_bytes(), target);
        let module = context.create_module_from_ir(buffer).unwrap();
        module.verify().unwrap();
        let frame = module.get_function(&function_with_suffix(&module, "frame")).unwrap();
        let alloc = module.get_function("__wosy_core_alloc").unwrap();
        let free = module.get_function("__wosy_core_free").unwrap();
        let mut found = Vec::new();
        let mut allocated = None;
        let mut freed = None;
        let mut writes = 0;
        for row in frame.get_basic_blocks().into_iter().flat_map(|block| block.get_instructions()) {
            if row.get_opcode() == InstructionOpcode::Store && row.to_string().contains("store ptr @wosy_fn__") {
                writes += 1;
            }
            if row.get_opcode() != InstructionOpcode::Call { continue; }
            let call = CallSiteValue::try_from(row).unwrap();
            match call.get_called_fn_value() {
                Some(callee) if callee == alloc => {
                    assert!(allocated.replace(call.try_as_basic_value().basic().unwrap()).is_none());
                    found.push("allocate");
                }
                Some(callee) if callee == frame => found.push("descend"),
                Some(callee) if callee == free => {
                    assert!(freed.replace(row.get_operand(0).unwrap().value().unwrap()).is_none());
                    found.push("release");
                }
                None => {
                    assert_eq!(call.count_arguments(), 1, "{target}: {ir}");
                    let argument = row.get_operand(0).unwrap().value().unwrap().into_int_value();
                    assert_eq!(argument.get_type().get_bit_width(), if target == "native64" { 64 } else { 32 });
                    found.push("checked read");
                }
                _ => {}
            }
        }
        assert_eq!(found, ["allocate", "descend", "checked read", "release"], "{target}: {ir}");
        assert_eq!(writes, 2, "{target}: {ir}");
        assert_eq!(allocated, freed, "{target}: {ir}");

        if let Some(out) = std::env::var_os("WOSY_LOCAL_CHECKED_READER_IR_DIR") {
            let mut ir = ir.replace("@main(", "@wosy_authored_main(");
            for name in ["frame_zero", "frame_two", "frame_three", "joined_left", "joined_right"] {
                let symbol = function_with_suffix(&module, name);
                ir.push_str(&format!(
                    "\ndefine i8 @wosy_{name}() {{\nentry:\n  %result = call i8 @{symbol}()\n  ret i8 %result\n}}\n"
                ));
            }
            std::fs::write(std::path::PathBuf::from(out).join(format!("{target}.ll")), ir).unwrap();
        }

        let effectful = text.replace("u8(*u8) read_bonus", "u8 extra = 1;\nu8(*u8) read_bonus")
            .replace("*reader + 1", "*reader + extra");
        let requires_family = |text: &str| {
            let prepared = prepare(text);
            let build = analyze_scalar_build(&prepared);
            assert!(build.diagnostics().iter().any(|diagnostic| diagnostic.code == "B0003"
                && diagnostic.message == "local shared callable family has no closed checked-read authority"),
                "{target}: expected closed-family diagnostic, got {:?}", build.diagnostics());
        };
        requires_family(&effectful);
        let unknown_replacement = text.replacen(
            "        if (choose) { read_slot = read_bonus; };",
            "        u8(*u8) unknown = read_bonus; if (choose) { read_slot = unknown; };",
            1,
        );
        requires_family(&unknown_replacement);

        let conflict = text.replace("        u8 observed = read_slot(selected);\n        bytes[0] = tag + 20;",
            "        bytes[0] = tag + 20;\n        u8 observed = read_slot(selected);");
        let prepared = prepare(&conflict);
        let build = analyze_scalar_build(&prepared);
        assert!(build.diagnostics().iter().any(|diagnostic| diagnostic.code == "B0003"),
            "{target}: expected a Build loan conflict, got {:?}", build.diagnostics());
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
fn recursive_runtime_array_retains_each_local_owner_through_child_return() {
    let text = r#"%%start
u8(u64, u8) recursive_owner = fn(depth, tag) {
    if (depth == 0) {
        tag
    } else {
        u8[depth] bytes;
        bytes[0] = tag;
        u64 step = 1;
        u8 tag_step = 1;
        u8 nested = recursive_owner(depth - step, tag - tag_step);
        nested + bytes[0]
    }
};
u8 observed = recursive_owner(3, 3);
%%end"#;
    let user = source("workbench", "src/recursive_owner.w");
    for (target, target_layout) in [
        ("native64", ScalarTargetLayout::NATIVE64),
        ("wasm32", ScalarTargetLayout::WASM32),
    ] {
        let prepared = prepare_scalar_project(
            vec![(declarations(&user, text), Vec::new())],
            vec![user.clone()],
            ScalarPreparationInputs { target_layout },
        )
        .unwrap_or_else(|error| panic!("{target}: {:?}", error.diagnostics));
        let build = analyze_scalar_build(&prepared);
        assert!(build.diagnostics().is_empty(), "{target}: {:?}", build.diagnostics());
        let ir = emit_scalar_build_project_llvm(&build).unwrap().to_text();
        let context = Context::create();
        let buffer = MemoryBuffer::create_from_memory_range_copy(ir.as_bytes(), target);
        let module = context.create_module_from_ir(buffer).unwrap();
        module.verify().unwrap();
        let owner = function_with_suffix(&module, "recursive_owner");
        let calls = direct_calls(&module, &owner);
        assert!(calls.contains(&owner), "{target}: no recursive call: {calls:?}");
        assert_eq!(calls.iter().filter(|name| *name == "__wosy_core_alloc").count(), 1, "{target}");
        assert_eq!(calls.iter().filter(|name| *name == "__wosy_core_free").count(), 1, "{target}");
        if let Some(out) = std::env::var_os("WOSY_RECURSIVE_OWNER_IR_DIR") {
            let mut witness = ir.replace("@main(", "@wosy_authored_main(");
            witness.push_str(&format!(
                "\ndefine i8 @wosy_recursive_owner_witness(i64 %depth, i8 %tag) {{\nentry:\n  %result = call i8 @{owner}(i64 %depth, i8 %tag)\n  ret i8 %result\n}}\n"
            ));
            std::fs::write(std::path::PathBuf::from(out).join(format!("{target}.ll")), witness)
                .unwrap();
        }
    }
}

#[test]
fn recursive_runtime_array_replacement_returns_the_new_owner_to_the_final_receiver() {
    let text = include_str!("fixtures/recursive-replacement.w");
    let user = source("workbench", "src/recursive_replacement.w");
    for (target, target_layout) in [
        ("native64", ScalarTargetLayout::NATIVE64),
        ("wasm32", ScalarTargetLayout::WASM32),
    ] {
        let prepared = prepare_scalar_project(
            vec![(declarations(&user, text), Vec::new())],
            vec![user.clone()],
            ScalarPreparationInputs { target_layout },
        )
        .unwrap_or_else(|error| panic!("{target}: {:?}", error.diagnostics));
        let build = analyze_scalar_build(&prepared);
        assert!(build.diagnostics().is_empty(), "{target}: {:?}", build.diagnostics());
        let ir = emit_scalar_build_project_llvm(&build).unwrap().to_text();
        let context = Context::create();
        let buffer = MemoryBuffer::create_from_memory_range_copy(ir.as_bytes(), target);
        let module = context.create_module_from_ir(buffer).unwrap();
        module.verify().unwrap();
        let receive = function_with_suffix(&module, "recursive_replacement");
        let descend = function_with_suffix(&module, "recursive_return");
        let replace = function_with_suffix(&module, "replace_owner");
        let calls = direct_calls(&module, &descend);
        assert!(calls.contains(&descend) && calls.contains(&replace), "{target}: {calls:?}");

        let free = "call void @__wosy_core_free";
        let replacement_body = ir.lines().find(|line| line.starts_with("define ")
            && line.contains(&format!("@{replace}("))).and_then(|header| ir.split_once(header))
            .expect("replacement definition").1
            .split_once("\n}").unwrap().0;
        let (before_copy, _) = replacement_body.split_once("load i8, ptr %array_element")
            .expect("replacement reads original");
        assert!(before_copy.contains("call ptr @__wosy_core_alloc"), "{replacement_body}");
        assert!(!replacement_body.contains(free), "{replacement_body}");
        let receiver_body = ir.split_once(&format!("define i8 @{receive}(")).unwrap().1
            .split_once("\n}").unwrap().0;
        let (before_transfer, after_transfer) = receiver_body.split_once(&format!("@{descend}(")).unwrap();
        let (_, after_use) = after_transfer.split_once("load i8, ptr %array_element")
            .expect("outer receiver uses replacement");
        assert!(!before_transfer.contains(free), "{receiver_body}");
        assert_eq!(after_use.matches(free).count(), 1, "{receiver_body}");

        if let Some(out) = std::env::var_os("WOSY_RECURSIVE_REPLACEMENT_IR_DIR") {
            let mut witness = ir.replace("@main(", "@wosy_authored_main(");
            witness.push_str(&format!(
                "\ndefine i8 @wosy_recursive_replacement_witness(i64 %depth, i64 %original_length, i64 %replacement_length) {{\nentry:\n  %result = call i8 @{receive}(i64 %depth, i64 %original_length, i64 %replacement_length)\n  ret i8 %result\n}}\n"
            ));
            std::fs::write(std::path::PathBuf::from(out).join(format!("{target}.ll")), witness)
                .unwrap();
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
        let step = lookup("_read_line_discard_step");
        // The one panic is the existing null check for the checked reborrow,
        // not an extra call on the ordinary recurring path.
        assert_eq!(
            direct_calls(&module, &discard),
            vec!["__wosy_core_system_panic".to_owned(), step.clone()]
        );
        let blocks = module.get_function(&discard).unwrap().get_basic_blocks();
        let named_block = |name: &[u8]| {
            blocks
                .iter()
                .copied()
                .find(|block| block.get_name().to_bytes() == name)
                .unwrap_or_else(|| panic!("{target_layout:?}: missing block {name:?}"))
        };
        let header = named_block(b"self_tail.header");
        let failure = named_block(b"deref_null_panic");
        let continuation = named_block(b"deref_null_continue");
        let block_calls = |block: inkwell::basic_block::BasicBlock<'_>| {
            block.get_instructions()
                .filter(|instruction| instruction.get_opcode() == InstructionOpcode::Call)
                .map(|instruction| {
                    CallSiteValue::try_from(instruction).unwrap().get_called_fn_value()
                        .expect("ordinary direct call").get_name().to_str().unwrap().to_owned()
                })
                .collect::<Vec<_>>()
        };
        assert_eq!(block_calls(failure), vec!["__wosy_core_system_panic"]);
        assert_eq!(block_calls(continuation), vec![step]);
        assert_eq!(
            failure.get_terminator().unwrap().get_opcode(),
            InstructionOpcode::Unreachable
        );
        let backedge = continuation.get_terminator().unwrap();
        assert_eq!(backedge.get_opcode(), InstructionOpcode::Br);
        assert_eq!(backedge.get_operand(0).unwrap().block(), Some(header));
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
fn selected_recommit_mixed_result_keeps_saved_scalar_and_original_backing() {
    let app = source("workbench", "src/selected_recommit_mixed.w");
    let factories = include_str!("fixtures/structured-local-choice.w");
    let (factories, _) = factories.split_once("u64(u64, bool) drive").unwrap();
    for (order, owner, output, values, receive) in [
        ("owner-first", 0, "(Packet, u64)", "moved, saved", "Packet received, u64 saved"),
        ("scalar-first", 1, "(u64, Packet)", "saved, moved", "u64 saved, Packet received"),
    ] {
        let text = format!("{factories}
Packet(u64) neighbor = fn(length) {{
    u8[length] bytes; bytes[0] = 39;
    *?u8 data = null;
    unsafe {{ data = &?bytes[0]; }};
    Packet packet = {{ .data = data; .length = length; }};
    packet
}};
Packet(Packet) relay = fn(packet) {{ packet }};
u64(*Packet) amount = fn(view) {{ (*view).length }};
{output}(bool) shipment = fn(take) {{
    Packet selected = if (take) {{ make_short(2) }} else {{ make_long(2) }};
    Packet sibling = neighbor(7);
    sibling.length;
    Packet moved = relay(selected);
    u64 saved = amount(&moved);
    {values}
}};
u64(bool) driver = fn(take) {{
    {receive} = shipment(take);
    received.length + saved
}};
u64() short_case = fn {{ driver(true) }};
u64() long_case = fn {{ driver(false) }};
short_case();
long_case();
%%end");
        for (target, target_layout) in [("native64", ScalarTargetLayout::NATIVE64), ("wasm32", ScalarTargetLayout::WASM32)] {
            for (route, project) in [("single", false), ("project", true)] {
                let prepared = prepare_scalar_project(vec![(declarations(&app, &text), vec![])],
                    vec![app.clone()], ScalarPreparationInputs { target_layout }).unwrap();
                let build = analyze_scalar_build(&prepared);
                assert!(build.diagnostics().is_empty(), "{order}/{target}/{route}: {:?}", build.diagnostics());
                let ir = if project { emit_scalar_build_project_llvm(&build) }
                    else { wosy_compiler::emit_scalar_build_single_module_llvm(&build) }
                    .unwrap_or_else(|error| panic!("{order}/{target}/{route}: {error}")).to_text();
                let context = Context::create();
                let module = context.create_module_from_ir(
                    MemoryBuffer::create_from_memory_range_copy(ir.as_bytes(), target)).unwrap();
                module.verify().unwrap();
                let symbol = |name: &str| if project { function_with_suffix(&module, name) } else { name.to_owned() };
                let find = |name: &str| module.get_function(&symbol(name)).unwrap();
                let (ship, driver, neighbor, release, reader) = (find("shipment"), find("driver"), find("neighbor"),
                    module.get_function("__wosy_core_free").unwrap(), find("amount"));
                let mut sibling = None;
                let mut releases = Vec::new();
                let mut base = None;
                let mut transported = [None; 2];
                let mut reader_value = None;
                let mut stores = Vec::new();
                for row in ship.get_basic_blocks().into_iter().flat_map(|block| block.get_instructions()) {
                    if row.get_opcode() == InstructionOpcode::Phi && row.to_string().contains("phi ptr") {
                        assert!(base.replace(inkwell::values::PhiValue::try_from(row).unwrap().as_basic_value()).is_none());
                    }
                    if row.get_opcode() == InstructionOpcode::Store {
                        stores.push((row.get_operand(0).unwrap().value().unwrap(),
                            row.get_operand(1).unwrap().value().unwrap()));
                    }
                    if row.get_opcode() == InstructionOpcode::Return {
                        assert!(reader_value.is_some());
                        let mut pair = row.get_operand(0).unwrap().value().unwrap();
                        while let Some(insert) = pair.as_instruction_value().filter(|inst|
                            inst.get_opcode() == InstructionOpcode::InsertValue)
                        {
                            let indices = insert.get_indices();
                            let [lane] = indices.as_slice() else { panic!("expected one ordered lane: {ir}") };
                            let lane = *lane as usize;
                            assert!(lane < transported.len());
                            if transported[lane].is_none() {
                                // Walk newest to oldest; the original descriptor
                                // insertion may be superseded by its backing.
                                transported[lane] = insert.get_operand(1).unwrap().value();
                            }
                            pair = insert.get_operand(0).unwrap().value().unwrap();
                        }
                    }
                    let Ok(call) = CallSiteValue::try_from(row) else { continue };
                    match call.get_called_fn_value() {
                        Some(target) if target == neighbor => assert!(sibling.replace(call.try_as_basic_value().basic().unwrap()).is_none()),
                        Some(target) if target == release => releases.push(row.get_operand(0).unwrap().value().unwrap()),
                        Some(target) if target == reader =>
                            assert!(reader_value.replace(call.try_as_basic_value().basic().unwrap()).is_none()),
                        _ => {}
                    }
                }
                assert_eq!(transported[owner], base);
                assert!(base.is_some());
                let scalar = transported[1 - owner].expect("one ordinary transported scalar");
                assert_eq!(scalar.into_int_value().get_type().get_bit_width(), 64);
                let read = reader_value.expect("one certified scalar read");
                if scalar != read {
                    let load = scalar.as_instruction_value().expect("saved scalar read instruction");
                    assert_eq!(load.get_opcode(), InstructionOpcode::Load);
                    let address = load.get_operand(0).unwrap().value().unwrap();
                    assert!(stores.iter().filter(|(_, slot)| *slot == address).map(|(value, _)| *value).eq([read]),
                        "the scalar lane must read the exact slot written by the certified reader");
                }
                assert_ne!(sibling, base);
                assert_eq!(releases, [sibling.expect("one independent local sibling")]);
                let mut pair = None;
                let mut released = None;
                let mut caller_scalar = None;
                for row in driver.get_basic_blocks().into_iter().flat_map(|block| block.get_instructions()) {
                    if row.get_opcode() == InstructionOpcode::ExtractValue && row.get_indices() == [(1 - owner) as u32] {
                        let mut from = row.get_operand(0).unwrap().value().unwrap();
                        // Caller materialization may install the descriptor in
                        // the owner lane before unpacking the untouched scalar.
                        while let Some(insert) = from.as_instruction_value().filter(|inst|
                            inst.get_opcode() == InstructionOpcode::InsertValue && inst.get_indices() == [owner as u32])
                        {
                            from = insert.get_operand(0).unwrap().value().unwrap();
                        }
                        if Some(from) == pair {
                            assert!(caller_scalar.replace(row).is_none());
                        }
                    }
                    let Ok(call) = CallSiteValue::try_from(row) else { continue };
                    match call.get_called_fn_value() {
                        Some(target) if target == ship => assert!(pair.replace(call.try_as_basic_value().basic().unwrap()).is_none()),
                        Some(target) if target == release => assert!(released.replace(row.get_operand(0).unwrap().value().unwrap()).is_none()),
                        _ => {}
                    }
                }
                let extracted = released.expect("one caller release").as_instruction_value().unwrap();
                assert_eq!(extracted.get_opcode(), InstructionOpcode::ExtractValue);
                assert_eq!(extracted.get_indices(), [owner as u32]);
                assert_eq!(extracted.get_operand(0).unwrap().value(), pair);
                assert_eq!(caller_scalar.expect("ordinary result extracted independently")
                    .get_type().into_int_type().get_bit_width(), 64);
                if let Some(out) = std::env::var_os("WOSY_MIXED_SELECTED_IR_DIR") {
                    let out = std::path::PathBuf::from(out).join(format!("{order}-{route}"));
                    std::fs::create_dir_all(&out).unwrap();
                    let mut witness = ir.replace("@main(", "@wosy_authored_main(");
                    for case in ["short_case", "long_case"] {
                        witness.push_str(&format!("\ndefine i64 @wosy_{case}() {{\nentry:\n  %v = call i64 @{}()\n  ret i64 %v\n}}\n", symbol(case)));
                    }
                    std::fs::write(out.join(format!("{target}.ll")), witness).unwrap();
                }
            }
        }
    }
}

#[test]
fn mixed_owned_struct_result_has_separate_base_and_scalar_lanes() {
    let app = source("workbench", "src/mixed_owned_struct.w");
    let text = r#"%%start
struct Packet { *?u8 data; u64 length; }
(Packet, u32)(u64) make_packet = fn(length) {
    u8[length] backing;
    backing[0] = 19;
    *?u8 data = null;
    unsafe { data = &?backing[0]; };
    Packet packet = { .data = data; .length = length; };
    packet, 37
};
Packet plain_descriptor = { .data = null; .length = 3; };
(Packet, u32)() plain_packet = fn { plain_descriptor, 11 };
u64() consume_packet = fn {
    Packet packet, u32 marker = make_packet(2);
    Packet ordinary, u32 bonus = plain_packet();
    ordinary.length = 5;
    u8 byte = unsafe { core.load<u8>(packet.data) };
    ordinary.length + core.int_extend<u64>(byte) + core.int_extend<u64>(marker) + core.int_extend<u64>(bonus)
};
u64 result = consume_packet();
if (result != 72) { core.system_panic(); };
%%end"#;
    for target_layout in [ScalarTargetLayout::NATIVE64, ScalarTargetLayout::WASM32] {
        let prepared = prepare_scalar_project(
            vec![(declarations(&app, text), Vec::new())],
            vec![app.clone()],
            ScalarPreparationInputs { target_layout },
        ).unwrap_or_else(|error| panic!("{target_layout:?}: {:?}", error.diagnostics));
        let build = analyze_scalar_build(&prepared);
        assert!(build.diagnostics().is_empty(), "{:#?}", build.diagnostics());
        let text = emit_scalar_build_project_llvm(&build).unwrap().to_text();
        let context = Context::create();
        let buffer = MemoryBuffer::create_from_memory_range_copy(text.as_bytes(), "mixed-owner");
        let module = context.create_module_from_ir(buffer).unwrap();
        module.verify().unwrap();
        let producer = function_with_suffix(&module, "make_packet");
        let consumer = function_with_suffix(&module, "consume_packet");
        let plain = function_with_suffix(&module, "plain_packet");
        use inkwell::values::AnyValue;
        let producer = module.get_function(&producer).unwrap();
        let caller = module.get_function(&consumer).unwrap();
        let definition = producer.print_to_string().to_string();
        let plain_definition = module.get_function(&plain).unwrap().print_to_string().to_string();
        assert!(!plain_definition.contains("alloca"), "{plain_definition}");
        let plain_global_suffix = format!("__{}", b"global_plain_descriptor".iter()
            .map(|byte| format!("{byte:02x}")).collect::<String>());
        assert!(plain_definition.contains(&plain_global_suffix), "{plain_definition}");
        assert!(plain_definition.contains("ret { ptr, i32 }"), "{plain_definition}");
        let call = caller.print_to_string().to_string();
        assert!(definition.contains("define internal { ptr, i32 }"), "{definition}");
        assert!(definition.contains("ret { ptr, i32 }"), "{definition}");
        assert!(!definition.contains("call void @__wosy_core_free"), "{definition}");
        assert_eq!(call.matches(&format!("call {{ ptr, i32 }} @{}(", producer.get_name().to_str().unwrap())).count(), 1, "{call}");
        assert!(call.contains(&format!("call {{ ptr, i32 }} @{plain}()")), "{call}");
        assert!(call.contains("extractvalue { ptr, i32 } %aggregate_original_base, 0"), "{call}");
        assert!(call.contains("extractvalue { ptr, i32 } %aggregate_values, 1"), "{call}");
        assert_eq!(call.matches("call void @__wosy_core_free").count(), 1, "{call}");
        assert!(call.contains("call void @__wosy_core_free(ptr %aggregate_base)"), "{call}");
        let release = call.find("call void @__wosy_core_free").unwrap();
        assert!(call[..release].contains("store i64 5"), "{call}");
        assert!(call[..release].contains("load i8"), "{call}");
        assert!(call[..release].contains("add i64"), "{call}");
        assert!(call[release..].contains("ret i64"), "{call}");
    }
}

#[test]
fn mixed_reversed_result_forwards_ordinary_scalar_and_owns_exact_second_lane() {
    let app = source("workbench", "src/reversed_owned_struct.w");
    let text = r#"%%start
struct Packet { *?u8 data; u64 length; }
(u32, Packet)(u64) make_packet = fn(length) {
    u8[length] backing;
    backing[0] = 19;
    *?u8 data = null;
    unsafe { data = &?backing[0]; };
    Packet packet = { .data = data; .length = length; };
    37, packet
};
(u32, Packet)(u64) relay_packet = fn(length) {
    u32 marker, Packet packet = make_packet(length);
    marker, packet
};
u64() consume_packet = fn {
    u32 marker, Packet packet = relay_packet(2);
    u8 byte = unsafe { core.load<u8>(packet.data) };
    packet.length + core.int_extend<u64>(byte) + core.int_extend<u64>(marker)
};
u64 result = consume_packet();
if (result != 58) { core.system_panic(); };
%%end"#;
    for target_layout in [ScalarTargetLayout::NATIVE64, ScalarTargetLayout::WASM32] {
        let prepared = prepare_scalar_project(
            vec![(declarations(&app, text), Vec::new())],
            vec![app.clone()],
            ScalarPreparationInputs { target_layout },
        ).unwrap_or_else(|error| panic!("{target_layout:?}: {:?}", error.diagnostics));
        let build = analyze_scalar_build(&prepared);
        assert!(build.diagnostics().is_empty(), "{:#?}", build.diagnostics());
        let llvm = emit_scalar_build_project_llvm(&build).unwrap().to_text();
        let context = Context::create();
        let buffer = MemoryBuffer::create_from_memory_range_copy(llvm.as_bytes(), "reversed-owner");
        let module = context.create_module_from_ir(buffer).unwrap();
        module.verify().unwrap();
        use inkwell::values::AnyValue;
        let producer = function_with_suffix(&module, "make_packet");
        let relay = function_with_suffix(&module, "relay_packet");
        let caller = function_with_suffix(&module, "consume_packet");
        let producer_body = module.get_function(&producer).unwrap().print_to_string().to_string();
        let relay_fn = module.get_function(&relay).unwrap();
        let relay_body = relay_fn.print_to_string().to_string();
        let relay_instructions = relay_fn.get_basic_blocks().into_iter()
            .flat_map(|block| block.get_instructions()).collect::<Vec<_>>();
        let mut relay_calls = relay_instructions.iter().copied().filter(|inst| inst.get_opcode() == InstructionOpcode::Call);
        let call = relay_calls.next().expect("relay must call the producer");
        assert!(relay_calls.next().is_none(), "{relay_body}");
        assert_eq!(CallSiteValue::try_from(call).unwrap().get_called_fn_value(), module.get_function(&producer));
        assert_eq!(call.get_operand(1).unwrap().value(), relay_fn.get_last_param());
        let returned = relay_instructions.iter().find(|inst| inst.get_opcode() == InstructionOpcode::Return).unwrap();
        assert_eq!(returned.get_operand(0).unwrap().value(), CallSiteValue::try_from(call).unwrap().try_as_basic_value().basic());
        assert!(!relay_body.contains("extractvalue") && !relay_body.contains("insertvalue"), "{relay_body}");
        let caller_body = module.get_function(&caller).unwrap().print_to_string().to_string();
        assert!(producer_body.contains("ret { i32, ptr }"), "{producer_body}");
        assert!(!producer_body.contains("call void @__wosy_core_free"), "{producer_body}");
        assert_eq!(caller_body.matches(&format!("call {{ i32, ptr }} @{relay}(")).count(), 1, "{caller_body}");
        assert!(caller_body.contains("extractvalue { i32, ptr } %aggregate_original_base, 1"), "{caller_body}");
        assert!(caller_body.contains("extractvalue { i32, ptr } %aggregate_values, 0"), "{caller_body}");
        assert_eq!(caller_body.matches("call void @__wosy_core_free").count(), 1, "{caller_body}");
        let release = caller_body.find("call void @__wosy_core_free(ptr %aggregate_base)").expect("exact owner release");
        assert!(caller_body[..release].contains("load i8"), "{caller_body}");

        let wrong_lanes = text.replace("u32 marker, Packet packet = relay_packet(2)", "Packet packet, u32 marker = relay_packet(2)");
        assert!(prepare_scalar_project(
            vec![(declarations(&app, &wrong_lanes), Vec::new())],
            vec![app.clone()],
            ScalarPreparationInputs { target_layout },
        ).is_err());
    }
}

#[test]
fn mixed_reversed_result_joins_scalar_writes_without_changing_the_owner() {
    let app = source("workbench", "src/reversed_owned_join.w");
    let text = r#"%%start
struct Packet { *?u8 data; u64 length; }
(u32, Packet)(u64, bool) make_packet = fn(length, choose_other) {
    u8[length] backing;
    backing[0] = 19;
    *?u8 data = null;
    unsafe { data = &?backing[0]; };
    Packet packet = { .data = data; .length = length; };
    u32 marker = 37;
    if (choose_other) { marker = 43; };
    marker, packet
};
u64(bool) consume_packet = fn(choose_other) {
    u32 marker, Packet packet = make_packet(2, choose_other);
    u8 byte = unsafe { core.load<u8>(packet.data) };
    packet.length + core.int_extend<u64>(byte) + core.int_extend<u64>(marker)
};
u64 primary = consume_packet(false);
u64 alternate = consume_packet(true);
if (primary != 58 || alternate != 64) { core.system_panic(); };
%%end"#;
    for target_layout in [ScalarTargetLayout::NATIVE64, ScalarTargetLayout::WASM32] {
        let prepared = prepare_scalar_project(
            vec![(declarations(&app, text), Vec::new())],
            vec![app.clone()],
            ScalarPreparationInputs { target_layout },
        ).unwrap_or_else(|error| panic!("{target_layout:?}: {:?}", error.diagnostics));
        let build = analyze_scalar_build(&prepared);
        assert!(build.diagnostics().is_empty(), "{:#?}", build.diagnostics());
        let llvm = emit_scalar_build_project_llvm(&build).unwrap().to_text();
        let context = Context::create();
        let buffer = MemoryBuffer::create_from_memory_range_copy(llvm.as_bytes(), "reversed-owner-join");
        let module = context.create_module_from_ir(buffer).unwrap();
        module.verify().unwrap();
        use inkwell::values::AnyValue;
        let producer = function_with_suffix(&module, "make_packet");
        let consumer = function_with_suffix(&module, "consume_packet");
        let producer_ir = module.get_function(&producer).unwrap().print_to_string().to_string();
        let consumer_ir = module.get_function(&consumer).unwrap().print_to_string().to_string();
        assert!(producer_ir.contains("br i1 ") && producer_ir.contains("ret { i32, ptr }"), "{producer_ir}");
        assert!(!producer_ir.contains("call void @__wosy_core_free"), "{producer_ir}");
        assert_eq!(consumer_ir.matches(&format!("call {{ i32, ptr }} @{producer}(")).count(), 1, "{consumer_ir}");
        assert!(consumer_ir.contains("extractvalue { i32, ptr } %aggregate_original_base, 1"), "{consumer_ir}");
        assert_eq!(consumer_ir.matches("call void @__wosy_core_free").count(), 1, "{consumer_ir}");
        let release = consumer_ir.find("call void @__wosy_core_free(ptr %aggregate_base)").expect("exact owner release");
        assert!(consumer_ir[..release].contains("load i8"), "{consumer_ir}");
    }
}

#[test]
fn mixed_owned_struct_forwarder_transports_original_receipts_without_copying() {
    let app = source("workbench", "src/mixed_owner_forward.w");
    let text = r#"%%start
struct Packet { *?u8 data; u64 length; }
(Packet, u32)(u64, u8, u32) forward_packet = fn(length, value, marker) {
    Packet packet, u32 status = relay_packet(length, value, marker);
    packet, status
};
(Packet, u32)(u64, u8, u32) make_packet = fn(length, value, marker) {
    u8[length] backing;
    backing[0] = value;
    *?u8 data = null;
    unsafe { data = &?backing[0]; };
    Packet packet = { .data = data; .length = length; };
    packet, marker
};
(Packet, u32)(u64, u8, u32) relay_packet = fn(length, value, marker) {
    Packet packet, u32 status = make_packet(length, value, marker);
    packet, status
};
Packet shared_packet = { .data = null; .length = 5; };
(Packet, u32)() ordinary_packet = fn { shared_packet, 7 };
u64() receive_packets = fn {
    Packet first, u32 first_status = relay_packet(2, 19, 37);
    Packet second, u32 second_status = forward_packet(3, 41, 43);
    Packet plain, u32 plain_status = ordinary_packet();
    u8 first_byte = unsafe { core.load<u8>(first.data) };
    u8 second_byte = unsafe { core.load<u8>(second.data) };
    core.int_extend<u64>(first_byte) + core.int_extend<u64>(second_byte) + core.int_extend<u64>(first_status) + core.int_extend<u64>(second_status) + plain.length + core.int_extend<u64>(plain_status)
};
u64 result = receive_packets();
if (result != 152) { core.system_panic(); };
%%end"#;
    for target_layout in [ScalarTargetLayout::NATIVE64, ScalarTargetLayout::WASM32] {
        let prepared = prepare_scalar_project(
            vec![(declarations(&app, text), Vec::new())],
            vec![app.clone()],
            ScalarPreparationInputs { target_layout },
        ).unwrap_or_else(|error| panic!("{target_layout:?}: {:?}", error.diagnostics));
        let build = analyze_scalar_build(&prepared);
        assert!(build.diagnostics().is_empty(), "{:#?}", build.diagnostics());
        let text = emit_scalar_build_project_llvm(&build).unwrap().to_text();
        let context = Context::create();
        let buffer = MemoryBuffer::create_from_memory_range_copy(text.as_bytes(), "mixed-forward");
        let module = context.create_module_from_ir(buffer).unwrap();
        module.verify().unwrap();
        let producer = function_with_suffix(&module, "make_packet");
        let relay = function_with_suffix(&module, "relay_packet");
        let forward = function_with_suffix(&module, "forward_packet");
        let receiver = function_with_suffix(&module, "receive_packets");
        let plain = function_with_suffix(&module, "ordinary_packet");
        let (producer, relay, forward) = (
            module.get_function(&producer).unwrap(),
            module.get_function(&relay).unwrap(),
            module.get_function(&forward).unwrap(),
        );
        let caller = module.get_function(&receiver).unwrap();
        fn calls<'ctx>(function: inkwell::values::FunctionValue<'ctx>)
            -> Vec<(inkwell::values::InstructionValue<'ctx>, CallSiteValue<'ctx>)>
        {
            function.get_basic_blocks().into_iter().flat_map(|block| block.get_instructions())
                .filter(|inst| inst.get_opcode() == InstructionOpcode::Call)
                .filter_map(|inst| CallSiteValue::try_from(inst).ok().map(|call| (inst, call))).collect()
        }
        use inkwell::values::AnyValue;
        for (wrapper, callee) in [(relay, producer), (forward, relay)] {
            let wrapper_calls = calls(wrapper);
            let [(inst, forwarded)] = wrapper_calls.as_slice() else { panic!("wrapper must only call its selected callee") };
            assert_eq!(forwarded.get_called_fn_value(), Some(callee));
            assert_eq!(inst.get_operand(3).unwrap().value(), wrapper.get_last_param());
            let relay_ir = wrapper.print_to_string().to_string();
            assert!(relay_ir.contains("define internal { ptr, i32 }"), "{relay_ir}");
            assert!(relay_ir.contains("ret { ptr, i32 } %aggregate_original_base"), "{relay_ir}");
            assert!(!relay_ir.contains("memcpy") && !relay_ir.contains("__wosy_core_free"), "{relay_ir}");
            assert!(!relay_ir.contains("extractvalue") && !relay_ir.contains("insertvalue"), "{relay_ir}");
        }
        let caller_calls = calls(caller);
        let receipts = caller_calls.iter().filter(|(_, call)|
            call.get_called_fn_value().is_some_and(|callee| callee == relay || callee == forward)).collect::<Vec<_>>();
        let [(first_inst, first), (second_inst, second)] = receipts.as_slice() else { panic!("two receiver destinations required") };
        assert_eq!(first.get_called_fn_value(), Some(relay));
        assert_eq!(second.get_called_fn_value(), Some(forward));
        assert_ne!(first_inst.get_operand(3).unwrap().value(), second_inst.get_operand(3).unwrap().value());
        let first_base = first.try_as_basic_value().basic().unwrap();
        let second_base = second.try_as_basic_value().basic().unwrap();
        let releases = caller_calls.iter()
            .filter(|(_, call)| call.get_called_fn_value() == module.get_function("__wosy_core_free"))
            .collect::<Vec<_>>();
        let [(release_first, _), (release_second, _)] = releases.as_slice() else { panic!("two obligations required") };
        fn base_from<'ctx>(release: &inkwell::values::InstructionValue<'ctx>) -> inkwell::values::InstructionValue<'ctx> {
            use inkwell::values::BasicValue;
            release.get_operand(0).unwrap().value().unwrap().as_instruction_value().unwrap()
        }
        let (one, two) = (base_from(release_first), base_from(release_second));
        assert_eq!(one.get_opcode(), InstructionOpcode::ExtractValue);
        assert_eq!(two.get_opcode(), InstructionOpcode::ExtractValue);
        let released = [one.get_operand(0).unwrap().value(), two.get_operand(0).unwrap().value()];
        assert!(released.contains(&Some(first_base)) && released.contains(&Some(second_base)));
        let caller_ir = caller.print_to_string().to_string();
        assert!(caller_ir.contains(&format!("call {{ ptr, i32 }} @{plain}()")), "{caller_ir}");
        fn descriptor_byte_source<'ctx>(
            load: inkwell::values::InstructionValue<'ctx>,
        ) -> Option<(inkwell::values::BasicValueEnum<'ctx>, inkwell::values::BasicValueEnum<'ctx>)> {
            fn through<'ctx>(
                operand: inkwell::values::BasicValueEnum<'ctx>, opcode: InstructionOpcode,
            ) -> Option<inkwell::values::InstructionValue<'ctx>> {
                use inkwell::values::BasicValue;
                let instruction = operand.as_instruction_value()?;
                (instruction.get_opcode() == opcode).then_some(instruction)
            }
            if load.get_opcode() != InstructionOpcode::Load
                || !load.print_to_string().to_string().contains("load i8,")
            {
                return None;
            }
            let address = through(load.get_operand(0)?.value()?, InstructionOpcode::IntToPtr)?;
            let field_load = through(address.get_operand(0)?.value()?, InstructionOpcode::Load)?;
            let field = through(field_load.get_operand(0)?.value()?, InstructionOpcode::GetElementPtr)?;
            let offset = field.get_operand(1)?.value()?.into_int_value();
            if offset.get_zero_extended_constant()? != 0 { return None; }
            let descriptor = through(field.get_operand(0)?.value()?, InstructionOpcode::ExtractValue)?;
            let result = through(descriptor.get_operand(0)?.value()?, InstructionOpcode::InsertValue)?;
            Some((result.get_operand(0)?.value()?, result.get_operand(1)?.value()?))
        }
        let observed = caller.get_basic_blocks().into_iter().flat_map(|block| block.get_instructions())
            .take_while(|instruction| instruction != release_first)
            .filter_map(descriptor_byte_source).collect::<Vec<_>>();
        assert_eq!(observed, [
            (first_base, first_inst.get_operand(3).unwrap().value().unwrap()),
            (second_base, second_inst.get_operand(3).unwrap().value().unwrap()),
        ], "{caller_ir}");
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
fn mixed_result_direct_cross_module_receive_preserves_qualified_owner_identity() {
    let owner_text = r#"%%start
struct Packet { *?u8 data; u64 length; }
Packet stored = { .data = null; .length = 7; };
(u32, Packet)(u64) make_packet = fn(length) {
    u8[length] backing;
    backing[0] = 19;
    *?u8 data = null;
    unsafe { data = &?backing[0]; };
    Packet packet = { .data = data; .length = length; };
    37, packet
};
%%end"#;
    let decoy_text = r#"%%start
struct Packet { *?u8 data; u64 length; }
Packet stored = { .data = null; .length = 7; };
(u32, Packet)(u64) make_packet = fn(length) {
    u32 marker = core.int_trunc<u32>(length);
    marker + 38, stored
};
%%end"#;
    assert_eq!(owner_text.find("make_packet"), decoy_text.find("make_packet"));
    let app_text = r#"%%start
owned = namespace payload "owner.w";
plain = namespace payload "decoy.w";

u64() receive = fn {
    u32 marker, owned.Packet packet = owned.make_packet(2);
    u32 other, plain.Packet ordinary = plain.make_packet(3);
    u8 byte = unsafe { core.load<u8>(packet.data) };
    core.int_extend<u64>(marker) + core.int_extend<u64>(byte) + packet.length +
        core.int_extend<u64>(other) + ordinary.length
};
if (receive() != 106) { core.system_panic(); };
%%end"#;
    let forwarded_text = app_text.replace(
        "plain = namespace payload \"decoy.w\";",
        "plain = namespace payload \"decoy.w\";
relay = namespace payload \"relay.w\";",
    ).replace("owned.make_packet(2)", "relay.forward(2)");
    let relay_text = r#"%%start
owned = namespace payload "owner.w";
(u32, owned.Packet)(u64) forward = fn(count) {
    u32 marker, owned.Packet packet = owned.make_packet(count);
    marker, packet
};
%%end"#;
    let second_text = r#"%%start
owned = namespace payload "owner.w";
relay = namespace payload "relay.w";
(u32, owned.Packet)(u64) forward_again = fn(count) {
    u32 marker, owned.Packet packet = relay.forward(count);
    marker, packet
};
%%end"#;
    let chained_text = forwarded_text.replace(
        "relay = namespace payload \"relay.w\";",
        "next = namespace payload \"next.w\";",
    ).replace("relay.forward(2)", "next.forward_again(2)");
    for (app_text, relays) in [
        (app_text, &[][..]),
        (forwarded_text.as_str(), &[relay_text][..]),
        (chained_text.as_str(), &[relay_text, second_text][..]),
    ] {
        verify_qualified_mixed_result(app_text, owner_text, decoy_text, relays);
    }
}

fn verify_qualified_mixed_result(app_text: &str, owner_text: &str, decoy_text: &str, relay_texts: &[&str]) {
    let app = source("workbench", "src/cross_mixed.w");
    let owner = source("payload", "src/owner.w");
    let decoy = source("payload", "src/decoy.w");
    let relay = source("payload", "src/relay.w");
    let next = source("payload", "src/next.w");
    for target_layout in [ScalarTargetLayout::NATIVE64, ScalarTargetLayout::WASM32] {
        let mut app_namespaces = vec![namespace("owned", &owner), namespace("plain", &decoy)];
        if relay_texts.len() == 1 {
            app_namespaces.push(namespace("relay", &relay));
        } else if relay_texts.len() == 2 {
            app_namespaces.push(namespace("next", &next));
        }
        let mut inputs = vec![
            (declarations(&app, app_text), app_namespaces),
            (declarations(&owner, owner_text), Vec::new()),
            (declarations(&decoy, decoy_text), Vec::new()),
        ];
        let mut sources = vec![app.clone(), owner.clone(), decoy.clone()];
        if let Some(text) = relay_texts.first() {
            inputs.push((declarations(&relay, text), vec![namespace("owned", &owner)]));
            sources.push(relay.clone());
        }
        if let Some(text) = relay_texts.get(1) {
            inputs.push((declarations(&next, text), vec![namespace("owned", &owner), namespace("relay", &relay)]));
            sources.push(next.clone());
        }
        let prepared = prepare_scalar_project(inputs, sources, ScalarPreparationInputs { target_layout })
            .unwrap_or_else(|error| panic!("{target_layout:?}: {:?}", error.diagnostics));
        let build = analyze_scalar_build(&prepared);
        assert!(build.diagnostics().is_empty(), "{:?}", build.diagnostics());
        let ir = emit_scalar_build_project_llvm(&build).unwrap().to_text();
        let context = Context::create();
        let buffer = MemoryBuffer::create_from_memory_range_copy(ir.as_bytes(), "qualified-mixed-owner");
        let module = context.create_module_from_ir(buffer).unwrap();
        module.verify().unwrap();
        use inkwell::values::AnyValue;
        let owner_suffix = format!("__{}__", owner.path.bytes().map(|b| format!("{b:02x}")).collect::<String>());
        let decoy_suffix = format!("__{}__", decoy.path.bytes().map(|b| format!("{b:02x}")).collect::<String>());
        let make = |path: &str| module.get_functions().find(|function| {
            let name = function.get_name().to_str().unwrap();
            name.contains(path) && name.ends_with("__6d616b655f7061636b6574")
        }).expect("exact namespace callee");
        let owner = make(&owner_suffix);
        let ordinary = make(&decoy_suffix);
        let receiver = module.get_function(&function_with_suffix(&module, "receive")).unwrap().print_to_string().to_string();
        assert_eq!(owner.count_params(), 2);
        assert_eq!(ordinary.count_params(), 1);
        let mut called_owner = owner;
        for name in ["forward", "forward_again"].into_iter().take(relay_texts.len()) {
            let relay = module.get_function(&function_with_suffix(&module, name)).unwrap();
            let relay_ir = relay.print_to_string().to_string();
            let mut calls = relay.get_basic_blocks().into_iter().flat_map(|block| block.get_instructions())
                .filter(|inst| inst.get_opcode() == InstructionOpcode::Call);
            let call = calls.next().expect("exact forwarded producer call");
            assert!(calls.next().is_none(), "{relay_ir}");
            let called = CallSiteValue::try_from(call).unwrap();
            assert_eq!(called.get_called_fn_value(), Some(called_owner));
            assert_eq!(call.get_operand(1).unwrap().value(), relay.get_last_param());
            let returned = relay.get_basic_blocks().into_iter().flat_map(|block| block.get_instructions())
                .find(|inst| inst.get_opcode() == InstructionOpcode::Return).expect("whole pair return");
            assert_eq!(returned.get_operand(0).unwrap().value(), called.try_as_basic_value().basic());
            called_owner = relay;
        }
        for function in [called_owner, ordinary] {
            let target = function.get_name().to_str().unwrap();
            assert_eq!(receiver.matches(&format!("call {{ i32, ptr }} @{target}(")).count(), 1, "{receiver}");
        }
        assert_eq!(receiver.matches("call void @__wosy_core_free(").count(), 1, "{receiver}");
        assert!(receiver.contains("extractvalue { i32, ptr } %aggregate_original_base, 1"), "{receiver}");
        let release = receiver.find("call void @__wosy_core_free(ptr %aggregate_base)").unwrap();
        assert!(receiver[..release].contains("load i8"), "{receiver}");
    }
}

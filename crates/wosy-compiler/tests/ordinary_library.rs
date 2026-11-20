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
    let app = source("workbench", "src/run.w");
    let std = source("std", "src/bootstrap.w");
    let preview = source("std", "src/wasi/preview1.w");
    let text = "%%start\nstd = namespace std \"bootstrap.w\";\nstd.print(\"ordinary\");\n%%end";
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
        assert!(direct_calls(&module, "main")
            .iter()
            .any(|target| target == STD_PRINT));
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

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

        // The ordinary compiler must still reject an incompatible forwarded
        // result; these assertions do not require or claim automatic cleanup.
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

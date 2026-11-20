use super::*;

fn assert_function_result(result: &FunctionReferenceResult, function: &ScalarFunction) {
    assert_eq!(result.cfg, function.reference_cfg);
    assert_eq!(result.arms.len(), function.overload_arms.len());
    for (result, arm) in result.arms.iter().zip(&function.overload_arms) {
        assert_function_result(result, arm);
    }
}

fn assert_module_results(results: &[(usize, FunctionReferenceResult)], items: &[ScalarItem]) {
    for (index, result) in results {
        let ScalarItem::Function(function) = &items[*index] else {
            panic!("actual function ordinal")
        };
        assert_function_result(result, function);
    }
}

#[test]
fn complete_program_analysis_borrows_input_and_matches_legacy_attachment_after_all_stages() {
    for target in [ScalarTargetLayout::WASM32, ScalarTargetLayout::NATIVE64] {
        for (text, succeeds, scratch_copies) in [
            (TEXT, true, 0),
            ("%%start\n*u8(*u8) forward = fn(input) { input };\n*u8(*u8) relay = fn(input) { forward(input) };\n%%end", true, 0),
            ("%%start\nforward = overload { generic T; T(T) => fn(input) { input }; };\nu8(u8) caller = fn(input) { u8 first = forward(input); forward(first) };\n%%end", true, 2),
            ("%%start\nstruct Payload { u8 value; }\nPayload(Payload) consume = fn(value) { next = value; next; next };\n%%end", false, 0),
        ] {
            let parsed = parse_source(source(), text.into(), &[]);
            assert!(parsed.diagnostics.is_empty(), "{:?}", parsed.diagnostics);
            let (typed, occurrences) = typecheck_scalar_program_with_occurrences(derive_scalar_declarations_from_cst(parsed.result.canonical_cst()), target);
            assert!(typed.diagnostics.is_empty(), "{:?}", typed.diagnostics);
            let program = typed.program;
            let before = program.clone();
            let sites = resolve_program_call_sites_with_occurrences(&program, occurrences).unwrap();
            let scope = layout::observations::Scope::new();
            let stages = stage_observations::Scope::new();
            let results = analyze_program_references(&program, &sites);
            let counts = scope.counts();
            let stage_counts = stages.counts();
            assert_eq!(counts.program_copies, 0);
            assert_eq!(counts.module_copies, 0);
            assert_eq!(counts.module_constructions, 0);
            assert_eq!(counts.cfg_copies, scratch_copies);
            assert_eq!(stage_counts, stage_observations::Counts { call_sites: 0, reference_origins: 1, output_contracts: 1, copy_specialization: usize::from(succeeds) });
            drop(stages);
            drop(scope);
            assert_eq!(program, before);
            assert_eq!(results.errors.is_empty(), succeeds);
            let mut installed = program.clone();
            let scope = layout::observations::Scope::new();
            let stages = stage_observations::Scope::new();
            let errors = record_program_reference_origins(&mut installed, &sites);
            assert_eq!(scope.counts(), counts);
            assert_eq!(stages.counts(), stage_counts);
            drop(stages);
            drop(scope);
            assert_eq!(results.errors, errors);
            assert_module_results(&results.updates, &installed.items);
            let mut moved = before;
            let errors = results.install(&mut moved);
            assert_eq!(moved, installed);
            let public = derive_scalar_program_with_layout(&parsed.result, target);
            assert_eq!(public.program, installed);
            assert_eq!(public.diagnostics, errors.iter().map(ReferenceAnalysisError::diagnostic).collect::<Vec<_>>());
        }
    }
}

fn module(path: &str, text: &str, target: ScalarTargetLayout) -> ScalarModule {
    let identity =
        SourceIdentity::new("project".into(), "package".into(), path.into(), "r1".into());
    let parsed = parse_source(identity, text.into(), &[]);
    assert!(parsed.diagnostics.is_empty(), "{:?}", parsed.diagnostics);
    let declarations = derive_scalar_declarations_from_cst(parsed.result.canonical_cst());
    assert!(
        declarations.diagnostics.is_empty(),
        "{:?}",
        declarations.diagnostics
    );
    let mut module = ScalarModule::new(declarations.source, declarations.items, Vec::new());
    module.structs = declarations.structs;
    module.enums = declarations.enums;
    module.target_layout = target;
    module
}

#[test]
fn complete_project_analysis_preserves_ordinals_specializations_and_ordered_partial_errors() {
    for target in [ScalarTargetLayout::WASM32, ScalarTargetLayout::NATIVE64] {
        for failing in [false, true] {
            let library = module("library.w", "%%start\nforward = overload { generic T; T(T) => fn(input) { input }; };\n*u8(*u8) checked = fn(input) { input };\n%%end", target);
            let mut main = module(
                "main.w",
                if failing {
                    "%%start\nlibrary = namespace package \"library.w\";\nstruct Payload { u8 value; }\nPayload(Payload) consume = fn(value) { next = value; next; next };\nPayload(Payload) consume_again = fn(value) { next = value; next; next };\nu8(u8) copied = fn(input) { library.forward(input) };\n%%end"
                } else {
                    "%%start\nlibrary = namespace package \"library.w\";\nu8(u8) copied = fn(input) { library.forward(input) };\n*u8(*u8) relay = fn(input) { library.checked(input) };\n%%end"
                },
                target,
            );
            main.namespace_bindings.push(ScalarNamespaceBinding {
                binding: "library".into(),
                target: library.source.clone(),
                span: ByteSpan::new(0, 0),
            });
            let mut empty = ScalarModule::new(
                SourceIdentity::new(
                    "project".into(),
                    "package".into(),
                    "empty.w".into(),
                    "r1".into(),
                ),
                Vec::new(),
                Vec::new(),
            );
            empty.target_layout = target;
            let modules = vec![empty, main, library];
            let order = modules.iter().map(|module| module.source.clone()).collect();
            let (typed, occurrences) =
                typecheck_scalar_project_with_occurrences(ScalarProject::new(modules, order));
            assert!(typed.diagnostics.is_empty(), "{:?}", typed.diagnostics);
            let modules = typed.project.modules;
            let before = modules.clone();
            let calls = resolve_project_call_sites_with_occurrences(&modules, occurrences);
            let before_calls = calls.modules.clone();
            let scope = layout::observations::Scope::new();
            let stages = stage_observations::Scope::new();
            let results = analyze_project_references(&modules, &calls);
            let counts = scope.counts();
            let stage_counts = stages.counts();
            assert_eq!(counts.program_copies, 0);
            assert_eq!(counts.module_copies, 0);
            assert_eq!(counts.module_constructions, 0);
            assert_eq!(counts.cfg_copies, if failing { 0 } else { 1 });
            assert_eq!(
                stage_counts,
                stage_observations::Counts {
                    call_sites: 0,
                    reference_origins: 3,
                    output_contracts: 1,
                    copy_specialization: if failing { 0 } else { 3 }
                }
            );
            drop(stages);
            drop(scope);
            let scope = layout::observations::Scope::new();
            let stages = stage_observations::Scope::new();
            let repeated = analyze_project_references(&modules, &calls);
            assert_eq!(scope.counts(), counts);
            assert_eq!(stages.counts(), stage_counts);
            drop(stages);
            drop(scope);
            assert_project_reference_results(&results, &repeated);
            assert_eq!(calls.modules, before_calls);
            assert_eq!(modules, before);
            assert_eq!(results.errors.is_empty(), !failing);
            if failing {
                assert_eq!(results.errors.len(), 2);
                let spans = results
                    .errors
                    .iter()
                    .map(|error| match error {
                        ReferenceAnalysisError::ConflictingPlace { source_span, .. } => {
                            source_span.range.start
                        }
                        _ => panic!("ordered ownership errors"),
                    })
                    .collect::<Vec<_>>();
                assert!(spans[0] < spans[1]);
            }
            assert_eq!(
                results
                    .updates
                    .iter()
                    .map(|(index, _)| *index)
                    .collect::<Vec<_>>(),
                vec![0, 1, 2]
            );
            let mut installed = modules.clone();
            let scope = layout::observations::Scope::new();
            let stages = stage_observations::Scope::new();
            let errors = record_project_reference_origins(&mut installed, &calls);
            assert_eq!(scope.counts(), counts);
            assert_eq!(stages.counts(), stage_counts);
            drop(stages);
            drop(scope);
            assert_eq!(results.errors, errors);
            for (index, updates) in &results.updates {
                assert_module_results(updates, &installed[*index].items);
            }
            let mut moved = modules;
            assert_eq!(results.install(&mut moved), errors);
            assert_eq!(moved, installed);
        }
    }
}

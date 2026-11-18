use super::*;
use crate::parse_source;
use reference::stage_observations::{Counts, Scope};

mod occurrence_inventory;

mod neutral_assignments;

mod neutral_branches;

fn parsed(text: &str) -> wosy_syntax::ParseResult {
    let identity = SourceIdentity::new(
        "project".into(),
        "package".into(),
        "phase.w".into(),
        "r1".into(),
    );
    let parsed = parse_source(identity, text.into(), &[]);
    assert!(parsed.diagnostics.is_empty(), "{:?}", parsed.diagnostics);
    parsed.result
}

fn raw_project(
    declarations: ScalarSourceDeclarations,
    target: ScalarTargetLayout,
) -> ScalarProject {
    assert!(declarations.diagnostics.is_empty());
    let order = vec![declarations.source.clone()];
    let mut module = ScalarModule::new(declarations.source, declarations.items, Vec::new());
    module.structs = declarations.structs;
    module.enums = declarations.enums;
    module.target_layout = target;
    ScalarProject::new(vec![module], order)
}

fn assert_query_stages(counts: Counts, valid: bool) {
    assert_eq!(
        counts,
        Counts {
            call_sites: 0,
            reference_origins: 1,
            output_contracts: 1,
            copy_specialization: usize::from(valid),
        }
    );
}

#[test]
fn standalone_preparation_retains_static_inputs_for_repeatable_owned_queries_and_legacy_output() {
    for target in [ScalarTargetLayout::WASM32, ScalarTargetLayout::NATIVE64] {
        for (text, valid) in [
            ("%%start\nu8(u8) identity = fn(input) { next = input; next };\nu8(u8) caller = fn(input) { identity(input) };\n%%end", true),
            ("%%start\nstruct Payload { u8 value; }\nPayload(Payload) consume = fn(input) { next = input; next; next };\n%%end", false),
        ] {
            let parsed = parsed(text);
            let declarations = derive_scalar_declarations_from_cst(parsed.canonical_cst());
            let scope = Scope::new();
            let copies = layout::observations::Scope::new();
            let prepared = PreparedProgram::new(declarations, target).unwrap_or_else(|error| panic!("{:?}", error.diagnostics));
            assert_eq!(scope.counts(), Counts { call_sites: 1, ..Counts::default() });
            assert_eq!(copies.counts().program_copies, 0);
            assert_eq!(copies.counts().module_copies, 0);
            assert_eq!(copies.counts().cfg_copies, 0);
            drop(copies);
            drop(scope);
            let before = prepared.program.clone();
            assert!(!before.resolved_assignment_outputs.is_empty());
            let mut installed = Vec::new();
            let mut ordered_errors = Vec::new();
            for _ in 0..2 {
                let scope = Scope::new();
                let copies = layout::observations::Scope::new();
                let result = prepared.analyze().unwrap();
                assert_query_stages(scope.counts(), valid);
                assert_eq!(copies.counts().program_copies, 0);
                assert_eq!(copies.counts().module_copies, 0);
                assert_eq!(copies.counts().cfg_copies, 0);
                drop(copies);
                drop(scope);
                assert_eq!(prepared.program, before);
                let mut output = before.clone();
                let errors = result.install(&mut output);
                assert_eq!(errors.is_empty(), valid);
                installed.push(output);
                ordered_errors.push(errors);
            }
            assert_eq!(installed[0], installed[1]);
            assert_eq!(ordered_errors[0], ordered_errors[1]);
            let finished = prepared.finish();
            assert_eq!(finished.program, installed[0]);
            assert_eq!(finished.diagnostics, ordered_errors[0].iter().map(ReferenceAnalysisError::diagnostic).collect::<Vec<_>>());
            let public = derive_scalar_program_from_cst_with_layout(parsed.canonical_cst(), target);
            assert_eq!(public.program, finished.program);
            assert_eq!(public.diagnostics, finished.diagnostics);
        }
    }
}

#[test]
fn project_preparation_retains_static_inputs_for_repeatable_owned_queries_and_legacy_output() {
    for target in [ScalarTargetLayout::WASM32, ScalarTargetLayout::NATIVE64] {
        for (text, valid) in [
            ("%%start\nu8(u8) identity = fn(input) { next = input; next };\nu8(u8) caller = fn(input) { identity(input) };\n%%end", true),
            ("%%start\nstruct Payload { u8 value; }\nPayload(Payload) consume = fn(input) { next = input; next; next };\n%%end", false),
        ] {
            let parsed = parsed(text);
            let raw = raw_project(derive_scalar_declarations_from_cst(parsed.canonical_cst()), target);
            let public_input = raw.clone();
            let scope = Scope::new();
            let copies = layout::observations::Scope::new();
            let prepared = PreparedProject::new(raw).unwrap_or_else(|error| panic!("{:?}", error.diagnostics));
            assert_eq!(scope.counts(), Counts { call_sites: 1, ..Counts::default() });
            assert_eq!(copies.counts().program_copies, 0);
            assert_eq!(copies.counts().module_copies, 0);
            assert_eq!(copies.counts().cfg_copies, 0);
            drop(copies);
            drop(scope);
            let before = prepared.project.clone();
            assert!(!before.modules[0].resolved_assignment_outputs.is_empty());
            let mut installed = Vec::new();
            let mut ordered_errors = Vec::new();
            for _ in 0..2 {
                let scope = Scope::new();
                let copies = layout::observations::Scope::new();
                let result = prepared.analyze();
                assert_query_stages(scope.counts(), valid);
                assert_eq!(copies.counts().program_copies, 0);
                assert_eq!(copies.counts().module_copies, 0);
                assert_eq!(copies.counts().cfg_copies, 0);
                drop(copies);
                drop(scope);
                assert_eq!(prepared.project, before);
                let mut output = before.clone();
                let errors = result.install(&mut output.modules);
                assert_eq!(errors.is_empty(), valid);
                installed.push(output);
                ordered_errors.push(errors);
            }
            assert_eq!(installed[0], installed[1]);
            assert_eq!(ordered_errors[0], ordered_errors[1]);
            let finished = prepared.finish();
            assert_eq!(finished.project, installed[0]);
            assert_eq!(finished.diagnostics, ordered_errors[0].iter().map(ReferenceAnalysisError::diagnostic).collect::<Vec<_>>());
            let public = validate_scalar_project(public_input);
            assert_eq!(public.project, finished.project);
            assert_eq!(public.diagnostics, finished.diagnostics);
        }
    }
}

#[test]
fn static_rejection_returns_original_validation_with_zero_lookup_or_ownership_entries() {
    let text = "%%start\ni32(i32) bad = fn(input) { bool next = input; next };\n%%end";
    for target in [ScalarTargetLayout::WASM32, ScalarTargetLayout::NATIVE64] {
        let parsed = parsed(text);
        let expected = typecheck_scalar_program(
            derive_scalar_declarations_from_cst(parsed.canonical_cst()),
            target,
        );
        assert!(!expected.diagnostics.is_empty());
        let declarations = derive_scalar_declarations_from_cst(parsed.canonical_cst());
        let scope = Scope::new();
        let Err(actual) = PreparedProgram::new(declarations, target) else {
            panic!("static rejection")
        };
        assert_eq!(scope.counts(), Counts::default());
        drop(scope);
        assert_eq!(actual.program, expected.program);
        assert_eq!(actual.diagnostics, expected.diagnostics);
        let raw = raw_project(
            derive_scalar_declarations_from_cst(parsed.canonical_cst()),
            target,
        );
        let expected = typecheck_scalar_project(raw.clone());
        let scope = Scope::new();
        let Err(actual) = PreparedProject::new(raw) else {
            panic!("static project rejection")
        };
        assert_eq!(scope.counts(), Counts::default());
        drop(scope);
        assert_eq!(actual.project, expected.project);
        assert_eq!(actual.diagnostics, expected.diagnostics);
    }
}

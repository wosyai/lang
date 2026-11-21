use super::*;
use crate::parse_source;
use reference::stage_observations::{Counts, Scope};

fn source() -> SourceIdentity {
    SourceIdentity::new(
        "project".into(),
        "package".into(),
        "standalone.w".into(),
        "r1".into(),
    )
}

fn declarations(text: &str) -> ScalarSourceDeclarations {
    let parsed = parse_source(source(), text.into(), &[]);
    assert!(parsed.diagnostics.is_empty(), "{:?}", parsed.diagnostics);
    let declarations = derive_scalar_declarations_from_cst(parsed.result.canonical_cst());
    assert!(
        declarations.diagnostics().is_empty(),
        "{:?}",
        declarations.diagnostics()
    );
    declarations
}

fn raw_project(
    declarations: ScalarSourceDeclarations,
    target_layout: ScalarTargetLayout,
) -> ScalarProject {
    let ScalarSourceDeclarations {
        source,
        items,
        structs,
        enums,
        diagnostics,
    } = declarations;
    assert!(diagnostics.is_empty());
    let initialization_order = vec![source.clone()];
    let mut module = ScalarModule::new(source, items, Vec::new());
    module.structs = structs;
    module.enums = enums;
    module.target_layout = target_layout;
    ScalarProject::new(vec![module], initialization_order)
}

fn assert_empty_function_cfg(function: &ScalarFunction) {
    assert!(function.reference_cfg.facts.is_empty());
    assert!(function.reference_cfg.points.is_empty());
    assert!(function.reference_cfg.initial_points.is_empty());
    assert!(function.reference_cfg.allocations.is_empty());
    assert!(function.reference_cfg.specialized_policies.is_empty());
    for arm in &function.overload_arms {
        assert_empty_function_cfg(arm);
    }
}

fn assert_empty_cfgs(program: &ScalarProgram) {
    for item in &program.items {
        if let ScalarItem::Function(function) = item {
            assert_empty_function_cfg(function);
        }
    }
}

fn use_static_cfgs(typed: &ScalarFunction, built: &mut ScalarFunction) {
    assert_empty_function_cfg(typed);
    built.reference_cfg = typed.reference_cfg.clone();
    assert_eq!(typed.overload_arms.len(), built.overload_arms.len());
    for (typed, built) in typed.overload_arms.iter().zip(&mut built.overload_arms) {
        use_static_cfgs(typed, built);
    }
}

fn assert_same_static_structure(typed: &ScalarProgram, built: &ScalarProgram) {
    let mut comparable = built.clone();
    assert_eq!(typed.items.len(), comparable.items.len());
    for (typed, built) in typed.items.iter().zip(&mut comparable.items) {
        if let (ScalarItem::Function(typed), ScalarItem::Function(built)) = (typed, built) {
            use_static_cfgs(typed, built);
        }
    }
    assert_eq!(typed, &comparable);
}

#[test]
fn valid_standalone_composition_preserves_complete_static_structure_and_enters_all_stages_once() {
    let text = "%%start\nstruct Payload { u8 value; }\nenum Marker { ready; }\ni32(i32) identity = fn(value) { next = value; next };\n%%end";
    for target in [ScalarTargetLayout::WASM32, ScalarTargetLayout::NATIVE64] {
        let extracted = declarations(text);
        let scope = Scope::new();
        let typed = typecheck_scalar_program(extracted, target);
        assert_eq!(scope.counts(), Counts::default());
        drop(scope);
        assert!(typed.diagnostics.is_empty(), "{:?}", typed.diagnostics);
        assert_empty_cfgs(&typed.program);
        assert_eq!(typed.program.resolved_assignment_outputs.len(), 1);
        let parsed = parse_source(source(), text.into(), &[]);
        assert!(parsed.diagnostics.is_empty());
        let scope = Scope::new();
        let public = derive_scalar_program_with_layout(&parsed.result, target);
        assert_eq!(
            scope.counts(),
            Counts {
                call_sites: 1,
                reference_origins: 1,
                output_contracts: 1,
                copy_specialization: 1
            }
        );
        drop(scope);
        assert_eq!(public.diagnostics, typed.diagnostics);
        let ScalarItem::Function(completed) = &public.program.items[0] else {
            panic!("identity")
        };
        assert!(!completed.reference_cfg.points.is_empty());
        assert_same_static_structure(&typed.program, &public.program);
    }
}

#[test]
fn standalone_static_rejection_keeps_ordered_diagnostics_discarded_facts_and_zero_public_queries() {
    let text = "%%start\ni32(i32) good = fn(value) { next = value; next };\ni32(bool) bad = fn(value) { value };\nbool(i32) other = fn(value) { value };\n%%end";
    for target in [ScalarTargetLayout::WASM32, ScalarTargetLayout::NATIVE64] {
        let extracted = declarations(text);
        let expected = extracted.items[1..]
            .iter()
            .map(|item| {
                let ScalarItem::Function(function) = item else {
                    panic!("invalid function")
                };
                let message = "expression type does not match expected type";
                Diagnostic {
                    code: "B0003".into(),
                    severity: DiagnosticSeverity::Error,
                    message: message.into(),
                    labels: vec![DiagnosticLabel {
                        kind: DiagnosticLabelKind::Primary,
                        span: SourceSpan::new(source(), function.body.span),
                        message: message.into(),
                    }],
                    notes: Vec::new(),
                }
            })
            .collect::<Vec<_>>();
        assert_eq!(expected.len(), 2);
        let scope = Scope::new();
        let typed = typecheck_scalar_program(extracted, target);
        assert_eq!(scope.counts(), Counts::default());
        drop(scope);
        assert_eq!(typed.diagnostics, expected);
        assert!(typed.program.resolved_assignment_outputs.is_empty());
        assert_empty_cfgs(&typed.program);
        let parsed = parse_source(source(), text.into(), &[]);
        assert!(parsed.diagnostics.is_empty());
        let scope = Scope::new();
        let public = derive_scalar_program_with_layout(&parsed.result, target);
        assert_eq!(scope.counts(), Counts::default());
        drop(scope);
        assert_eq!(public.diagnostics, expected);
        assert_eq!(public.program, typed.program);
    }
}

#[test]
fn moved_local_keeps_shared_assignment_key_map_and_exact_inherited_query_diagnostics() {
    let text = "%%start\nstruct Payload { u8 value; }\nenum Marker { ready; }\nPayload(Payload) consume = fn(value) { next = value; next; next };\n%%end";
    for target in [ScalarTargetLayout::WASM32, ScalarTargetLayout::NATIVE64] {
        let extracted = declarations(text);
        let scope = Scope::new();
        let typed = typecheck_scalar_program(extracted, target);
        assert_eq!(scope.counts(), Counts::default());
        drop(scope);
        assert!(typed.diagnostics.is_empty(), "{:?}", typed.diagnostics);
        assert_empty_cfgs(&typed.program);
        let ScalarItem::Function(function) = &typed.program.items[0] else {
            panic!("consume")
        };
        let ScalarBlockItem::Assignment(assignment) = &function.body.items[0] else {
            panic!("assignment")
        };
        let key = ResolvedAssignmentOutputId {
            source: source(),
            function_span: function.span,
            target_span: assignment.targets[0].target_span,
            output_index: 0,
        };
        assert_eq!(typed.program.resolved_assignment_outputs.len(), 1);
        assert_eq!(
            typed.program.resolved_assignment_outputs.get(&key),
            Some(&ScalarType::Struct(ScalarStructId {
                source: source(),
                index: 0
            }))
        );
        let raw = raw_project(declarations(text), target);
        let scope = Scope::new();
        let project_typed = typecheck_scalar_project(raw);
        assert_eq!(scope.counts(), Counts::default());
        drop(scope);
        assert!(
            project_typed.diagnostics.is_empty(),
            "{:?}",
            project_typed.diagnostics
        );
        assert_eq!(
            project_typed.project.modules[0].resolved_assignment_outputs,
            typed.program.resolved_assignment_outputs
        );
        assert_eq!(
            project_typed.project.modules[0]
                .resolved_assignment_outputs
                .get(&key),
            typed.program.resolved_assignment_outputs.get(&key)
        );
        let parsed = parse_source(source(), text.into(), &[]);
        assert!(parsed.diagnostics.is_empty());
        let scope = Scope::new();
        let public = derive_scalar_program_with_layout(&parsed.result, target);
        assert_eq!(
            scope.counts(),
            Counts {
                call_sites: 1,
                reference_origins: 1,
                output_contracts: 1,
                copy_specialization: 0
            }
        );
        drop(scope);
        let unavailable = text.rfind("next };").unwrap() as u32;
        let moved = text.find("next; next").unwrap() as u32;
        let message = "read or move of unavailable place";
        assert_eq!(
            public.diagnostics,
            vec![Diagnostic {
                code: "B0003".into(),
                severity: DiagnosticSeverity::Error,
                message: message.into(),
                labels: vec![
                    DiagnosticLabel {
                        kind: DiagnosticLabelKind::Primary,
                        span: SourceSpan::new(
                            source(),
                            ByteSpan::new(unavailable, unavailable + 4)
                        ),
                        message: message.into()
                    },
                    DiagnosticLabel {
                        kind: DiagnosticLabelKind::Secondary,
                        span: SourceSpan::new(source(), ByteSpan::new(moved, moved + 4)),
                        message: "origin of conflicting place or loan".into()
                    },
                ],
                notes: Vec::new(),
            }]
        );
        assert_eq!(
            public.program.resolved_assignment_outputs,
            typed.program.resolved_assignment_outputs
        );
        assert_same_static_structure(&typed.program, &public.program);
    }
}

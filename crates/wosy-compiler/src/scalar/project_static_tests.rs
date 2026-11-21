use super::*;
use crate::parse_source;
use reference::stage_observations::{Counts, Scope};

fn source() -> SourceIdentity {
    SourceIdentity::new(
        "project".into(),
        "package".into(),
        "static.w".into(),
        "r1".into(),
    )
}

fn raw_project(text: &str, target_layout: ScalarTargetLayout) -> ScalarProject {
    let parsed = parse_source(source(), text.into(), &[]);
    assert!(parsed.diagnostics.is_empty(), "{:?}", parsed.diagnostics);
    let ScalarSourceDeclarations {
        source,
        items,
        structs,
        enums,
        diagnostics,
    } = derive_scalar_declarations_from_cst(parsed.result.canonical_cst());
    assert!(diagnostics.is_empty(), "{diagnostics:?}");
    let initialization_order = vec![source.clone()];
    let mut module = ScalarModule::new(source, items, Vec::new());
    module.structs = structs;
    module.enums = enums;
    module.target_layout = target_layout;
    ScalarProject::new(vec![module], initialization_order)
}

fn assert_empty_cfgs(project: &ScalarProject) {
    for module in &project.modules {
        for item in &module.items {
            if let ScalarItem::Function(function) = item {
                assert!(function.reference_cfg.facts.is_empty());
                assert!(function.reference_cfg.points.is_empty());
                assert!(function.reference_cfg.initial_points.is_empty());
                assert!(function.reference_cfg.allocations.is_empty());
                assert!(function.reference_cfg.specialized_policies.is_empty());
            }
        }
    }
}

#[test]
fn private_static_entry_publishes_typed_assignment_without_querying_and_public_entry_preserves_ownership_failure(
) {
    let text = "%%start\nstruct Payload { u8 value; }\nenum Marker { ready; }\nPayload(Payload) consume = fn(value) { next = value; next; next };\n%%end";
    for target in [ScalarTargetLayout::WASM32, ScalarTargetLayout::NATIVE64] {
        let raw = raw_project(text, target);
        assert_empty_cfgs(&raw);
        let scope = Scope::new();
        let static_result = typecheck_scalar_project(raw);
        assert_eq!(scope.counts(), Counts::default());
        drop(scope);
        assert!(
            static_result.diagnostics.is_empty(),
            "{:?}",
            static_result.diagnostics
        );
        assert_empty_cfgs(&static_result.project);
        let module = &static_result.project.modules[0];
        assert_eq!(module.source, source());
        assert_eq!(module.target_layout, target);
        assert_eq!(module.structs.len(), 1);
        assert_eq!(
            module.structs[0].id,
            ScalarStructId {
                source: source(),
                index: 0
            }
        );
        assert_eq!(module.enums.len(), 1);
        assert_eq!(
            module.enums[0].id,
            ScalarEnumId {
                source: source(),
                index: 0
            }
        );
        let ScalarItem::Function(function) = &module.items[0] else {
            panic!("consume")
        };
        let ScalarBlockItem::Assignment(assignment) = &function.body.items[0] else {
            panic!("assignment")
        };
        assert_eq!(module.resolved_assignment_outputs.len(), 1);
        let key = ResolvedAssignmentOutputId {
            source: source(),
            function_span: function.span,
            target_span: assignment.targets[0].target_span,
            output_index: 0,
        };
        assert_eq!(
            module.resolved_assignment_outputs.get(&key),
            Some(&ScalarType::Struct(module.structs[0].id.clone()))
        );

        let raw = raw_project(text, target);
        let scope = Scope::new();
        let public = validate_scalar_project(raw);
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
            public.project.modules[0].resolved_assignment_outputs,
            module.resolved_assignment_outputs
        );
    }
}

#[test]
fn valid_public_composition_enters_each_inherited_reference_stage_once() {
    let text = "%%start\ni32(i32) identity = fn(value) { value };\n%%end";
    for target in [ScalarTargetLayout::WASM32, ScalarTargetLayout::NATIVE64] {
        let raw = raw_project(text, target);
        let scope = Scope::new();
        let static_result = typecheck_scalar_project(raw);
        assert!(
            static_result.diagnostics.is_empty(),
            "{:?}",
            static_result.diagnostics
        );
        assert_eq!(scope.counts(), Counts::default());
        assert_empty_cfgs(&static_result.project);
        drop(scope);
        let raw = raw_project(text, target);
        let scope = Scope::new();
        let public = validate_scalar_project(raw);
        assert!(public.diagnostics.is_empty(), "{:?}", public.diagnostics);
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
        let ScalarItem::Function(completed) = &public.project.modules[0].items[0] else {
            panic!("completed identity")
        };
        let ScalarItem::Function(typed) = &static_result.project.modules[0].items[0] else {
            panic!("typed identity")
        };
        assert_eq!(completed.signature, typed.signature);
        assert_eq!(completed.body, typed.body);
        assert!(!completed.reference_cfg.points.is_empty());
    }
}

#[test]
fn static_rejection_preserves_exact_diagnostics_and_public_zero_query_guard() {
    let text = "%%start\ni32(i32) good = fn(value) { next = value; next };\ni32(bool) bad = fn(value) { value };\n%%end";
    for target in [ScalarTargetLayout::WASM32, ScalarTargetLayout::NATIVE64] {
        let raw = raw_project(text, target);
        let ScalarItem::Function(bad) = &raw.modules[0].items[1] else {
            panic!("bad")
        };
        let span = bad.body.span;
        let scope = Scope::new();
        let static_result = typecheck_scalar_project(raw);
        assert_eq!(scope.counts(), Counts::default());
        drop(scope);
        let message = "expression type does not match expected type";
        assert_eq!(
            static_result.diagnostics,
            vec![Diagnostic {
                code: "B0003".into(),
                severity: DiagnosticSeverity::Error,
                message: message.into(),
                labels: vec![DiagnosticLabel {
                    kind: DiagnosticLabelKind::Primary,
                    span: SourceSpan::new(source(), span),
                    message: message.into()
                }],
                notes: Vec::new(),
            }]
        );
        assert!(static_result.project.modules[0]
            .resolved_assignment_outputs
            .is_empty());
        assert_empty_cfgs(&static_result.project);
        let raw = raw_project(text, target);
        let scope = Scope::new();
        let public = validate_scalar_project(raw);
        assert_eq!(scope.counts(), Counts::default());
        drop(scope);
        assert_eq!(public.diagnostics, static_result.diagnostics);
        assert_eq!(public.project, static_result.project);
    }
}

#[test]
fn project_static_lookup_rejection_preserves_ordered_mixed_diagnostics() {
    let lookup = "%%start\nstruct Callback { i32(i32) run; }\ni32(i32) identity = fn(input) { input };\nCallback callback = { .run = identity; };\ni32(i32) invoke = fn(input) { callback.run(input) };\n%%end";
    let origin = "%%start\nstruct Payload { i32 value; }\nPayload(Payload[2], u64) take = fn(values, index) { values[index] };\n%%end";
    let valid = "%%start\ni32(i32) identity = fn(input) { input };\n%%end";
    let inputs = [
        ("z-origin.w", origin),
        ("y-lookup.w", lookup),
        ("x-valid.w", valid),
        ("b-origin.w", origin),
        ("a-lookup.w", lookup),
    ];
    let sources = inputs.map(|(path, _)| {
        SourceIdentity::new("project".into(), "package".into(), path.into(), "r1".into())
    });
    let declarations = || {
        sources.iter().zip(inputs).map(|(source, (_, text))| {
            let parsed = parse_source(source.clone(), text.into(), &[]);
            assert!(parsed.diagnostics.is_empty(), "{:?}", parsed.diagnostics);
            let declarations = derive_scalar_declarations_from_cst(parsed.result.canonical_cst());
            assert!(
                declarations.diagnostics.is_empty(),
                "{:?}",
                declarations.diagnostics
            );
            declarations
        })
    };
    let order = sources.iter().rev().cloned().collect::<Vec<_>>();
    let span = |source: usize, token: &str, start: usize| {
        let start = start as u32;
        SourceSpan::new(
            sources[source].clone(),
            ByteSpan::new(start, start + token.len() as u32),
        )
    };
    let lookup_error = |source| {
        let call = "callback.run(input)";
        Diagnostic {
            code: "B0003".into(),
            severity: DiagnosticSeverity::Error,
            message: "missing resolved call target".into(),
            labels: vec![DiagnosticLabel {
                kind: DiagnosticLabelKind::Primary,
                span: span(source, call, lookup.find(call).unwrap()),
                message: "missing resolved call target".into(),
            }],
            notes: Vec::new(),
        }
    };
    let origin_error = |source| Diagnostic {
        code: "B0003".into(),
        severity: DiagnosticSeverity::Error,
        message: "array elements cannot be moved individually".into(),
        labels: vec![DiagnosticLabel {
            kind: DiagnosticLabelKind::Primary,
            span: span(
                source,
                "values[index] ",
                origin.find("values[index]").unwrap(),
            ),
            message: "array elements cannot be moved individually".into(),
        }],
        notes: Vec::new(),
    };
    let expected = vec![
        origin_error(0),
        lookup_error(1),
        origin_error(3),
        lookup_error(4),
    ];
    for target_layout in [ScalarTargetLayout::WASM32, ScalarTargetLayout::NATIVE64] {
        let stages = Scope::new();
        let Err(rejected) = prepare_scalar_project(
            declarations()
                .map(|declarations| (declarations, Vec::new()))
                .collect(),
            order.clone(),
            ScalarPreparationInputs { target_layout },
        ) else {
            panic!("static lookup must reject preparation");
        };
        let preparation_counts = Counts {
            call_sites: inputs.len(),
            ..Counts::default()
        };
        assert_eq!(stages.counts(), preparation_counts);
        drop(stages);
        assert_eq!(rejected.diagnostics, vec![lookup_error(1), lookup_error(4)]);
        assert_eq!(rejected.project.initialization_order, order);
        assert_empty_cfgs(&rejected.project);
        let rejected_modules = rejected.project.modules.clone();

        let stages = Scope::new();
        assert_eq!(rejected.into_ordered_diagnostics(), expected);
        let diagnostic_counts = Counts {
            reference_origins: 3,
            ..Counts::default()
        };
        assert_eq!(stages.counts(), diagnostic_counts);
        drop(stages);

        let modules = declarations()
            .map(
                |ScalarSourceDeclarations {
                     source,
                     items,
                     structs,
                     enums,
                     ..
                 }| {
                    let mut module = ScalarModule::new(source, items, Vec::new());
                    module.structs = structs;
                    module.enums = enums;
                    module.target_layout = target_layout;
                    module.layouts = ScalarTargetLayouts::new(target_layout);
                    module
                },
            )
            .collect();
        let stages = Scope::new();
        let attached = validate_scalar_project(ScalarProject::new(modules, order.clone()));
        assert_eq!(
            stages.counts(),
            Counts {
                call_sites: preparation_counts.call_sites,
                ..diagnostic_counts
            }
        );
        assert_eq!(attached.diagnostics, expected);
        for index in [1, 4] {
            assert_eq!(attached.project.modules[index], rejected_modules[index]);
        }
        for (module, item) in [(0, 0), (2, 0), (3, 0)] {
            let ScalarItem::Function(function) = &attached.project.modules[module].items[item]
            else {
                panic!("callable at module {module}");
            };
            assert!(!function.reference_cfg.points.is_empty(), "module {module}");
        }
    }
}

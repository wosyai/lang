use super::*;
use crate::parse_source;

#[cfg(test)]
mod project_origins;

mod output_contracts;

mod specialization;

mod call_resolution;

mod call_projection;

mod binding_outputs;

mod block_outputs;

mod assignment_outputs;

mod assignment_destinations;

mod dereference_points;

mod complete_results;

fn assert_project_reference_results(
    left: &ProjectReferenceResults,
    right: &ProjectReferenceResults,
) {
    fn assert_function(left: &FunctionReferenceResult, right: &FunctionReferenceResult) {
        assert_eq!(left.cfg, right.cfg);
        assert_eq!(left.arms.len(), right.arms.len());
        for (left, right) in left.arms.iter().zip(&right.arms) {
            assert_function(left, right);
        }
    }
    assert_eq!(left.errors, right.errors);
    assert_eq!(left.updates.len(), right.updates.len());
    for ((left_module, left), (right_module, right)) in left.updates.iter().zip(&right.updates) {
        assert_eq!(left_module, right_module);
        assert_eq!(left.len(), right.len());
        for ((left_item, left), (right_item, right)) in left.iter().zip(right) {
            assert_eq!(left_item, right_item);
            assert_function(left, right);
        }
    }
}

const TEXT: &str = "%%start\nstruct copy Point { u8 value; }\nenum Marker { ready; }\npick = overload { u8(u8) => fn(input) { u8 next = input; next }; u32(u32) => fn(input) { u32 next = input; next }; };\nu8(Point) read = fn(input) { next = input; pick(next.value) };\n%%end";

fn source() -> SourceIdentity {
    SourceIdentity::new(
        "project".into(),
        "package".into(),
        "inputs.w".into(),
        "r1".into(),
    )
}

#[test]
fn real_constructor_borrows_nonempty_inventories_and_prepared_call_rows_by_identity() {
    let parsed = parse_source(source(), TEXT.into(), &[]);
    assert!(parsed.diagnostics.is_empty(), "{:?}", parsed.diagnostics);
    let extracted = derive_scalar_declarations_from_cst(parsed.result.canonical_cst());
    let typed = typecheck_scalar_program(extracted, ScalarTargetLayout::WASM32);
    assert!(typed.diagnostics.is_empty(), "{:?}", typed.diagnostics);
    let program = typed.program;
    let call_sites = resolve_program_call_sites(&program).unwrap();
    let module_names = HashSet::from(["pick".to_owned(), "read".to_owned()]);
    let ScalarItem::Function(function) = &program.items[1] else {
        panic!("read")
    };
    let call_sites = &call_sites.functions[&1];
    assert!(!call_sites.sites.is_empty());
    assert!(!program.structs.is_empty());
    assert!(!program.enums.is_empty());
    assert!(!program.resolved_assignment_outputs.is_empty());
    let scope = layout::observations::Scope::new();
    let builder = ReferenceFlowBuilder::new(
        &program.source,
        function,
        &module_names,
        call_sites,
        &program.structs,
        &program.enums,
    );
    assert!(std::ptr::eq(builder.module_names, &module_names));
    assert!(std::ptr::eq(builder.call_sites, call_sites));
    assert!(std::ptr::eq(builder.structs, program.structs.as_slice()));
    assert!(std::ptr::eq(builder.enums, program.enums.as_slice()));
    assert_eq!(scope.counts(), layout::observations::Counts::default());
}

#[test]
fn actual_function_and_arm_consumers_copy_zero_aggregates_and_match_public_cfgs() {
    for target in [ScalarTargetLayout::WASM32, ScalarTargetLayout::NATIVE64] {
        let parsed = parse_source(source(), TEXT.into(), &[]);
        assert!(parsed.diagnostics.is_empty(), "{:?}", parsed.diagnostics);
        let extracted = derive_scalar_declarations_from_cst(parsed.result.canonical_cst());
        let typed = typecheck_scalar_program(extracted, target);
        assert!(typed.diagnostics.is_empty(), "{:?}", typed.diagnostics);
        let mut program = typed.program;
        let call_sites = resolve_program_call_sites(&program).unwrap();
        let module_names = HashSet::from(["pick".to_owned(), "read".to_owned()]);
        let public = derive_scalar_program_with_layout(&parsed.result, target);
        assert_eq!(public.diagnostics, typed.diagnostics);
        assert_eq!(program.items.len(), 2);
        for (index, (item, expected)) in program
            .items
            .iter_mut()
            .zip(&public.program.items)
            .enumerate()
        {
            let (ScalarItem::Function(function), ScalarItem::Function(expected)) = (item, expected)
            else {
                panic!("function")
            };
            let scope = layout::observations::Scope::new();
            let errors = record_function_reference_origins(
                &program.source,
                function,
                &module_names,
                &call_sites.functions[&index],
                &program.structs,
                &program.enums,
            );
            assert_eq!(scope.counts(), layout::observations::Counts::default());
            drop(scope);
            assert!(errors.is_empty(), "{errors:?}");
            assert_eq!(function, expected);
            if function.name == "pick" {
                assert_eq!(function.overload_arms.len(), 2);
                for arm in &function.overload_arms {
                    assert!(!arm.reference_cfg.points.is_empty());
                    assert!(!arm.reference_cfg.initial_points.is_empty());
                }
            } else {
                assert!(function
                    .reference_cfg
                    .points
                    .iter()
                    .any(|point| matches!(point.kind, CfgPointKind::Call { .. })));
            }
        }
        assert_eq!(program, public.program);
    }
}

#[test]
fn immutable_producer_preserves_preexisting_cfg_and_entire_input_on_both_targets() {
    for target in [ScalarTargetLayout::WASM32, ScalarTargetLayout::NATIVE64] {
        let parsed = parse_source(source(), TEXT.into(), &[]);
        assert!(parsed.diagnostics.is_empty(), "{:?}", parsed.diagnostics);
        let public = derive_scalar_program_with_layout(&parsed.result, target);
        assert!(public.diagnostics.is_empty(), "{:?}", public.diagnostics);
        let program = public.program;
        let before = program.clone();
        let call_sites = resolve_program_call_sites(&program).unwrap();
        let module_names = HashSet::from(["pick".to_owned(), "read".to_owned()]);
        for (index, item) in program.items.iter().enumerate() {
            let ScalarItem::Function(function) = item else {
                panic!("function")
            };
            let calls = &call_sites.functions[&index];
            for (callable, calls) in std::iter::once((function, calls))
                .chain(function.overload_arms.iter().zip(&calls.arms))
            {
                assert!(!callable.reference_cfg.points.is_empty());
                assert!(!callable.reference_cfg.initial_points.is_empty());
                let scope = layout::observations::Scope::new();
                let (cfg, errors) = derive_function_reference_cfg(
                    &program.source,
                    callable,
                    &module_names,
                    calls,
                    &program.structs,
                    &program.enums,
                    true,
                );
                assert_eq!(scope.counts(), layout::observations::Counts::default());
                drop(scope);
                assert!(errors.is_empty(), "{errors:?}");
                assert_eq!(cfg, callable.reference_cfg);
                assert_eq!(program, before);
            }
        }
    }
}

#[test]
fn child_errors_keep_source_order_partial_cfgs_and_suppress_parent_initial_transfer() {
    let text = "%%start\npick = overload { u8(u8) => fn(input) { next = input; next }; u32(u32) => fn(input) { next = input; next }; };\n%%end";
    for target in [ScalarTargetLayout::WASM32, ScalarTargetLayout::NATIVE64] {
        let parsed = parse_source(source(), text.into(), &[]);
        assert!(parsed.diagnostics.is_empty(), "{:?}", parsed.diagnostics);
        let extracted = derive_scalar_declarations_from_cst(parsed.result.canonical_cst());
        let typed = typecheck_scalar_program(extracted, target);
        assert!(typed.diagnostics.is_empty(), "{:?}", typed.diagnostics);
        let mut program = typed.program;
        program.resolved_assignment_outputs = ScalarAssignmentTyping::default();
        let calls = resolve_program_call_sites(&program).unwrap();
        let ScalarItem::Function(parent) = &program.items[0] else {
            panic!("overload")
        };
        assert_eq!(parent.overload_arms.len(), 2);
        let before = parent.clone();
        let module_names = HashSet::from(["pick".to_owned()]);
        let expected_errors = parent
            .overload_arms
            .iter()
            .map(|arm| {
                let ScalarBlockItem::Assignment(assignment) = &arm.body.items[0] else {
                    panic!("assignment")
                };
                ReferenceAnalysisError::MissingContext {
                    source_span: SourceSpan::new(source(), assignment.targets[0].target_span),
                    reason: ReferenceAnalysisErrorReason::MissingResolvedBinding,
                }
            })
            .collect::<Vec<_>>();
        let mut child_cfgs = Vec::new();
        for (position, (arm, expected)) in parent
            .overload_arms
            .iter()
            .zip(&expected_errors)
            .enumerate()
        {
            let (cfg, errors) = derive_function_reference_cfg(
                &program.source,
                arm,
                &module_names,
                &calls.functions[&0].arms[position],
                &program.structs,
                &program.enums,
                true,
            );
            assert_eq!(errors, vec![expected.clone()]);
            assert!(!cfg.points.is_empty());
            assert!(matches!(cfg.points[0].kind, CfgPointKind::Entry));
            assert!(cfg.initial_points.is_empty());
            child_cfgs.push(cfg);
        }
        let (gated_parent, errors) = derive_function_reference_cfg(
            &program.source,
            parent,
            &module_names,
            &calls.functions[&0],
            &program.structs,
            &program.enums,
            false,
        );
        assert!(errors.is_empty(), "{errors:?}");
        assert!(!gated_parent.points.is_empty());
        assert!(gated_parent.initial_points.is_empty());
        let (successful_parent, errors) = derive_function_reference_cfg(
            &program.source,
            parent,
            &module_names,
            &calls.functions[&0],
            &program.structs,
            &program.enums,
            true,
        );
        assert!(errors.is_empty(), "{errors:?}");
        assert!(!successful_parent.initial_points.is_empty());
        assert_eq!(parent, &before);
        let mut installed = parent.clone();
        installed.reference_cfg = successful_parent.clone();
        for arm in &mut installed.overload_arms {
            arm.reference_cfg = successful_parent.clone();
        }
        let errors = record_function_reference_origins(
            &program.source,
            &mut installed,
            &module_names,
            &calls.functions[&0],
            &program.structs,
            &program.enums,
        );
        assert_eq!(errors, expected_errors);
        assert_eq!(installed.reference_cfg, gated_parent);
        assert!(installed.reference_cfg.initial_points.is_empty());
        let mut expected = before;
        expected.reference_cfg = gated_parent;
        for (arm, cfg) in expected.overload_arms.iter_mut().zip(child_cfgs) {
            arm.reference_cfg = cfg;
        }
        assert_eq!(installed, expected);
    }
}

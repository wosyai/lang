use super::*;
use crate::parse_source;

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
fn real_constructor_borrows_all_five_nonempty_inventories_by_identity() {
    let parsed = parse_source(source(), TEXT.into(), &[]);
    assert!(parsed.diagnostics.is_empty(), "{:?}", parsed.diagnostics);
    let extracted = derive_scalar_declarations_from_cst(parsed.result.canonical_cst());
    let typed = typecheck_scalar_program(extracted, ScalarTargetLayout::WASM32);
    assert!(typed.diagnostics.is_empty(), "{:?}", typed.diagnostics);
    let program = typed.program;
    let module = ScalarModule::from_program(program.clone(), Vec::new());
    let call_sites = resolve_module_call_sites(
        &program.source,
        &program.items,
        std::slice::from_ref(&module),
    )
    .unwrap();
    let module_names = HashSet::from(["pick".to_owned(), "read".to_owned()]);
    let ScalarItem::Function(function) = &program.items[1] else {
        panic!("read")
    };
    assert!(!call_sites.is_empty());
    assert!(!program.structs.is_empty());
    assert!(!program.enums.is_empty());
    assert!(!program.resolved_assignment_outputs.is_empty());
    let scope = layout::observations::Scope::new();
    let builder = ReferenceFlowBuilder::new(
        &program.source,
        function,
        &module_names,
        &call_sites,
        &program.structs,
        &program.enums,
        program.resolved_assignment_outputs.as_map(),
    );
    assert!(std::ptr::eq(builder.module_names, &module_names));
    assert!(std::ptr::eq(builder.call_sites, call_sites.as_slice()));
    assert!(std::ptr::eq(builder.structs, program.structs.as_slice()));
    assert!(std::ptr::eq(builder.enums, program.enums.as_slice()));
    assert!(std::ptr::eq(
        builder.assignment_outputs,
        program.resolved_assignment_outputs.as_map()
    ));
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
        let module = ScalarModule::from_program(program.clone(), Vec::new());
        let call_sites = resolve_module_call_sites(
            &program.source,
            &program.items,
            std::slice::from_ref(&module),
        )
        .unwrap();
        let module_names = HashSet::from(["pick".to_owned(), "read".to_owned()]);
        let public = derive_scalar_program_with_layout(&parsed.result, target);
        assert_eq!(public.diagnostics, typed.diagnostics);
        assert_eq!(program.items.len(), 2);
        for (item, expected) in program.items.iter_mut().zip(&public.program.items) {
            let (ScalarItem::Function(function), ScalarItem::Function(expected)) = (item, expected)
            else {
                panic!("function")
            };
            let scope = layout::observations::Scope::new();
            let errors = record_function_reference_origins(
                &program.source,
                function,
                &module_names,
                &call_sites,
                &program.structs,
                &program.enums,
                program.resolved_assignment_outputs.as_map(),
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

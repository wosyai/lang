use super::*;

#[test]
fn checked_assignment_retains_original_destination_before_retargeting_on_both_pipelines_and_layouts(
) {
    let text = "%%start\nunit() check = fn { u8 first = 1; u8 second = 2; *!u8 selected = &!first; selected, *selected = &!second, 9; u8 observed = *selected; u8 first_expected = 9; u8 observed_expected = 2; if (first != first_expected || observed != observed_expected) { core.system_panic(); }; };\ncheck();\n%%end";
    for target in [ScalarTargetLayout::WASM32, ScalarTargetLayout::NATIVE64] {
        let parsed = parse_source(source(), text.into(), &[]);
        assert!(parsed.diagnostics.is_empty(), "{:?}", parsed.diagnostics);
        let single = derive_scalar_program_with_layout(&parsed.result, target);
        assert!(single.diagnostics.is_empty(), "{:?}", single.diagnostics);
        let ScalarSourceDeclarations {
            source,
            items,
            structs,
            enums,
            diagnostics,
        } = derive_scalar_declarations_from_cst(parsed.result.canonical_cst());
        assert!(diagnostics.is_empty());
        let order = vec![source.clone()];
        let mut module = ScalarModule::new(source, items, Vec::new());
        module.structs = structs;
        module.enums = enums;
        module.target_layout = target;
        let project = validate_scalar_project(ScalarProject::new(vec![module], order));
        assert_eq!(project.diagnostics, single.diagnostics);
        assert_eq!(project.project.modules[0].items, single.program.items);
        for items in [&single.program.items, &project.project.modules[0].items] {
            let ScalarItem::Function(function) = &items[0] else {
                panic!("check")
            };
            let cfg = &function.reference_cfg;
            let direct = cfg
                .points
                .iter()
                .find(|point| matches!(point.kind, CfgPointKind::Assign { .. }))
                .expect("retarget install");
            let CfgPointKind::Assign {
                rhs_point: pointer_rhs,
                target: selected,
                ..
            } = &direct.kind
            else {
                unreachable!()
            };
            let through = cfg
                .points
                .iter()
                .find(|point| matches!(point.kind, CfgPointKind::AssignThrough { .. }))
                .expect("retained dereference install");
            let CfgPointKind::AssignThrough {
                rhs_point: byte_rhs,
                pointer_point,
                candidates,
                ..
            } = &through.kind
            else {
                unreachable!()
            };
            assert!(pointer_rhs.0 < byte_rhs.0);
            assert!(byte_rhs.0 < pointer_point.0);
            assert!(pointer_point.0 < direct.id.0);
            assert!(direct.id.0 < through.id.0);
            let rhs_start = text.find("&!second, 9").unwrap() as u32;
            assert_eq!(
                cfg.points[pointer_rhs.0].source_span.range,
                ByteSpan::new(rhs_start, rhs_start + 8)
            );
            assert_eq!(
                cfg.points[byte_rhs.0].source_span.range,
                ByteSpan::new(rhs_start + 10, rhs_start + 11)
            );
            let selector = &cfg.points[pointer_point.0];
            let CfgPointKind::Read { binding } = &selector.kind else {
                panic!("direct checked-pointer evaluation")
            };
            assert_eq!(binding, &selected.binding);
            let selector_start = text.find("*selected =").unwrap() as u32 + 1;
            assert_eq!(
                selector.source_span,
                SourceSpan::new(
                    single.program.source.clone(),
                    ByteSpan::new(selector_start, selector_start + 8)
                )
            );
            assert_eq!(
                cfg.points
                    .iter()
                    .filter(|point| point.source_span == selector.source_span
                        && matches!(point.kind, CfgPointKind::Read { .. }))
                    .count(),
                1
            );
            assert_eq!(selector.scope, direct.scope);
            assert_eq!(selector.scope, through.scope);
            let first = text.find("first =").unwrap() as u32;
            let second = text.find("second =").unwrap() as u32;
            assert_eq!(candidates.len(), 1);
            assert_eq!(
                candidates[0].place.binding.declaration_span,
                ByteSpan::new(first, first + 5)
            );
            assert_eq!(candidates[0].place.source, single.program.source);
            assert_eq!(candidates[0].place.function_span, function.span);
            let original = selector
                .possible_origins
                .as_ref()
                .expect("retained first origin");
            assert_eq!(original.len(), 1);
            let ReferenceOrigin::Fresh { loan, .. } = &cfg.facts[original.first().unwrap().0]
            else {
                panic!("first storage origin")
            };
            assert_eq!(
                loan.origin_place.binding.declaration_span,
                ByteSpan::new(first, first + 5)
            );
            let later_start = text.find("*selected;").unwrap() as u32 + 1;
            let later = cfg
                .points
                .iter()
                .find(|point| {
                    point.source_span.range == ByteSpan::new(later_start, later_start + 8)
                        && matches!(point.kind, CfgPointKind::Read { .. })
                })
                .expect("later selected read");
            assert!(through.id.0 < later.id.0);
            let origins = later
                .possible_origins
                .as_ref()
                .expect("retargeted second origin");
            assert_eq!(origins.len(), 1);
            let ReferenceOrigin::Fresh { loan, .. } = &cfg.facts[origins.first().unwrap().0] else {
                panic!("second storage origin")
            };
            assert_eq!(
                loan.origin_place.binding.declaration_span,
                ByteSpan::new(second, second + 6)
            );
        }
    }
}

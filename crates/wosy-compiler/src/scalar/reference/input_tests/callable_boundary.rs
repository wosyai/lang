use super::*;

fn assert_boundary(
    source: &SourceIdentity,
    function: &ScalarFunction,
    calls: &FunctionCallSites,
    names: &HashSet<String>,
) {
    let ScalarType::Callable {
        parameters,
        outputs,
    } = &function.signature
    else {
        unreachable!()
    };
    assert_eq!(calls.boundary.parameters.len(), function.parameters.len());
    for (ordinal, slot) in calls.boundary.parameters.iter().enumerate() {
        assert_eq!(slot.ordinal, ordinal);
        assert_eq!(slot.name, function.parameters[ordinal]);
        assert_eq!(slot.span, function.parameter_spans[ordinal]);
        assert_eq!(slot.ty, parameters[ordinal]);
    }
    assert_eq!(calls.boundary.generics, function.generic_parameters);
    assert_eq!(calls.boundary.results.len(), outputs.outputs.len());
    for (ordinal, slot) in calls.boundary.results.iter().enumerate() {
        assert_eq!(slot.ordinal, ordinal);
        assert_eq!(slot.span, outputs.outputs[ordinal].span);
        assert_eq!(slot.ty, outputs.outputs[ordinal].ty);
    }
    let root = &calls.structure.blocks[calls.structure.root.0];
    assert_eq!(
        root.final_outputs().count(),
        calls.boundary.root_outputs.len()
    );
    for (final_slot, typed) in root.final_outputs().zip(&calls.boundary.root_outputs) {
        assert_eq!(typed.output, final_slot.output);
        assert_eq!(
            typed.ty,
            function.body.final_output_values[final_slot.output].ty
        );
    }
    let expected = derive_function_reference_cfg(source, function, names, calls, &[], &[], true);
    let mut detached = function.clone();
    detached.parameters.clear();
    detached.parameter_spans.clear();
    detached.generic_parameters.clear();
    detached.signature = ScalarType::Error;
    for output in &mut detached.body.final_output_values {
        output.ty = ScalarType::Error;
    }
    let actual = derive_function_reference_cfg(source, &detached, names, calls, &[], &[], true);
    assert_eq!(actual, expected);
    for (function, calls) in function.overload_arms.iter().zip(&calls.arms) {
        assert_boundary(source, function, calls, names);
    }
}

#[test]
fn prepared_callable_boundaries_preserve_parameter_ordinals_results_forwarding_and_arms() {
    let text = "%%start\n(u8, *u8)(u8, *u8) mixed = fn(value, input) { value, input };\n(u8, *u8)(u8, *u8) forward = fn(value, input) { mixed(value, input) };\npick = overload { *u8(u8, *u8) => fn(value, input) { value; input }; *u32(u32, *u32) => fn(value, input) { value; input }; };\n%%end";
    for target in [ScalarTargetLayout::WASM32, ScalarTargetLayout::NATIVE64] {
        let parsed = parse_source(source(), text.into(), &[]);
        assert!(parsed.diagnostics.is_empty(), "{:?}", parsed.diagnostics);
        let extracted = derive_scalar_declarations_from_cst(parsed.result.canonical_cst());
        let typed = typecheck_scalar_program(extracted, target);
        assert!(typed.diagnostics.is_empty(), "{:?}", typed.diagnostics);
        let program = typed.program;
        let calls = resolve_program_call_sites(&program).unwrap();
        let names = HashSet::from(["mixed".to_owned(), "forward".to_owned(), "pick".to_owned()]);
        for (index, item) in program.items.iter().enumerate() {
            let ScalarItem::Function(function) = item else {
                unreachable!()
            };
            let row = calls.function(index);
            assert_boundary(&program.source, function, row, &names);
            if index < 2 {
                assert_eq!(row.boundary.results.len(), 2);
                assert_eq!(row.boundary.results[0].ty, ScalarType::U8);
                assert!(matches!(
                    row.boundary.results[1].ty,
                    ScalarType::CheckedReference { .. }
                ));
                let (cfg, errors) = derive_function_reference_cfg(
                    &program.source,
                    function,
                    &names,
                    row,
                    &[],
                    &[],
                    true,
                );
                assert!(errors.is_empty(), "{errors:?}");
                let borrowed = cfg
                    .facts
                    .iter()
                    .filter_map(|fact| match fact {
                        ReferenceOrigin::BorrowedFrom { parameter, loan } => {
                            Some((*parameter, loan))
                        }
                        _ => None,
                    })
                    .collect::<Vec<_>>();
                assert!(borrowed.iter().any(|(parameter, loan)| *parameter == 1
                    && loan.creation_span == row.boundary.parameters[1].span));
            }
        }
        assert_eq!(calls.function(0).boundary.root_outputs.len(), 2);
        assert_eq!(calls.function(1).boundary.root_outputs.len(), 1);
        assert_eq!(calls.function(2).arms.len(), 2);
    }
}

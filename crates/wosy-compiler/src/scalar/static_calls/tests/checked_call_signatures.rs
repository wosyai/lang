use super::*;
use crate::scalar::control::{CheckedCallSignature, ModuleInitializationDetail};
use crate::scalar::reference::{
    FunctionCallSites, OccurrenceEdge as Edge, OccurrenceNode as Node, OccurrenceOrdinal,
    OccurrenceParent as Parent, OccurrenceStructure,
};

fn child(structure: &OccurrenceStructure, parent: Parent, edge: Edge) -> OccurrenceOrdinal {
    let Node::Expression(ordinal) = structure.edges[&(parent, edge)] else {
        panic!("expression occurrence")
    };
    ordinal
}

fn assert_signature(
    row: &CheckedCallSignature,
    parameters: &[ScalarType],
    outputs: &[ScalarType],
    overload_arm: Option<usize>,
) {
    assert_eq!(row.parameters, parameters);
    assert_eq!(
        row.outputs
            .outputs
            .iter()
            .map(|out| &out.ty)
            .collect::<Vec<_>>(),
        outputs.iter().collect::<Vec<_>>(),
    );
    assert_eq!(row.overload_arm, overload_arm);
}

#[test]
fn prepared_generic_calls_keep_concrete_signatures_at_module_body_and_length_edges() {
    let parsed = parsed(
        "%%start\ngeneric T;\nT(T) echo = fn(value) { value };\n\
         u8 initial = echo<u8>(1);\n\
         u8[echo<u64>(2)] module_bytes;\n\
         i32(i32) run = fn(n) { u8[echo<u64>(3)] local_bytes; local_bytes; echo<i32>(n) };\n%%end",
    );
    let check = |initialization: &control::ModuleInitializationRegion,
                 function: &FunctionCallSites| {
        let ModuleInitializationDetail::Occurrences(topology) = &initialization.detail else {
            panic!("module initialization occurrences")
        };
        let root = Parent::Block(topology.structure.root);
        let initial = child(
            &topology.structure,
            root,
            Edge::BindingRoot { item: 1, output: 0 },
        );
        let module_length = child(
            &topology.structure,
            root,
            Edge::BindingAllocationLength {
                item: 2,
                position: 0,
            },
        );
        assert_eq!(topology.calls, [initial, module_length]);
        assert_eq!(topology.checked_call_signatures.len(), 2);
        let root = Parent::Block(function.structure.root);
        let local_length = child(
            &function.structure,
            root,
            Edge::BindingAllocationLength {
                item: 0,
                position: 0,
            },
        );
        let output = function.structure.blocks[function.structure.root.0]
            .final_outputs()
            .next()
            .unwrap()
            .expression;
        assert_eq!(function.allocation_length_calls, [local_length]);
        assert_eq!(function.call_occurrences[0].ordinal, output);
        assert_eq!(function.checked_call_signatures.len(), 2);
        for (row, ty) in [
            (&topology.checked_call_signatures[&initial], ScalarType::U8),
            (
                &topology.checked_call_signatures[&module_length],
                ScalarType::U64,
            ),
            (
                &function.checked_call_signatures[&local_length],
                ScalarType::U64,
            ),
            (&function.checked_call_signatures[&output], ScalarType::I32),
        ] {
            let types = std::slice::from_ref(&ty);
            assert_signature(row, types, types, None);
        }
    };
    for target in [ScalarTargetLayout::NATIVE64, ScalarTargetLayout::WASM32] {
        let project = prepare_extracted(&parsed, target);
        check(
            project.initialization_region(0).unwrap(),
            project.calls.function(0, 3),
        );
        let program = PreparedProgram::new(
            derive_scalar_declarations_from_cst(parsed.canonical_cst()),
            target,
        )
        .unwrap_or_else(|failure| panic!("{:?}", failure.diagnostics));
        let calls = program.calls.as_ref().unwrap();
        check(&calls.initialization, calls.function(3));
    }
}

#[test]
fn prepared_contextual_overload_keeps_selected_arm_and_ordered_mixed_outputs() {
    let parsed = parsed(
        "%%start\nchoose = overload {\n\
         (i32, u8)(i32, bool) => fn(value, flag) { flag; u8 byte = 1; value, byte };\n\
         (i64, bool)(i64, bool) => fn(value, flag) { value, flag };\n\
         };\n\
         generic T;\nT(T) echo = fn(value) { value };\n\
         i64() run = fn() { choose(1, echo<bool>(true)) };\n%%end",
    );
    let check = |function: &FunctionCallSites| {
        let call = function.structure.blocks[function.structure.root.0]
            .final_outputs()
            .next()
            .unwrap()
            .expression;
        let argument = child(
            &function.structure,
            Parent::Expression(call),
            Edge::CallArgument(1),
        );
        assert_eq!(
            function
                .call_occurrences
                .iter()
                .map(|entry| entry.ordinal)
                .collect::<Vec<_>>(),
            [call, argument],
        );
        assert_eq!(function.checked_call_signatures.len(), 2);
        assert_signature(
            &function.checked_call_signatures[&call],
            &[ScalarType::I64, ScalarType::Bool],
            &[ScalarType::I64, ScalarType::Bool],
            Some(1),
        );
        assert_signature(
            &function.checked_call_signatures[&argument],
            &[ScalarType::Bool],
            &[ScalarType::Bool],
            None,
        );
    };
    for target in [ScalarTargetLayout::NATIVE64, ScalarTargetLayout::WASM32] {
        let project = prepare_extracted(&parsed, target);
        check(project.calls.function(0, 2));
        let program = PreparedProgram::new(
            derive_scalar_declarations_from_cst(parsed.canonical_cst()),
            target,
        )
        .unwrap_or_else(|failure| panic!("{:?}", failure.diagnostics));
        check(program.calls.as_ref().unwrap().function(2));
    }
}

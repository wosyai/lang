use super::*;
use crate::scalar::control::{
    BindingInitialization, BindingOutput, BlockItemKind, CheckedCallSignature, ConditionalRegion,
    ModuleInitializationDetail,
};
use crate::scalar::reference::{
    CallOccurrence, FunctionCallSites, OccurrenceEdge as Edge, OccurrenceNode as Node,
    OccurrenceOrdinal, OccurrenceParent as Parent, OccurrenceRole, OccurrenceStructure,
};

fn child(structure: &OccurrenceStructure, parent: Parent, edge: Edge) -> OccurrenceOrdinal {
    let Node::Expression(ordinal) = structure.edges[&(parent, edge)] else {
        panic!("expression occurrence")
    };
    ordinal
}

#[test]
fn public_prepare_retains_checked_expression_and_place_types_by_occurrence() {
    let parsed = parsed(
        "%%start\nu16 initial = 1;\n\
         (i16, u8)() run = fn() { u8[2] cells = [1, 2]; cells; -1, cells[0] };\n%%end",
    );
    let check = |initialization: &control::ModuleInitializationRegion,
                 function: &FunctionCallSites| {
        let ModuleInitializationDetail::Occurrences(topology) = &initialization.detail else {
            panic!("module initialization occurrences")
        };
        let initial = child(
            &topology.structure,
            Parent::Block(topology.structure.root),
            Edge::BindingRoot { item: 0, output: 0 },
        );
        assert_eq!(
            topology.structure.checked_types.expressions[&initial],
            ScalarType::U16,
        );

        let structure = &function.structure;
        let types = &structure.checked_types;
        let mut outputs = structure.blocks[structure.root.0].final_outputs();
        let first = outputs.next().unwrap();
        let second = outputs.next().unwrap();
        assert_eq!((first.output, second.output), (0, 1));
        assert!(first.expression.0 < second.expression.0);
        assert!(outputs.next().is_none());
        for slot in [first, second] {
            assert_eq!(
                child(
                    structure,
                    Parent::Block(structure.root),
                    Edge::FinalOutput {
                        item: slot.item,
                        output: slot.output,
                    },
                ),
                slot.expression,
            );
        }
        assert_eq!(types.expressions[&first.expression], ScalarType::I16);
        assert_eq!(types.expressions[&second.expression], ScalarType::U8);

        let negated_literal = child(
            structure,
            Parent::Expression(first.expression),
            Edge::UnaryOperand,
        );
        assert_eq!(types.expressions.get(&negated_literal), None);

        let Node::Place(place) =
            structure.edges[&(Parent::Expression(second.expression), Edge::ExpressionPlace)]
        else {
            panic!("indexed place")
        };
        assert_ne!(place, second.expression);
        assert_eq!(types.places[&place], ScalarType::U8);
        assert_eq!(types.expressions.get(&place), None);
        assert_eq!(types.places.get(&second.expression), None);
        let Node::Place(base) = structure.edges[&(Parent::Place(place), Edge::PlaceBase)] else {
            panic!("indexed base")
        };
        assert!(matches!(
            &types.places[&base],
            ScalarType::Array { element, length: 2, .. } if **element == ScalarType::U8
        ));
        let index = child(structure, Parent::Place(place), Edge::PlaceIndex);
        assert_eq!(types.expressions[&index], ScalarType::U64);
    };
    for target in [ScalarTargetLayout::NATIVE64, ScalarTargetLayout::WASM32] {
        let project = prepare_extracted(&parsed, target);
        check(
            project.initialization_region(0).unwrap(),
            project.calls.function(0, 1),
        );
        let program = PreparedProgram::new(
            derive_scalar_declarations_from_cst(parsed.canonical_cst()),
            target,
        )
        .unwrap_or_else(|failure| panic!("{:?}", failure.diagnostics));
        let calls = program.calls.as_ref().unwrap();
        check(&calls.initialization, calls.function(1));
    }
}

fn place_child(structure: &OccurrenceStructure, parent: Parent, edge: Edge) -> OccurrenceOrdinal {
    let Node::Place(ordinal) = structure.edges[&(parent, edge)] else {
        panic!("place occurrence")
    };
    ordinal
}

#[test]
fn prepare_retains_checked_dereference_and_reborrow_place_types() {
    let parsed = parsed(
        "%%start
struct Record { u8 value; }
(u16, u8, *!u32, *u16)(*u16, *Record, *!u32) run = fn(shared, item, writer) {
    *(&*shared), (*item).value, &!(*writer), &*(&*shared)
};
%%end",
    );
    let check = |structure: &OccurrenceStructure| {
        let types = &structure.checked_types;
        let place = |expression, expected: &ScalarType| {
            let place = place_child(
                structure,
                Parent::Expression(expression),
                Edge::ExpressionPlace,
            );
            assert_eq!(types.places.get(&place), Some(expected));
            assert!(!types.places.contains_key(&expression));
            place
        };
        let pointer = |dereference| {
            child(
                structure,
                Parent::Place(dereference),
                Edge::DereferencePointer,
            )
        };
        let reference = |mutability, inner| ScalarType::CheckedReference {
            mutability,
            inner: Box::new(inner),
        };
        let shared = reference(ScalarReferenceMutability::Shared, ScalarType::U16);
        let mutable = reference(ScalarReferenceMutability::Mutable, ScalarType::U32);
        let mut outputs = structure.blocks[structure.root.0].final_outputs();
        let read = outputs.next().unwrap().expression;
        let field = outputs.next().unwrap().expression;
        let mutable_address = outputs.next().unwrap().expression;
        let nested_address = outputs.next().unwrap().expression;
        assert!(outputs.next().is_none());

        assert_eq!(types.expressions[&read], ScalarType::U16);
        assert_eq!(types.expressions[&field], ScalarType::U8);
        assert_eq!(types.expressions[&mutable_address], mutable);
        assert_eq!(types.expressions[&nested_address], shared);

        for expression in [read, nested_address] {
            let reborrow = pointer(place(expression, &ScalarType::U16));
            assert_eq!(types.expressions[&reborrow], shared);
            let parent = pointer(place(reborrow, &ScalarType::U16));
            assert_eq!(types.expressions[&parent], shared);
        }
        let parent = pointer(place(mutable_address, &ScalarType::U32));
        assert_eq!(types.expressions[&parent], mutable);

        let base = place_child(
            structure,
            Parent::Place(place(field, &ScalarType::U8)),
            Edge::PlaceBase,
        );
        let record = &types.places[&base];
        assert!(matches!(record, ScalarType::Struct(_)));
        assert_eq!(
            types.expressions[&pointer(base)],
            reference(ScalarReferenceMutability::Shared, record.clone()),
        );
    };
    for target_layout in [ScalarTargetLayout::NATIVE64, ScalarTargetLayout::WASM32] {
        let project = prepare_extracted(&parsed, target_layout);
        check(&project.calls.function(0, 0).structure);
        let program = PreparedProgram::new(
            derive_scalar_declarations_from_cst(parsed.canonical_cst()),
            target_layout,
        )
        .unwrap_or_else(|failure| panic!("{:?}", failure.diagnostics));
        check(&program.calls.as_ref().unwrap().function(0).structure);
    }
}

fn destination(structure: &OccurrenceStructure, item: usize) -> OccurrenceOrdinal {
    place_child(
        structure,
        Parent::Block(structure.root),
        Edge::AssignmentDestination {
            item,
            destination: 0,
        },
    )
}

#[test]
fn prepare_retains_checked_assignment_destination_paths() {
    let parsed = parsed(
        "%%start
struct Record { u8 value; u8[2] bytes; }
unit(*!Record, *!u16) run = fn(writer, number) {
    u8 existing = 0;
    existing = 1;
    inferred = true;
    u8[2] cells = [1, 2];
    cells[0] = 3;
    (*writer).value = 4;
    (*writer).bytes[1] = 5;
    *number = 6;
    existing; inferred; cells;
};
%%end",
    );
    let check = |structure: &OccurrenceStructure| {
        let types = &structure.checked_types;
        let base = |place| place_child(structure, Parent::Place(place), Edge::PlaceBase);
        let array = |place| {
            assert!(matches!(
                &types.places[&place],
                ScalarType::Array { element, length: 2, .. } if **element == ScalarType::U8
            ));
        };
        let mutable_pointer = |place, inner: &ScalarType| {
            let pointer = child(structure, Parent::Place(place), Edge::DereferencePointer);
            assert!(matches!(
                &types.expressions[&pointer],
                ScalarType::CheckedReference { mutability: ScalarReferenceMutability::Mutable, inner: checked }
                    if checked.as_ref() == inner
            ));
        };
        let index = |place| {
            let selector = child(structure, Parent::Place(place), Edge::PlaceIndex);
            assert_eq!(types.expressions[&selector], ScalarType::U64);
        };
        assert_eq!(types.places[&destination(structure, 1)], ScalarType::U8);
        assert_eq!(types.places[&destination(structure, 2)], ScalarType::Bool);

        let cell = destination(structure, 4);
        let field = destination(structure, 5);
        let byte = destination(structure, 6);
        for place in [cell, field, byte] {
            assert_eq!(types.places[&place], ScalarType::U8);
        }
        for place in [cell, byte] {
            array(base(place));
            index(place);
        }
        let record = &types.places[&base(field)];
        assert!(matches!(record, ScalarType::Struct(_)));
        assert_eq!(&types.places[&base(base(byte))], record);
        mutable_pointer(base(field), record);
        mutable_pointer(base(base(byte)), record);

        let scalar = destination(structure, 7);
        assert_eq!(types.places[&scalar], ScalarType::U16);
        mutable_pointer(scalar, &ScalarType::U16);
    };
    for target_layout in [ScalarTargetLayout::NATIVE64, ScalarTargetLayout::WASM32] {
        let project = prepare_extracted(&parsed, target_layout);
        check(&project.calls.function(0, 0).structure);
        let program = PreparedProgram::new(
            derive_scalar_declarations_from_cst(parsed.canonical_cst()),
            target_layout,
        )
        .unwrap_or_else(|failure| panic!("{:?}", failure.diagnostics));
        check(&program.calls.as_ref().unwrap().function(0).structure);
    }
}

#[test]
fn prepare_retains_checked_namespace_assignment_destination() {
    let main = parsed(
        "%%start\nimported = namespace package \"values.w\";\n\
         unit() run = fn { imported.value = 4; };\n%%end",
    );
    let dependency = SourceIdentity::new(
        "project".into(),
        "package".into(),
        "values.w".into(),
        "r1".into(),
    );
    let values = parse_source(
        dependency.clone(),
        "%%start\nu16 value = 1;\n%%end".into(),
        &[],
    );
    assert!(values.diagnostics.is_empty(), "{:?}", values.diagnostics);
    for target_layout in [ScalarTargetLayout::NATIVE64, ScalarTargetLayout::WASM32] {
        let main_declarations = derive_scalar_declarations_from_cst(main.canonical_cst());
        let namespace = main_declarations.namespaces().next().unwrap();
        let binding = ScalarNamespaceBinding {
            binding: namespace.binding.clone(),
            target: dependency.clone(),
            span: namespace.span,
        };
        let project = crate::prepare_scalar_project(
            vec![
                (main_declarations, vec![binding]),
                (
                    derive_scalar_declarations_from_cst(values.result.canonical_cst()),
                    Vec::new(),
                ),
            ],
            vec![dependency.clone(), main.source.clone()],
            crate::ScalarPreparationInputs { target_layout },
        )
        .unwrap_or_else(|failure| panic!("{:?}", failure.diagnostics));
        let structure = &project.calls.function(0, 1).structure;
        let place = destination(structure, 0);
        assert_eq!(structure.checked_types.places[&place], ScalarType::U16);
    }
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
        let literal = child(
            &function.structure,
            Parent::Expression(call),
            Edge::CallArgument(0),
        );
        let types = &function.structure.checked_types.expressions;
        assert_eq!(types[&literal], ScalarType::I64);
        assert_eq!(types[&argument], ScalarType::Bool);
        assert_eq!(types[&call], ScalarType::I64);
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

#[test]
fn recursive_calls_retain_static_occurrences_and_ordered_positions() {
    let parsed = parsed(
        "%%start
(i32, bool)(i32, bool) descend = fn(count, flag) {
    if (count == 0) { count, flag } else {
        i32 remaining, bool result = descend(count - 1, flag);
        remaining, result
    }
};
(bool, i32)(i32, bool) even_step = fn(count, flag) {
    if (count == 0) { flag, count } else {
        bool result, i32 remaining = odd_step(flag, count - 1);
        result, remaining
    }
};
(bool, i32)(bool, i32) odd_step = fn(flag, count) {
    if (count == 0) { !flag, count } else {
        bool result, i32 remaining = even_step(count - 1, flag);
        result, remaining
    }
};
%%end",
    );
    let number_first = [ScalarType::I32, ScalarType::Bool];
    let flag_first = [ScalarType::Bool, ScalarType::I32];
    for target in [ScalarTargetLayout::NATIVE64, ScalarTargetLayout::WASM32] {
        let stages = Scope::new();
        let prepared = prepare_extracted(&parsed, target);
        assert_eq!(
            stages.counts(),
            Counts {
                call_sites: 1,
                ..Counts::default()
            }
        );
        drop(stages);

        let [module] = prepared.project().modules.as_slice() else {
            panic!("one prepared source module")
        };
        assert_eq!(module.source, parsed.source);
        assert_eq!(module.target_layout, target);
        let [ScalarItem::Function(descend), ScalarItem::Function(even), ScalarItem::Function(odd)] =
            module.items.as_slice()
        else {
            panic!("three recursive source declarations")
        };
        assert_eq!(descend.name, "descend");
        assert_eq!(even.name, "even_step");
        assert_eq!(odd.name, "odd_step");
        for (item, (caller, callee, parameters, outputs)) in [
            (descend, descend, &number_first, &number_first),
            (even, odd, &flag_first, &flag_first),
            (odd, even, &number_first, &flag_first),
        ]
        .into_iter()
        .enumerate()
        {
            let row = prepared.calls.function(0, item);
            let structure = &row.structure;
            let root_outputs = structure.blocks[structure.root.0]
                .final_outputs()
                .collect::<Vec<_>>();
            assert_eq!(
                root_outputs
                    .iter()
                    .map(|slot| slot.output)
                    .collect::<Vec<_>>(),
                [0, 1],
            );
            let branch = root_outputs[0].expression;
            let ConditionalRegion::Value {
                then_block,
                else_block,
                ..
            } = &row.conditionals[&branch]
            else {
                panic!("recursive function with a base branch")
            };
            let Some((
                ScalarExpression::If {
                    then_branch,
                    else_branch,
                    ..
                },
                _,
            )) = caller.body.final_expressions().next()
            else {
                panic!("source conditional")
            };
            assert_eq!(structure.blocks[then_block.0].span, then_branch.span);
            let ScalarBlockItem::LocalBinding(source_binding) = &else_branch.items[0] else {
                panic!("recursive call binding")
            };
            let ScalarExpression::Call { arguments, .. } = &source_binding.value else {
                panic!("syntactic recursive call")
            };
            let recursive = &structure.blocks[else_block.0];
            assert_eq!(recursive.span, else_branch.span);
            let BlockItemKind::Binding(binding) = &recursive.items[0].kind else {
                panic!("canonical binding region")
            };
            let BindingInitialization::Outputs(values) = &binding.initialization else {
                panic!("recursive call output positions")
            };
            let call = values.primary;
            let call_span = expression_span(&source_binding.value);
            assert_eq!(
                row.call_occurrences,
                [CallOccurrence {
                    ordinal: call,
                    site: 0,
                }]
            );
            assert_eq!(row.sites.len(), 1);
            assert_eq!(
                row.sites[0].source_span,
                SourceSpan::new(module.source.clone(), call_span)
            );
            assert_eq!(row.occurrences[call.0].role, OccurrenceRole::Expression);
            assert_eq!(row.occurrences[call.0].span, call_span);
            assert_eq!(
                row.occurrences
                    .iter()
                    .filter(|entry| entry.span == call_span)
                    .count(),
                1
            );
            assert_eq!(
                row.sites[0].target,
                ResolvedCallTarget::ModuleCallable {
                    source: module.source.clone(),
                    declaration_span: callee.name_span,
                    concrete: ConcreteCallSelection::Declaration,
                }
            );

            let parent = Parent::Expression(call);
            assert_eq!(arguments.len(), parameters.len());
            let positions =
                [0, 1].map(|position| child(structure, parent, Edge::CallArgument(position)));
            assert!(positions[0].0 < positions[1].0);
            for ((ordinal, argument), ty) in positions.into_iter().zip(arguments).zip(parameters) {
                assert_eq!(row.occurrences[ordinal.0].span, expression_span(argument));
                assert_eq!(&structure.checked_types.expressions[&ordinal], ty);
            }
            assert_eq!(row.checked_call_signatures.len(), 1);
            assert_signature(
                &row.checked_call_signatures[&call],
                parameters,
                outputs,
                None,
            );
            assert_eq!(
                child(
                    structure,
                    Parent::Block(*else_block),
                    Edge::BindingRoot { item: 0, output: 0 },
                ),
                call
            );
            assert_eq!(
                values.secondary,
                [BindingOutput::ProjectPrimary { output: 1 }]
            );
            assert_eq!(
                binding
                    .receivers()
                    .iter()
                    .map(|slot| (slot.position, &slot.ty))
                    .collect::<Vec<_>>(),
                outputs.iter().enumerate().collect::<Vec<_>>()
            );
        }

        let initialization = prepared.initialization_region(0).unwrap().clone();
        let inventory = [0, 1, 2].map(|item| prepared.calls.function(0, item).clone());
        let builds = [(), ()].map(|()| {
            let stages = Scope::new();
            let build = crate::analyze_scalar_build(&prepared);
            assert!(
                build.diagnostics().is_empty(),
                "{target:?}: {:?}",
                build.diagnostics()
            );
            assert_query_stages(stages.counts(), true);
            assert!(std::ptr::eq(build.prepared(), &prepared));
            assert_eq!(prepared.initialization_region(0), Some(&initialization));
            for (item, before) in inventory.iter().enumerate() {
                assert_eq!(prepared.calls.function(0, item), before);
            }
            build
        });
        for item in 0..inventory.len() {
            let [first, second] = builds
                .each_ref()
                .map(|build| build.reference_cfg(0, item, None).unwrap());
            assert!(!first.points.is_empty());
            assert!(!std::ptr::eq(first, second));
            assert_eq!(first, second);
        }
    }
}

use super::*;
use crate::scalar::control::{
    BindingInitialization, BindingOutput, BlockItemKind, CheckedCallSignature, ConditionalRegion,
    FoldedOccurrence, ModuleInitializationDetail,
};
use crate::scalar::overload::observations as overload_observations;
use crate::scalar::reference::{
    CallOccurrence, FunctionCallSites, OccurrenceEdge as Edge, OccurrenceNode as Node,
    OccurrenceOrdinal, OccurrenceParent as Parent, OccurrenceRole, OccurrenceStructure,
};
use std::fmt::Write;

fn child(structure: &OccurrenceStructure, parent: Parent, edge: Edge) -> OccurrenceOrdinal {
    let Node::Expression(ordinal) = structure.edges[&(parent, edge)] else {
        panic!("expression occurrence")
    };
    ordinal
}

#[test]
fn prepare_publishes_module_call_targets_by_canonical_ordinal_on_both_layouts() {
    use crate::scalar::control::ModuleInitializationCallTarget as Target;

    let text = "%%start
left = namespace package \"library.w\";
right = namespace package \"library.w\";
disk = extern wasm \"disk_abi\" { i32() read; };
struct Reader { i32() read; }
i32() local = fn { 7 };
i32() global = local;
i32 padding, i32() secondary = 0, local;
Reader reader = { .read = local; };
i32 first = left.read();
i32 second = if (true) { i32() local = global; copy = local; local() + copy() + right.read() } else { disk.read() };
i32 third = reader.read() + global() + secondary() + local();
i32 selected = right.pick(13);
u64 wide = core.int_extend<u64>(third);
%%end";
    let main = parsed(text);
    let library_source = SourceIdentity::new(
        "project".into(),
        "package".into(),
        "library.w".into(),
        "r1".into(),
    );
    let library_text = "%%start\ni32() read = fn { 11 };\npick = overload { i32(i32) => fn(value) { value }; };\n%%end";
    let library = parse_source(library_source.clone(), library_text.into(), &[]);
    assert!(library.diagnostics.is_empty(), "{:?}", library.diagnostics);
    let location = |source: &SourceIdentity, text: &str, marker: &str, len: u32| {
        let start = text.find(marker).unwrap() as u32;
        SourceSpan::new(source.clone(), ByteSpan::new(start, start + len))
    };
    for target_layout in [ScalarTargetLayout::NATIVE64, ScalarTargetLayout::WASM32] {
        let declarations = derive_scalar_declarations_from_cst(main.canonical_cst());
        let namespaces = declarations.items[..2]
            .iter()
            .map(|item| {
                let ScalarItem::Namespace(namespace) = item else {
                    panic!("namespace")
                };
                ScalarNamespaceBinding {
                    binding: namespace.binding.clone(),
                    target: library_source.clone(),
                    span: namespace.binding_span,
                }
            })
            .collect();
        let stages = Scope::new();
        let prepared = crate::prepare_scalar_project(
            vec![
                (declarations, namespaces),
                (
                    derive_scalar_declarations_from_cst(library.result.canonical_cst()),
                    Vec::new(),
                ),
            ],
            vec![library_source.clone(), main.source.clone()],
            crate::ScalarPreparationInputs { target_layout },
        )
        .unwrap_or_else(|error| panic!("{target_layout:?}: {:?}", error.diagnostics));
        assert_eq!(
            stages.counts(),
            Counts {
                call_sites: 2,
                ..Counts::default()
            }
        );
        let initialization = prepared.initialization_region(0).unwrap();
        let ModuleInitializationDetail::Occurrences(topology) = &initialization.detail else {
            panic!("module initialization topology")
        };
        let imported = Target::Direct {
            declaration: location(&library_source, library_text, "read =", 4),
            concrete: ConcreteCallSelection::Declaration,
        };
        let expected = [
            ("left.read()", imported.clone()),
            (
                "local()",
                Target::CallableBinding(location(&main.source, text, "local = global", 5)),
            ),
            (
                "copy()",
                Target::CallableBinding(location(&main.source, text, "copy =", 4)),
            ),
            ("right.read()", imported),
            (
                "disk.read()",
                Target::Extern(location(&main.source, text, "read;", 4)),
            ),
            (
                "reader.read()",
                Target::CallableField {
                    receiver: location(&main.source, text, "reader =", 6),
                    field: prepared.project().modules[0].structs[0].fields[0]
                        .id
                        .clone(),
                },
            ),
            (
                "global()",
                Target::CallableBinding(location(&main.source, text, "global =", 6)),
            ),
            (
                "secondary()",
                Target::CallableBinding(location(&main.source, text, "secondary =", 9)),
            ),
            (
                "local()",
                Target::Direct {
                    declaration: location(&main.source, text, "local = fn", 5),
                    concrete: ConcreteCallSelection::Declaration,
                },
            ),
            (
                "right.pick(13)",
                Target::Direct {
                    declaration: location(&library_source, library_text, "pick =", 4),
                    concrete: ConcreteCallSelection::Overload(ScalarOverloadSelection {
                        arm_index: 0,
                        substitutions: BTreeMap::new(),
                    }),
                },
            ),
            (
                "core.int_extend<u64>(third)",
                Target::Core(CoreOperationId::IntExtend),
            ),
        ];
        assert_eq!(topology.calls.len(), expected.len());
        for (&ordinal, (spelling, selection)) in topology.calls.iter().zip(expected) {
            let span = topology.occurrences[ordinal.0].span;
            assert_eq!(&text[span.start as usize..span.end as usize], spelling);
            assert_eq!(initialization.call_target(ordinal), Some(&selection));
            assert!(
                topology.checked_call_signatures.contains_key(&ordinal),
                "{spelling}"
            );
        }
    }
}

#[test]
fn prepare_rejects_nested_extern_symbols_through_either_import_alias() {
    let text = "%%start
left = namespace package \"library.w\";
right = namespace package \"library.w\";
i32 first = left.read(1);
i32 second = if (true) { right.read(2) } else { 0 };
%%end";
    let main = parsed(text);
    let library_source = SourceIdentity::new(
        "project".into(),
        "package".into(),
        "library.w".into(),
        "r1".into(),
    );
    let library = parse_source(
        library_source.clone(),
        "%%start
left = extern wasm \"left_abi\" { i32() read; };
right = extern wasm \"right_abi\" { i32(i32) read; };
%%end"
            .into(),
        &[],
    );
    assert!(library.diagnostics.is_empty(), "{:?}", library.diagnostics);

    for target_layout in [ScalarTargetLayout::NATIVE64, ScalarTargetLayout::WASM32] {
        let main_declarations = derive_scalar_declarations_from_cst(main.canonical_cst());
        let namespaces = [0, 1]
            .map(|item| {
                let ScalarItem::Namespace(namespace) = &main_declarations.items[item] else {
                    panic!("namespace declaration")
                };
                ScalarNamespaceBinding {
                    binding: namespace.binding.clone(),
                    target: library_source.clone(),
                    span: namespace.binding_span,
                }
            })
            .to_vec();
        let scope = Scope::new();
        let Err(failure) = crate::prepare_scalar_project(
            vec![
                (main_declarations, namespaces),
                (
                    derive_scalar_declarations_from_cst(library.result.canonical_cst()),
                    Vec::new(),
                ),
            ],
            vec![library_source.clone(), main.source.clone()],
            crate::ScalarPreparationInputs { target_layout },
        ) else {
            panic!("{target_layout:?}: imported nested extern symbols passed preparation");
        };
        assert_eq!(scope.counts(), Counts::default());
        assert_eq!(
            failure.diagnostics.len(),
            2,
            "{target_layout:?}: {failure:?}"
        );
        for (alias, diagnostic) in ["left.", "right."].into_iter().zip(&failure.diagnostics) {
            let start = (text.find(&format!("{alias}read")).unwrap() + alias.len()) as u32;
            assert_eq!(diagnostic.code, "M0002");
            assert_eq!(diagnostic.message, "unknown module member");
            let [primary, secondary] = diagnostic.labels.as_slice() else {
                panic!("namespace diagnostic labels: {diagnostic:?}");
            };
            assert_eq!(primary.kind, crate::DiagnosticLabelKind::Primary);
            assert_eq!(
                primary.span,
                SourceSpan::new(main.source.clone(), ByteSpan::new(start, start + 4)),
            );
            assert_eq!(secondary.kind, crate::DiagnosticLabelKind::Secondary);
            assert_eq!(
                secondary.span,
                SourceSpan::new(library_source.clone(), ByteSpan::new(0, 0)),
            );
        }
    }
}

#[test]
fn prepares_and_builds_local_extern_with_same_named_generic_on_both_targets() {
    for target_layout in [ScalarTargetLayout::NATIVE64, ScalarTargetLayout::WASM32] {
        for (call, valid) in [("disk.read()", true), ("disk.read<i32>()", false)] {
            let text = format!(
                "%%start
generic T;
T(T) read = fn(value) {{ value }};
disk = extern wasm \"disk_abi\" {{ (i32, u64)() read; }};
(i32, u64)() run = fn {{
    i32 first, u64 second = {call};
    first, second
}};
%%end"
            );
            let parsed = parsed(&text);
            let declarations = derive_scalar_declarations_from_cst(parsed.canonical_cst());
            let start = text.find(call).unwrap() as u32;
            let call_span = SourceSpan::new(
                parsed.source.clone(),
                ByteSpan::new(start, start + call.len() as u32),
            );
            let result = crate::prepare_scalar_project(
                vec![(declarations, Vec::new())],
                vec![parsed.source.clone()],
                crate::ScalarPreparationInputs { target_layout },
            );
            if valid {
                let prepared = result.unwrap_or_else(|failure| {
                    panic!("{target_layout:?}: {:?}", failure.diagnostics)
                });
                let module = &prepared.project().modules[0];
                let [ScalarItem::Function(_), ScalarItem::Extern(extern_decl), ScalarItem::Function(_)] =
                    module.items.as_slice()
                else {
                    panic!("generic, extern, and caller declarations");
                };
                let caller = prepared.calls.function(0, 2);
                let [site] = caller.sites.as_slice() else {
                    panic!("one resolved call site");
                };
                assert_eq!(site.source_span, call_span);
                assert_eq!(
                    site.target,
                    ResolvedCallTarget::ExternCallable {
                        source: module.source.clone(),
                        declaration_span: extern_decl.functions[0].name_span,
                    },
                );
                let call = caller.call_occurrences[0].ordinal;
                assert_signature(
                    &caller.checked_call_signatures[&call],
                    &[],
                    &[ScalarType::I32, ScalarType::U64],
                    None,
                );
                let build = crate::analyze_scalar_build(&prepared);
                assert!(
                    build.diagnostics().is_empty(),
                    "{target_layout:?}: {:?}",
                    build.diagnostics()
                );
                assert!(build.reference_cfg(0, 2, None).is_some());
            } else {
                let Err(failure) = result else {
                    panic!("{target_layout:?}: explicit extern type argument passed preparation");
                };
                let [diagnostic] = failure.diagnostics.as_slice() else {
                    panic!("{target_layout:?}: {:?}", failure.diagnostics);
                };
                assert_eq!(diagnostic.code, "B0004");
                assert_eq!(
                    diagnostic.message,
                    "generic argument arity does not match callable declaration"
                );
                let [label] = diagnostic.labels.as_slice() else {
                    panic!("one primary call label: {diagnostic:?}");
                };
                assert_eq!(label.kind, crate::DiagnosticLabelKind::Primary);
                assert_eq!(label.span, call_span);
            }
        }
    }
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

#[test]
fn preparation_publishes_signed_integer_magnitudes_as_folded_unary_operands() {
    let parsed = parsed(
        "%%start\n-2147483648;\ni8 narrow = -128;\ni8() run = fn { -2147483648; -128 };\n%%end",
    );
    let check_signed_pair = |structure: &OccurrenceStructure,
                             occurrences: &[reference::OccurrenceMetadata],
                             roots: [Edge; 2],
                             spans: [ByteSpan; 2]| {
        assert_eq!(structure.folded_expressions.len(), 2);
        for (index, ((edge, span), ty)) in roots
            .into_iter()
            .zip(spans)
            .zip([ScalarType::I32, ScalarType::I8])
            .enumerate()
        {
            let signed = child(structure, Parent::Block(structure.root), edge);
            let magnitude = child(structure, Parent::Expression(signed), Edge::UnaryOperand);
            assert_eq!(signed, OccurrenceOrdinal(index * 2));
            assert_eq!(magnitude, OccurrenceOrdinal(index * 2 + 1));
            assert_eq!(occurrences[signed.0].span, span);
            assert_eq!(
                occurrences[magnitude.0].span,
                ByteSpan::new(span.start + 1, span.end)
            );
            assert_eq!(structure.checked_types.expressions[&signed], ty);
            assert_eq!(structure.checked_types.expressions.get(&magnitude), None);
            assert_eq!(
                structure.folded_expressions[&magnitude],
                FoldedOccurrence::SignedIntegerMagnitude,
            );
        }
    };
    let check = |initialization: &control::ModuleInitializationRegion,
                 function: &FunctionCallSites| {
        let ModuleInitializationDetail::Occurrences(topology) = &initialization.detail else {
            panic!("module initialization occurrences")
        };
        check_signed_pair(
            &topology.structure,
            &topology.occurrences,
            [
                Edge::ExpressionItem { item: 0 },
                Edge::BindingRoot { item: 1, output: 0 },
            ],
            [ByteSpan::new(8, 19), ByteSpan::new(33, 37)],
        );
        check_signed_pair(
            &function.structure,
            &function.occurrences,
            [
                Edge::ExpressionItem { item: 0 },
                Edge::FinalOutput { item: 1, output: 0 },
            ],
            [ByteSpan::new(55, 66), ByteSpan::new(68, 72)],
        );
    };
    for target in [ScalarTargetLayout::NATIVE64, ScalarTargetLayout::WASM32] {
        let project = prepare_extracted(&parsed, target);
        check(
            project.initialization_region(0).unwrap(),
            project.calls.function(0, 2),
        );
        let program = PreparedProgram::new(
            derive_scalar_declarations_from_cst(parsed.canonical_cst()),
            target,
        )
        .unwrap_or_else(|failure| panic!("{:?}", failure.diagnostics));
        let calls = program.calls.as_ref().unwrap();
        check(&calls.initialization, calls.function(2));
    }
}

#[test]
fn preparation_preserves_the_out_of_range_signed_integer_diagnostic_span() {
    let parsed = parsed("%%start\ni32 value = -2147483649;\n%%end");
    let declarations = || derive_scalar_declarations_from_cst(parsed.canonical_cst());
    for target_layout in [ScalarTargetLayout::NATIVE64, ScalarTargetLayout::WASM32] {
        let source = declarations().source().clone();
        let Err(project) = crate::prepare_scalar_project(
            vec![(declarations(), Vec::new())],
            vec![source.clone()],
            crate::ScalarPreparationInputs { target_layout },
        ) else {
            panic!("project preparation accepted an out-of-range signed integer")
        };
        let Err(program) = PreparedProgram::new(declarations(), target_layout) else {
            panic!("standalone preparation accepted an out-of-range signed integer")
        };
        for diagnostics in [project.diagnostics, program.diagnostics] {
            let [diagnostic] = diagnostics.as_slice() else {
                panic!("expected one range diagnostic: {diagnostics:?}");
            };
            assert_eq!(diagnostic.code, "B0010");
            assert_eq!(
                diagnostic.message,
                "integer literal is outside the resolved target type range"
            );
            let [label] = diagnostic.labels.as_slice() else {
                panic!("expected one signed literal label: {diagnostic:?}");
            };
            assert_eq!(label.kind, crate::DiagnosticLabelKind::Primary);
            assert_eq!(
                label.span,
                SourceSpan {
                    source: source.clone(),
                    range: ByteSpan::new(20, 31)
                }
            );
        }
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
    assert_eq!(
        row.overload.as_ref().map(|selection| selection.arm_index),
        overload_arm
    );
}

#[test]
fn prepare_publishes_checked_core_memory_signatures_in_initialization_and_functions() {
    use crate::scalar::control::ModuleInitializationCallTarget as Target;

    let body = "unsafe {
        u16 slot = 0;
        *?u16 raw = &?slot;
        *?u8 allocated = core.alloc(16, 8);
        core.free(allocated);
        core.invalidate(&slot);
        core.rebind(&!slot, raw, 3);
        core.free(&!slot);
        core.free(raw);
    };
    core.system_panic();";
    let text = format!("%%start\nunit() run = fn {{ {body} }};\n{body}\n%%end");
    let parsed = parsed(&text);
    let raw_u8 = ScalarType::RawPointer(Box::new(ScalarType::U8));
    let raw_u16 = ScalarType::RawPointer(Box::new(ScalarType::U16));
    let reference = |mutability| ScalarType::CheckedReference {
        mutability,
        inner: Box::new(ScalarType::U16),
    };
    let shared = reference(ScalarReferenceMutability::Shared);
    let mutable = reference(ScalarReferenceMutability::Mutable);
    let expected: [(_, _, &[ScalarType], &[ScalarType]); 7] = [
        (
            "core.alloc(16, 8)",
            CoreOperationId::Alloc,
            &[ScalarType::U64, ScalarType::U64],
            std::slice::from_ref(&raw_u8),
        ),
        (
            "core.free(allocated)",
            CoreOperationId::Free,
            std::slice::from_ref(&raw_u8),
            &[],
        ),
        (
            "core.invalidate(&slot)",
            CoreOperationId::Invalidate,
            &[shared],
            &[],
        ),
        (
            "core.rebind(&!slot, raw, 3)",
            CoreOperationId::Rebind,
            &[mutable.clone(), raw_u16.clone(), ScalarType::U64],
            &[],
        ),
        ("core.free(&!slot)", CoreOperationId::Free, &[mutable], &[]),
        ("core.free(raw)", CoreOperationId::Free, &[raw_u16], &[]),
        (
            "core.system_panic()",
            CoreOperationId::SystemPanic,
            &[],
            &[],
        ),
    ];
    let check_call = |structure: &OccurrenceStructure,
                      ordinal,
                      span: ByteSpan,
                      signature: &CheckedCallSignature,
                      spelling,
                      parameters: &[ScalarType],
                      outputs: &[ScalarType]| {
        assert_eq!(&text[span.start as usize..span.end as usize], spelling);
        assert_eq!(signature.outputs.span, span);
        assert_signature(signature, parameters, outputs, None);
        for (position, parameter) in parameters.iter().enumerate() {
            let argument = child(
                structure,
                Parent::Expression(ordinal),
                Edge::CallArgument(position),
            );
            assert_eq!(&structure.checked_types.expressions[&argument], parameter);
        }
    };
    let check = |initialization: &control::ModuleInitializationRegion,
                 function: &FunctionCallSites| {
        let ModuleInitializationDetail::Occurrences(topology) = &initialization.detail else {
            panic!("module initialization occurrences")
        };
        assert_eq!(topology.calls.len(), expected.len());
        assert_eq!(topology.checked_call_signatures.len(), expected.len());
        assert_eq!(function.call_occurrences.len(), expected.len());
        assert_eq!(function.checked_call_signatures.len(), expected.len());
        for ((&module_call, function_call), &(spelling, operation, parameters, outputs)) in topology
            .calls
            .iter()
            .zip(&function.call_occurrences)
            .zip(&expected)
        {
            assert_eq!(
                initialization.call_target(module_call),
                Some(&Target::Core(operation))
            );
            assert_eq!(
                function.sites[function_call.site].target,
                ResolvedCallTarget::Core(operation)
            );
            for (structure, ordinal, span, signature) in [
                (
                    &topology.structure,
                    module_call,
                    topology.occurrences[module_call.0].span,
                    &topology.checked_call_signatures[&module_call],
                ),
                (
                    &function.structure,
                    function_call.ordinal,
                    function.occurrences[function_call.ordinal.0].span,
                    &function.checked_call_signatures[&function_call.ordinal],
                ),
            ] {
                check_call(
                    structure, ordinal, span, signature, spelling, parameters, outputs,
                );
            }
        }
    };
    for target in [ScalarTargetLayout::NATIVE64, ScalarTargetLayout::WASM32] {
        let stages = Scope::new();
        let project = prepare_extracted(&parsed, target);
        check(
            project.initialization_region(0).unwrap(),
            project.calls.function(0, 0),
        );
        let program = PreparedProgram::new(
            derive_scalar_declarations_from_cst(parsed.canonical_cst()),
            target,
        )
        .unwrap_or_else(|failure| panic!("{target:?}: {:?}", failure.diagnostics));
        let calls = program.calls.as_ref().unwrap();
        check(&calls.initialization, calls.function(0));
        assert_eq!(
            stages.counts(),
            Counts {
                call_sites: 2,
                ..Counts::default()
            }
        );
    }
}

#[test]
fn prepare_publishes_checked_core_value_signatures_in_initialization_and_functions() {
    use crate::scalar::control::ModuleInitializationCallTarget as Target;
    use CoreOperationId::*;
    use ScalarType::*;

    let body = "
        u64 extended = core.int_extend<u64>(1);
        u8 truncated = core.int_trunc<u8>(1);
        f64 from_unsigned = core.uint_to_float<f64>(42);
        f32 from_signed = core.sint_to_float<f32>(-3);
        i32 to_signed = core.float_to_sint_trunc<i32>(from_unsigned);
        u64 to_unsigned = core.float_to_uint_trunc<u64>(from_signed);
        f32 narrow = core.float_trunc<f32>(1.5);
        f64 wide = core.float_extend<f64>(1.5);
        u32 bits = core.bitcast<u32>('a');
        artifact_id current = core.this_artifact_id();
        extended; truncated; to_signed; to_unsigned; narrow; wide; bits; current;
        unsafe {
            u16 slot = 0;
            *?u16 raw = &?slot;
            *?u16 next = core.offset<u16>(raw, -1);
            u16 loaded = core.load<u16>(next);
            *?u8 cast = core.pointer_cast<*?u8>(raw);
            *u16 checked = core.pointer_cast<*u16>(raw);
            loaded; cast; checked;
        };";
    let text = format!("%%start\nunit() run = fn {{ {body} }};\nunsafe {{ {body} }};\n%%end");
    let parsed = parsed(&text);
    let raw_u16 = RawPointer(Box::new(U16));
    let raw_u8 = RawPointer(Box::new(U8));
    let shared_u16 = CheckedReference {
        mutability: ScalarReferenceMutability::Shared,
        inner: Box::new(U16),
    };
    let expected: [(_, _, &[ScalarType], _); 14] = [
        ("core.int_extend<u64>(1)", IntExtend, &[I32], U64),
        ("core.int_trunc<u8>(1)", IntTrunc, &[I64], U8),
        ("core.uint_to_float<f64>(42)", UintToFloat, &[U32], F64),
        ("core.sint_to_float<f32>(-3)", SintToFloat, &[I32], F32),
        (
            "core.float_to_sint_trunc<i32>(from_unsigned)",
            FloatToSintTrunc,
            &[F64],
            I32,
        ),
        (
            "core.float_to_uint_trunc<u64>(from_signed)",
            FloatToUintTrunc,
            &[F32],
            U64,
        ),
        ("core.float_trunc<f32>(1.5)", FloatTrunc, &[F64], F32),
        ("core.float_extend<f64>(1.5)", FloatExtend, &[F32], F64),
        ("core.bitcast<u32>('a')", Bitcast, &[Char], U32),
        ("core.this_artifact_id()", ThisArtifactId, &[], ArtifactId),
        (
            "core.offset<u16>(raw, -1)",
            Offset,
            &[raw_u16.clone(), I64],
            raw_u16.clone(),
        ),
        (
            "core.load<u16>(next)",
            Load,
            std::slice::from_ref(&raw_u16),
            U16,
        ),
        (
            "core.pointer_cast<*?u8>(raw)",
            PointerCast,
            std::slice::from_ref(&raw_u16),
            raw_u8,
        ),
        (
            "core.pointer_cast<*u16>(raw)",
            PointerCast,
            std::slice::from_ref(&raw_u16),
            shared_u16,
        ),
    ];
    let check = |initialization: &control::ModuleInitializationRegion,
                 function: &FunctionCallSites| {
        let ModuleInitializationDetail::Occurrences(topology) = &initialization.detail else {
            panic!("module initialization occurrences")
        };
        assert_eq!(topology.calls.len(), expected.len());
        assert_eq!(function.call_occurrences.len(), expected.len());
        for ((&module_call, function_call), (spelling, operation, parameters, output)) in topology
            .calls
            .iter()
            .zip(&function.call_occurrences)
            .zip(&expected)
        {
            assert_eq!(
                initialization.call_target(module_call),
                Some(&Target::Core(*operation))
            );
            assert_eq!(
                function.sites[function_call.site].target,
                ResolvedCallTarget::Core(*operation)
            );
            for (structure, ordinal, span, signature) in [
                (
                    &topology.structure,
                    module_call,
                    topology.occurrences[module_call.0].span,
                    topology.checked_call_signatures.get(&module_call),
                ),
                (
                    &function.structure,
                    function_call.ordinal,
                    function.occurrences[function_call.ordinal.0].span,
                    function.checked_call_signatures.get(&function_call.ordinal),
                ),
            ] {
                let signature =
                    signature.unwrap_or_else(|| panic!("missing signature for {spelling}"));
                assert_eq!(signature.outputs.span, span);
                assert_eq!(&text[span.start as usize..span.end as usize], *spelling);
                assert_signature(signature, parameters, std::slice::from_ref(output), None);
                assert_eq!(&structure.checked_types.expressions[&ordinal], output);
                for (position, parameter) in parameters.iter().enumerate() {
                    let argument = child(
                        structure,
                        Parent::Expression(ordinal),
                        Edge::CallArgument(position),
                    );
                    assert_eq!(&structure.checked_types.expressions[&argument], parameter);
                }
            }
        }
    };
    for target in [ScalarTargetLayout::NATIVE64, ScalarTargetLayout::WASM32] {
        let project = prepare_extracted(&parsed, target);
        check(
            project.initialization_region(0).unwrap(),
            project.calls.function(0, 0),
        );
        let program = PreparedProgram::new(
            derive_scalar_declarations_from_cst(parsed.canonical_cst()),
            target,
        )
        .unwrap_or_else(|failure| panic!("{target:?}: {:?}", failure.diagnostics));
        let calls = program.calls.as_ref().unwrap();
        check(&calls.initialization, calls.function(0));
    }
}

#[test]
fn prepare_publishes_declare_artifact_signature_only_with_checked_utf8_context() {
    use crate::scalar::control::ModuleInitializationCallTarget as Target;

    let body = "artifact_id initial = core.declare_artifact(\"module\");
                artifact_id() run = fn { core.declare_artifact(\"callable\") };";
    let main = parsed(&format!(
        "%%start\nstd = namespace std \"bootstrap.w\";\n{body}\n%%end"
    ));
    let std_source = SourceIdentity::new(
        "project".into(),
        "std".into(),
        "bootstrap.w".into(),
        "r1".into(),
    );
    let std = parse_source(
        std_source.clone(),
        "%%start\nstruct utf8 { *?u8 data; u64 length; }\n%%end".into(),
        &[],
    );
    assert!(std.diagnostics.is_empty(), "{:?}", std.diagnostics);
    let standalone = parse_source(
        std_source.clone(),
        "%%start
struct utf8 { *?u8 data; u64 length; }
artifact_id initial = core.declare_artifact(\"module\");
artifact_id(bool, utf8) run = fn(choose, input) {
    core.declare_artifact(if (choose) { input } else { \"callable\" })
};
%%end"
            .into(),
        &[],
    );
    assert!(
        standalone.diagnostics.is_empty(),
        "{:?}",
        standalone.diagnostics
    );

    let check = |initialization: &control::ModuleInitializationRegion,
                 function: &FunctionCallSites,
                 utf8: &ScalarType| {
        let ModuleInitializationDetail::Occurrences(topology) = &initialization.detail else {
            panic!("module initialization occurrences")
        };
        let [module_call] = topology.calls.as_slice() else {
            panic!("one initialization call")
        };
        let [function_call] = function.call_occurrences.as_slice() else {
            panic!("one callable call")
        };
        let operation = CoreOperationId::DeclareArtifact;
        assert_eq!(
            initialization.call_target(*module_call),
            Some(&Target::Core(operation))
        );
        assert_eq!(
            function.sites[function_call.site].target,
            ResolvedCallTarget::Core(operation)
        );
        for (structure, ordinal, signature) in [
            (
                &topology.structure,
                *module_call,
                topology.checked_call_signatures.get(module_call),
            ),
            (
                &function.structure,
                function_call.ordinal,
                function.checked_call_signatures.get(&function_call.ordinal),
            ),
        ] {
            let argument = child(
                structure,
                Parent::Expression(ordinal),
                Edge::CallArgument(0),
            );
            assert_eq!(
                structure.checked_types.expressions.get(&argument),
                Some(utf8)
            );
            assert_signature(
                signature.expect("checked artifact declaration signature"),
                std::slice::from_ref(utf8),
                &[ScalarType::ArtifactId],
                None,
            );
        }
    };

    for target_layout in [ScalarTargetLayout::NATIVE64, ScalarTargetLayout::WASM32] {
        let declarations = derive_scalar_declarations_from_cst(main.canonical_cst());
        let namespace = declarations.namespaces().next().unwrap();
        let binding = ScalarNamespaceBinding {
            binding: namespace.binding.clone(),
            target: std_source.clone(),
            span: namespace.binding_span,
        };
        let project = crate::prepare_scalar_project(
            vec![
                (declarations, vec![binding]),
                (
                    derive_scalar_declarations_from_cst(std.result.canonical_cst()),
                    Vec::new(),
                ),
            ],
            vec![std_source.clone(), main.source.clone()],
            crate::ScalarPreparationInputs { target_layout },
        )
        .unwrap_or_else(|failure| panic!("{target_layout:?}: {:?}", failure.diagnostics));
        let utf8 = ScalarType::Struct(project.project().modules[1].structs[0].id.clone());
        check(
            project.initialization_region(0).unwrap(),
            project.calls.function(0, 2),
            &utf8,
        );

        let program = PreparedProgram::new(
            derive_scalar_declarations_from_cst(standalone.result.canonical_cst()),
            target_layout,
        )
        .unwrap_or_else(|failure| panic!("{target_layout:?}: {:?}", failure.diagnostics));
        let utf8 = ScalarType::Struct(program.program.structs[0].id.clone());
        let calls = program.calls.as_ref().unwrap();
        check(&calls.initialization, calls.function(1), &utf8);
    }
}

#[test]
fn standalone_prepare_rejects_invalid_declare_artifact_context() {
    for (package, declarations, operand, code, message) in [
        (
            "std",
            "struct utf8 { *?u8 data; u64 length; }",
            "true",
            "B0003",
            "expression type does not match expected type",
        ),
        (
            "std",
            "",
            "missing, \"extra\"",
            "B0004",
            "call argument arity does not match callable type",
        ),
        ("std", "", "\"name\"", "B0003", "std.utf8 is unavailable"),
        ("app", "", "\"name\"", "B0003", "std.utf8 is unavailable"),
        (
            "app",
            "struct utf8 { *?u8 data; u64 length; }",
            "\"name\"",
            "B0003",
            "std.utf8 is unavailable",
        ),
    ] {
        let call = format!("core.declare_artifact({operand})");
        let text = format!("%%start\n{declarations}\nartifact_id artifact = {call};\n%%end");
        let start = text.find(&call).unwrap();
        let source = SourceIdentity::new(
            "project".into(),
            package.into(),
            "bootstrap.w".into(),
            "r1".into(),
        );
        let span = SourceSpan::new(
            source.clone(),
            ByteSpan::new(start as u32, (start + call.len()) as u32),
        );
        let parsed = parse_source(source, text, &[]);
        assert!(parsed.diagnostics.is_empty(), "{:?}", parsed.diagnostics);
        for target in [ScalarTargetLayout::NATIVE64, ScalarTargetLayout::WASM32] {
            let result = PreparedProgram::new(
                derive_scalar_declarations_from_cst(parsed.result.canonical_cst()),
                target,
            );
            let Err(failure) = result else {
                panic!("{target:?} {package}: {call} passed preparation");
            };
            let [diagnostic] = failure.diagnostics.as_slice() else {
                panic!("{target:?} {package}: {call}: {:?}", failure.diagnostics);
            };
            assert_eq!(diagnostic.code, code);
            assert_eq!(diagnostic.message, message);
            let [label] = diagnostic.labels.as_slice() else {
                panic!("one primary call label: {diagnostic:?}");
            };
            assert_eq!(label.kind, crate::DiagnosticLabelKind::Primary);
            assert_eq!(label.span, span);
        }
    }
}

#[test]
fn public_prepare_types_independent_secondary_binding_roots() {
    let parsed = parsed(
        "%%start
generic T;
T(T) echo = fn(value) { value };
u8 first, i16 second, u32 third = 1, 1, echo<u32>(2);
u32(u32) run = fn(input) {
    u8 left, i16 middle, u32 right = 1, 1, echo<u32>(input);
    input; left; middle; right
};
%%end",
    );
    let check = |structure: &OccurrenceStructure, item| {
        let roots = [0, 1, 2].map(|output| {
            child(
                structure,
                Parent::Block(structure.root),
                Edge::BindingRoot { item, output },
            )
        });
        assert!(roots.windows(2).all(|pair| pair[0].0 < pair[1].0));
        let types = &structure.checked_types.expressions;
        assert_eq!(
            roots.map(|root| types.get(&root)),
            [
                Some(&ScalarType::U8),
                Some(&ScalarType::I16),
                Some(&ScalarType::U32),
            ]
        );
        let argument = child(
            structure,
            Parent::Expression(roots[2]),
            Edge::CallArgument(0),
        );
        assert_eq!(types[&argument], ScalarType::U32);
        roots[2]
    };
    let u32s = &[ScalarType::U32];
    for target_layout in [ScalarTargetLayout::NATIVE64, ScalarTargetLayout::WASM32] {
        let project = prepare_extracted(&parsed, target_layout);
        let initialization = project.initialization_region(0).unwrap();
        let ModuleInitializationDetail::Occurrences(topology) = &initialization.detail else {
            panic!("module initialization occurrences")
        };
        let call = check(&topology.structure, 1);
        assert_signature(&topology.checked_call_signatures[&call], u32s, u32s, None);
        let function = project.calls.function(0, 2);
        let call = check(&function.structure, 0);
        assert_signature(&function.checked_call_signatures[&call], u32s, u32s, None);

        let program = PreparedProgram::new(
            derive_scalar_declarations_from_cst(parsed.canonical_cst()),
            target_layout,
        )
        .unwrap_or_else(|failure| panic!("{:?}", failure.diagnostics));
        let calls = program.calls.as_ref().unwrap();
        let ModuleInitializationDetail::Occurrences(topology) = &calls.initialization.detail else {
            panic!("module initialization occurrences")
        };
        let call = check(&topology.structure, 1);
        assert_signature(&topology.checked_call_signatures[&call], u32s, u32s, None);
        let function = calls.function(2);
        let call = check(&function.structure, 0);
        assert_signature(&function.checked_call_signatures[&call], u32s, u32s, None);
    }
}

#[test]
fn secondary_binding_enum_variant_preparation_probe() {
    let parsed = parsed(
        "%%start
enum Status { ready; }
u8 first, Status status = 1, Status::ready;
Status() select = fn() {
    u8 marker, Status selected = 1, if (true) { Status value = Status::ready; value } else { Status::ready };
    marker; selected
};
%%end",
    );
    let mut failures = Vec::new();
    for target_layout in [ScalarTargetLayout::NATIVE64, ScalarTargetLayout::WASM32] {
        let declarations = derive_scalar_declarations_from_cst(parsed.canonical_cst());
        let order = vec![declarations.source().clone()];
        if let Err(failure) = crate::prepare_scalar_project(
            vec![(declarations, Vec::new())],
            order,
            crate::ScalarPreparationInputs { target_layout },
        ) {
            failures.push(("project", target_layout, failure.diagnostics));
        }
        if let Err(failure) = PreparedProgram::new(
            derive_scalar_declarations_from_cst(parsed.canonical_cst()),
            target_layout,
        ) {
            failures.push(("standalone", target_layout, failure.diagnostics));
        }
    }
    assert!(failures.is_empty(), "{failures:#?}");
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

#[test]
fn inherent_multi_output_growth_is_counted_separately() {
    let columns = [
        ("i32", ScalarType::I32, "11", "21"),
        ("bool", ScalarType::Bool, "true", "false"),
        ("u8", ScalarType::U8, "12", "22"),
        ("u16", ScalarType::U16, "13", "23"),
    ];
    for target in [ScalarTargetLayout::NATIVE64, ScalarTargetLayout::WASM32] {
        for outputs in [4, 8, 16] {
            let columns: Vec<_> = columns.iter().cloned().cycle().take(outputs).collect();
            let types: Vec<_> = columns.iter().map(|(_, ty, ..)| ty.clone()).collect();
            let [signature, then_values, else_values] = [0, 1, 2].map(|field| {
                columns
                    .iter()
                    .map(|(name, _, then_value, else_value)| {
                        [*name, *then_value, *else_value][field]
                    })
                    .collect::<Vec<_>>()
                    .join(", ")
            });
            let mut snapshot_at_four_sites = None;
            for sites in [4, 8, 16] {
                let mut text = format!(
                    "%%start\n({signature})(bool) many = fn(flag) {{ if (flag) {{ {then_values} }} else {{ {else_values} }} }};\ni32() run = fn() {{\n"
                );
                for site in 0..sites {
                    let receivers = columns
                        .iter()
                        .enumerate()
                        .map(|(output, (name, ..))| format!("{name} value_{site}_{output}"))
                        .collect::<Vec<_>>()
                        .join(", ");
                    let flag = site % 2 == 0;
                    writeln!(text, "{receivers} = many({flag}); value_{site}_0;").unwrap();
                }
                text.push_str("0\n};\n%%end");
                let parsed = parsed(&text);
                let (prepared, copies, headers) = {
                    let stages = Scope::new();
                    let copies = layout::observations::Scope::new();
                    let headers = overload_observations::Scope::new();
                    let prepared = prepare_extracted(&parsed, target);
                    assert_eq!(
                        stages.counts(),
                        Counts {
                            call_sites: 1,
                            ..Counts::default()
                        }
                    );
                    (prepared, copies.counts(), headers.counts())
                };
                assert_eq!(prepared.project().modules[0].target_layout, target);
                assert_eq!(
                    [
                        copies.program_copies,
                        copies.module_copies,
                        copies.struct_copies,
                        copies.cfg_copies,
                        copies.copied_payload_units,
                    ],
                    [0; 5]
                );
                assert_eq!(
                    [
                        headers.construction_program_copies,
                        headers.construction_module_copies,
                        headers.construction_struct_copies,
                        headers.construction_cfg_copies,
                        headers.construction_payload_units,
                    ],
                    [0; 5]
                );
                assert_eq!(
                    [
                        headers.program_copies,
                        headers.module_copies,
                        headers.struct_copies,
                        headers.cfg_copies,
                        headers.copied_context_payload_units,
                    ],
                    [0; 5]
                );
                assert_eq!((headers.contexts, headers.modules), (1, 1));
                assert_eq!(headers.snapshot.callable_headers, 2);
                assert_eq!(headers.snapshot.type_nodes, outputs + 4);
                if let Some(baseline) = &snapshot_at_four_sites {
                    assert_eq!(
                        &headers.snapshot, baseline,
                        "{target:?}: {sites} × {outputs}"
                    );
                } else {
                    snapshot_at_four_sites = Some(headers.snapshot.clone());
                }

                let callee = prepared.calls.function(0, 0);
                let root = &callee.structure.blocks[callee.structure.root.0];
                assert_eq!(root.items.len(), 1);
                let BlockItemKind::FinalOutput(primary) = root.items[0].kind else {
                    panic!("one canonical tail producer");
                };
                assert_eq!(callee.conditionals.len(), 1);
                assert!(callee.conditionals.contains_key(&primary.expression));
                assert!(callee.call_occurrences.is_empty());
                assert_eq!(callee.occurrences.len(), 2 + 2 * outputs);
                let finals: Vec<_> = root.final_outputs().collect();
                assert_eq!(finals.len(), outputs);
                assert_eq!(callee.structure.conditional_output_projections.len(), 1);
                assert_eq!(
                    callee.structure.conditional_output_projections[&primary.expression].types,
                    types
                );
                for (position, slot) in finals.iter().enumerate() {
                    assert_eq!(
                        (slot.expression, slot.output),
                        (primary.expression, position)
                    );
                }

                let caller = prepared.calls.function(0, 1);
                let structure = &caller.structure;
                let root = &structure.blocks[structure.root.0];
                let mut argument_ordinals = HashSet::new();
                for (_, &node) in structure
                    .edges
                    .iter()
                    .filter(|((_, edge), _)| matches!(edge, Edge::CallArgument(_)))
                {
                    let Node::Expression(argument) = node else {
                        panic!("argument expression")
                    };
                    assert!(argument_ordinals.insert(argument));
                }
                let argument_count = argument_ordinals.len();
                assert_eq!(argument_count, sites);
                assert_eq!(caller.occurrences.len(), 3 * sites + 1);
                assert_eq!(caller.call_occurrences.len(), sites);
                assert_eq!(caller.sites.len(), sites);
                assert_eq!(caller.checked_call_signatures.len(), sites);
                assert_eq!(root.items.len(), 2 * sites + 1);

                let mut projection_slots = 0;
                let mut receiver_slots = 0;
                for (site, entry) in caller.call_occurrences.iter().enumerate() {
                    let call = entry.ordinal;
                    assert_eq!(entry.site, site);
                    let argument =
                        child(structure, Parent::Expression(call), Edge::CallArgument(0));
                    assert!(argument_ordinals.remove(&argument));
                    assert_eq!(
                        structure.checked_types.expressions[&argument],
                        ScalarType::Bool
                    );
                    assert_signature(
                        &caller.checked_call_signatures[&call],
                        &[ScalarType::Bool],
                        &types,
                        None,
                    );

                    let item = 2 * site;
                    let BlockItemKind::Binding(binding) = &root.items[item].kind else {
                        panic!("independent call binding group");
                    };
                    let BindingInitialization::Outputs(values) = &binding.initialization else {
                        panic!("ordered call projections");
                    };
                    assert_eq!(values.primary, call);
                    assert_eq!(
                        child(
                            structure,
                            Parent::Block(structure.root),
                            Edge::BindingRoot { item, output: 0 },
                        ),
                        call
                    );
                    assert_eq!(values.secondary.len(), outputs - 1);
                    for (index, output) in values.secondary.iter().enumerate() {
                        assert_eq!(*output, BindingOutput::ProjectPrimary { output: index + 1 });
                    }
                    projection_slots += 1 + values.secondary.len();
                    assert_eq!(binding.receivers().len(), outputs);
                    for (position, slot) in binding.receivers().iter().enumerate() {
                        assert_eq!(slot.position, position);
                        assert_eq!(slot.name, format!("value_{site}_{position}"));
                        assert_eq!(slot.ty, types[position]);
                    }
                    receiver_slots += binding.receivers().len();
                }
                assert!(argument_ordinals.is_empty());
                let signature_slots: usize = caller
                    .checked_call_signatures
                    .values()
                    .map(|signature| signature.outputs.outputs.len())
                    .sum();
                assert_eq!(
                    [signature_slots, projection_slots, receiver_slots],
                    [sites * outputs; 3]
                );
                eprintln!(
                    "{target:?} {sites} sites × {outputs} results: {} caller occurrences, {} callee occurrences, {argument_count} argument occurrences; {signature_slots} signature, {projection_slots} binding projection and {receiver_slots} receiver slots; {} compact header type nodes",
                    caller.occurrences.len(), callee.occurrences.len(), headers.snapshot.type_nodes,
                );
            }
        }
    }
}

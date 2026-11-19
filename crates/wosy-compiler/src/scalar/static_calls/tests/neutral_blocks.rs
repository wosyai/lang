use super::*;
use crate::scalar::control::{
    BindingAllocationLengthSlot, BindingInitialization, BindingOutput, BindingOutputs,
    BlockItemKind, ModuleInitializationDetail,
};
use reference::{
    OccurrenceEdge as Edge, OccurrenceMetadata, OccurrenceNode as Node, OccurrenceOrdinal,
    OccurrenceParent as Parent, OccurrenceStructure,
};

#[test]
fn prepared_block_table_owns_ordered_bindings_nested_scopes_and_final_output_slots() {
    let text = "%%start\n(u8, bool)(bool) pair = fn(flag) { u8 value = 1; value, flag };\nunit(bool) run = fn(flag) { u8 value, bool valid = pair(flag); value; valid; unsafe { value; }; };\n%%end";
    for target in [ScalarTargetLayout::WASM32, ScalarTargetLayout::NATIVE64] {
        let parsed = parsed(text);
        let declarations = derive_scalar_declarations_from_cst(parsed.canonical_cst());
        let stages = Scope::new();
        let copies = layout::observations::Scope::new();
        let prepared = PreparedProgram::new(declarations, target)
            .unwrap_or_else(|error| panic!("{:?}", error.diagnostics));
        assert_eq!(
            stages.counts(),
            Counts {
                call_sites: 1,
                ..Counts::default()
            }
        );
        assert_eq!(copies.counts().program_copies, 0);
        assert_eq!(copies.counts().module_copies, 0);
        assert_eq!(copies.counts().cfg_copies, 0);
        drop(copies);
        drop(stages);
        let calls = prepared.calls.as_ref().unwrap();
        let pair = calls.function(0);
        let pair_block = &pair.structure.blocks[pair.structure.root.0];
        let ScalarItem::Function(pair_syntax) = &prepared.program.items[0] else {
            unreachable!()
        };
        assert_eq!(pair_block.span, pair_syntax.body.span);
        assert_eq!(
            pair_block
                .items
                .iter()
                .map(|item| item.position)
                .collect::<Vec<_>>(),
            vec![0, 1, 2]
        );
        assert!(matches!(
            pair_block.items[0].kind,
            BlockItemKind::Binding(_)
        ));
        let outputs = pair_block.final_outputs().collect::<Vec<_>>();
        assert_eq!(outputs.len(), 2);
        for (position, slot) in outputs.iter().enumerate() {
            let BlockItemKind::FinalOutput(stored) = &pair_block.items[position + 1].kind else {
                panic!("stored final slot")
            };
            assert!(std::ptr::eq(*slot, stored));
            assert_eq!(slot.item, position + 1);
            assert_eq!(slot.output, position);
            assert_eq!(
                slot.span,
                pair_syntax.body.final_output_values[position].span
            );
        }
        let row = calls.function(1);
        let root = &row.structure.blocks[row.structure.root.0];
        let ScalarItem::Function(run) = &prepared.program.items[1] else {
            unreachable!()
        };
        assert_eq!(root.span, run.body.span);
        assert_eq!(
            root.items
                .iter()
                .map(|item| item.position)
                .collect::<Vec<_>>(),
            vec![0, 1, 2, 3]
        );
        let BlockItemKind::Binding(binding) = &root.items[0].kind else {
            panic!("binding install region")
        };
        let BindingInitialization::Outputs(BindingOutputs { primary, secondary }) =
            &binding.initialization
        else {
            panic!("call initializer")
        };
        assert_eq!(
            row.structure.edges[&(
                Parent::Block(row.structure.root),
                Edge::BindingRoot { item: 0, output: 0 }
            )],
            Node::Expression(*primary)
        );
        assert_eq!(
            secondary.as_slice(),
            [BindingOutput::ProjectPrimary { output: 1 }]
        );
        assert!(matches!(
            root.items[1].kind,
            BlockItemKind::Evaluation { .. }
        ));
        assert!(matches!(
            root.items[2].kind,
            BlockItemKind::Evaluation { .. }
        ));
        let BlockItemKind::Evaluation { expression } = root.items[3].kind else {
            panic!("nested block expression")
        };
        let Node::Block(nested) =
            row.structure.edges[&(Parent::Expression(expression), Edge::BlockBody)]
        else {
            unreachable!()
        };
        let region = &row.structure.blocks[nested.0];
        let ScalarBlockItem::Expression(ScalarExpression::Block(syntax)) = &run.body.items[3]
        else {
            panic!("nested syntax block")
        };
        assert_eq!(region.span, syntax.span);
        assert_eq!(region.items.len(), 1);
        assert_eq!(region.items[0].position, 0);
        assert!(matches!(
            region.items[0].kind,
            BlockItemKind::Evaluation { .. }
        ));
        assert_eq!(region.final_outputs().count(), 0);
        assert_eq!(root.final_outputs().count(), 0);
        let before = prepared.program.clone();
        let stages = Scope::new();
        let results = prepared.analyze().unwrap();
        assert_eq!(
            stages.counts(),
            Counts {
                call_sites: 0,
                reference_origins: 1,
                output_contracts: 1,
                copy_specialization: 1
            }
        );
        drop(stages);
        let mut installed = before.clone();
        let errors = results.install(&mut installed);
        assert!(errors.is_empty(), "{errors:?}");
        let ScalarItem::Function(run) = &installed.items[1] else {
            unreachable!()
        };
        let points = &run.reference_cfg.points;
        let enters = points
            .iter()
            .filter(|point| matches!(point.kind, CfgPointKind::ScopeEnter))
            .collect::<Vec<_>>();
        let exits = points
            .iter()
            .filter(|point| matches!(point.kind, CfgPointKind::ScopeExit))
            .collect::<Vec<_>>();
        assert_eq!(enters.len(), 2);
        assert_eq!(exits.len(), 2);
        assert_eq!(enters[0].source_span.range, root.span);
        assert_eq!(enters[1].source_span.range, region.span);
        assert_eq!(exits[0].source_span.range, region.span);
        assert_eq!(exits[1].source_span.range, root.span);
        let calls = points
            .iter()
            .filter_map(|point| match &point.kind {
                CfgPointKind::Call {
                    output,
                    argument_points,
                    ..
                } => Some((*output, argument_points)),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(calls.len(), 2);
        assert_eq!((calls[0].0, calls[1].0), (0, 1));
        assert_eq!(calls[0].1, calls[1].1);
        assert_eq!(prepared.program, before);
        let finished = prepared.finish();
        assert_eq!(finished.program, installed);
        assert!(finished.diagnostics.is_empty());
    }
}

#[test]
fn public_prepare_retains_typed_allocation_lengths_without_query_calls() {
    let text = "%%start\nu64(u64) length = fn(n) { n };\nu8[length(length(1) + 2)] module_bytes;\nu64(u64) run = fn(n) { u8[length(length(n) + 3)] local_bytes; local_bytes; length(n) };\n%%end";
    let parsed = parsed(text);
    let check = |structure: &OccurrenceStructure,
                 occurrences: &[OccurrenceMetadata],
                 structural_calls: &[OccurrenceOrdinal],
                 item,
                 input,
                 increment| {
        let block = &structure.blocks[structure.root.0];
        let binding = block
            .items
            .iter()
            .find(|entry| entry.position == item)
            .unwrap();
        let BlockItemKind::Binding(binding) = &binding.kind else {
            panic!("binding region")
        };
        let BindingInitialization::Allocation { lengths } = &binding.initialization else {
            panic!("allocation initializer")
        };
        let [BindingAllocationLengthSlot {
            position: 0,
            expression,
            ty: ScalarType::U64,
        }] = lengths.as_slice()
        else {
            panic!("one U64 allocation length slot: {lengths:?}")
        };
        let root = Parent::Block(structure.root);
        assert!(!structure
            .edges
            .contains_key(&(root, Edge::BindingRoot { item, output: 0 })));
        assert_eq!(
            structure.edges[&(root, Edge::BindingAllocationLength { item, position: 0 })],
            Node::Expression(*expression)
        );
        let child = |parent, edge| {
            let Node::Expression(child) = structure.edges[&(Parent::Expression(parent), edge)]
            else {
                panic!("allocation length operand")
            };
            child
        };
        let spelling = |ordinal: OccurrenceOrdinal| {
            let span = occurrences[ordinal.0].span;
            &text[span.start as usize..span.end as usize]
        };
        let argument = child(*expression, Edge::CallArgument(0));
        let nested = child(argument, Edge::BinaryLeft);
        assert_eq!(structural_calls, [*expression, nested]);
        assert_eq!(
            spelling(*expression),
            format!("length(length({input}) + {increment})")
        );
        assert_eq!(spelling(child(nested, Edge::CallArgument(0))), input);
        assert_eq!(spelling(child(argument, Edge::BinaryRight)), increment);
    };
    for target in [ScalarTargetLayout::WASM32, ScalarTargetLayout::NATIVE64] {
        let prepared = prepare_extracted(&parsed, target);
        let initialization = prepared.initialization_region(0).unwrap();
        let ModuleInitializationDetail::Occurrences(topology) = &initialization.detail else {
            panic!("module initialization topology")
        };
        check(
            &topology.structure,
            &topology.occurrences,
            &topology.calls,
            1,
            "1",
            "2",
        );
        assert!(!topology.structure.edges.contains_key(&(
            Parent::Block(topology.structure.root),
            Edge::QueryAllocationPlaceholder { item: 1, output: 0 },
        )));
        let function = prepared.calls.function(0, 2);
        check(
            &function.structure,
            &function.occurrences,
            &function.allocation_length_calls,
            0,
            "n",
            "3",
        );
        let ScalarItem::Function(length) = &prepared.project().modules[0].items[0] else {
            panic!("length declaration")
        };
        let target = ResolvedCallTarget::ModuleCallable {
            source: parsed.source.clone(),
            declaration_span: length.name_span,
            concrete: ConcreteCallSelection::Declaration,
        };
        assert_eq!(function.allocation_length_call_targets.len(), 2);
        for call in &function.allocation_length_calls {
            assert_eq!(function.allocation_length_call_targets[call], target);
        }
        let root = Parent::Block(function.structure.root);
        assert_ne!(
            function.structure.edges[&(
                root,
                Edge::QueryAllocationPlaceholder { item: 0, output: 0 }
            )],
            function.structure.edges[&(
                root,
                Edge::BindingAllocationLength {
                    item: 0,
                    position: 0
                }
            )],
        );
        let [site] = function.sites.as_slice() else {
            panic!("only the final-output call belongs to the query")
        };
        assert_eq!(site.target, target);
        let span = site.source_span.range;
        assert_eq!(&text[span.start as usize..span.end as usize], "length(n)");
        let output = function.structure.blocks[function.structure.root.0]
            .final_outputs()
            .next()
            .unwrap();
        assert_eq!(function.call_occurrences.len(), 1);
        assert_eq!(function.call_occurrences[0].ordinal, output.expression);
        let build = crate::analyze_scalar_build(&prepared);
        assert!(build.diagnostics().is_empty(), "{:?}", build.diagnostics());
        let cfg = build.reference_cfg(0, 2, None).unwrap();
        assert_eq!(
            cfg.points
                .iter()
                .filter(|point| matches!(point.kind, CfgPointKind::Call { .. }))
                .count(),
            1
        );
    }
}

#[test]
fn callable_allocation_length_targets_use_pre_binding_scope() {
    let parsed = parsed(
        "%%start\ngeneric T;\nT(T) echo = fn(value) { value };\n\
         unit((u64(u64))) run = fn(length) { u8[echo<u64>(length(1))] bytes; bytes; };\n%%end",
    );
    for target in [ScalarTargetLayout::WASM32, ScalarTargetLayout::NATIVE64] {
        let prepared = prepare_extracted(&parsed, target);
        let items = &prepared.project().modules[0].items;
        let (ScalarItem::Function(echo), ScalarItem::Function(run)) = (&items[0], &items[1]) else {
            panic!("echo and run declarations")
        };
        let function = prepared.calls.function(0, 1);
        let [outer, inner] = function.allocation_length_calls.as_slice() else {
            panic!("generic length call and its callable parameter argument")
        };
        assert_eq!(function.allocation_length_call_targets.len(), 2);
        assert_eq!(
            function.allocation_length_call_targets[outer],
            ResolvedCallTarget::ModuleCallable {
                source: parsed.source.clone(),
                declaration_span: echo.name_span,
                concrete: ConcreteCallSelection::Declaration,
            },
        );
        assert_eq!(
            function.allocation_length_call_targets[inner],
            ResolvedCallTarget::LocalCallable(ReferenceBindingId {
                source: parsed.source.clone(),
                function_span: run.span,
                declaration_span: run.parameter_spans[0],
                block_span: run.body.span,
                kind: ReferenceBindingKind::Declared,
            }),
        );
        assert_eq!(
            function.structure.edges[&(Parent::Expression(*outer), Edge::CallArgument(0))],
            Node::Expression(*inner),
        );
        assert!(function.sites.is_empty());
        assert!(function.call_occurrences.is_empty());
        let build = crate::analyze_scalar_build(&prepared);
        assert!(build.diagnostics().is_empty(), "{:?}", build.diagnostics());
        let cfg = build.reference_cfg(0, 1, None).unwrap();
        assert!(cfg
            .points
            .iter()
            .all(|point| !matches!(point.kind, CfgPointKind::Call { .. })));
    }
}

#[test]
fn raw_allocation_placeholder_preserves_callable_query_diagnostics() {
    let text = "%%start\nu64(u64) length = fn(n) { n };\nunit(u64) run = fn(n) { u8[length(n)] bytes; bytes; };\n%%end";
    let parsed = parsed(text);
    for target in [ScalarTargetLayout::WASM32, ScalarTargetLayout::NATIVE64] {
        let declarations = derive_scalar_declarations_from_cst(parsed.canonical_cst());
        let raw = raw_project(declarations, target);
        for call in [true, false] {
            let mut project = raw.clone();
            let module = &mut project.modules[0];
            let ScalarItem::Function(run) = &mut module.items[1] else {
                panic!("run function")
            };
            let ScalarBlockItem::LocalBinding(binding) = &mut run.body.items[0] else {
                panic!("allocation binding")
            };
            assert!(binding.is_allocation);
            let span = expression_span(&binding.value);
            let source_span = SourceSpan::new(module.source.clone(), span);
            let missing = "missing".to_owned();
            let expected = if call {
                let mut placeholder = binding.receivers[0].allocation_length.clone().unwrap();
                let ScalarExpression::Call {
                    name,
                    span: call_span,
                    ..
                } = &mut placeholder
                else {
                    panic!("length call")
                };
                *name = missing;
                *call_span = span;
                binding.value = placeholder;
                ReferenceAnalysisError::MissingContext {
                    source_span,
                    reason: ReferenceAnalysisErrorReason::MissingResolvedCallTarget,
                }
            } else {
                binding.value = ScalarExpression::Name {
                    name: missing.clone(),
                    span,
                };
                ReferenceAnalysisError::MissingBinding {
                    source_span,
                    name: missing,
                }
            };
            let validation = crate::validate_scalar_project(project);
            assert_eq!(validation.diagnostics, [expected.diagnostic()]);
        }
    }
}

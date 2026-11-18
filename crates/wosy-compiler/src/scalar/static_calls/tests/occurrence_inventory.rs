use super::*;
use reference::{
    BlockOrdinal, OccurrenceEdge as Edge, OccurrenceNode as Node, OccurrenceParent as Parent,
};
use reference::{CallOccurrence, OccurrenceOrdinal, OccurrenceRole};

#[test]
fn preparation_publishes_stable_callable_owned_expression_place_and_call_ordinals() {
    let text = "%%start\nu8 padding = 0;\nbool(bool) echo = fn(flag) { flag };\n(u8, bool)(bool) pair = fn(flag) { u8 value = 1; value, flag };\nu64(bool) index = fn(flag) { flag; 0 };\nprobe = overload { unit(bool) => fn(flag) { u8 a, bool valid = pair(echo(flag)); a; valid; u8[2] cells = [1, 2]; if (flag) { echo(flag); } else { echo(flag); }; cells[index(flag)] = a; cells; }; unit(u8) => fn(byte) { byte; }; };\n%%end";
    let mut expected_inventory = None;
    for target in [ScalarTargetLayout::WASM32, ScalarTargetLayout::NATIVE64] {
        let parsed = parsed(text);
        let declarations = derive_scalar_declarations_from_cst(parsed.canonical_cst());
        let scope = Scope::new();
        let copies = layout::observations::Scope::new();
        let prepared = PreparedProgram::new(declarations, target)
            .unwrap_or_else(|error| panic!("{:?}", error.diagnostics));
        assert_eq!(
            scope.counts(),
            Counts {
                call_sites: 1,
                ..Counts::default()
            }
        );
        assert_eq!(copies.counts().program_copies, 0);
        assert_eq!(copies.counts().module_copies, 0);
        assert_eq!(copies.counts().cfg_copies, 0);
        drop(copies);
        drop(scope);
        let calls = prepared.calls.as_ref().unwrap();
        let owner = calls.function(4);
        assert!(owner.occurrences.is_empty());
        assert_eq!(owner.arms.len(), 2);
        let row = &owner.arms[0];
        let root = Parent::Block(row.structure.root);
        assert_eq!(row.structure.root, BlockOrdinal(0));
        assert_eq!(row.structure.blocks.len(), 3);
        assert_eq!(row.structure.edges.len(), 22);
        for (edge, node) in [
            (
                Edge::BindingRoot { item: 0, output: 0 },
                Node::Expression(OccurrenceOrdinal(0)),
            ),
            (
                Edge::ExpressionItem { item: 1 },
                Node::Expression(OccurrenceOrdinal(3)),
            ),
            (
                Edge::ExpressionItem { item: 2 },
                Node::Expression(OccurrenceOrdinal(4)),
            ),
            (
                Edge::BindingRoot { item: 3, output: 0 },
                Node::Expression(OccurrenceOrdinal(5)),
            ),
            (
                Edge::ExpressionItem { item: 4 },
                Node::Expression(OccurrenceOrdinal(8)),
            ),
            (
                Edge::AssignmentRhs { item: 5, output: 0 },
                Node::Expression(OccurrenceOrdinal(14)),
            ),
            (
                Edge::AssignmentDestination {
                    item: 5,
                    destination: 0,
                },
                Node::Place(OccurrenceOrdinal(15)),
            ),
            (
                Edge::ExpressionItem { item: 6 },
                Node::Expression(OccurrenceOrdinal(19)),
            ),
        ] {
            assert_eq!(row.structure.edges[&(root, edge)], node);
        }
        assert!(!row
            .structure
            .edges
            .contains_key(&(root, Edge::BindingRoot { item: 0, output: 1 })));
        for (parent, edge, node) in [
            (
                Parent::Expression(OccurrenceOrdinal(0)),
                Edge::CallArgument(0),
                Node::Expression(OccurrenceOrdinal(1)),
            ),
            (
                Parent::Expression(OccurrenceOrdinal(1)),
                Edge::CallArgument(0),
                Node::Expression(OccurrenceOrdinal(2)),
            ),
            (
                Parent::Expression(OccurrenceOrdinal(5)),
                Edge::AggregateElement(0),
                Node::Expression(OccurrenceOrdinal(6)),
            ),
            (
                Parent::Expression(OccurrenceOrdinal(5)),
                Edge::AggregateElement(1),
                Node::Expression(OccurrenceOrdinal(7)),
            ),
            (
                Parent::Expression(OccurrenceOrdinal(8)),
                Edge::Condition,
                Node::Expression(OccurrenceOrdinal(9)),
            ),
            (
                Parent::Expression(OccurrenceOrdinal(8)),
                Edge::ThenBranch,
                Node::Block(BlockOrdinal(1)),
            ),
            (
                Parent::Expression(OccurrenceOrdinal(8)),
                Edge::ElseBranch,
                Node::Block(BlockOrdinal(2)),
            ),
            (
                Parent::Block(BlockOrdinal(1)),
                Edge::ExpressionItem { item: 0 },
                Node::Expression(OccurrenceOrdinal(10)),
            ),
            (
                Parent::Expression(OccurrenceOrdinal(10)),
                Edge::CallArgument(0),
                Node::Expression(OccurrenceOrdinal(11)),
            ),
            (
                Parent::Block(BlockOrdinal(2)),
                Edge::ExpressionItem { item: 0 },
                Node::Expression(OccurrenceOrdinal(12)),
            ),
            (
                Parent::Expression(OccurrenceOrdinal(12)),
                Edge::CallArgument(0),
                Node::Expression(OccurrenceOrdinal(13)),
            ),
            (
                Parent::Place(OccurrenceOrdinal(15)),
                Edge::PlaceBase,
                Node::Place(OccurrenceOrdinal(16)),
            ),
            (
                Parent::Place(OccurrenceOrdinal(15)),
                Edge::PlaceIndex,
                Node::Expression(OccurrenceOrdinal(17)),
            ),
            (
                Parent::Expression(OccurrenceOrdinal(17)),
                Edge::CallArgument(0),
                Node::Expression(OccurrenceOrdinal(18)),
            ),
        ] {
            assert_eq!(row.structure.edges[&(parent, edge)], node);
        }
        assert_eq!(row.occurrences.len(), 20);
        assert_eq!(
            row.call_occurrences,
            vec![
                CallOccurrence {
                    ordinal: OccurrenceOrdinal(0),
                    site: 0
                },
                CallOccurrence {
                    ordinal: OccurrenceOrdinal(1),
                    site: 1
                },
                CallOccurrence {
                    ordinal: OccurrenceOrdinal(10),
                    site: 2
                },
                CallOccurrence {
                    ordinal: OccurrenceOrdinal(12),
                    site: 3
                },
                CallOccurrence {
                    ordinal: OccurrenceOrdinal(17),
                    site: 4
                },
            ]
        );
        assert_eq!(row.sites.len(), 5);
        for (site, item) in row.sites.iter().zip([2, 1, 1, 1, 3]) {
            let ScalarItem::Function(callee) = &prepared.program.items[item] else {
                panic!("callee item ordinal")
            };
            assert_eq!(
                site.target,
                ResolvedCallTarget::ModuleCallable {
                    source: prepared.program.source.clone(),
                    declaration_span: callee.name_span,
                    concrete: ConcreteCallSelection::Declaration,
                }
            );
        }
        assert_eq!(owner.arms[1].occurrences.len(), 1);
        assert!(owner.arms[1].call_occurrences.is_empty());
        let ScalarItem::Function(function) = &prepared.program.items[4] else {
            panic!("actual function item ordinal")
        };
        let arm = &function.overload_arms[0];
        assert_eq!(row.structure.blocks[0], arm.body.span);
        let ScalarBlockItem::Expression(ScalarExpression::If {
            then_branch,
            else_branch,
            ..
        }) = &arm.body.items[4]
        else {
            panic!("conditional item")
        };
        assert_eq!(row.structure.blocks[1], then_branch.span);
        assert_eq!(row.structure.blocks[2], else_branch.span);
        let ScalarBlockItem::LocalBinding(binding) = &arm.body.items[0] else {
            panic!("projected binding")
        };
        let ScalarExpression::Call { arguments, .. } = &binding.value else {
            panic!("primary call")
        };
        let ScalarExpression::Call {
            arguments: nested, ..
        } = &arguments[0]
        else {
            panic!("nested argument")
        };
        assert_eq!(row.occurrences[0].span, expression_span(&binding.value));
        assert_eq!(row.occurrences[1].span, expression_span(&arguments[0]));
        assert_eq!(row.occurrences[2].span, expression_span(&nested[0]));
        assert_eq!(
            row.occurrences
                .iter()
                .filter(|metadata| metadata.span == expression_span(&arguments[0]))
                .count(),
            1
        );
        let assignment = arm
            .body
            .items
            .iter()
            .find_map(|item| match item {
                ScalarBlockItem::Assignment(assignment) => Some(assignment),
                _ => None,
            })
            .unwrap();
        let group = &row.assignments[&(row.structure.root, 5)];
        assert_eq!(group.span, assignment.span);
        assert_eq!(
            group
                .rhs_evaluations()
                .map(|root| (root.position, root.expression))
                .collect::<Vec<_>>(),
            vec![(0, OccurrenceOrdinal(14))]
        );
        assert!(group.projected_outputs().next().is_none());
        assert_eq!(
            group
                .destinations()
                .map(|slot| (slot.position, slot.destination, slot.target_span))
                .collect::<Vec<_>>(),
            vec![(0, OccurrenceOrdinal(15), assignment.targets[0].target_span)]
        );
        assert_eq!(
            group.previous_observations().collect::<Vec<_>>(),
            group.installs().collect::<Vec<_>>()
        );
        let place = &assignment.targets[0].place;
        let ScalarPlace::Index { base, index, .. } = place else {
            panic!("projected selector")
        };
        assert_eq!(row.occurrences[15].span, scalar_place_target_span(place));
        assert_eq!(row.occurrences[16].span, scalar_place_target_span(base));
        assert_eq!(row.occurrences[17].span, expression_span(index));
        for (ordinal, metadata) in row.occurrences.iter().enumerate() {
            assert_eq!(
                metadata.role,
                if ordinal == 15 || ordinal == 16 {
                    OccurrenceRole::Place
                } else {
                    OccurrenceRole::Expression
                }
            );
        }
        for association in &row.call_occurrences {
            let metadata = row.occurrences[association.ordinal.0];
            assert_eq!(metadata.role, OccurrenceRole::Expression);
            assert_eq!(
                row.sites[association.site].source_span,
                SourceSpan::new(prepared.program.source.clone(), metadata.span)
            );
        }
        let qualified = (
            prepared.program.source.clone(),
            4,
            0,
            row.occurrences.clone(),
            row.call_occurrences.clone(),
            row.structure.clone(),
        );
        if let Some(expected) = &expected_inventory {
            assert_eq!(&qualified, expected);
        } else {
            expected_inventory = Some(qualified);
        }
        let before = prepared.program.clone();
        let scope = Scope::new();
        let results = prepared.analyze().unwrap();
        assert_eq!(scope.counts().call_sites, 0);
        drop(scope);
        let mut installed = before.clone();
        let errors = results.install(&mut installed);
        assert_eq!(
            errors,
            vec![ReferenceAnalysisError::MissingContext {
                source_span: SourceSpan::new(before.source.clone(), ByteSpan::new(338, 342)),
                reason: ReferenceAnalysisErrorReason::MissingResolvedBinding,
            }]
        );
        assert_eq!(prepared.program, before);
        let finished = prepared.finish();
        assert_eq!(finished.program, installed);
        assert_eq!(
            finished.diagnostics,
            errors
                .iter()
                .map(ReferenceAnalysisError::diagnostic)
                .collect::<Vec<_>>()
        );
        let public = derive_scalar_program_from_cst_with_layout(parsed.canonical_cst(), target);
        assert_eq!(finished.program, public.program);
        assert_eq!(finished.diagnostics, public.diagnostics);
    }
}

#[test]
fn preparation_keeps_equal_span_independent_occurrences_and_earlier_legacy_call_row() {
    let text = "%%start\nu8(u8) number = fn(input) { input };\nunit(u8) probe = fn(input) { u8 left, u8 right = number(input) + input, number(input) + input; left; right; };\n%%end";
    for target in [ScalarTargetLayout::WASM32, ScalarTargetLayout::NATIVE64] {
        let parsed = parsed(text);
        let validated = derive_scalar_program_from_cst_with_layout(parsed.canonical_cst(), target);
        assert!(
            validated.diagnostics.is_empty(),
            "{:?}",
            validated.diagnostics
        );
        let mut declarations = derive_scalar_declarations_from_cst(parsed.canonical_cst());
        let ScalarItem::Function(function) = &mut declarations.items[1] else {
            panic!("probe")
        };
        let ScalarBlockItem::LocalBinding(binding) = &mut function.body.items[0] else {
            panic!("independent roots")
        };
        let ScalarExpression::Binary {
            left,
            span: root_span,
            ..
        } = &binding.value
        else {
            panic!("primary root")
        };
        let root_span = *root_span;
        let call_span = expression_span(left);
        let ScalarExpression::Binary { left, span, .. } = &mut binding.output_values[1].value
        else {
            panic!("second root")
        };
        *span = root_span;
        let ScalarExpression::Call { span, .. } = left.as_mut() else {
            panic!("second nested call")
        };
        *span = call_span;
        let scope = Scope::new();
        let copies = layout::observations::Scope::new();
        let mut prepared = PreparedProgram::new(declarations, target)
            .unwrap_or_else(|error| panic!("{:?}", error.diagnostics));
        assert_eq!(
            scope.counts(),
            Counts {
                call_sites: 1,
                ..Counts::default()
            }
        );
        assert_eq!(copies.counts().program_copies, 0);
        assert_eq!(copies.counts().module_copies, 0);
        assert_eq!(copies.counts().cfg_copies, 0);
        drop(copies);
        drop(scope);
        let row = prepared.calls.as_ref().unwrap().function(1);
        let root = Parent::Block(row.structure.root);
        assert_eq!(row.structure.blocks.len(), 1);
        assert_eq!(row.structure.edges.len(), 10);
        assert_eq!(
            row.structure.edges[&(root, Edge::BindingRoot { item: 0, output: 0 })],
            Node::Expression(OccurrenceOrdinal(0))
        );
        assert_eq!(
            row.structure.edges[&(root, Edge::BindingRoot { item: 0, output: 1 })],
            Node::Expression(OccurrenceOrdinal(4))
        );
        for (parent, edge, child) in [
            (0, Edge::BinaryLeft, 1),
            (0, Edge::BinaryRight, 3),
            (1, Edge::CallArgument(0), 2),
            (4, Edge::BinaryLeft, 5),
            (4, Edge::BinaryRight, 7),
            (5, Edge::CallArgument(0), 6),
        ] {
            assert_eq!(
                row.structure.edges[&(Parent::Expression(OccurrenceOrdinal(parent)), edge)],
                Node::Expression(OccurrenceOrdinal(child))
            );
        }
        assert_eq!(row.occurrences.len(), 10);
        assert_eq!(row.occurrences[0], row.occurrences[4]);
        assert_eq!(row.occurrences[0].span, root_span);
        assert_eq!(row.occurrences[1], row.occurrences[5]);
        assert_eq!(row.occurrences[1].span, call_span);
        assert_eq!(
            row.call_occurrences,
            vec![
                CallOccurrence {
                    ordinal: OccurrenceOrdinal(1),
                    site: 0
                },
                CallOccurrence {
                    ordinal: OccurrenceOrdinal(5),
                    site: 0
                },
            ]
        );
        assert_eq!(row.sites.len(), 1);
        assert_eq!(row.sites[0].source_span.range, call_span);
        let sites_before = row.sites.clone();
        let before = prepared.program.clone();
        let scope = Scope::new();
        let results = prepared.analyze().unwrap();
        assert_eq!(scope.counts().call_sites, 0);
        drop(scope);
        let mut installed = before.clone();
        assert!(results.install(&mut installed).is_empty());
        assert_eq!(prepared.program, before);
        let row = prepared.calls.as_mut().unwrap().function_mut(1);
        assert_eq!(row.call_occurrences.remove(1).ordinal, OccurrenceOrdinal(5));
        assert_eq!(
            row.call_occurrences,
            vec![CallOccurrence {
                ordinal: OccurrenceOrdinal(1),
                site: 0
            }]
        );
        assert_eq!(row.sites, sites_before);
        let scope = Scope::new();
        let results = prepared.analyze().unwrap();
        assert_eq!(scope.counts().call_sites, 0);
        drop(scope);
        let mut partial = before.clone();
        let errors = results.install(&mut partial);
        assert_eq!(
            errors,
            vec![ReferenceAnalysisError::MissingContext {
                source_span: SourceSpan::new(before.source.clone(), call_span),
                reason: ReferenceAnalysisErrorReason::MissingResolvedCallTarget,
            }]
        );
        let ScalarItem::Function(probe) = &partial.items[1] else {
            unreachable!()
        };
        let retained = probe
            .reference_cfg
            .points
            .iter()
            .filter(|point| matches!(point.kind, CfgPointKind::Call { .. }))
            .collect::<Vec<_>>();
        assert_eq!(retained.len(), 1);
        assert_eq!(retained[0].source_span.range, call_span);
        assert!(probe.reference_cfg.initial_points.is_empty());
        assert_eq!(prepared.program, before);
        let finished = prepared.finish();
        assert_eq!(finished.program, partial);
        assert_eq!(
            finished.diagnostics,
            errors
                .iter()
                .map(ReferenceAnalysisError::diagnostic)
                .collect::<Vec<_>>()
        );
    }
}

use super::*;
use crate::scalar::control::AssignmentContinuation;
use reference::{OccurrenceEdge, OccurrenceNode, OccurrenceParent};

#[test]
fn preparation_publishes_neutral_assignment_stages_consumed_by_the_existing_query() {
    let text = "%%start\nbool(bool) echo = fn(flag) { flag };\nu8(u8) number = fn(value) { value };\n(u8, bool, u8)(bool) triple = fn(flag) { u8 small = 1; u8 large = 2; small, flag, large };\nunit(bool) probe = fn(flag) { u8 first = 0; bool valid = false; u8 last = 0; first; valid; last; first, valid, last = triple(echo(flag)); first; valid; last; first, last = first, number(last); first; last; };\n%%end";
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
        let row = prepared.calls.as_ref().unwrap().function(3);
        let ScalarItem::Function(function) = &prepared.program.items[3] else {
            panic!("probe")
        };
        let root = OccurrenceParent::Block(row.structure.root);
        assert_eq!(row.assignments.len(), 2);
        assert_eq!(row.sites.len(), 3);
        assert_eq!(row.call_occurrences.len(), 3);
        let mut inspected = 0;
        for (item, syntax) in function.body.items.iter().enumerate() {
            let ScalarBlockItem::Assignment(assignment) = syntax else {
                continue;
            };
            let group = &row.assignments[&(row.structure.root, item)];
            assert_eq!(group.span, assignment.span);
            assert_eq!(group.continuation(), AssignmentContinuation::Unit);
            let evaluations = group.rhs_evaluations().collect::<Vec<_>>();
            assert_eq!(evaluations.len(), assignment.values.len());
            for (position, evaluation) in evaluations.iter().enumerate() {
                assert_eq!(evaluation.position, position);
                assert_eq!(
                    row.structure.edges[&(
                        root,
                        OccurrenceEdge::AssignmentRhs {
                            item,
                            output: position
                        }
                    )],
                    OccurrenceNode::Expression(evaluation.expression)
                );
                assert_eq!(
                    row.occurrences[evaluation.expression.0].span,
                    expression_span(&assignment.values[position])
                );
            }
            let projections = group.projected_outputs().collect::<Vec<_>>();
            assert_eq!(
                projections
                    .iter()
                    .map(|projection| projection.output)
                    .collect::<Vec<_>>(),
                assignment.rhs().projected_outputs().collect::<Vec<_>>()
            );
            for projection in &projections {
                assert_eq!(projection.producer, *evaluations[0]);
            }
            if inspected == 0 {
                assert_eq!(evaluations.len(), 1);
                assert_eq!(projections.len(), 2);
                let primary = evaluations[0].expression;
                let association = row
                    .call_occurrences
                    .iter()
                    .find(|association| association.ordinal == primary)
                    .unwrap();
                let ScalarItem::Function(triple) = &prepared.program.items[2] else {
                    unreachable!()
                };
                assert_eq!(
                    row.sites[association.site].target,
                    ResolvedCallTarget::ModuleCallable {
                        source: prepared.program.source.clone(),
                        declaration_span: triple.name_span,
                        concrete: ConcreteCallSelection::Declaration
                    }
                );
            } else {
                assert_eq!(evaluations.len(), 2);
                assert!(projections.is_empty());
                assert!(matches!(
                    assignment.values[1],
                    ScalarExpression::Call { .. }
                ));
            }
            let destinations = group.destinations().collect::<Vec<_>>();
            let previous = group.previous_observations().collect::<Vec<_>>();
            let installs = group.installs().collect::<Vec<_>>();
            assert_eq!(destinations.len(), assignment.targets.len());
            assert_eq!(destinations, previous);
            assert_eq!(destinations, installs);
            for (position, ((slot, previous), install)) in
                destinations.iter().zip(previous).zip(installs).enumerate()
            {
                assert!(std::ptr::eq(*slot, previous));
                assert!(std::ptr::eq(*slot, install));
                assert_eq!(slot.position, position);
                assert_eq!(slot.target_span, assignment.targets[position].target_span);
                assert_eq!(
                    row.structure.edges[&(
                        root,
                        OccurrenceEdge::AssignmentDestination {
                            item,
                            destination: position
                        }
                    )],
                    OccurrenceNode::Place(slot.destination)
                );
            }
            inspected += 1;
        }
        assert_eq!(inspected, 2);
        let before = prepared.program.clone();
        let stages = Scope::new();
        let copies = layout::observations::Scope::new();
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
        assert_eq!(copies.counts().program_copies, 0);
        assert_eq!(copies.counts().module_copies, 0);
        assert_eq!(copies.counts().cfg_copies, 0);
        drop(copies);
        drop(stages);
        let mut installed = before.clone();
        let errors = results.install(&mut installed);
        assert!(errors.is_empty(), "{errors:?}");
        assert_eq!(prepared.program, before);
        let finished = prepared.finish();
        assert_eq!(finished.program, installed);
        assert!(finished.diagnostics.is_empty());
        let public = derive_scalar_program_from_cst_with_layout(parsed.canonical_cst(), target);
        assert_eq!(finished.program, public.program);
        assert_eq!(finished.diagnostics, public.diagnostics);
    }
}

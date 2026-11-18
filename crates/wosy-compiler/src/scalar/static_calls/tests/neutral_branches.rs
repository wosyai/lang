use super::*;
use crate::scalar::control::{BlockPosition, ConditionalRegion};
use reference::{
    FunctionCallSites, OccurrenceEdge as Edge, OccurrenceNode as Node, OccurrenceParent as Parent,
};

fn assert_arm(row: &FunctionCallSites, arm: reference::BlockOrdinal, syntax: &ScalarBlock) {
    let region = &row.structure.blocks[arm.0];
    assert_eq!(region.span, syntax.span);
    let expected = syntax
        .items_with_output_roles()
        .enumerate()
        .filter_map(|(item, (_, role))| match role {
            ScalarBlockItemRole::FinalOutput { output, metadata } => {
                Some((item, output, metadata.span))
            }
            ScalarBlockItemRole::Ordinary => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(region.final_outputs().count(), expected.len());
    for (slot, (item, output, span)) in region.final_outputs().zip(expected) {
        assert_eq!((slot.item, slot.output, slot.span), (item, output, span));
        assert_eq!(
            row.structure.edges[&(Parent::Block(arm), Edge::FinalOutput { item, output })],
            Node::Expression(slot.expression)
        );
    }
}

#[test]
fn prepared_branch_regions_preserve_polarity_continuations_and_inherited_query_effect_order() {
    let text = "%%start\nu8 visits = 0;\nbool(bool) tick = fn(flag) { visits = visits + 1; flag };\nu8(u8) number = fn(value) { visits = visits + 1; value };\nbool(bool, bool) conjunction = fn(left, right) { tick(left) && tick(right) };\nbool(bool, bool) disjunction = fn(left, right) { tick(left) || tick(right) };\nu8(bool, u8) choose = fn(flag, value) { if (tick(tick(flag))) { number(value) } else { number(value) } };\nunit(bool) route = fn(flag) { if (tick(flag)) { tick(flag); }; bool running = flag; while (tick(running)) { running = false; } running; };\n%%end";
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
        for (item, polarity, bypass) in [(3, true, false), (4, false, true)] {
            let row = calls.function(item);
            let ScalarItem::Function(function) = &prepared.program.items[item] else {
                panic!("boolean function")
            };
            let Node::Expression(ordinal) = row.structure.edges[&(
                Parent::Block(row.structure.root),
                Edge::FinalOutput { item: 0, output: 0 },
            )] else {
                unreachable!()
            };
            let region = row.short_circuits[&ordinal];
            assert_eq!(row.short_circuits.len(), 1);
            assert_eq!(region.evaluate_right_when, polarity);
            assert_eq!(region.bypass, bypass);
            assert_eq!(region.join, ordinal);
            assert_eq!(region.span, expression_span(&function.body.expressions[0]));
            assert_eq!(
                row.structure.edges[&(Parent::Expression(ordinal), Edge::BinaryLeft)],
                Node::Expression(region.left)
            );
            assert_eq!(
                row.structure.edges[&(Parent::Expression(ordinal), Edge::BinaryRight)],
                Node::Expression(region.right)
            );
            assert!(region.left.0 < region.right.0);
        }
        let choose = calls.function(5);
        let ScalarItem::Function(choose_syntax) = &prepared.program.items[5] else {
            unreachable!()
        };
        let ScalarBlockItem::Expression(
            expression @ ScalarExpression::If {
                condition,
                then_branch,
                else_branch,
                ..
            },
        ) = &choose_syntax.body.items[0]
        else {
            panic!("value conditional")
        };
        let Node::Expression(choose_ordinal) = choose.structure.edges[&(
            Parent::Block(choose.structure.root),
            Edge::FinalOutput { item: 0, output: 0 },
        )] else {
            unreachable!()
        };
        let ConditionalRegion::Value {
            span,
            condition: test,
            then_block,
            else_block,
            resume,
        } = &choose.conditionals[&choose_ordinal]
        else {
            panic!("value region")
        };
        assert_eq!(*span, expression_span(expression));
        assert_eq!(*resume, choose_ordinal);
        assert_eq!(
            choose.structure.edges[&(Parent::Expression(choose_ordinal), Edge::Condition)],
            Node::Expression(*test)
        );
        assert_arm(choose, *then_block, then_branch);
        assert_arm(choose, *else_block, else_branch);
        let ScalarExpression::Call { arguments, .. } = condition.as_ref() else {
            panic!("effectful condition call")
        };
        let expected_choose_calls = [
            expression_span(&arguments[0]),
            expression_span(condition),
            then_branch.final_output_values[0].span,
            else_branch.final_output_values[0].span,
        ];
        let route = calls.function(6);
        let ScalarItem::Function(route_syntax) = &prepared.program.items[6] else {
            unreachable!()
        };
        let ScalarBlockItem::Expression(
            unit @ ScalarExpression::UnitIf {
                then_branch: unit_body,
                ..
            },
        ) = &route_syntax.body.items[0]
        else {
            panic!("unit conditional")
        };
        let root = Parent::Block(route.structure.root);
        let Node::Expression(unit_ordinal) =
            route.structure.edges[&(root, Edge::ExpressionItem { item: 0 })]
        else {
            unreachable!()
        };
        let ConditionalRegion::Unit {
            span,
            condition: test,
            then_block,
            fallthrough,
        } = route.conditionals[&unit_ordinal]
        else {
            panic!("unit fallthrough region")
        };
        assert_eq!(span, expression_span(unit));
        assert_eq!(fallthrough, unit_ordinal);
        assert_eq!(route.structure.blocks[then_block.0].span, unit_body.span);
        assert_eq!(
            route.structure.edges[&(Parent::Expression(unit_ordinal), Edge::Condition)],
            Node::Expression(test)
        );
        let ScalarBlockItem::While(loop_syntax) = &route_syntax.body.items[2] else {
            panic!("while item ordinal")
        };
        let loop_region = route.loops[&(route.structure.root, 2)];
        assert_eq!(loop_region.span, loop_syntax.span);
        assert_eq!(
            loop_region.body_to_test(),
            (loop_region.body, loop_region.test)
        );
        assert_eq!(
            loop_region.on_false,
            BlockPosition {
                block: route.structure.root,
                item: 3
            }
        );
        assert_eq!(
            route.structure.edges[&(root, Edge::WhileCondition { item: 2 })],
            Node::Expression(loop_region.test)
        );
        assert_eq!(
            route.structure.edges[&(root, Edge::WhileBody { item: 2 })],
            Node::Block(loop_region.body)
        );
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
        for item in [3, 4] {
            let ScalarItem::Function(function) = &installed.items[item] else {
                unreachable!()
            };
            assert_eq!(
                function
                    .reference_cfg
                    .points
                    .iter()
                    .filter(|point| matches!(point.kind, CfgPointKind::Call { .. }))
                    .count(),
                2
            );
            assert_eq!(
                function
                    .reference_cfg
                    .points
                    .iter()
                    .filter(|point| matches!(point.kind, CfgPointKind::Branch { .. }))
                    .count(),
                0
            );
        }
        let ScalarItem::Function(function) = &installed.items[5] else {
            unreachable!()
        };
        let points = &function.reference_cfg.points;
        let actual = points
            .iter()
            .filter(|point| matches!(point.kind, CfgPointKind::Call { .. }))
            .map(|point| point.source_span.range)
            .collect::<Vec<_>>();
        assert_eq!(actual, expected_choose_calls);
        let ScalarItem::Function(function) = &installed.items[6] else {
            unreachable!()
        };
        let points = &function.reference_cfg.points;
        let test = points
            .iter()
            .find(|point| matches!(point.kind, CfgPointKind::LoopTest { .. }))
            .unwrap();
        let body_end = points
            .iter()
            .find(|point| {
                matches!(point.kind, CfgPointKind::ScopeExit)
                    && point.source_span.range == loop_syntax.body.span
            })
            .unwrap();
        assert!(body_end.successors.contains(&test.id));
        let branch = points
            .iter()
            .find(|point| {
                matches!(point.kind, CfgPointKind::Branch { .. })
                    && point.source_span.range == expression_span(&loop_syntax.condition)
            })
            .unwrap();
        assert_eq!(branch.successors.len(), 2);
        assert!(matches!(
            points[branch.successors[0].0].kind,
            CfgPointKind::ScopeEnter
        ));
        assert!(matches!(
            points[branch.successors[1].0].kind,
            CfgPointKind::Join
        ));
        assert_eq!(prepared.program, before);
        let finished = prepared.finish();
        assert_eq!(finished.program, installed);
        assert!(finished.diagnostics.is_empty());
    }
}

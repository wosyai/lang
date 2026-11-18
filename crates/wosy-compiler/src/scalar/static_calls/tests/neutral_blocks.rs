use super::*;
use crate::scalar::control::{BindingOutput, BlockItemKind};
use reference::{OccurrenceEdge as Edge, OccurrenceNode as Node, OccurrenceParent as Parent};

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
        assert_eq!(
            row.structure.edges[&(
                Parent::Block(row.structure.root),
                Edge::BindingRoot { item: 0, output: 0 }
            )],
            Node::Expression(binding.primary)
        );
        assert_eq!(
            binding.secondary,
            vec![BindingOutput::ProjectPrimary { output: 1 }]
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

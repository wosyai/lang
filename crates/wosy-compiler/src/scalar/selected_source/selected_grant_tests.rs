use super::*;

const CHAIN: &str = "%%start\nunit() inspect = fn { u32 value = 1; *u32 root = &value; *u32 child = &(*root); *u32 grandchild = &(*child); *u32 sibling = &(*root); *grandchild; *sibling; *u32 copied = sibling; *copied; };\n%%end";
const SAVED: &str = "%%start\nu32() f = fn { u32 left = 1; u32 right = 2; *!u32 target = &!left; *!u32 replacement = &!right; u32 old = (*(target = replacement) = 9); old + left + *target };\n%%end";

fn checked_class(tables: &EvaluatedValueTables, view: &SourceValueViewRef) -> SourceGrantClassRef {
    let SourcePayloadView::Checked {
        state: CheckedValueState::Live,
        referents,
    } = &tables.source_views[view.index].payload
    else {
        panic!("actual live checked view");
    };
    let [referent] = referents.as_slice() else {
        panic!("one selected referent");
    };
    let SourceCheckedGrantState::Selected(class) = &referent.loan else {
        panic!("actual selected class");
    };
    class.clone()
}

fn binding(cfg: &ScalarReferenceCfg, name: &str) -> ReferenceBindingId {
    let bindings = cfg.checked_bindings.iter().filter(|(_, place)| matches!(&place.place, ScalarPlace::Name { name: actual, .. } if actual == name)).map(|(binding, _)| binding.clone()).collect::<Vec<_>>();
    let [binding] = bindings.as_slice() else {
        panic!("one canonical declared checked binding");
    };
    binding.clone()
}

fn chain(layout: ScalarTargetLayout, project: bool) {
    let cfg = tests::source_cfg(CHAIN, "inspect", layout, project);
    let tables = &cfg.evaluated_values;
    selected_grant::validate(&cfg).expect("canonical selected roots and ancestry");
    let copied_binding = binding(&cfg, "copied");
    let sibling_binding = binding(&cfg, "sibling");
    let roots = tables
        .source_grant_creations
        .iter()
        .filter(|witness| witness.parent == SourceGrantParent::Root)
        .collect::<Vec<_>>();
    let [root] = roots.as_slice() else {
        panic!("one actual direct root");
    };
    assert_eq!(tables.source_grant_creations.len(), 4);
    let root_row = &tables.source_configurations[root.output.index];
    assert!(root_row.grant_ancestry.is_empty());
    for witness in &tables.source_grant_creations {
        let input = &tables.source_configurations[witness.input.index];
        let output = &tables.source_configurations[witness.output.index];
        assert!(input
            .grants
            .iter()
            .all(|grant| grant.class != witness.child));
        assert_eq!(output.grants.len(), input.grants.len() + 1);
        assert!(input
            .grants
            .iter()
            .all(|grant| output.grants.contains(grant)));
        assert!(input
            .grant_ancestry
            .iter()
            .all(|pair| output.grant_ancestry.contains(pair)));
        if let SourceGrantParent::Selected {
            class,
            source,
            selection,
        } = &witness.parent
        {
            assert_eq!(source.point, selection.observation.case.value.point);
            assert_eq!(*class, checked_class(tables, &selection.observation.view));
            let equations = tables
                .source_port_equations
                .iter()
                .filter(|equation| equation.contributor == witness.contributor)
                .collect::<Vec<_>>();
            assert!(!equations.is_empty());
            for equation in equations {
                assert_eq!(
                    port_monitor::query(
                        tables,
                        &equation.input,
                        &[selection.coordinate.clone()],
                        &cfg.initial_points[witness.event.point.0].source_span
                    )
                    .expect("ONE actual input valuation"),
                    vec![selection.clone()]
                );
            }
            for old in &input.grants {
                assert_eq!(
                    output.grant_ancestry.contains(&SourceGrantAncestryFact {
                        ancestor: old.class.clone(),
                        descendant: witness.child.clone()
                    }),
                    old.class == *class
                        || input.grant_ancestry.contains(&SourceGrantAncestryFact {
                            ancestor: old.class.clone(),
                            descendant: class.clone()
                        })
                );
                assert!(!output.grant_ancestry.contains(&SourceGrantAncestryFact {
                    ancestor: witness.child.clone(),
                    descendant: old.class.clone()
                }));
            }
        }
    }
    let complete = tables
        .source_configurations
        .iter()
        .filter(|row| row.grants.len() == 4)
        .collect::<Vec<_>>();
    assert!(!complete.is_empty());
    let mut copies = 0;
    for row in complete {
        assert_eq!(
            row.grant_ancestry.len(),
            4,
            "root->child/root->grandchild/child->grandchild/root->sibling"
        );
        let copied = row
            .slots
            .iter()
            .filter(|slot| {
                tables.source_slots[slot.coordinate.index].role
                    == SourceCoordinateRole::Binding(copied_binding.clone())
            })
            .collect::<Vec<_>>();
        for copy in copied {
            copies += 1;
            let sibling = row
                .slots
                .iter()
                .filter(|slot| {
                    tables.source_slots[slot.coordinate.index].role
                        == SourceCoordinateRole::Binding(sibling_binding.clone())
                })
                .collect::<Vec<_>>();
            let [sibling] = sibling.as_slice() else {
                panic!("actual stored sibling pointer");
            };
            assert_eq!(
                checked_class(tables, &copy.view),
                checked_class(tables, &sibling.view)
            );
        }
    }
    assert!(
        copies > 0,
        "actual checked copy class transport is observed"
    );
    assert!(
        tables.source_move_witnesses.is_empty(),
        "shared copying introduces zero Move grant"
    );
    assert!(tables.owner_consumers.is_empty());
}

fn saved(layout: ScalarTargetLayout, project: bool) {
    let cfg = tests::source_cfg(SAVED, "f", layout, project);
    let tables = &cfg.evaluated_values;
    selected_grant::validate(&cfg).expect("actual saved-old and replacement classes");
    let target_binding = binding(&cfg, "target");
    assert_eq!(tables.source_grant_creations.len(), 2);
    assert!(tables
        .source_grant_creations
        .iter()
        .all(|witness| witness.parent == SourceGrantParent::Root));
    assert!(!tables.source_move_witnesses.is_empty());
    for witness in &tables.source_move_witnesses {
        selected_move::validate(
            tables,
            &cfg.initial_points,
            &witness.id,
            &cfg.initial_points[witness.event.point.0].source_span,
        )
        .expect("actual checked Move class conservation");
        let input = &tables.source_configurations[witness.input.index];
        let output = &tables.source_configurations[witness.output.index];
        assert_eq!(input.grants, output.grants);
        assert_eq!(input.grant_ancestry, output.grant_ancestry);
        let before = selected_slot(
            tables,
            input,
            &SourceCoordinateRole::Evaluated(witness.source.clone()),
        )
        .expect("original available pointer");
        let after = selected_slot(
            tables,
            output,
            &SourceCoordinateRole::Evaluated(witness.source.clone()),
        )
        .expect("same moved evaluated pointer");
        assert_eq!(
            checked_class(tables, &before.view),
            checked_class(tables, &after.view)
        );
        assert_eq!(after.availability, ValueAvailability::Available);
    }
    let mut observed = 0;
    for row in &tables.source_configurations {
        for old in &row.saved {
            if matches!(
                tables.source_views[old.view.index].payload,
                SourcePayloadView::Checked { .. }
            ) {
                let targets = row
                    .slots
                    .iter()
                    .filter(|slot| {
                        tables.source_slots[slot.coordinate.index].role
                            == SourceCoordinateRole::Binding(target_binding.clone())
                    })
                    .collect::<Vec<_>>();
                let [target] = targets.as_slice() else {
                    panic!("actual current target");
                };
                let previous = checked_class(tables, &old.view);
                let current = checked_class(tables, &target.view);
                if current != previous {
                    observed += 1;
                    assert_eq!(row.grants.len(), 2);
                    assert!(row.grant_ancestry.is_empty());
                    let saved_slot = selected_slot(
                        tables,
                        row,
                        &SourceCoordinateRole::Saved(old.selection.clone()),
                    )
                    .expect("independent saved coordinate");
                    assert_eq!(checked_class(tables, &saved_slot.view), previous);
                }
            }
        }
    }
    assert!(observed > 0, "saved-old retains its root after replacement");
    assert!(tables.owner_consumers.is_empty());
}

fn support() -> (
    ScalarReferenceCfg,
    SourceGrantCreationWitness,
    SourceCheckedReferentView,
) {
    let cfg = tests::source_cfg(CHAIN, "inspect", ScalarTargetLayout::NATIVE64, false);
    let roots = cfg
        .evaluated_values
        .source_grant_creations
        .iter()
        .filter(|witness| witness.parent == SourceGrantParent::Root)
        .cloned()
        .collect::<Vec<_>>();
    let [root] = roots.as_slice() else {
        panic!("one source root");
    };
    let root = root.clone();
    let row = &cfg.evaluated_values.source_configurations[root.output.index];
    let grant = &row.grants[0];
    let referent = SourceCheckedReferentView {
        instance: grant.referent.clone(),
        coordinate: grant.coordinate.clone(),
        projection: grant.projection.clone(),
        grant: cfg.initial_points[root.event.point.0].source_span.clone(),
        mode: grant.mode,
        loan: SourceCheckedGrantState::Selected(root.child.clone()),
    };
    (cfg, root, referent)
}

#[test]
fn selected_grant_chain_copy_program_native64() {
    chain(ScalarTargetLayout::NATIVE64, false);
}
#[test]
fn selected_grant_chain_copy_program_wasm32() {
    chain(ScalarTargetLayout::WASM32, false);
}
#[test]
fn selected_grant_chain_copy_project_native64() {
    chain(ScalarTargetLayout::NATIVE64, true);
}
#[test]
fn selected_grant_chain_copy_project_wasm32() {
    chain(ScalarTargetLayout::WASM32, true);
}
#[test]
fn selected_grant_saved_move_program_native64() {
    saved(ScalarTargetLayout::NATIVE64, false);
}
#[test]
fn selected_grant_saved_move_program_wasm32() {
    saved(ScalarTargetLayout::WASM32, false);
}
#[test]
fn selected_grant_saved_move_project_native64() {
    saved(ScalarTargetLayout::NATIVE64, true);
}
#[test]
fn selected_grant_saved_move_project_wasm32() {
    saved(ScalarTargetLayout::WASM32, true);
}

#[test]
fn selected_grant_support_same_site_roots_stay_distinct() {
    let (cfg, root, referent) = support();
    let mut row = cfg.evaluated_values.source_configurations[root.output.index].clone();
    let next =
        selected_grant::introduce(&mut row, &root.event, &referent, &SourceGrantParent::Root)
            .expect("repeat SAME source event as separate support transition");
    assert_ne!(next, root.child);
    assert_eq!(row.grants.len(), 2);
    assert!(row.grant_ancestry.is_empty());
    assert_eq!(row.grants[0].site, row.grants[1].site);
}

#[test]
fn selected_grant_support_same_site_siblings_and_grandchild_equation() {
    let (cfg, root, referent) = support();
    let tables = &cfg.evaluated_values;
    let children = tables.source_grant_creations.iter().filter(|witness| matches!(&witness.parent, SourceGrantParent::Selected { class, .. } if *class == root.child)).collect::<Vec<_>>();
    assert_eq!(children.len(), 2);
    for actual in children {
        let mut row = tables.source_configurations[root.output.index].clone();
        let first = selected_grant::introduce(&mut row, &actual.event, &referent, &actual.parent)
            .expect("selected source parent");
        let sibling = selected_grant::introduce(&mut row, &actual.event, &referent, &actual.parent)
            .expect("distinct same-site sibling");
        assert_ne!(first, sibling);
        assert!(!row.grant_ancestry.contains(&SourceGrantAncestryFact {
            ancestor: first.clone(),
            descendant: sibling.clone()
        }));
        let SourceGrantParent::Selected {
            source, selection, ..
        } = &actual.parent
        else {
            unreachable!();
        };
        let old_pairs = row.grant_ancestry.clone();
        let parent = SourceGrantParent::Selected {
            class: first.clone(),
            source: source.clone(),
            selection: selection.clone(),
        };
        let grandchild = selected_grant::introduce(&mut row, &actual.event, &referent, &parent)
            .expect("paired update support equation");
        assert!(old_pairs
            .iter()
            .all(|pair| row.grant_ancestry.contains(pair)));
        for old in [root.child.clone(), first, sibling.clone()] {
            assert_eq!(
                row.grant_ancestry.contains(&SourceGrantAncestryFact {
                    ancestor: old.clone(),
                    descendant: grandchild.clone()
                }),
                old != sibling
            );
            assert!(!row.grant_ancestry.contains(&SourceGrantAncestryFact {
                ancestor: grandchild.clone(),
                descendant: old
            }));
        }
    }
}

#[test]
fn selected_grant_support_unknown_parent_is_rejected() {
    let (cfg, root, referent) = support();
    let mut row = cfg.evaluated_values.source_configurations[root.output.index].clone();
    let witnesses = cfg.evaluated_values.source_grant_creations.iter().filter(|witness| matches!(&witness.parent, SourceGrantParent::Selected { class, .. } if *class == root.child)).collect::<Vec<_>>();
    for witness in witnesses {
        let SourceGrantParent::Selected {
            source, selection, ..
        } = &witness.parent
        else {
            unreachable!();
        };
        let before = row.clone();
        let parent = SourceGrantParent::Selected {
            class: SourceGrantClassRef {
                owner: root.event.owner.clone(),
                variable: SupportedInstanceVariable(row.grants.len()),
            },
            source: source.clone(),
            selection: selection.clone(),
        };
        assert!(selected_grant::introduce(&mut row, &witness.event, &referent, &parent).is_err());
        assert_eq!(row, before);
    }
}

#[test]
fn selected_grant_wrong_selected_parent_is_rejected() {
    let mut cfg = tests::source_cfg(CHAIN, "inspect", ScalarTargetLayout::NATIVE64, false);
    let witnesses = cfg
        .evaluated_values
        .source_grant_creations
        .iter_mut()
        .filter(|witness| matches!(witness.parent, SourceGrantParent::Selected { .. }))
        .collect::<Vec<_>>();
    for witness in witnesses {
        let SourceGrantParent::Selected { class, .. } = &mut witness.parent else {
            unreachable!();
        };
        *class = witness.child.clone();
    }
    assert!(selected_grant::validate(&cfg).is_err());
}

#[test]
fn selected_grant_missing_ancestor_pair_is_rejected() {
    let mut cfg = tests::source_cfg(CHAIN, "inspect", ScalarTargetLayout::NATIVE64, false);
    for row in &mut cfg.evaluated_values.source_configurations {
        row.grant_ancestry.clear();
    }
    assert!(selected_grant::validate(&cfg).is_err());
}

#[test]
fn selected_grant_foreign_referent_mode_is_rejected() {
    let (mut cfg, root, _) = support();
    for row in &mut cfg.evaluated_values.source_configurations {
        for grant in &mut row.grants {
            if grant.class == root.child {
                grant.mode = ScalarReferenceMutability::Mutable;
            }
        }
    }
    assert!(selected_grant::validate(&cfg).is_err());
}

#[test]
fn selected_grant_uncovered_parent_keeps_explicit_pending_constructor() {
    let (cfg, root, _) = support();
    let children = cfg.evaluated_values.source_grant_creations.iter().filter(|witness| matches!(&witness.parent, SourceGrantParent::Selected { class, .. } if *class == root.child)).collect::<Vec<_>>();
    assert_eq!(children.len(), 2);
    for child in children {
        let mut tables = cfg.evaluated_values.clone();
        let SourceGrantParent::Selected { selection, .. } = &child.parent else {
            unreachable!();
        };
        let SourcePayloadView::Checked { referents, .. } =
            &mut tables.source_views[selection.observation.view.index].payload
        else {
            panic!("actual checked parent");
        };
        let [referent] = referents.as_mut_slice() else {
            panic!("one exact referent");
        };
        referent.loan = SourceCheckedGrantState::Pending {
            event: root.event.clone(),
        };
        let mut row = tables.source_configurations[child.input.index].clone();
        let before = row.clone();
        let equations = tables
            .source_port_equations
            .iter()
            .filter(|equation| equation.contributor == child.contributor)
            .cloned()
            .collect::<Vec<_>>();
        let [equation] = equations.as_slice() else {
            panic!("one exact Borrow equation");
        };
        let rule = &tables.source_contributors[child.contributor.index];
        let [value] = rule.outputs.as_slice() else {
            panic!("one actual Borrow output");
        };
        let value = value.clone();
        let values = tables
            .values
            .iter()
            .filter(|fact| fact.id == value)
            .collect::<Vec<_>>();
        let [fact] = values.as_slice() else {
            panic!("canonical Borrow output shape");
        };
        let shape = fact.shape.clone();
        let (view, creation, inputs) = selected_grant::borrow(
            &mut tables,
            &mut row,
            &cfg.initial_points[child.event.point.0],
            &child.event,
            &equation.input,
            &value,
            &shape,
            None,
        )
        .expect("real incomplete parent constructor");
        let view = view.expect("explicit checked Pending state");
        assert!(selected_grant::has_pending(&tables, &view));
        assert!(creation.is_none());
        assert_eq!(row, before);
        assert_eq!(inputs, vec![selection.coordinate.clone()]);
        let SourcePayloadView::Checked { referents, .. } = &tables.source_views[view.index].payload
        else {
            unreachable!();
        };
        assert_eq!(
            referents[0].loan,
            SourceCheckedGrantState::Pending {
                event: child.event.clone()
            }
        );
        assert_eq!(
            tables.source_grant_creations, cfg.evaluated_values.source_grant_creations,
            "zero positive creation witness from pending input"
        );
    }
}

#[test]
fn selected_grant_serialization_preserves_classes_ancestry_and_parent_membership() {
    let cfg = tests::source_cfg(CHAIN, "inspect", ScalarTargetLayout::NATIVE64, false);
    let encoded =
        serde_json::to_vec(&cfg.evaluated_values).expect("all required grant fields serialize");
    let decoded: EvaluatedValueTables =
        serde_json::from_slice(&encoded).expect("all required grant fields deserialize");
    assert_eq!(
        decoded.source_grant_creations,
        cfg.evaluated_values.source_grant_creations
    );
    assert_eq!(
        decoded.source_configurations,
        cfg.evaluated_values.source_configurations
    );
    assert_eq!(decoded.source_views, cfg.evaluated_values.source_views);
}

#[test]
fn selected_grant_move_reader_requires_actual_creation_witness() {
    let cfg = tests::source_cfg(SAVED, "f", ScalarTargetLayout::NATIVE64, false);
    let mut tables = cfg.evaluated_values.clone();
    assert!(!tables.source_move_witnesses.is_empty());
    tables.source_grant_creations.clear();
    for witness in &tables.source_move_witnesses {
        let point = &cfg.initial_points[witness.event.point.0];
        let error = selected_move::validate(
            &tables,
            &cfg.initial_points,
            &witness.id,
            &point.source_span,
        )
        .expect_err("checked Move consumes actual selected creation witness");
        let ReferenceAnalysisError::MissingContext {
            source_span,
            reason: ReferenceAnalysisErrorReason::InvalidAssignmentCorrespondence,
        } = error
        else {
            panic!("qualified original Move diagnostic");
        };
        assert_eq!(source_span, point.source_span);
    }
}

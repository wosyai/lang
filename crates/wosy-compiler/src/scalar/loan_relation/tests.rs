use super::*;
use crate::scalar::{ByteSpan, ProducerFunctionBodyId, SourceIdentity};

fn site(point: usize, path: &str) -> LoanGrantSite {
    let source = SourceIdentity::new(
        "finite-loan-support".into(),
        "support".into(),
        path.into(),
        "epoch-1".into(),
    );
    LoanGrantSite {
        owner: ProducerCfgOwnerId::Function {
            source: source.clone(),
            declaration: ByteSpan::new(10, 100),
            body: ProducerFunctionBodyId::Declaration,
        },
        point: CfgPointId(point),
        source_span: SourceSpan::new(source, ByteSpan::new(20, 25)),
    }
}

fn fact(point: usize, target: usize, mode: ScalarReferenceMutability) -> LoanClassFact {
    LoanClassFact {
        site: site(point, "support.w"),
        target: LoanTargetId(target),
        mode,
    }
}

fn shared(point: usize) -> LoanClassFact {
    fact(point, 7, ScalarReferenceMutability::Shared)
}

fn chain() -> LoanRelation {
    let mut relation = LoanRelation::new(5);
    relation.create_root(LoanSupportId(0), shared(1));
    relation
        .create_child(LoanSupportId(0), LoanSupportId(1), shared(2))
        .unwrap();
    relation
        .create_child(LoanSupportId(1), LoanSupportId(2), shared(3))
        .unwrap();
    relation.copy(LoanSupportId(1), LoanSupportId(3)).unwrap();
    relation.create_root(
        LoanSupportId(4),
        fact(4, 8, ScalarReferenceMutability::Mutable),
    );
    relation
}

fn substitutions(
    relation: &LoanRelation,
) -> (Vec<LoanTargetSubstitution>, Vec<LoanSiteSubstitution>) {
    let targets: BTreeSet<_> = relation.classes().iter().map(|fact| fact.target).collect();
    let targets = targets
        .into_iter()
        .map(|from| LoanTargetSubstitution {
            from,
            to: LoanTargetId(from.0 + 100),
        })
        .collect();
    let mut sites = Vec::new();
    for fact in relation.classes() {
        if !sites
            .iter()
            .any(|row: &LoanSiteSubstitution| row.from == fact.site)
        {
            let mut to = site(fact.site.point.0 + 10, "imported.w");
            to.owner = ProducerCfgOwnerId::Literal {
                enclosing: Box::new(to.owner.clone()),
                capture_point: CfgPointId(42),
                literal: to.source_span.clone(),
            };
            sites.push(LoanSiteSubstitution {
                from: fact.site.clone(),
                to,
            });
        }
    }
    (targets, sites)
}

#[test]
fn finite_loan_relation_new_has_frozen_unavailable_supports() {
    let relation = LoanRelation::new(3);
    assert_eq!(relation.supports(), &vec![LoanSupport::Unavailable; 3]);
    assert!(relation.classes().is_empty());
    assert!(relation.ancestors().is_empty());
    assert_eq!(
        relation.pair(LoanSupportId(0), LoanSupportId(1)),
        Err(LoanRelationError::Unavailable(LoanSupportId(0)))
    );
}

#[test]
fn finite_loan_relation_fresh_same_site_roots_are_distinct() {
    let mut relation = LoanRelation::new(2);
    assert_eq!(
        relation.create_root(LoanSupportId(0), shared(1)),
        LoanClassId(0)
    );
    assert_eq!(
        relation.create_root(LoanSupportId(1), shared(1)),
        LoanClassId(1)
    );
    assert_eq!(relation.classes()[0], relation.classes()[1]);
    assert_eq!(
        relation.pair(LoanSupportId(0), LoanSupportId(1)),
        Ok(LoanPair::Unrelated)
    );
    assert!(relation.ancestors().is_empty());
    relation.create_root(LoanSupportId(0), shared(1));
    assert_eq!(relation.classes().len(), 2);
    assert_eq!(
        relation.pair(LoanSupportId(0), LoanSupportId(1)),
        Ok(LoanPair::Unrelated)
    );
}

#[test]
fn finite_loan_relation_shared_copy_keeps_equality_and_ancestry() {
    let mut relation = LoanRelation::new(3);
    relation.create_root(LoanSupportId(0), shared(1));
    relation
        .create_child(LoanSupportId(0), LoanSupportId(1), shared(2))
        .unwrap();
    relation.copy(LoanSupportId(1), LoanSupportId(2)).unwrap();
    assert_eq!(
        relation.pair(LoanSupportId(1), LoanSupportId(2)),
        Ok(LoanPair::Equal)
    );
    assert_eq!(
        relation.pair(LoanSupportId(0), LoanSupportId(2)),
        Ok(LoanPair::Ancestor)
    );
    let before = relation.clone();
    relation.copy(LoanSupportId(1), LoanSupportId(1)).unwrap();
    assert_eq!(relation, before);
}

#[test]
fn finite_loan_relation_mutable_copy_rejects_unchanged() {
    let mut relation = LoanRelation::new(2);
    relation.create_root(
        LoanSupportId(0),
        fact(1, 7, ScalarReferenceMutability::Mutable),
    );
    relation.create_root(LoanSupportId(1), shared(2));
    let before = relation.clone();
    assert_eq!(
        relation.copy(LoanSupportId(0), LoanSupportId(1)),
        Err(LoanRelationError::MoveOnlyCopy(LoanSupportId(0)))
    );
    assert_eq!(relation, before);
}

#[test]
fn finite_loan_relation_move_transports_identity_and_rejects_same_support() {
    let mut relation = chain();
    let before = relation.clone();
    assert_eq!(
        relation.transfer(LoanSupportId(4), LoanSupportId(4)),
        Err(LoanRelationError::SameSupportTransfer(LoanSupportId(4)))
    );
    assert_eq!(relation, before);
    relation
        .transfer(LoanSupportId(1), LoanSupportId(4))
        .unwrap();
    assert_eq!(relation.supports()[1], LoanSupport::Unavailable);
    assert_eq!(
        relation.pair(LoanSupportId(3), LoanSupportId(4)),
        Ok(LoanPair::Equal)
    );
    assert_eq!(
        relation.pair(LoanSupportId(0), LoanSupportId(4)),
        Ok(LoanPair::Ancestor)
    );
    assert_eq!(
        relation.pair(LoanSupportId(4), LoanSupportId(2)),
        Ok(LoanPair::Ancestor)
    );
    assert_eq!(relation.classes().len(), 3);
    let mut mutable = LoanRelation::new(2);
    let grant = fact(1, 7, ScalarReferenceMutability::Mutable);
    mutable.create_root(LoanSupportId(0), grant.clone());
    mutable
        .transfer(LoanSupportId(0), LoanSupportId(1))
        .unwrap();
    assert_eq!(mutable.classes(), &[grant]);
    assert_eq!(
        mutable.supports(),
        &[
            LoanSupport::Unavailable,
            LoanSupport::Available(LoanClassId(0))
        ]
    );
}

#[test]
fn finite_loan_relation_parent_child_grandchild_strict_ancestry() {
    let relation = chain();
    for (left, right) in [(0, 1), (1, 2), (0, 2)] {
        assert_eq!(
            relation.pair(LoanSupportId(left), LoanSupportId(right)),
            Ok(LoanPair::Ancestor)
        );
        assert_eq!(
            relation.pair(LoanSupportId(right), LoanSupportId(left)),
            Ok(LoanPair::Descendant)
        );
    }
    assert_eq!(
        relation.pair(LoanSupportId(2), LoanSupportId(2)),
        Ok(LoanPair::Equal)
    );
    assert_eq!(relation.ancestors().len(), 3);
}

#[test]
fn finite_loan_relation_child_witness_uses_incoming_parent_and_canonical_child() {
    let mut relation = LoanRelation::new(3);
    relation.create_root(LoanSupportId(0), shared(9));
    relation.create_root(LoanSupportId(2), shared(1));
    let new = fact(2, 19, ScalarReferenceMutability::Mutable);
    let witness = relation
        .create_child(LoanSupportId(2), LoanSupportId(0), new.clone())
        .unwrap();
    assert_eq!(
        witness,
        LoanParentWitness {
            site: new.site.clone(),
            parent_support: LoanSupportId(2),
            parent: LoanClassId(1),
            child: LoanClassId(0),
        }
    );
    assert_eq!(relation.classes()[0], new);
    assert_eq!(
        relation.ancestors(),
        &BTreeSet::from([(LoanClassId(1), LoanClassId(0))])
    );
    assert_eq!(
        relation.pair(LoanSupportId(2), LoanSupportId(0)),
        Ok(LoanPair::Ancestor)
    );
}

#[test]
fn finite_loan_relation_same_site_siblings_unrelated() {
    let mut relation = LoanRelation::new(3);
    relation.create_root(LoanSupportId(0), shared(1));
    relation
        .create_child(LoanSupportId(0), LoanSupportId(1), shared(2))
        .unwrap();
    relation
        .create_child(LoanSupportId(0), LoanSupportId(2), shared(2))
        .unwrap();
    assert_eq!(relation.classes()[1], relation.classes()[2]);
    assert_eq!(
        relation.pair(LoanSupportId(1), LoanSupportId(2)),
        Ok(LoanPair::Unrelated)
    );
    assert_eq!(
        relation.pair(LoanSupportId(2), LoanSupportId(1)),
        Ok(LoanPair::Unrelated)
    );
    assert_eq!(
        relation.pair(LoanSupportId(0), LoanSupportId(1)),
        Ok(LoanPair::Ancestor)
    );
    assert_eq!(
        relation.pair(LoanSupportId(0), LoanSupportId(2)),
        Ok(LoanPair::Ancestor)
    );
}

#[test]
fn finite_loan_relation_ancestor_survives_intermediate_kill() {
    let mut relation = chain();
    relation.clear(LoanSupportId(1));
    relation.clear(LoanSupportId(3));
    assert_eq!(relation.classes().len(), 3);
    assert_eq!(
        relation.ancestors(),
        &BTreeSet::from([(LoanClassId(0), LoanClassId(1))])
    );
    assert_eq!(
        relation.pair(LoanSupportId(0), LoanSupportId(2)),
        Ok(LoanPair::Ancestor)
    );
    assert_eq!(
        relation.pair(LoanSupportId(2), LoanSupportId(0)),
        Ok(LoanPair::Descendant)
    );
}

#[test]
fn finite_loan_relation_alpha_equality_after_different_dead_insertion_order() {
    let mut left = LoanRelation::new(4);
    left.create_root(LoanSupportId(3), shared(90));
    left.create_root(LoanSupportId(2), shared(1));
    left.create_root(LoanSupportId(0), shared(91));
    left.create_child(LoanSupportId(2), LoanSupportId(1), shared(2))
        .unwrap();
    left.clear(LoanSupportId(3));
    left.clear(LoanSupportId(0));
    let mut right = LoanRelation::new(4);
    right.create_root(LoanSupportId(0), shared(91));
    right.create_root(LoanSupportId(2), shared(1));
    right
        .create_child(LoanSupportId(2), LoanSupportId(1), shared(2))
        .unwrap();
    right.create_root(LoanSupportId(3), shared(90));
    right.clear(LoanSupportId(0));
    right.clear(LoanSupportId(3));
    assert_eq!(left, right);
    assert_eq!(
        left.ancestors(),
        &BTreeSet::from([(LoanClassId(1), LoanClassId(0))])
    );
}

#[test]
fn finite_loan_relation_clear_removes_exactly_ended_support() {
    let mut relation = chain();
    let before = relation.clone();
    relation.clear(LoanSupportId(1));
    assert_eq!(relation.classes().len(), before.classes().len());
    for support in [0, 2, 3, 4] {
        let LoanSupport::Available(old) = before.supports()[support] else {
            panic!("available original")
        };
        let LoanSupport::Available(new) = relation.supports()[support] else {
            panic!("available surviving")
        };
        assert_eq!(relation.classes()[new.0], before.classes()[old.0]);
        for other in [0, 2, 3, 4] {
            assert_eq!(
                relation.pair(LoanSupportId(support), LoanSupportId(other)),
                before.pair(LoanSupportId(support), LoanSupportId(other))
            );
        }
    }
    assert_eq!(relation.supports()[1], LoanSupport::Unavailable);
    relation.clear(LoanSupportId(0));
    assert_eq!(
        relation.pair(LoanSupportId(3), LoanSupportId(2)),
        Ok(LoanPair::Ancestor)
    );
    assert_eq!(
        relation.pair(LoanSupportId(4), LoanSupportId(2)),
        Ok(LoanPair::Unrelated)
    );
}

#[test]
fn finite_loan_relation_null_invalid_preserve_spans_and_reject_sources_unchanged() {
    let mut relation = LoanRelation::new(4);
    relation.create_root(LoanSupportId(0), shared(1));
    relation.create_root(LoanSupportId(1), shared(2));
    relation.create_root(LoanSupportId(2), shared(3));
    let null_span = site(10, "null.w").source_span;
    let invalid_span = site(11, "invalid.w").source_span;
    relation.set_null(LoanSupportId(0), null_span.clone());
    relation.set_invalid(LoanSupportId(1), invalid_span.clone());
    assert_eq!(relation.classes(), &[shared(3)]);
    assert_eq!(relation.supports()[0], LoanSupport::Null(null_span.clone()));
    assert_eq!(
        relation.supports()[1],
        LoanSupport::Invalid(invalid_span.clone())
    );
    for (source, error) in [
        (
            LoanSupportId(0),
            LoanRelationError::Null {
                support: LoanSupportId(0),
                source_span: null_span,
            },
        ),
        (
            LoanSupportId(1),
            LoanRelationError::Invalid {
                support: LoanSupportId(1),
                source_span: invalid_span,
            },
        ),
        (
            LoanSupportId(3),
            LoanRelationError::Unavailable(LoanSupportId(3)),
        ),
    ] {
        let before = relation.clone();
        assert_eq!(relation.pair(source, LoanSupportId(2)), Err(error.clone()));
        assert_eq!(relation.pair(LoanSupportId(2), source), Err(error.clone()));
        assert_eq!(relation.copy(source, LoanSupportId(2)), Err(error.clone()));
        assert_eq!(
            relation.transfer(source, LoanSupportId(2)),
            Err(error.clone())
        );
        assert_eq!(
            relation.create_child(source, LoanSupportId(2), shared(4)),
            Err(error)
        );
        assert_eq!(relation, before);
    }
}

#[test]
fn finite_loan_relation_qualified_remap_support_permutation_preserves_complete_pairs() {
    let relation = chain();
    let (targets, sites) = substitutions(&relation);
    let order = [
        LoanSupportId(4),
        LoanSupportId(2),
        LoanSupportId(3),
        LoanSupportId(0),
        LoanSupportId(1),
    ];
    let imported = relation.remap(&order, &targets, &sites).unwrap();
    assert_eq!(imported.classes().len(), relation.classes().len());
    for left in 0..order.len() {
        for right in 0..order.len() {
            assert_eq!(
                imported.pair(LoanSupportId(left), LoanSupportId(right)),
                relation.pair(order[left], order[right])
            );
        }
        let LoanSupport::Available(old) = relation.supports()[order[left].0] else {
            panic!("available original")
        };
        let LoanSupport::Available(new) = imported.supports()[left] else {
            panic!("available imported")
        };
        let original = &relation.classes()[old.0];
        let actual = &imported.classes()[new.0];
        assert_eq!(actual.target, LoanTargetId(original.target.0 + 100));
        assert_eq!(actual.mode, original.mode);
        assert_eq!(
            actual.site,
            sites
                .iter()
                .find(|row| row.from == original.site)
                .unwrap()
                .to
        );
    }
    assert_eq!(relation, chain());
}

#[test]
fn finite_loan_relation_coincident_imported_sites_keep_distinct_classes() {
    let mut relation = LoanRelation::new(3);
    relation.create_root(LoanSupportId(0), shared(1));
    relation
        .create_child(LoanSupportId(0), LoanSupportId(1), shared(2))
        .unwrap();
    relation
        .create_child(LoanSupportId(0), LoanSupportId(2), shared(3))
        .unwrap();
    let (targets, mut sites) = substitutions(&relation);
    for row in &mut sites {
        row.to = site(50, "coincident.w");
    }
    let imported = relation
        .remap(
            &[LoanSupportId(0), LoanSupportId(1), LoanSupportId(2)],
            &targets,
            &sites,
        )
        .unwrap();
    assert_eq!(imported.classes().len(), 3);
    assert!(imported
        .classes()
        .iter()
        .all(|fact| fact == &imported.classes()[0]));
    assert_eq!(
        imported.pair(LoanSupportId(1), LoanSupportId(2)),
        Ok(LoanPair::Unrelated)
    );
    assert_eq!(
        imported.pair(LoanSupportId(0), LoanSupportId(2)),
        Ok(LoanPair::Ancestor)
    );
}

#[test]
fn finite_loan_relation_remap_rejects_missing_duplicate_and_orphan_rows() {
    let relation = chain();
    let before = relation.clone();
    let (targets, sites) = substitutions(&relation);
    let order = (0..5).map(LoanSupportId).collect::<Vec<_>>();
    assert_eq!(
        relation.remap(&order, &targets[..1], &sites),
        Err(LoanRelationError::MissingTarget(LoanTargetId(8)))
    );
    let mut duplicate = targets.clone();
    duplicate.push(targets[0]);
    assert_eq!(
        relation.remap(&order, &duplicate, &sites),
        Err(LoanRelationError::DuplicateTarget(LoanTargetId(7)))
    );
    let mut orphan = targets.clone();
    orphan.push(LoanTargetSubstitution {
        from: LoanTargetId(99),
        to: LoanTargetId(199),
    });
    assert_eq!(
        relation.remap(&order, &orphan, &sites),
        Err(LoanRelationError::ForeignTarget(LoanTargetId(99)))
    );
    assert_eq!(
        relation.remap(&order, &targets, &sites[..3]),
        Err(LoanRelationError::MissingSite(sites[3].from.clone()))
    );
    let mut duplicate = sites.clone();
    duplicate.push(sites[0].clone());
    assert_eq!(
        relation.remap(&order, &targets, &duplicate),
        Err(LoanRelationError::DuplicateSite(sites[0].from.clone()))
    );
    let foreign = site(99, "foreign.w");
    let mut orphan = sites.clone();
    orphan.push(LoanSiteSubstitution {
        from: foreign.clone(),
        to: site(199, "imported.w"),
    });
    assert_eq!(
        relation.remap(&order, &targets, &orphan),
        Err(LoanRelationError::ForeignSite(foreign))
    );
    for invalid_order in [
        vec![LoanSupportId(0)],
        vec![
            LoanSupportId(0),
            LoanSupportId(1),
            LoanSupportId(2),
            LoanSupportId(3),
            LoanSupportId(3),
        ],
        vec![
            LoanSupportId(0),
            LoanSupportId(1),
            LoanSupportId(2),
            LoanSupportId(3),
            LoanSupportId(5),
        ],
    ] {
        assert_eq!(
            relation.remap(&invalid_order, &targets, &sites),
            Err(LoanRelationError::InvalidSupportOrder)
        );
    }
    assert_eq!(relation, before);
}

#[test]
fn finite_loan_relation_many_to_one_target_remap_preserves_all_grant_pairs() {
    let relation = chain();
    let (mut targets, mut sites) = substitutions(&relation);
    for row in &mut targets {
        row.to = LoanTargetId(100);
    }
    for row in &mut sites {
        row.to = site(50, "aliased-caller.w");
    }
    let order = [
        LoanSupportId(4),
        LoanSupportId(3),
        LoanSupportId(2),
        LoanSupportId(1),
        LoanSupportId(0),
    ];
    let imported = relation.remap(&order, &targets, &sites).unwrap();
    assert_eq!(imported.classes().len(), 4);
    assert!(imported
        .classes()
        .iter()
        .all(|fact| fact.target == LoanTargetId(100)));
    assert!(imported
        .classes()
        .iter()
        .all(|fact| fact.site == sites[0].to));
    for left in 0..order.len() {
        for right in 0..order.len() {
            assert_eq!(
                imported.pair(LoanSupportId(left), LoanSupportId(right)),
                relation.pair(order[left], order[right])
            );
        }
    }
    assert_eq!(
        imported.pair(LoanSupportId(0), LoanSupportId(4)),
        Ok(LoanPair::Unrelated)
    );
    assert_eq!(
        imported.pair(LoanSupportId(1), LoanSupportId(3)),
        Ok(LoanPair::Equal)
    );
    assert_eq!(
        imported.pair(LoanSupportId(4), LoanSupportId(2)),
        Ok(LoanPair::Ancestor)
    );
    assert_eq!(relation, chain());
}

#[test]
fn finite_loan_relation_remap_transports_tags_and_empty_relation() {
    let mut relation = LoanRelation::new(3);
    let null_span = site(1, "null.w").source_span;
    let invalid_span = site(2, "invalid.w").source_span;
    relation.set_null(LoanSupportId(0), null_span.clone());
    relation.set_invalid(LoanSupportId(1), invalid_span.clone());
    let imported = relation
        .remap(
            &[LoanSupportId(2), LoanSupportId(0), LoanSupportId(1)],
            &[],
            &[],
        )
        .unwrap();
    assert_eq!(
        imported.supports(),
        &[
            LoanSupport::Unavailable,
            LoanSupport::Null(null_span),
            LoanSupport::Invalid(invalid_span)
        ]
    );
    assert!(imported.classes().is_empty());
    assert!(imported.ancestors().is_empty());
    let empty = LoanRelation::new(0);
    assert_eq!(empty.remap(&[], &[], &[]), Ok(empty));
}

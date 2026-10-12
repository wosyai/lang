use super::*;

const CHAIN: &str = "%%start\nunit() inspect = fn { u32 value = 1; *!u32 root = &!value; *!u32 child = &!(*root); *!u32 grandchild = &!(*child); *grandchild; *child; *root; };\n%%end";
const SAVED: &str = "%%start\nu32() inspect = fn { u32 left = 1; u32 right = 2; *!u32 target = &!left; *!u32 replacement = &!right; u32 old = (*(target = replacement) = 9); old + left + *target };\n%%end";

fn source(text: &str, layout: ScalarTargetLayout, project: bool) -> ScalarReferenceCfg {
    common_loan::tests::cfg(text, layout, project)
}

fn stage(cfg: &ScalarReferenceCfg) -> &StagedLoanQueryAnalysis {
    let rows = cfg
        .evaluated_values
        .staged_loan_queries
        .iter()
        .filter(|row| row.id.owner == cfg.producer_owner)
        .collect::<Vec<_>>();
    let [row] = rows.as_slice() else {
        panic!("one actual worker-finalized stage");
    };
    row
}

fn common_index(cfg: &ScalarReferenceCfg) -> usize {
    let rows = cfg
        .evaluated_values
        .common_loans
        .iter()
        .enumerate()
        .filter(|(_, row)| row.input.owner == cfg.producer_owner)
        .map(|(index, _)| index)
        .collect::<Vec<_>>();
    let [index] = rows.as_slice() else {
        panic!("one qualified common epoch");
    };
    *index
}

fn common(cfg: &ScalarReferenceCfg) -> &CommonLoanAnalysis {
    &cfg.evaluated_values.common_loans[common_index(cfg)]
}

fn prove(cfg: &ScalarReferenceCfg) {
    let staged = stage(cfg);
    let common = cfg
        .evaluated_values
        .common_loans
        .iter()
        .filter(|common| common.input.owner == cfg.producer_owner)
        .collect::<Vec<_>>();
    let [common] = common.as_slice() else {
        panic!("one common epoch");
    };
    assert_eq!(staged.epoch, common.input);
    assert_eq!(staged.coverage, common.pending);
    let StagedLoanQueryResult::Candidates {
        slices,
        queries,
        pending,
    } = &staged.result
    else {
        panic!("actual mutable common candidates");
    };
    assert_eq!(slices.len(), common.candidates.len());
    assert!(pending.is_empty(), "{pending:#?}");
    assert!(!queries.is_empty());
    for (ordinal, candidate) in common.candidates.iter().enumerate() {
        let id = LocalLoanCandidateRef {
            owner: cfg.producer_owner.clone(),
            ordinal,
        };
        let slice = &slices[ordinal];
        assert_eq!(slice.candidate, id);
        assert!(slice.roles.contains(&candidate.role));
        assert!(candidate
            .sites
            .iter()
            .all(|site| slice.sites.contains(site)));
        assert!(slice
            .points
            .iter()
            .any(|point| point.event.event == candidate.event
                && point.event.source_span == candidate.source_span));
        let mut expected = 0;
        for rule in cfg
            .evaluated_values
            .source_contributors
            .iter()
            .filter(|rule| rule.event == candidate.event)
        {
            for equation in cfg
                .evaluated_values
                .source_port_equations
                .iter()
                .filter(|equation| equation.contributor == rule.id)
            {
                expected += 1;
                let rows = queries
                    .iter()
                    .filter(|query| {
                        query.candidate == id
                            && query.contributor == rule.id
                            && query.equation == *equation
                    })
                    .collect::<Vec<_>>();
                let [query] = rows.as_slice() else {
                    panic!("one separate query per actual contributor and equation");
                };
                assert_eq!(query.input, rule.input);
                assert_eq!(query.output, rule.output);
                let row = &cfg.evaluated_values.source_configurations[query.input.index];
                assert_eq!(query.frame, row.frame);
                assert!(query
                    .pending
                    .contains(&LocalLoanPendingReason::RemainingSelectedUses));
                assert!(query
                    .pending
                    .contains(&LocalLoanPendingReason::CompleteFiniteNormalization));
                for pair in &query.pairs {
                    let selections = port_monitor::query(
                        &cfg.evaluated_values,
                        &equation.input,
                        std::slice::from_ref(&pair.support.selected.coordinate),
                        &candidate.source_span,
                    )
                    .expect("same input monitor");
                    assert_eq!(selections, vec![pair.support.selected.clone()]);
                    assert!(row.grants.contains(&pair.support.grant));
                    assert!(row.grants.contains(&pair.protected));
                    assert_eq!(
                        common.input.sites[pair.support.site.0].event,
                        pair.support.grant.site
                    );
                    selected_grant::read(
                        &cfg.evaluated_values,
                        row,
                        &pair.support.selected,
                        &candidate.source_span,
                    )
                    .expect("actual creation witness");
                    if pair.outcome == LocalLoanPairOutcome::PairedAuthority {
                        let LocalLoanAccess::Through(access) = &query.access else {
                            panic!("paired authority requires through access");
                        };
                        assert_eq!(access.grant.referent, pair.protected.referent);
                        assert!(
                            access.grant.class == pair.protected.class
                                || row
                                    .grant_ancestry
                                    .iter()
                                    .any(|ancestor| ancestor.ancestor == pair.protected.class
                                        && ancestor.descendant == access.grant.class)
                        );
                    }
                }
            }
        }
        assert!(expected > 0);
        assert_eq!(
            queries.iter().filter(|query| query.candidate == id).count(),
            expected
        );
    }
    for draft in &cfg.evaluated_values.pending_owner_consumers {
        assert!(draft
            .requirements
            .contains(&OwnerConsumerRequirement::CandidateLocalQueries(
                staged.id.clone()
            )));
        assert!(draft
            .requirements
            .contains(&OwnerConsumerRequirement::StagedLoanValidation(
                SourceEventId {
                    owner: cfg.producer_owner.clone(),
                    point: draft.candidate.consumer.point
                }
            )));
        assert!(draft.requirements.iter().any(|requirement| matches!(
            requirement,
            OwnerConsumerRequirement::FiniteSupportNormalization(_)
        )));
    }
    assert!(cfg.evaluated_values.owner_consumers.is_empty());
}

fn prove_chain(layout: ScalarTargetLayout, project: bool) {
    let cfg = source(CHAIN, layout, project);
    prove(&cfg);
    let StagedLoanQueryResult::Candidates { queries, .. } = &stage(&cfg).result else {
        unreachable!();
    };
    let mut creations = 0;
    let mut paired = 0;
    for query in queries {
        let common = common(&cfg);
        let candidate = &common.candidates[query.candidate.ordinal];
        if matches!(
            cfg.initial_points[candidate.event.point.0].kind,
            CfgPointKind::Borrow {
                request: ReferenceBorrowRequest::Through { .. }
            }
        ) {
            let LocalLoanAccess::Through(access) = &query.access else {
                panic!("through Borrow must query the available saved parent");
            };
            let witnesses = cfg
                .evaluated_values
                .source_grant_creations
                .iter()
                .filter(|witness| {
                    witness.event == candidate.event && witness.contributor == query.contributor
                })
                .collect::<Vec<_>>();
            let [witness] = witnesses.as_slice() else {
                panic!("one actual child creation");
            };
            let SourceGrantParent::Selected {
                class,
                source,
                selection,
            } = &witness.parent
            else {
                panic!("actual saved parent");
            };
            assert_eq!(*class, access.grant.class);
            assert_eq!(*selection, access.selected);
            assert_eq!(selection.observation.case.value, *source);
            assert_ne!(witness.child, access.grant.class);
            assert!(
                !cfg.evaluated_values.source_configurations[query.input.index]
                    .grants
                    .iter()
                    .any(|grant| grant.class == witness.child)
            );
            creations += 1;
        }
        paired += query
            .pairs
            .iter()
            .filter(|pair| pair.outcome == LocalLoanPairOutcome::PairedAuthority)
            .count();
    }
    assert!(creations >= 2);
    assert!(paired > 0);
}

fn prove_saved(layout: ScalarTargetLayout, project: bool) {
    let cfg = source(SAVED, layout, project);
    prove(&cfg);
    let StagedLoanQueryResult::Candidates { queries, .. } = &stage(&cfg).result else {
        unreachable!();
    };
    let mut saved_accesses = 0;
    let mut disjoint = 0;
    for query in queries {
        let LocalLoanAccess::Through(access) = &query.access else {
            continue;
        };
        let value = &access.selected.observation.case.value;
        if matches!(
            cfg.initial_points[value.point.0].kind,
            CfgPointKind::AssignmentResult { .. }
        ) {
            let row = &cfg.evaluated_values.source_configurations[query.input.index];
            assert!(row
                .saved
                .iter()
                .any(|saved| saved.view == access.selected.observation.view));
            for pair in &query.pairs {
                if pair.outcome == LocalLoanPairOutcome::Disjoint {
                    assert_ne!(access.grant.referent, pair.protected.referent);
                    assert_ne!(access.grant.class, pair.protected.class);
                    assert_ne!(access.grant.site, pair.protected.site);
                    disjoint += 1;
                }
            }
            saved_accesses += 1;
        }
    }
    assert!(saved_accesses > 0);
    assert!(disjoint > 0);
}

#[test]
fn local_loan_chain_program_native64() {
    prove_chain(ScalarTargetLayout::NATIVE64, false);
}
#[test]
fn local_loan_chain_program_wasm32() {
    prove_chain(ScalarTargetLayout::WASM32, false);
}
#[test]
fn local_loan_chain_project_native64() {
    prove_chain(ScalarTargetLayout::NATIVE64, true);
}
#[test]
fn local_loan_chain_project_wasm32() {
    prove_chain(ScalarTargetLayout::WASM32, true);
}
#[test]
fn local_loan_saved_program_native64() {
    prove_saved(ScalarTargetLayout::NATIVE64, false);
}
#[test]
fn local_loan_saved_program_wasm32() {
    prove_saved(ScalarTargetLayout::WASM32, false);
}
#[test]
fn local_loan_saved_project_native64() {
    prove_saved(ScalarTargetLayout::NATIVE64, true);
}
#[test]
fn local_loan_saved_project_wasm32() {
    prove_saved(ScalarTargetLayout::WASM32, true);
}

#[test]
fn local_loan_common_only_skips_slice_and_tuple_work() {
    let cfg = source(
        "%%start\nunit() inspect = fn { u32 value = 1; *u32 pointer = &value; *pointer; };\n%%end",
        ScalarTargetLayout::NATIVE64,
        false,
    );
    assert!(common(&cfg).candidates.is_empty());
    assert_eq!(stage(&cfg).result, StagedLoanQueryResult::CommonOnly);
    assert_eq!(stage(&cfg).epoch, common(&cfg).input);
}

#[test]
fn local_loan_stale_epoch_retains_original_invalid_label() {
    let mut cfg = source(CHAIN, ScalarTargetLayout::NATIVE64, false);
    let index = common_index(&cfg);
    cfg.evaluated_values.common_loans[index].input.events[0]
        .successors
        .clear();
    let error = local_loan::register(&mut cfg).expect_err("stale epoch");
    assert!(
        matches!(error, ReferenceAnalysisError::MissingContext { source_span, reason: ReferenceAnalysisErrorReason::InvalidAssignmentCorrespondence } if source_span == cfg.initial_points[0].source_span)
    );
}

#[test]
fn local_loan_foreign_epoch_rejects() {
    let mut cfg = source(CHAIN, ScalarTargetLayout::NATIVE64, false);
    let index = common_index(&cfg);
    cfg.evaluated_values.common_loans[index].input.owner =
        ProducerCfgOwnerId::Module(cfg.initial_points[0].source_span.source.clone());
    assert!(local_loan::register(&mut cfg).is_err());
}

#[test]
fn local_loan_absent_monitor_remains_pending() {
    let mut cfg = source(CHAIN, ScalarTargetLayout::NATIVE64, false);
    let candidate = common(&cfg).candidates[0].clone();
    let mut rule = cfg
        .evaluated_values
        .source_contributors
        .iter()
        .find(|rule| rule.event == candidate.event)
        .expect("actual candidate contributor")
        .clone();
    rule.id.index = cfg.evaluated_values.source_contributors.len();
    let contributor = rule.id.clone();
    cfg.evaluated_values.source_contributors.push(rule);
    local_loan::register(&mut cfg).expect("explicit missing monitor coverage");
    let StagedLoanQueryResult::Candidates {
        pending, queries, ..
    } = &stage(&cfg).result
    else {
        unreachable!();
    };
    assert!(pending.contains(&LocalLoanPendingReason::MissingMonitor(contributor.clone())));
    assert!(queries.iter().all(|query| query.contributor != contributor));
}

#[test]
fn local_loan_foreign_monitor_rejects_original_candidate_source() {
    let mut cfg = source(CHAIN, ScalarTargetLayout::NATIVE64, false);
    let candidate = common(&cfg).candidates[0].clone();
    let contributors = cfg
        .evaluated_values
        .source_contributors
        .iter()
        .filter(|rule| rule.event == candidate.event)
        .map(|rule| rule.id.clone())
        .collect::<Vec<_>>();
    let equation = cfg
        .evaluated_values
        .source_port_equations
        .iter_mut()
        .find(|equation| contributors.contains(&equation.contributor))
        .expect("actual candidate equation");
    equation.input.owner = ProducerCfgOwnerId::Module(candidate.source_span.source.clone());
    let error = local_loan::register(&mut cfg).expect_err("foreign monitor");
    assert!(
        matches!(error, ReferenceAnalysisError::MissingContext { source_span, reason: ReferenceAnalysisErrorReason::InvalidAssignmentCorrespondence } if source_span == candidate.source_span)
    );
}

#[test]
fn local_loan_reader_rejects_missing_draft_query_requirement() {
    let mut cfg = source(SAVED, ScalarTargetLayout::NATIVE64, false);
    assert!(!cfg.evaluated_values.pending_owner_consumers.is_empty());
    for draft in &mut cfg.evaluated_values.pending_owner_consumers {
        draft.requirements.retain(|requirement| {
            !matches!(
                requirement,
                OwnerConsumerRequirement::CandidateLocalQueries(_)
            )
        });
    }
    assert!(local_loan::validate(&cfg).is_err());
}

#[test]
fn local_loan_serde_preserves_exact_equations_and_pending() {
    let cfg = source(SAVED, ScalarTargetLayout::NATIVE64, false);
    let encoded = serde_json::to_string(stage(&cfg)).expect("serialize actual stage");
    let decoded: StagedLoanQueryAnalysis =
        serde_json::from_str(&encoded).expect("deserialize stage");
    assert_eq!(&decoded, stage(&cfg));
}

fn authority_pair(cfg: &ScalarReferenceCfg) -> (&LocalLoanTupleQuery, &LocalLoanPair) {
    let StagedLoanQueryResult::Candidates { queries, .. } = &stage(cfg).result else {
        panic!("candidate queries");
    };
    queries
        .iter()
        .find_map(|query| {
            let LocalLoanAccess::Through(access) = &query.access else {
                return None;
            };
            query
                .pairs
                .iter()
                .find(|pair| {
                    pair.outcome == LocalLoanPairOutcome::PairedAuthority
                        && pair.protected.class != access.grant.class
                        && pair.support.grant.class == pair.protected.class
                })
                .map(|pair| (query, pair))
        })
        .expect("actual strict ancestor paired input")
}

#[test]
fn local_loan_reversed_ancestry_grants_zero_paired_authority() {
    let cfg = source(CHAIN, ScalarTargetLayout::NATIVE64, false);
    let (query, paired) = authority_pair(&cfg);
    let LocalLoanAccess::Through(access) = &query.access else {
        unreachable!();
    };
    let mut row = cfg.evaluated_values.source_configurations[query.input.index].clone();
    row.grant_ancestry.retain(|pair| {
        !(pair.ancestor == paired.protected.class && pair.descendant == access.grant.class)
    });
    row.grant_ancestry.push(SourceGrantAncestryFact {
        ancestor: access.grant.class.clone(),
        descendant: paired.protected.class.clone(),
    });
    assert_eq!(
        local_loan::pair(
            &row,
            &query.access,
            ScalarReferenceMutability::Mutable,
            &paired.protected
        ),
        LocalLoanPairOutcome::IncompatiblePair
    );
}

#[test]
fn local_loan_same_site_distinct_grants_grants_zero_paired_authority() {
    let cfg = source(CHAIN, ScalarTargetLayout::NATIVE64, false);
    let (query, _) = authority_pair(&cfg);
    let LocalLoanAccess::Through(access) = &query.access else {
        unreachable!();
    };
    let mut row = cfg.evaluated_values.source_configurations[query.input.index].clone();
    let referent = SourceCheckedReferentView {
        instance: access.grant.referent.clone(),
        coordinate: access.grant.coordinate.clone(),
        projection: access.grant.projection.clone(),
        grant: cfg.initial_points[access.grant.site.point.0]
            .source_span
            .clone(),
        mode: access.grant.mode,
        loan: SourceCheckedGrantState::Selected(access.grant.class.clone()),
    };
    let class = selected_grant::introduce(
        &mut row,
        &access.grant.site,
        &referent,
        &SourceGrantParent::Root,
    )
    .expect("fresh same-site support class");
    let grants = row
        .grants
        .iter()
        .filter(|grant| grant.class == class)
        .collect::<Vec<_>>();
    let [grant] = grants.as_slice() else {
        panic!("one new grant");
    };
    assert_eq!(grant.site, access.grant.site);
    assert_ne!(grant.class, access.grant.class);
    assert_eq!(
        local_loan::pair(
            &row,
            &query.access,
            ScalarReferenceMutability::Mutable,
            grant
        ),
        LocalLoanPairOutcome::IncompatiblePair
    );
}

#[test]
fn local_loan_siblings_have_zero_paired_ancestry() {
    let cfg = source(CHAIN, ScalarTargetLayout::NATIVE64, false);
    let (query, paired) = authority_pair(&cfg);
    let LocalLoanAccess::Through(access) = &query.access else {
        unreachable!();
    };
    let mut row = cfg.evaluated_values.source_configurations[query.input.index].clone();
    let referent = SourceCheckedReferentView {
        instance: access.grant.referent.clone(),
        coordinate: access.grant.coordinate.clone(),
        projection: access.grant.projection.clone(),
        grant: cfg.initial_points[access.grant.site.point.0]
            .source_span
            .clone(),
        mode: ScalarReferenceMutability::Mutable,
        loan: SourceCheckedGrantState::Selected(access.grant.class.clone()),
    };
    assert_eq!(paired.support.grant.class, paired.protected.class);
    let sibling = selected_grant::introduce(
        &mut row,
        &access.grant.site,
        &referent,
        &SourceGrantParent::Selected {
            class: paired.protected.class.clone(),
            source: paired.support.selected.observation.case.value.clone(),
            selection: paired.support.selected.clone(),
        },
    )
    .expect("same-parent sibling support");
    let grants = row
        .grants
        .iter()
        .filter(|grant| grant.class == sibling)
        .collect::<Vec<_>>();
    let [sibling] = grants.as_slice() else {
        panic!("fresh sibling class");
    };
    assert!(!row
        .grant_ancestry
        .iter()
        .any(|pair| pair.ancestor == sibling.class && pair.descendant == access.grant.class));
    assert!(!row
        .grant_ancestry
        .iter()
        .any(|pair| pair.ancestor == access.grant.class && pair.descendant == sibling.class));
    assert_eq!(
        local_loan::pair(
            &row,
            &query.access,
            ScalarReferenceMutability::Mutable,
            sibling
        ),
        LocalLoanPairOutcome::IncompatiblePair
    );
}

#[test]
fn local_loan_actual_slot_kill_keeps_replacement_without_global_reintroduction() {
    let cfg = source("%%start\nunit() inspect = fn { u32 left = 1; u32 right = 2; *!u32 pointer = &!left; *!u32 next = &!right; pointer = next; *pointer; left; };\n%%end", ScalarTargetLayout::NATIVE64, false);
    let common = common(&cfg);
    let StagedLoanQueryResult::Candidates { slices, .. } = &stage(&cfg).result else {
        panic!("actual conservative candidate");
    };
    let mut kills = 0;
    for slice in slices {
        for point in &slice.points {
            let CfgPointKind::Assign { target, .. } =
                &cfg.initial_points[point.event.event.point.0].kind
            else {
                continue;
            };
            let roles = common
                .input
                .roles
                .iter()
                .enumerate()
                .filter(|(_, role)| {
                    role.source == CommonLoanSourceRole::Binding(target.binding.clone())
                })
                .map(|(index, _)| CommonLoanRoleId(index))
                .collect::<Vec<_>>();
            let [role] = roles.as_slice() else {
                panic!("exact assigned binding role");
            };
            let flows = point
                .event
                .facts
                .iter()
                .filter_map(|fact| match fact.kind {
                    CommonLoanFactKind::Flow { source, target, .. } if target == *role => {
                        Some(source)
                    }
                    _ => None,
                })
                .collect::<Vec<_>>();
            let [source] = flows.as_slice() else {
                panic!("actual replacement flow");
            };
            let expected = point
                .incoming
                .iter()
                .filter(|member| member.role == *source)
                .map(|member| member.site)
                .collect::<BTreeSet<_>>();
            assert_eq!(expected.len(), 1);
            let actual = point
                .outgoing
                .iter()
                .filter(|member| member.role == *role)
                .map(|member| member.site)
                .collect::<BTreeSet<_>>();
            assert_eq!(actual, expected);
            assert!(common.points[point.event.event.point.0]
                .outgoing
                .iter()
                .any(|member| member.role == *role && !expected.contains(&member.site)));
            let facts = &point.event.facts;
            let kill = facts
                .iter()
                .position(|fact| fact.kind == CommonLoanFactKind::KillSlot(*role))
                .expect("actual slot kill");
            let flow = facts.iter().position(|fact| matches!(fact.kind, CommonLoanFactKind::Flow { target, .. } if target == *role)).expect("actual slot replacement");
            assert!(kill < flow);
            kills += 1;
        }
    }
    assert!(kills > 0);
}

#[test]
fn local_loan_dependency_slice_excludes_disconnected_source_loan() {
    let cfg = source("%%start\nunit() inspect = fn { u32 left = 1; u32 right = 2; *!u32 root = &!left; *!u32 child = &!(*root); *child; *root; *u32 unrelated = &right; *unrelated; };\n%%end", ScalarTargetLayout::NATIVE64, false);
    let common = common(&cfg);
    let StagedLoanQueryResult::Candidates { slices, .. } = &stage(&cfg).result else {
        panic!("mutable candidate slices");
    };
    let unrelated = cfg.checked_bindings.iter().filter(|(_, place)| matches!(&place.place, ScalarPlace::Name { name, .. } if name == "unrelated")).map(|(binding, _)| binding).collect::<Vec<_>>();
    let [unrelated] = unrelated.as_slice() else {
        panic!("one source unrelated binding");
    };
    let role = common
        .input
        .roles
        .iter()
        .enumerate()
        .filter(|(_, role)| role.source == CommonLoanSourceRole::Binding((*unrelated).clone()))
        .map(|(index, _)| CommonLoanRoleId(index))
        .collect::<Vec<_>>();
    let [role] = role.as_slice() else {
        panic!("one qualified unrelated common role");
    };
    assert!(!slices.is_empty());
    for slice in slices {
        assert!(!slice.roles.contains(role));
    }
}

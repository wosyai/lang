use super::*;
use crate::parse_source;

const CHAIN: &str = "%%start\nunit() inspect = fn { u32 value = 1; *u32 root = &value; *u32 child = &(*root); *u32 grandchild = &(*child); *u32 sibling = &(*root); *grandchild; *sibling; *u32 copied = sibling; *copied; };\n%%end";
const SAVED: &str = "%%start\nu32() inspect = fn { u32 left = 1; u32 right = 2; *!u32 target = &!left; *!u32 replacement = &!right; u32 old = (*(target = replacement) = 9); old + left + *target };\n%%end";

pub(in crate::scalar) fn cfg(
    text: &str,
    layout: ScalarTargetLayout,
    project: bool,
) -> ScalarReferenceCfg {
    let source = SourceIdentity::new(
        "common-loan-proof".into(),
        "app".into(),
        "src/main.w".into(),
        "r1".into(),
    );
    let parsed = parse_source(source.clone(), text.to_owned(), &[]);
    assert!(parsed.diagnostics.is_empty(), "{:#?}", parsed.diagnostics);
    let validation = derive_scalar_program_with_layout(&parsed.result, layout);
    assert!(
        validation.diagnostics.is_empty(),
        "{:#?}",
        validation.diagnostics
    );
    let items = if project {
        let validation = validate_scalar_project(ScalarProject::new(
            vec![ScalarModule::from_program(validation.program, Vec::new())],
            vec![source],
        ));
        assert!(
            validation.diagnostics.is_empty(),
            "{:#?}",
            validation.diagnostics
        );
        validation
            .project
            .modules
            .into_iter()
            .next()
            .expect("actual module")
            .items
    } else {
        validation.program.items
    };
    items
        .into_iter()
        .find_map(|item| match item {
            ScalarItem::Function(function) if function.name == "inspect" => {
                Some(function.reference_cfg)
            }
            _ => None,
        })
        .expect("actual function")
}

fn analysis(cfg: &ScalarReferenceCfg) -> &CommonLoanAnalysis {
    let rows = cfg
        .evaluated_values
        .common_loans
        .iter()
        .filter(|row| row.input.owner == cfg.producer_owner)
        .collect::<Vec<_>>();
    let [row] = rows.as_slice() else {
        panic!("one actual common producer epoch");
    };
    row
}

fn prove_chain(layout: ScalarTargetLayout, project: bool) {
    let cfg = cfg(CHAIN, layout, project);
    let row = analysis(&cfg);
    assert_eq!(
        row.input,
        extraction::extract(
            &cfg.evaluated_values,
            &cfg.producer_owner,
            &cfg.initial_points
        )
        .expect("same immutable source")
    );
    assert_eq!(row.input.sites.len(), 4);
    let mut children = 0;
    let mut copies = 0;
    for event in &row.input.events {
        for fact in &event.facts {
            assert_eq!(fact.event, event.event);
            assert_eq!(
                fact.source_span,
                cfg.initial_points[event.event.point.0].source_span
            );
            match &fact.kind {
                CommonLoanFactKind::Issue { output, site } => {
                    assert_eq!(row.input.sites[site.0].event, event.event);
                    assert!(
                        matches!(&row.input.roles[output.0].source, CommonLoanSourceRole::Evaluated(value) if value.point == event.event.point)
                    );
                    assert!(row.points[event.event.point.0].outgoing.contains(
                        &CommonLoanMembership {
                            role: *output,
                            site: *site
                        }
                    ));
                }
                CommonLoanFactKind::ParentSite { child, parent } => {
                    children += 1;
                    assert!(
                        matches!(row.input.sites[child.0].target, CommonLoanTarget::Through { pointer, .. } if pointer == *parent)
                    );
                    assert!(matches!(
                        row.input.roles[parent.0].source,
                        CommonLoanSourceRole::Evaluated(_)
                    ));
                }
                CommonLoanFactKind::Flow {
                    source,
                    target,
                    policy: CopyPolicy::Copy,
                } => {
                    copies += 1;
                    assert!(row.subset.contains(&CommonLoanSubset {
                        source: *source,
                        target: *target
                    }));
                }
                _ => {}
            }
        }
    }
    let binding = |name: &str| {
        let bindings = cfg
            .checked_bindings
            .iter()
            .filter(|(_, place)| {
                matches!(&place.place, ScalarPlace::Name { name: actual, .. } if actual == name)
            })
            .map(|(binding, _)| binding)
            .collect::<Vec<_>>();
        let [binding] = bindings.as_slice() else {
            panic!("one qualified checked binding for {name}");
        };
        assert_eq!(binding.source, cfg.initial_points[0].source_span.source);
        (*binding).clone()
    };
    let sibling = binding("sibling");
    let copied = binding("copied");
    let root = binding("root");
    let role = |source| {
        let expected = CommonLoanRole {
            owner: cfg.producer_owner.clone(),
            source,
            projection: Vec::new(),
        };
        let roles = row
            .input
            .roles
            .iter()
            .enumerate()
            .filter(|(_, role)| **role == expected)
            .map(|(index, _)| CommonLoanRoleId(index))
            .collect::<Vec<_>>();
        let [role] = roles.as_slice() else {
            panic!("one exact qualified common role");
        };
        *role
    };
    let bind = |binding: &ReferenceBindingId| {
        let points = cfg
            .initial_points
            .iter()
            .filter(|point| {
                matches!(&point.kind, CfgPointKind::Bind { binding: actual } if actual == binding)
            })
            .collect::<Vec<_>>();
        let [point] = points.as_slice() else {
            panic!("one canonical Bind for the qualified binding");
        };
        *point
    };
    let input = |point: &ReferenceFlowPoint| {
        let inputs = cfg
            .evaluated_values
            .binding_inputs
            .iter()
            .filter(|input| input.producer == cfg.producer_owner && input.consumer == point.id)
            .collect::<Vec<_>>();
        let [input] = inputs.as_slice() else {
            panic!("one canonical binding source");
        };
        assert_eq!(input.source.resolved.producer, cfg.producer_owner);
        *input
    };
    let event = |point: &ReferenceFlowPoint| {
        let expected = SourceEventId {
            owner: cfg.producer_owner.clone(),
            point: point.id,
        };
        let events = row
            .input
            .events
            .iter()
            .filter(|event| event.event == expected)
            .collect::<Vec<_>>();
        let [event] = events.as_slice() else {
            panic!("one common event for the canonical point");
        };
        assert_eq!(event.source_span, point.source_span);
        *event
    };
    let sites = |members: &[CommonLoanMembership], role| {
        members
            .iter()
            .filter(|member| member.role == role)
            .map(|member| member.site)
            .collect::<BTreeSet<_>>()
    };
    let sibling_bind = bind(&sibling);
    let sibling_input = input(sibling_bind);
    let sibling_borrow = &cfg.initial_points[sibling_input.source.point.0];
    let CfgPointKind::Borrow {
        request:
            ReferenceBorrowRequest::Through {
                pointer_point,
                mode,
                ..
            },
    } = &sibling_borrow.kind
    else {
        panic!("sibling originates at its actual through Borrow");
    };
    assert_eq!(*mode, ScalarReferenceMutability::Shared);
    assert!(matches!(
        &cfg.initial_points[pointer_point.0].kind,
        CfgPointKind::Read { binding } if *binding == root
    ));
    let sibling_event = event(sibling_borrow);
    assert_eq!(
        sibling_input.source.resolved.occurrence,
        sibling_borrow.source_span
    );
    let sibling_sites = row
        .input
        .sites
        .iter()
        .enumerate()
        .filter(|(_, site)| site.event == sibling_event.event)
        .collect::<Vec<_>>();
    let [(site_index, site)] = sibling_sites.as_slice() else {
        panic!("one common site from the original sibling producer");
    };
    let sibling_site = CommonLoanSiteId(*site_index);
    assert_eq!(site.source_span, sibling_borrow.source_span);
    assert_eq!(site.mode, ScalarReferenceMutability::Shared);
    let sibling_role = role(CommonLoanSourceRole::Binding(sibling.clone()));
    let original_role = role(CommonLoanSourceRole::Evaluated(
        sibling_input.source.clone(),
    ));
    let sibling_bind_event = event(sibling_bind);
    assert_eq!(
        sibling_bind_event
            .facts
            .iter()
            .map(|fact| fact.kind.clone())
            .collect::<Vec<_>>(),
        vec![
            CommonLoanFactKind::KillSlot(sibling_role),
            CommonLoanFactKind::Flow {
                source: original_role,
                target: sibling_role,
                policy: CopyPolicy::Copy,
            },
        ]
    );
    assert!(sibling_event.facts.iter().any(|fact| {
        fact.kind
            == CommonLoanFactKind::Issue {
                output: original_role,
                site: sibling_site,
            }
    }));
    let copied_bind = bind(&copied);
    let copied_input = input(copied_bind);
    let copy_read = &cfg.initial_points[copied_input.source.point.0];
    assert!(matches!(
        &copy_read.kind,
        CfgPointKind::Read { binding } if *binding == sibling
    ));
    assert_eq!(
        copied_input.source.resolved.occurrence,
        copy_read.source_span
    );
    let outputs = cfg
        .evaluated_values
        .values
        .iter()
        .filter(|value| value.id == copied_input.source)
        .collect::<Vec<_>>();
    let [output] = outputs.as_slice() else {
        panic!("one registered actual sibling Read output");
    };
    let shapes = cfg
        .evaluated_values
        .shapes
        .iter()
        .filter(|shape| shape.id == output.shape)
        .collect::<Vec<_>>();
    let [shape] = shapes.as_slice() else {
        panic!("one registered copy source shape");
    };
    assert!(matches!(
        shape.payload,
        ValuePayloadShape::Checked {
            mutability: ScalarReferenceMutability::Shared,
            ..
        }
    ));
    let copy_source_role = role(CommonLoanSourceRole::Evaluated(copied_input.source.clone()));
    let copied_role = role(CommonLoanSourceRole::Binding(copied.clone()));
    let read_event = event(copy_read);
    let bind_event = event(copied_bind);
    let read_flows = read_event
        .facts
        .iter()
        .filter(|fact| matches!(fact.kind, CommonLoanFactKind::Flow { .. }))
        .collect::<Vec<_>>();
    let [read_flow] = read_flows.as_slice() else {
        panic!("one actual sibling-to-copy-source Read flow");
    };
    assert_eq!(
        read_flow.kind,
        CommonLoanFactKind::Flow {
            source: sibling_role,
            target: copy_source_role,
            policy: CopyPolicy::Copy,
        }
    );
    assert_eq!(
        bind_event
            .facts
            .iter()
            .map(|fact| fact.kind.clone())
            .collect::<Vec<_>>(),
        vec![
            CommonLoanFactKind::KillSlot(copied_role),
            CommonLoanFactKind::Flow {
                source: copy_source_role,
                target: copied_role,
                policy: CopyPolicy::Copy,
            },
        ]
    );
    for (source, target) in [
        (sibling_role, copy_source_role),
        (copy_source_role, copied_role),
    ] {
        assert!(row.subset.contains(&CommonLoanSubset { source, target }));
    }
    for event in [read_event, bind_event] {
        assert!(event
            .facts
            .iter()
            .all(|fact| !matches!(fact.kind, CommonLoanFactKind::Issue { .. })));
        assert!(row.points[event.event.point.0].issued.is_empty());
    }
    let copy_point = &row.points[copied_bind.id.0];
    assert_eq!(copy_point.event, bind_event.event);
    let copy_sites = sites(&copy_point.incoming, copy_source_role);
    assert_eq!(copy_sites, BTreeSet::from([sibling_site]));
    assert_eq!(sites(&copy_point.incoming, copied_role), copy_sites);
    assert_eq!(sites(&copy_point.outgoing, copy_source_role), copy_sites);
    assert_eq!(sites(&copy_point.outgoing, copied_role), copy_sites);
    let sibling_point = &row.points[sibling_bind.id.0];
    assert_eq!(sibling_point.event, sibling_bind_event.event);
    assert_eq!(sites(&sibling_point.incoming, original_role), copy_sites);
    assert_eq!(sites(&sibling_point.outgoing, sibling_role), copy_sites);
    let read_point = &row.points[copy_read.id.0];
    assert_eq!(read_point.event, read_event.event);
    assert_eq!(sites(&read_point.incoming, sibling_role), copy_sites);
    assert_eq!(sites(&read_point.outgoing, copy_source_role), copy_sites);
    let pointer_reads = cfg
        .initial_points
        .iter()
        .filter(|point| matches!(&point.kind, CfgPointKind::Read { binding } if *binding == copied))
        .collect::<Vec<_>>();
    let [pointer_read] = pointer_reads.as_slice() else {
        panic!("one final copied pointer Read");
    };
    let pointer_outputs = cfg
        .evaluated_values
        .values
        .iter()
        .filter(|value| {
            value.id.resolved.producer == cfg.producer_owner && value.id.point == pointer_read.id
        })
        .collect::<Vec<_>>();
    let [pointer_output] = pointer_outputs.as_slice() else {
        panic!("one actual final pointer output");
    };
    assert_eq!(
        pointer_output.id.resolved.occurrence,
        pointer_read.source_span
    );
    let pointer_role = role(CommonLoanSourceRole::Evaluated(pointer_output.id.clone()));
    let pointer_event = event(pointer_read);
    assert!(pointer_event.facts.iter().any(|fact| {
        fact.kind
            == CommonLoanFactKind::Flow {
                source: copied_role,
                target: pointer_role,
                policy: CopyPolicy::Copy,
            }
    }));
    assert!(row.subset.contains(&CommonLoanSubset {
        source: copied_role,
        target: pointer_role
    }));
    let through_points = cfg
        .initial_points
        .iter()
        .filter(|point| matches!(&point.kind, CfgPointKind::ReadPlace { pointer_point: Some(pointer), .. } if *pointer == pointer_read.id))
        .collect::<Vec<_>>();
    let [through_point] = through_points.as_slice() else {
        panic!("one actual final *copied use");
    };
    let CfgPointKind::ReadPlace { place, .. } = &through_point.kind else {
        unreachable!();
    };
    let through_event = event(through_point);
    assert!(through_event.facts.iter().any(|fact| {
        fact.kind
            == CommonLoanFactKind::Use {
                role: pointer_role,
                mode: ScalarReferenceMutability::Shared,
                target: CommonLoanTarget::Through {
                    pointer: pointer_role,
                    place: place.clone(),
                },
                kind: CommonLoanUseKind::Access,
            }
    }));
    let pointer_point = &row.points[pointer_read.id.0];
    assert_eq!(pointer_point.event, pointer_event.event);
    assert_eq!(sites(&pointer_point.incoming, copied_role), copy_sites);
    assert_eq!(sites(&pointer_point.outgoing, pointer_role), copy_sites);
    assert!(pointer_point.demand_in.contains(&copied_role));
    assert!(pointer_point.demand_out.contains(&pointer_role));
    let through_state = &row.points[through_point.id.0];
    assert_eq!(through_state.event, through_event.event);
    assert_eq!(sites(&through_state.incoming, pointer_role), copy_sites);
    assert!(through_state.demand_in.contains(&pointer_role));
    assert!(through_state.may_live.contains(&sibling_site));
    assert_eq!(children, 3);
    assert!(copies > 0);
    assert!(row.subset.len() <= row.input.roles.len() * row.input.roles.len());
    assert!(cfg.evaluated_values.owner_consumers.is_empty());
}

fn prove_saved(layout: ScalarTargetLayout, project: bool) {
    let cfg = cfg(SAVED, layout, project);
    let row = analysis(&cfg);
    let mut moves = 0;
    let mut snapshots = 0;
    let mut results = 0;
    for event in &row.input.events {
        for (index, fact) in event.facts.iter().enumerate() {
            if let CommonLoanFactKind::Flow {
                source,
                target,
                policy,
            } = &fact.kind
            {
                match (
                    &row.input.roles[source.0].source,
                    &row.input.roles[target.0].source,
                ) {
                    (CommonLoanSourceRole::Binding(_), CommonLoanSourceRole::Evaluated(_))
                        if event.kind == source_epoch::SourceEventKind::Move =>
                    {
                        moves += 1;
                        assert_eq!(*policy, CopyPolicy::Move);
                        assert!(event.facts[index + 1..]
                            .iter()
                            .any(|fact| fact.kind == CommonLoanFactKind::KillSlot(*source)));
                    }
                    (CommonLoanSourceRole::Binding(_), CommonLoanSourceRole::Saved(saved)) => {
                        snapshots += 1;
                        assert_eq!(*saved, event.event);
                    }
                    (CommonLoanSourceRole::Saved(saved), CommonLoanSourceRole::Result(result)) => {
                        results += 1;
                        assert_ne!(saved, result);
                    }
                    _ => {}
                }
                assert!(row.subset.contains(&CommonLoanSubset {
                    source: *source,
                    target: *target
                }));
            }
        }
    }
    assert!(moves > 0 && snapshots > 0 && results > 0);
    let candidates = row
        .candidates
        .iter()
        .filter(|candidate| candidate.reason == CommonLoanCandidateReason::PossibleConflict)
        .collect::<Vec<_>>();
    assert!(
        !candidates.is_empty(),
        "actual mutable source produces qualified common candidates"
    );
    for candidate in candidates {
        let point = &cfg.initial_points[candidate.event.point.0];
        assert_eq!(candidate.event.owner, cfg.producer_owner);
        assert_eq!(candidate.source_span, point.source_span);
        assert!(row.input.events[candidate.event.point.0].facts.iter().any(|fact| matches!(&fact.kind, CommonLoanFactKind::Use { role, mode, target, kind: CommonLoanUseKind::Access } if *role == candidate.role && *mode == candidate.mode && *target == candidate.target)));
        for site in &candidate.sites {
            assert!(row.points[candidate.event.point.0].may_live.contains(site));
            assert!(
                candidate.mode == ScalarReferenceMutability::Mutable
                    || row.input.sites[site.0].mode == ScalarReferenceMutability::Mutable
            );
        }
    }
    assert!(
        !cfg.evaluated_values.pending_owner_consumers.is_empty(),
        "real ReturnOutput drafts consume the common epoch"
    );
    for draft in &cfg.evaluated_values.pending_owner_consumers {
        let event = SourceEventId {
            owner: draft.candidate.consumer.producer.clone(),
            point: draft.candidate.consumer.point,
        };
        assert!(draft
            .requirements
            .contains(&OwnerConsumerRequirement::StagedLoanValidation(
                event.clone()
            )));
        require_epoch(
            &cfg.evaluated_values,
            &cfg.producer_owner,
            &cfg.initial_points,
            &event,
        )
        .expect("actual same epoch consumer");
    }
    assert!(cfg.evaluated_values.owner_consumers.is_empty());
}

#[test]
fn common_loan_chain_program_native64() {
    prove_chain(ScalarTargetLayout::NATIVE64, false);
}
#[test]
fn common_loan_chain_program_wasm32() {
    prove_chain(ScalarTargetLayout::WASM32, false);
}
#[test]
fn common_loan_chain_project_native64() {
    prove_chain(ScalarTargetLayout::NATIVE64, true);
}
#[test]
fn common_loan_chain_project_wasm32() {
    prove_chain(ScalarTargetLayout::WASM32, true);
}
#[test]
fn common_loan_saved_program_native64() {
    prove_saved(ScalarTargetLayout::NATIVE64, false);
}
#[test]
fn common_loan_saved_program_wasm32() {
    prove_saved(ScalarTargetLayout::WASM32, false);
}
#[test]
fn common_loan_saved_project_native64() {
    prove_saved(ScalarTargetLayout::NATIVE64, true);
}
#[test]
fn common_loan_saved_project_wasm32() {
    prove_saved(ScalarTargetLayout::WASM32, true);
}

#[test]
fn common_loan_missing_epoch_rejected() {
    let mut cfg = cfg(SAVED, ScalarTargetLayout::NATIVE64, false);
    let event = analysis(&cfg)
        .input
        .events
        .iter()
        .find(|event| event.kind == source_epoch::SourceEventKind::ReturnOutput)
        .expect("actual return")
        .event
        .clone();
    cfg.evaluated_values.common_loans.clear();
    let error = require_epoch(
        &cfg.evaluated_values,
        &cfg.producer_owner,
        &cfg.initial_points,
        &event,
    )
    .expect_err("missing epoch");
    assert_eq!(
        error.source_span(),
        &cfg.initial_points[event.point.0].source_span
    );
}

#[test]
fn common_loan_serde_preserves_source_epoch() {
    let cfg = cfg(CHAIN, ScalarTargetLayout::NATIVE64, false);
    let row = analysis(&cfg);
    let restored: CommonLoanAnalysis =
        serde_json::from_str(&serde_json::to_string(row).expect("serialize")).expect("deserialize");
    assert_eq!(*row, restored);
    assert_eq!(row.common_question(), restored.common_question());
}

#[test]
fn common_loan_foreign_epoch_rejected() {
    let mut cfg = cfg(SAVED, ScalarTargetLayout::NATIVE64, false);
    let event = analysis(&cfg)
        .input
        .events
        .iter()
        .find(|event| event.kind == source_epoch::SourceEventKind::ReturnOutput)
        .expect("actual return")
        .event
        .clone();
    cfg.evaluated_values.common_loans[0].input.owner =
        ProducerCfgOwnerId::Module(SourceIdentity::new(
            "foreign".into(),
            "app".into(),
            "src/main.w".into(),
            "r1".into(),
        ));
    let error = require_epoch(
        &cfg.evaluated_values,
        &cfg.producer_owner,
        &cfg.initial_points,
        &event,
    )
    .expect_err("foreign epoch");
    assert_eq!(
        error.source_span(),
        &cfg.initial_points[event.point.0].source_span
    );
}

#[test]
fn common_loan_changed_successor_epoch_rejected() {
    let cfg = cfg(SAVED, ScalarTargetLayout::NATIVE64, false);
    let event = analysis(&cfg)
        .input
        .events
        .iter()
        .find(|event| event.kind == source_epoch::SourceEventKind::ReturnOutput)
        .expect("actual return")
        .event
        .clone();
    let mut points = cfg.initial_points.clone();
    points[0].successors.clear();
    let error = require_epoch(&cfg.evaluated_values, &cfg.producer_owner, &points, &event)
        .expect_err("changed actual CFG epoch");
    assert_eq!(error.source_span(), &points[event.point.0].source_span);
}

#[test]
fn common_loan_multi_output_retains_pending() {
    let cfg = cfg(CHAIN, ScalarTargetLayout::NATIVE64, false);
    let site = &analysis(&cfg).input.sites[0];
    let mut tables = cfg.evaluated_values.clone();
    let value = tables
        .values
        .iter()
        .find(|value| value.id.point == site.event.point)
        .expect("actual borrow output")
        .clone();
    let mut second = value.clone();
    second.id.resolved.output += 1;
    tables.values.push(second.clone());
    let input = extraction::extract(&tables, &cfg.producer_owner, &cfg.initial_points)
        .expect("explicit ambiguous coverage");
    assert!(input
        .pending
        .iter()
        .any(|pending| pending.event == site.event
            && pending.reason
                == CommonLoanPendingReason::AmbiguousOutputs(vec![
                    value.id.clone(),
                    second.id.clone()
                ])));
    assert!(!input.sites.iter().any(|actual| actual.event == site.event));
    assert_eq!(
        solver::solve(input).common_question(),
        CommonLoanQuestion::PendingValidation
    );
}

#[test]
fn common_loan_boundary_coverage_prevents_empty_certificate() {
    let cfg = cfg(
        "%%start\nunit(*u32) inspect = fn(address) { address; };\n%%end",
        ScalarTargetLayout::NATIVE64,
        false,
    );
    let row = analysis(&cfg);
    assert!(row
        .input
        .pending
        .iter()
        .any(|pending| pending.reason == CommonLoanPendingReason::Boundary));
    assert!(row.input.roles.iter().any(|role| matches!(
        role.source,
        CommonLoanSourceRole::Boundary(BoundaryPortRole::Parameter { .. })
    )));
    assert_eq!(row.common_question(), CommonLoanQuestion::PendingValidation);
}

#[test]
fn common_loan_capture_coverage_retains_actual_event() {
    let cfg = cfg("%%start\nunit() inspect = fn { u32 value = 1; u32() reader = fn { value }; reader; };\n%%end", ScalarTargetLayout::NATIVE64, false);
    let row = analysis(&cfg);
    let event = row
        .input
        .events
        .iter()
        .find(|event| event.kind == source_epoch::SourceEventKind::Capture)
        .expect("actual capture event");
    assert!(row.pending.contains(&CommonLoanPending {
        event: event.event.clone(),
        source_span: event.source_span.clone(),
        reason: CommonLoanPendingReason::Capture
    }));
    assert_eq!(row.common_question(), CommonLoanQuestion::PendingValidation);
}

#[test]
fn common_loan_same_site_parent_path_retains_candidate() {
    let cfg = cfg(CHAIN, ScalarTargetLayout::NATIVE64, false);
    let mut input = analysis(&cfg).input.clone();
    let root = input.sites[0].event.clone();
    for site in &mut input.sites {
        site.mode = ScalarReferenceMutability::Mutable;
        site.event = root.clone();
    }
    let row = solver::solve(input);
    for event in row.input.events.iter().filter(|event| {
        event
            .facts
            .iter()
            .any(|fact| matches!(fact.kind, CommonLoanFactKind::ParentSite { .. }))
    }) {
        let candidate = row
            .candidates
            .iter()
            .find(|candidate| {
                candidate.event == event.event
                    && candidate.reason == CommonLoanCandidateReason::PossibleConflict
            })
            .expect("possible ancestry provides zero authority");
        assert!(candidate.sites.contains(&CommonLoanSiteId(0)));
        assert_eq!(candidate.source_span, event.source_span);
        assert!(
            matches!(candidate.target, CommonLoanTarget::Through { pointer, .. } if pointer == candidate.role)
        );
    }
    assert_eq!(row.common_question(), CommonLoanQuestion::PendingValidation);
}

#[test]
fn common_loan_subset_is_function_wide_and_directional() {
    let cfg = cfg(CHAIN, ScalarTargetLayout::NATIVE64, false);
    let row = analysis(&cfg);
    let issue = row
        .input
        .events
        .iter()
        .flat_map(|event| &event.facts)
        .find_map(|fact| match fact.kind {
            CommonLoanFactKind::Issue {
                output,
                site: CommonLoanSiteId(0),
            } => Some(output),
            _ => None,
        })
        .expect("actual root output");
    let flows = row
        .input
        .events
        .iter()
        .flat_map(|event| &event.facts)
        .filter_map(|fact| match fact.kind {
            CommonLoanFactKind::Flow { source, target, .. } if source == issue => Some(target),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert!(!flows.is_empty());
    for target in flows {
        assert!(row.subset.contains(&CommonLoanSubset {
            source: issue,
            target
        }));
        assert!(!row.subset.contains(&CommonLoanSubset {
            source: target,
            target: issue
        }));
    }
    for index in 0..row.input.roles.len() {
        assert!(row.subset.contains(&CommonLoanSubset {
            source: CommonLoanRoleId(index),
            target: CommonLoanRoleId(index)
        }));
    }
}

use super::*;

fn evaluated_result(
    text: &str,
    layout: ScalarTargetLayout,
    project: bool,
) -> (ScalarReferenceCfg, Vec<Diagnostic>) {
    let parsed = parse_source(source(), text.to_owned(), &[]);
    assert!(
        parsed.diagnostics.is_empty(),
        "{text}\n{:?}",
        parsed.diagnostics
    );
    let single = derive_scalar_program_with_layout(&parsed.result, layout);
    let (items, diagnostics) = if project {
        let validated = validate_scalar_project(ScalarProject::new(
            vec![ScalarModule::from_program(single.program, Vec::new())],
            vec![source()],
        ));
        (
            validated.project.modules[0].items.clone(),
            validated.diagnostics,
        )
    } else {
        (single.program.items, single.diagnostics)
    };
    let function = items
        .iter()
        .find_map(|item| match item {
            ScalarItem::Function(function) if function.name == "run" => Some(function),
            _ => None,
        })
        .expect("run function");
    (function.reference_cfg.clone(), diagnostics)
}

fn evaluated_cfg(text: &str, layout: ScalarTargetLayout, project: bool) -> (ScalarReferenceCfg, source_boundary::SourceCapturePreflightInput) {
    let (cfg, diagnostics, capture_input) = source_boundary::evaluated_borrow_fixture_input(source(), text, layout, project);
    assert!(
        diagnostics.is_empty(),
        "{layout:?}, project={project}: {diagnostics:#?}\ninitial CFG: {:#?}",
        cfg.initial_points
            .iter()
            .map(|point| {
                (
                    point.id,
                    &point.source_span,
                    &point.kind,
                    &point.possible_origins,
                    &point.successors,
                )
            })
            .collect::<Vec<_>>()
    );
    (cfg, capture_input)
}

fn borrow_at<'a>(cfg: &'a ScalarReferenceCfg, text: &str, address: &str) -> &'a ReferenceFlowPoint {
    let start = text.find(address).unwrap() as u32;
    let span = SourceSpan::new(source(), ByteSpan::new(start, start + address.len() as u32));
    cfg.points
        .iter()
        .find(|point| {
            point.source_span == span && matches!(point.kind, CfgPointKind::Borrow { .. })
        })
        .expect("actual address producer")
}

fn sole_origin(point: &ReferenceFlowPoint) -> ReferenceOriginId {
    let origins = point.possible_origins.as_ref().expect("evaluated origins");
    assert_eq!(origins.len(), 1);
    *origins.first().unwrap()
}

fn span_of(text: &str, occurrence: &str) -> ByteSpan {
    let start = text.find(occurrence).unwrap() as u32;
    ByteSpan::new(start, start + occurrence.len() as u32)
}

fn through(cfg: &ScalarReferenceCfg, child: &ReferenceFlowPoint, parent: &ReferenceFlowPoint) {
    let cases = evaluated_borrow::accepted_cases(cfg, child.id);
    assert_eq!(cases.len(), 1);
    let ReferenceBorrowCase::Accepted { loan, access, .. } = cases[0] else {
        panic!("child borrow")
    };
    let ReferenceBorrowAccess::Through {
        pointer_point,
        parent_origin,
    } = access
    else {
        panic!("evaluated through access")
    };
    assert!(*pointer_point < child.id);
    let pointer = &cfg.points[pointer_point.0];
    assert!(matches!(pointer.kind, CfgPointKind::Read { .. }));
    assert_eq!(*parent_origin, sole_origin(pointer));
    assert_eq!(*parent_origin, sole_origin(parent));
    let parent_loan = evaluated_borrow::accepted_loan(cfg, parent.id);
    assert_eq!(loan.parent.as_deref(), Some(parent_loan));
    assert_eq!(loan.creation_source, child.source_span.source);
    assert_eq!(loan.creation_span, child.source_span.range);
    let ReferenceOrigin::Fresh {
        allocation: parent_allocation,
        loan: recorded_parent,
    } = &cfg.facts[parent_origin.0]
    else {
        panic!("actual fresh parent")
    };
    let ReferenceOrigin::Fresh {
        allocation: child_allocation,
        loan: recorded_child,
    } = &cfg.facts[sole_origin(child).0]
    else {
        panic!("actual fresh child")
    };
    assert_eq!(recorded_parent, parent_loan);
    assert_eq!(recorded_child, loan);
    assert_eq!(child_allocation, parent_allocation);
    let initial = &cfg.initial_points[child.id.0];
    assert_eq!(initial.kind, child.kind);
}

fn roundtrip(cfg: &ScalarReferenceCfg) {
    let restored: ScalarReferenceCfg =
        serde_json::from_str(&serde_json::to_string(cfg).unwrap()).unwrap();
    assert_eq!(*cfg, restored);
}

fn retransfer(cfg: &mut ScalarReferenceCfg, capture_input: &source_boundary::SourceCapturePreflightInput) -> Vec<ReferenceAnalysisError> {
    reference::retransfer_evaluated_borrow_test(cfg, capture_input)
}

fn storage_proof(layout: ScalarTargetLayout, project: bool) {
    let (cfg, _) = evaluated_cfg(EXACT, layout, project);
    for (binding, place) in &cfg.checked_bindings {
        assert_eq!(binding, &place.binding);
        assert_eq!(place.source, source());
        assert_eq!(place.function_span, binding.function_span);
        assert_eq!(place.block_span, binding.block_span);
        assert_eq!(place.declaration_span, binding.declaration_span);
        assert!(place.projections.is_empty());
        let ScalarPlace::Name { name, span } = &place.place else {
            unreachable!()
        };
        assert_eq!(*span, binding.declaration_span);
        assert!(name == "parent" || name == "child");
        let bind = cfg.points.iter().find(|point| matches!(&point.kind, CfgPointKind::Bind { binding: actual } if actual == binding)).unwrap();
        let install = cfg
            .checked_installs
            .iter()
            .find(|install| install.point == bind.id)
            .unwrap();
        let ReferenceCheckedInstallInput::Evaluated { value_point } = install.input else {
            unreachable!()
        };
        let evaluations = cfg
            .checked_install_evaluations
            .iter()
            .filter(|evaluation| evaluation.point == bind.id)
            .collect::<Vec<_>>();
        assert_eq!(evaluations.len(), 1);
        let evaluation = evaluations[0];
        assert_eq!(&evaluation.target, place);
        assert_eq!(evaluation.input, install.input);
        assert_eq!(
            evaluation.accepted_origins,
            cfg.points[value_point.0].possible_origins.clone().unwrap()
        );
        assert_eq!(
            evaluation.disposition,
            ReferenceCheckedInstallDisposition::Written
        );
        assert!(evaluation.previous_origins.is_empty());
    }
    roundtrip(&cfg);
    let branch = "%%start\nunit(bool) run = fn(flag) { u32 item = 1; if (flag) { *u32 selected = &item; selected; } else { *u32 selected = &item; selected; }; };\nrun(true);\n%%end";
    let (cfg, _) = evaluated_cfg(branch, layout, project);
    let slots = cfg.checked_bindings.values().collect::<Vec<_>>();
    assert_eq!(slots.len(), 2);
    assert_ne!(slots[0].binding, slots[1].binding);
    assert_ne!(slots[0].block_span, slots[1].block_span);
    assert!(slots
        .iter()
        .all(|slot| matches!(&slot.place, ScalarPlace::Name { name, .. } if name == "selected")));
    roundtrip(&cfg);
    let parameter_source = "%%start\nunit(*!u32) run = fn(parent) { *!u32 child = &!(*parent); *child = 2; *parent = 3; };\n%%end";
    let (cfg, _) = evaluated_cfg(parameter_source, layout, project);
    let seed = cfg
        .checked_installs
        .iter()
        .find(|install| {
            matches!(
                install.input,
                ReferenceCheckedInstallInput::Parameter { .. }
            )
        })
        .unwrap();
    let ReferenceCheckedInstallInput::Parameter { binding, origin } = &seed.input else {
        unreachable!()
    };
    assert_eq!(binding.kind, ReferenceBindingKind::Declared);
    assert!(
        matches!(&cfg.checked_bindings[binding].place, ScalarPlace::Name { name, span } if name == "parent" && *span == binding.declaration_span)
    );
    let ReferenceOrigin::BorrowedFrom { parameter, loan } = &cfg.facts[origin.0] else {
        unreachable!()
    };
    assert_eq!(*parameter, 0);
    assert_eq!(
        loan.origin_place.binding.kind,
        ReferenceBindingKind::BorrowedReferent { parameter: 0 }
    );
    assert_ne!(&loan.origin_place.binding, binding);
    let child = borrow_at(&cfg, parameter_source, "&!(*parent)");
    let child_loan = evaluated_borrow::accepted_loan(&cfg, child.id);
    assert_eq!(child_loan.parent.as_deref(), Some(loan));
    assert!(
        matches!(&cfg.facts[sole_origin(child).0], ReferenceOrigin::BorrowedFrom { parameter: actual, loan: recorded } if actual == parameter && recorded == child_loan)
    );
    roundtrip(&cfg);
}

fn rejected_proof(layout: ScalarTargetLayout, project: bool) {
    let sources = [
        EXACT,
        "%%start\nunit() run = fn { u32 item = 1; u32 old = 2; *!u32 parent = &!item; *!u32 destination = &!old; destination = &!(*parent); *destination = 3; *parent = 4; };\nrun();\n%%end",
        "%%start\nunit() run = fn { u32 item = 1; u32 old = 2; *!u32 parent = &!item; *!u32 destination = &!old; *!(*!u32) slot = &!destination; *slot = &!(*parent); *destination = 3; *parent = 4; };\nrun();\n%%end",
        "%%start\n*!u32(*!u32) run = fn(parent) { &!(*parent) };\n%%end",
    ];
    for text in sources {
        let (mut cfg, capture_input) = evaluated_cfg(text, layout, project);
        let child = borrow_at(&cfg, text, "&!(*parent)").clone();
        let original_child = sole_origin(&child);
        let scalar = cfg
            .initial_points
            .iter()
            .find(|point| matches!(&point.kind, CfgPointKind::Entry))
            .unwrap()
            .id;
        let CfgPointKind::Borrow {
            request: ReferenceBorrowRequest::Through { pointer_point, .. },
        } = &mut cfg.initial_points[child.id.0].kind
        else {
            unreachable!()
        };
        *pointer_point = scalar;
        let errors = retransfer(&mut cfg, &capture_input);
        assert!(errors.iter().any(|error| matches!(error, ReferenceAnalysisError::MissingContext { source_span, reason: ReferenceAnalysisErrorReason::MissingRequiredDereferenceOrigin } if source_span == &child.source_span)));
        let observations = cfg
            .borrow_evaluations
            .iter()
            .filter(|evaluation| evaluation.point == child.id)
            .collect::<Vec<_>>();
        assert!(!observations.is_empty());
        assert!(observations
            .iter()
            .all(|evaluation| evaluation.cases.is_empty() && evaluation.output.is_empty()));
        assert_eq!(
            cfg.points[child.id.0].possible_origins,
            Some(BTreeSet::new())
        );
        assert!(cfg
            .checked_install_evaluations
            .iter()
            .all(|evaluation| !evaluation.accepted_origins.contains(&original_child)));
        for evaluation in cfg.checked_install_evaluations.iter().filter(|evaluation| matches!(evaluation.input, ReferenceCheckedInstallInput::Evaluated { value_point } if value_point == child.id)) {
            assert!(evaluation.accepted_origins.is_empty());
            assert!(matches!(evaluation.disposition, ReferenceCheckedInstallDisposition::Uninitialized | ReferenceCheckedInstallDisposition::Retained));
            if evaluation.disposition == ReferenceCheckedInstallDisposition::Retained {
                assert!(!evaluation.previous_origins.is_empty());
                assert!(cfg.points.iter().any(|point| matches!(&point.kind, CfgPointKind::Read { binding } if binding == &evaluation.target.binding) && point.possible_origins.as_ref() == Some(&evaluation.previous_origins)));
            }
        }
        assert!(cfg
            .points
            .iter()
            .filter(|point| matches!(point.kind, CfgPointKind::ReturnOutput { checked: true, .. }))
            .all(|point| !point
                .possible_origins
                .as_ref()
                .unwrap()
                .contains(&original_child)));
        assert_eq!(
            cfg.initial_points[child.id.0].kind,
            cfg.points[child.id.0].kind
        );
        roundtrip(&cfg);
    }
    let (mut cfg, _) = evaluated_cfg(EXACT, layout, project);
    let child = borrow_at(&cfg, EXACT, "&!(*parent)").clone();
    let ReferenceBorrowCase::Accepted {
        access:
            ReferenceBorrowAccess::Through {
                pointer_point,
                parent_origin,
            },
        ..
    } = evaluated_borrow::accepted_cases(&cfg, child.id)[0]
    else {
        unreachable!()
    };
    let input = ReferenceBorrowInput::Through {
        pointer_point: *pointer_point,
        origins: BTreeSet::from([*parent_origin]),
    };
    let mut foreign_fact = cfg.facts[parent_origin.0].clone();
    let ReferenceOrigin::Fresh { allocation, .. } = &mut foreign_fact else {
        unreachable!()
    };
    allocation.creation_span = child.source_span.range;
    let foreign = evaluated_borrow::intern(&mut cfg.facts, foreign_fact);
    let mut case = evaluated_borrow::accepted_cases(&cfg, child.id)[0].clone();
    let ReferenceBorrowCase::Accepted {
        access: ReferenceBorrowAccess::Through { parent_origin, .. },
        ..
    } = &mut case
    else {
        unreachable!()
    };
    *parent_origin = foreign;
    let CfgPointKind::Borrow { request } = &child.kind else {
        unreachable!()
    };
    assert!(!evaluated_borrow::validates(
        &cfg.facts,
        &child,
        cfg.points[0].source_span.range,
        request,
        &input,
        &case
    ));
    let mut case = evaluated_borrow::accepted_cases(&cfg, child.id)[0].clone();
    let ReferenceBorrowCase::Accepted { loan, .. } = &mut case else {
        unreachable!()
    };
    loan.parent.as_mut().unwrap().creation_span = child.source_span.range;
    assert!(!evaluated_borrow::validates(
        &cfg.facts,
        &child,
        cfg.points[0].source_span.range,
        request,
        &input,
        &case
    ));
}

fn indexed_and_stable(layout: ScalarTargetLayout, project: bool) {
    let text = "%%start\nunit(u64) run = fn(index) { u32[2] items = [1, 2]; *!(u32[2]) parent = &!items; *!u32 child = &!(*parent)[index]; *child = 3; (*parent)[0] = 4; };\n%%end";
    let (mut cfg, capture_input) = evaluated_cfg(text, layout, project);
    let child = borrow_at(&cfg, text, "&!(*parent)[index]");
    let CfgPointKind::Borrow {
        request:
            ReferenceBorrowRequest::Through {
                pointer_point,
                indices,
                ..
            },
    } = &child.kind
    else {
        unreachable!()
    };
    assert_eq!(indices.len(), 1);
    assert!(*pointer_point < indices[0].value_point);
    assert!(indices[0].value_point < child.id);
    assert!(
        matches!(&cfg.points[indices[0].value_point.0].kind, CfgPointKind::Read { binding } if binding.declaration_span.start == text.find("index)").unwrap() as u32)
    );
    assert!(cfg.borrow_evaluations.iter().filter(|evaluation| evaluation.point == child.id).all(|evaluation| matches!(&evaluation.input, ReferenceBorrowInput::Through { pointer_point: actual, origins } if actual == pointer_point && origins.len() == 1)));
    let facts = cfg.facts.clone();
    let map = cfg.checked_bindings.clone();
    let observations = cfg.borrow_evaluations.clone();
    let installs = cfg.checked_install_evaluations.clone();
    assert!(retransfer(&mut cfg, &capture_input).is_empty());
    assert_eq!(cfg.facts, facts);
    assert_eq!(cfg.checked_bindings, map);
    assert_eq!(cfg.borrow_evaluations, observations);
    assert_eq!(cfg.checked_install_evaluations, installs);
    roundtrip(&cfg);
    let stable = "%%start\nunit(bool) run = fn(flag) { u32 item = 1; *!u32 parent = &!item; while (flag) { *!u32 child = &!(*parent); *child = 2; } *parent = 3; };\n%%end";
    let (mut cfg, capture_input) = evaluated_cfg(stable, layout, project);
    let facts = cfg.facts.clone();
    let observations = cfg.borrow_evaluations.clone();
    let map = cfg.checked_bindings.clone();
    let installs = cfg.checked_install_evaluations.clone();
    let requests = cfg.initial_points.clone();
    assert_eq!(cfg, evaluated_cfg(stable, layout, project).0);
    assert!(retransfer(&mut cfg, &capture_input).is_empty());
    assert_eq!(cfg.facts, facts);
    assert_eq!(cfg.borrow_evaluations, observations);
    assert_eq!(cfg.checked_bindings, map);
    assert_eq!(cfg.checked_install_evaluations, installs);
    assert_eq!(cfg.initial_points, requests);
    roundtrip(&cfg);
}

fn invalid_capture_and_stale(layout: ScalarTargetLayout, project: bool) {
    let null =
        "%%start\nunit() run = fn { *u32 parent = null; *u32 child = &(*parent); child; };\n%%end";
    let (cfg, capture_input) = evaluated_cfg(null, layout, project);
    let child = borrow_at(&cfg, null, "&(*parent)");
    let evaluation = cfg
        .borrow_evaluations
        .iter()
        .find(|evaluation| evaluation.point == child.id)
        .unwrap();
    let ReferenceBorrowCase::Invalid {
        parent_origin,
        origin,
    } = evaluation.cases[0]
    else {
        unreachable!()
    };
    assert!(matches!(
        cfg.facts[parent_origin.0],
        ReferenceOrigin::Null { .. }
    ));
    assert!(
        matches!(cfg.facts[origin.0], ReferenceOrigin::Invalid { conflict_span, .. } if conflict_span == child.source_span.range)
    );
    assert_eq!(evaluation.output, BTreeSet::from([origin]));
    let mut invalid = cfg.clone();
    let null_span = match invalid.facts[parent_origin.0] {
        ReferenceOrigin::Null { span } => span,
        _ => unreachable!(),
    };
    invalid.facts[parent_origin.0] = ReferenceOrigin::Invalid {
        origin_span: null_span,
        conflict_span: null_span,
    };
    assert!(retransfer(&mut invalid, &capture_input).is_empty());
    assert!(invalid
        .borrow_evaluations
        .iter()
        .filter(|evaluation| evaluation.point == child.id)
        .all(|evaluation| matches!(evaluation.cases[0], ReferenceBorrowCase::Invalid { .. })));
    roundtrip(&invalid);
    let shared = "%%start\nunit() run = fn { u32 item = 1; *u32 parent = &item; *!u32 child = &!(*parent); child; };\n%%end";
    let (shared_cfg, diagnostics) = evaluated_result(shared, layout, project);
    let address = SourceSpan::new(source(), span_of(shared, "&!(*parent)"));
    assert!(
        diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "B0003"
                && diagnostic.message
                    == "checked reborrow requires a compatible checked reference"
                && diagnostic.labels.iter().any(|label| label.kind
                    == DiagnosticLabelKind::Primary
                    && label.span == address)),
        "{diagnostics:#?}"
    );
    assert!(shared_cfg.initial_points.is_empty());
    assert!(shared_cfg.checked_install_evaluations.is_empty());
    for (vector, (text, consumed)) in [
        ("%%start\nunit() run = fn { u32 item = 1; *!u32 parent = &!item; *!u32 child = &!(*parent); *!u32 moved = child; *(*!u32) slot = &child; slot; *moved = 2; };\n%%end", "child;"),
        ("%%start\nunit() run = fn { u32 item = 1; *!u32 parent = &!item; *!u32 child = &!(*parent); *!u32 moved = child; unsafe { &?child; }; *moved = 2; };\n%%end", "child;"),
        ("%%start\nunit() run = fn { u32 item = 1; *!u32 parent = &!item; *!u32 child = &!(*parent); *!u32 moved = child; *child = 2; *moved = 3; };\n%%end", "child;"),
        ("%%start\nunit() run = fn { u32 item = 1; *!u32 parent = &!item; *!u32 moved = parent; unsafe { &?parent; }; *moved = 2; };\n%%end", "parent;"),
        ("%%start\nunit() run = fn { u32 item = 1; *!u32 parent = &!item; *!u32 moved = parent; *(*!u32) slot = &parent; slot; *moved = 2; };\n%%end", "parent;"),
        ("%%start\nunit() run = fn { u32 item = 1; *!u32 parent = &!item; *!u32 moved = parent; *parent = 2; *moved = 3; };\n%%end", "parent;"),
    ].into_iter().enumerate() {
        let (cfg, diagnostics) = evaluated_result(text, layout, project);
        assert!(!diagnostics.is_empty());
        let consumption = SourceSpan::new(source(), span_of(text, consumed));
        assert!(diagnostics.iter().any(|diagnostic| diagnostic.code == "B0003" && diagnostic.labels.iter().any(|label| label.kind == DiagnosticLabelKind::Secondary && label.span.range.start == consumption.range.start)), "{text}\n{diagnostics:#?}");
        let consumed_name = if consumed == "child;" { "child" } else { "parent" };
        let secondary = SourceSpan::new(source(), ByteSpan::new(consumption.range.start, consumption.range.start + consumed_name.len() as u32));
        let use_text = ["&child", "&?child", "*child = 2", "&?parent", "&parent", "*parent = 2"][vector];
        let use_span = span_of(text, use_text);
        let primary = if [2, 5].contains(&vector) {
            SourceSpan::new(source(), ByteSpan::new(use_span.start + 1, use_span.start + 1 + consumed_name.len() as u32))
        } else {
            SourceSpan::new(source(), use_span)
        };
        assert!(diagnostics.iter().any(|diagnostic| diagnostic.code == "B0003" && diagnostic.message == "read or move of unavailable place" && diagnostic.labels == vec![
            crate::DiagnosticLabel { kind: DiagnosticLabelKind::Primary, span: primary.clone(), message: "read or move of unavailable place".to_owned() },
            crate::DiagnosticLabel { kind: DiagnosticLabelKind::Secondary, span: secondary.clone(), message: "origin of conflicting place or loan".to_owned() },
        ]), "{text}\n{diagnostics:#?}");
        let moved = cfg.checked_bindings.values().find(|place| matches!(&place.place, ScalarPlace::Name { name, .. } if name == "moved")).unwrap();
        assert!(cfg.checked_install_evaluations.iter().any(|evaluation| evaluation.target == *moved && evaluation.disposition == ReferenceCheckedInstallDisposition::Written && evaluation.accepted_origins.len() == 1));
        let installation = cfg.checked_install_evaluations.iter().find(|evaluation| evaluation.target == *moved).unwrap();
        let origin = *installation.accepted_origins.first().unwrap();
        let address = if consumed_name == "child" { "&!(*parent)" } else { "&!item" };
        let original = borrow_at(&cfg, text, address);
        assert_eq!(origin, sole_origin(original));
        let ReferenceOrigin::Fresh { loan, allocation } = &cfg.facts[origin.0] else { unreachable!() };
        assert_eq!(loan.creation_source, source());
        assert_eq!(loan.creation_span, original.source_span.range);
        assert_eq!(loan, evaluated_borrow::accepted_loan(&cfg, original.id));
        if consumed_name == "child" {
            let parent = borrow_at(&cfg, text, "&!item");
            let ReferenceOrigin::Fresh { allocation: parent_allocation, loan: parent_loan } = &cfg.facts[sole_origin(parent).0] else { unreachable!() };
            assert_eq!(loan.parent.as_deref(), Some(parent_loan));
            assert_eq!(allocation, parent_allocation);
        }
        assert!(cfg.points.iter().any(|point| matches!(&point.kind, CfgPointKind::Read { binding } if binding == &moved.binding) && point.id > installation.point && point.possible_origins.as_ref() == Some(&installation.accepted_origins)));
        assert!(cfg.borrow_evaluations.iter().filter(|evaluation| cfg.points[evaluation.point.0].source_span == primary).all(|evaluation| evaluation.output.is_empty()));
        roundtrip(&cfg);
    }
    let captured = "%%start\nunit() run = fn { u32 item = 1; *!u32 parent = &!item; unit() write = fn { *!u32 child = &!(*parent); *!u32 grandchild = &!(*child); *grandchild = 2; *child = 3; }; write(); *parent = 4; };\n%%end";
    let (cfg, _) = evaluated_cfg(captured, layout, project);
    let parent = borrow_at(&cfg, captured, "&!item");
    let parent_origin = sole_origin(parent);
    let nested = &cfg.literal_cfgs[0].reference_cfg;
    assert!(nested
        .inputs
        .iter()
        .any(|input| input.origins.contains(&parent_origin)));
    let child = borrow_at(nested, captured, "&!(*parent)");
    let grandchild = borrow_at(nested, captured, "&!(*child)");
    let nested_parent = match &nested.facts[sole_origin(child).0] {
        ReferenceOrigin::Fresh { allocation, loan } => {
            let ReferenceOrigin::Fresh {
                allocation: enclosing_allocation,
                ..
            } = &cfg.facts[parent_origin.0]
            else {
                unreachable!()
            };
            assert_eq!(allocation, enclosing_allocation);
            loan.parent.as_deref().unwrap()
        }
        _ => unreachable!(),
    };
    assert_eq!(
        nested_parent,
        evaluated_borrow::accepted_loan(&cfg, parent.id)
    );
    assert_eq!(
        evaluated_borrow::accepted_loan(nested, child.id)
            .parent
            .as_deref(),
        Some(evaluated_borrow::accepted_loan(&cfg, parent.id))
    );
    through(nested, grandchild, child);
    assert!(cfg.points.iter().filter(|point| matches!(&point.kind, CfgPointKind::Read { binding } if cfg.checked_bindings.get(binding).is_some_and(|place| matches!(&place.place, ScalarPlace::Name { name, .. } if name == "parent")))).all(|point| point.possible_origins == Some(BTreeSet::from([parent_origin]))));
    roundtrip(&cfg);
}

fn conflicts_and_greatest(layout: ScalarTargetLayout, project: bool) {
    let captured = "%%start\nunit() run = fn { u32 item = 1; *!u32 parent = &!item; *u32 sibling = &(*parent); unit() read = fn { *sibling; }; *!u32 child = &!(*parent); *child = 2; read(); };\n%%end";
    let (cfg, diagnostics) = evaluated_result(captured, layout, project);
    let child = borrow_at(&cfg, captured, "&!(*parent)");
    let sibling_span = SourceSpan::new(source(), span_of(captured, "&(*parent)"));
    assert!(
        diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "B0003"
                && diagnostic
                    .labels
                    .iter()
                    .any(|label| label.kind == DiagnosticLabelKind::Primary
                        && label.span == child.source_span)
                && diagnostic
                    .labels
                    .iter()
                    .any(|label| label.kind == DiagnosticLabelKind::Secondary
                        && label.span == sibling_span)),
        "{diagnostics:#?}"
    );
    assert!(cfg
        .borrow_evaluations
        .iter()
        .filter(|evaluation| evaluation.point == child.id)
        .all(|evaluation| evaluation.output.is_empty()
            && evaluation
                .cases
                .iter()
                .all(|case| matches!(case, ReferenceBorrowCase::Rejected { .. }))));
    assert!(cfg.checked_install_evaluations.iter().any(|evaluation| matches!(evaluation.input, ReferenceCheckedInstallInput::Evaluated { value_point } if value_point == child.id) && evaluation.disposition == ReferenceCheckedInstallDisposition::Uninitialized));
    let text = "%%start\nunit() run = fn { u32 item = 1; *!u32 parent = &!item; *!u32 first = &!(*parent); *first = 2; *!u32 second = &!(*parent); *second = 3; *parent = 4; };\nrun();\n%%end";
    let (mut cfg, capture_input) = evaluated_cfg(text, layout, project);
    let requests = cfg
        .initial_points
        .iter()
        .filter_map(|point| match &point.kind {
            CfgPointKind::Borrow {
                request: ReferenceBorrowRequest::Through { pointer_point, .. },
            } => Some((point.id, *pointer_point)),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(requests.len(), 2);
    let CfgPointKind::Borrow {
        request: ReferenceBorrowRequest::Through { pointer_point, .. },
    } = &mut cfg.initial_points[requests[1].0 .0].kind
    else {
        unreachable!()
    };
    *pointer_point = requests[0].1;
    assert!(retransfer(&mut cfg, &capture_input).is_empty());
    for (point, _) in requests {
        assert_eq!(
            cfg.points[point.0].possible_origins.as_ref().unwrap().len(),
            1
        );
        assert!(cfg
            .borrow_evaluations
            .iter()
            .filter(|evaluation| evaluation.point == point)
            .all(|evaluation| evaluation.output.len() == 1));
    }
    roundtrip(&cfg);
}

fn shadow_storage_proof(layout: ScalarTargetLayout, project: bool) {
    let shadow = "%%start\nunit() run = fn { u32 item = 1; *u32 selected = &item; unsafe { u32 inner = 2; *u32 selected = &inner; *selected; }; *selected; };\n%%end";
    let (cfg, diagnostics) = evaluated_result(shadow, layout, project);
    assert!(diagnostics
        .iter()
        .any(|diagnostic| diagnostic.code == "B0002"
            && diagnostic.labels.iter().any(|label| label.kind
                == super::super::DiagnosticLabelKind::Primary
                && label.span == SourceSpan::new(source(), ByteSpan::new(87, 110)))));
    assert!(cfg.initial_points.is_empty());
    assert!(cfg.points.is_empty());
    assert!(cfg.checked_bindings.is_empty());
    assert!(cfg.checked_installs.is_empty());
    assert!(cfg.checked_install_evaluations.is_empty());
    assert!(cfg.borrow_evaluations.is_empty());
    roundtrip(&cfg);
    for text in [
        "%%start\nunit() run = fn { u32 item = 1; unsafe { *u32 selected = &item; *selected; }; unsafe { *u32 selected = &item; *selected; }; };\n%%end",
        "%%start\nunit() run = fn { u32 item = 1; *u32 outer = &item; unsafe { u32 inner = 2; *u32 nested = &inner; *nested; }; *outer; };\n%%end",
    ] {
        let (cfg, _) = evaluated_cfg(text, layout, project);
        let slots = cfg.checked_bindings.values().collect::<Vec<_>>();
        assert_eq!(slots.len(), 2);
        assert_ne!(slots[0].binding, slots[1].binding);
        assert_ne!(slots[0].block_span, slots[1].block_span);
        for (binding, slot) in &cfg.checked_bindings {
            assert_eq!(binding, &slot.binding);
            assert_eq!(slot.source, source());
            assert_eq!(slot.function_span, binding.function_span);
            assert_eq!(slot.declaration_span, binding.declaration_span);
            assert_eq!(slot.block_span, binding.block_span);
            let install = cfg.checked_install_evaluations.iter().find(|evaluation| evaluation.target == *slot && evaluation.disposition == ReferenceCheckedInstallDisposition::Written).unwrap();
            assert_eq!(install.accepted_origins.len(), 1);
            let origin = *install.accepted_origins.first().unwrap();
            let ReferenceOrigin::Fresh { loan, allocation } = &cfg.facts[origin.0] else { unreachable!() };
            assert_eq!(loan.creation_source, source());
            assert_eq!(allocation.source, source());
            assert!(cfg.points.iter().any(|point| matches!(&point.kind, CfgPointKind::Read { binding: read } if read == binding) && point.possible_origins.as_ref() == Some(&install.accepted_origins)));
        }
        if let Some(outer) = slots.iter().find(|slot| matches!(&slot.place, ScalarPlace::Name { name, .. } if name == "outer")) {
            let install = cfg.checked_install_evaluations.iter().find(|evaluation| evaluation.target == **outer).unwrap();
            let later = cfg.points.iter().rfind(|point| matches!(&point.kind, CfgPointKind::Read { binding } if binding == &outer.binding)).unwrap();
            assert_eq!(later.possible_origins.as_ref(), Some(&install.accepted_origins));
            assert!(cfg.points.iter().any(|point| matches!(point.kind, CfgPointKind::ScopeExit) && point.id < later.id));
        } else {
            assert!(slots.iter().all(|slot| matches!(&slot.place, ScalarPlace::Name { name, span } if name == "selected" && *span == slot.declaration_span)));
        }
        roundtrip(&cfg);
    }
}

fn imported_scalar(layout: ScalarTargetLayout, project: bool) {
    if !project {
        let text = "%%start\n*u32(*u32) relay = fn(input) { input }; unit() run = fn { u32 item = 1; *u32 parent = relay(&item); *u32 child = &(*parent); *child; };\n%%end";
        let (cfg, _) = evaluated_cfg(text, layout, false);
        let child = borrow_at(&cfg, text, "&(*parent)");
        let cases = evaluated_borrow::accepted_cases(&cfg, child.id);
        let ReferenceBorrowCase::Accepted {
            loan,
            access:
                ReferenceBorrowAccess::Through {
                    pointer_point,
                    parent_origin,
                },
            ..
        } = cases[0]
        else {
            unreachable!()
        };
        assert_eq!(sole_origin(&cfg.points[pointer_point.0]), *parent_origin);
        let ReferenceOrigin::Fresh { loan: parent, .. } = &cfg.facts[parent_origin.0] else {
            unreachable!()
        };
        assert_eq!(loan.parent.as_deref(), Some(parent));
        roundtrip(&cfg);
        return;
    }
    let main_source = source();
    let library_source = SourceIdentity {
        path: "src/library.w".to_owned(),
        ..main_source.clone()
    };
    let text = "%%start\nlibrary = namespace app \"src/library.w\"; unit() run = fn { u32 item = 1; *u32 parent = library.relay(&item); *u32 child = &(*parent); *child; };\n%%end";
    let library_text = "%%start\n*u32(*u32) relay = fn(input) { input };\n%%end";
    let main = parse_source(main_source.clone(), text.to_owned(), &[]);
    let library = parse_source(library_source.clone(), library_text.to_owned(), &[]);
    assert!(main.diagnostics.is_empty());
    assert!(library.diagnostics.is_empty());
    let main = derive_scalar_program_with_layout(&main.result, layout);
    let library = derive_scalar_program_with_layout(&library.result, layout);
    let validation = validate_scalar_project(ScalarProject::new(
        vec![
            ScalarModule::from_program(
                main.program,
                vec![ScalarNamespaceBinding {
                    binding: "library".to_owned(),
                    target: library_source.clone(),
                    span: ByteSpan::new(8, 15),
                }],
            ),
            ScalarModule::from_program(library.program, Vec::new()),
        ],
        vec![main_source.clone()],
    ));
    assert!(
        validation.diagnostics.is_empty(),
        "{:#?}",
        validation.diagnostics
    );
    let cfg = validation.project.modules[0]
        .items
        .iter()
        .find_map(|item| match item {
            ScalarItem::Function(function) if function.name == "run" => {
                Some(&function.reference_cfg)
            }
            _ => None,
        })
        .unwrap();
    let child = borrow_at(cfg, text, "&(*parent)");
    let cases = evaluated_borrow::accepted_cases(cfg, child.id);
    let ReferenceBorrowCase::Accepted {
        loan,
        origin,
        access:
            ReferenceBorrowAccess::Through {
                pointer_point,
                parent_origin,
            },
        ..
    } = cases[0]
    else {
        unreachable!()
    };
    assert_eq!(sole_origin(&cfg.points[pointer_point.0]), *parent_origin);
    let ReferenceOrigin::Fresh {
        loan: parent,
        allocation,
    } = &cfg.facts[parent_origin.0]
    else {
        unreachable!()
    };
    assert_eq!(loan.parent.as_deref(), Some(parent));
    assert_eq!(loan.creation_source, main_source);
    assert_eq!(loan.creation_span, child.source_span.range);
    assert!(
        matches!(&cfg.facts[origin.0], ReferenceOrigin::Fresh { allocation: child_allocation, loan: child_loan } if child_allocation == allocation && child_loan == loan)
    );
    assert!(cfg.points.iter().any(|point| matches!(&point.kind, CfgPointKind::Call { target: ResolvedCallTarget::ModuleCallable { source, .. }, .. } if source == &library_source)));
    roundtrip(cfg);
}

macro_rules! connected_proofs {
    ($name:ident, $layout:expr, $project:expr) => {
        #[test]
        fn $name() {
            storage_proof($layout, $project);
            rejected_proof($layout, $project);
            indexed_and_stable($layout, $project);
            invalid_capture_and_stale($layout, $project);
            conflicts_and_greatest($layout, $project);
            imported_scalar($layout, $project);
        }
    };
}

macro_rules! shadow_proofs {
    ($name:ident, $layout:expr, $project:expr) => {
        #[test]
        fn $name() {
            shadow_storage_proof($layout, $project);
        }
    };
}

shadow_proofs!(
    d3_reborrow_state_evaluated_borrow_shadow_native_program,
    ScalarTargetLayout::NATIVE64,
    false
);
shadow_proofs!(
    d3_reborrow_state_evaluated_borrow_shadow_native_project,
    ScalarTargetLayout::NATIVE64,
    true
);
shadow_proofs!(
    d3_reborrow_state_evaluated_borrow_shadow_wasm_program,
    ScalarTargetLayout::WASM32,
    false
);
shadow_proofs!(
    d3_reborrow_state_evaluated_borrow_shadow_wasm_project,
    ScalarTargetLayout::WASM32,
    true
);

connected_proofs!(
    d3_reborrow_state_evaluated_borrow_connected_native_program,
    ScalarTargetLayout::NATIVE64,
    false
);
connected_proofs!(
    d3_reborrow_state_evaluated_borrow_connected_native_project,
    ScalarTargetLayout::NATIVE64,
    true
);
connected_proofs!(
    d3_reborrow_state_evaluated_borrow_connected_wasm_program,
    ScalarTargetLayout::WASM32,
    false
);
connected_proofs!(
    d3_reborrow_state_evaluated_borrow_connected_wasm_project,
    ScalarTargetLayout::WASM32,
    true
);

fn exact(layout: ScalarTargetLayout, project: bool) {
    let (cfg, _) = evaluated_cfg(EXACT, layout, project);
    let parent = borrow_at(&cfg, EXACT, "&!item");
    let child = borrow_at(&cfg, EXACT, "&!(*parent)");
    let cases = evaluated_borrow::accepted_cases(&cfg, parent.id);
    assert_eq!(cases.len(), 1);
    let ReferenceBorrowCase::Accepted { loan, access, .. } = cases[0] else {
        unreachable!()
    };
    assert_eq!(*access, ReferenceBorrowAccess::DirectStorage);
    assert!(loan.parent.is_none());
    assert_eq!(loan.creation_span, ByteSpan::new(55, 61));
    assert_eq!(loan.creation_source, source());
    assert_eq!(
        child.source_span,
        SourceSpan::new(source(), ByteSpan::new(77, 88))
    );
    through(&cfg, child, parent);
    let parent_reads = cfg
        .points
        .iter()
        .filter(|point| {
            matches!(&point.kind, CfgPointKind::Read { binding }
            if binding.declaration_span == ByteSpan::new(46, 52))
        })
        .collect::<Vec<_>>();
    assert_eq!(parent_reads.len(), 2);
    let CfgPointKind::Borrow {
        request: ReferenceBorrowRequest::Through { pointer_point, .. },
    } = &child.kind
    else {
        unreachable!()
    };
    assert_eq!(*pointer_point, parent_reads[0].id);
    for read in parent_reads {
        assert_eq!(sole_origin(read), sole_origin(parent));
    }
    roundtrip(&cfg);
}

fn grandchild(layout: ScalarTargetLayout, project: bool) {
    let text = "%%start\nunit() run = fn { u32 item = 1; *!u32 parent = &!item; *!u32 child = &!(*parent); *!u32 grandchild = &!(*child); *grandchild = 2; *child = 3; *parent = 4; };\nrun();\n%%end";
    let (cfg, _) = evaluated_cfg(text, layout, project);
    let parent = borrow_at(&cfg, text, "&!item");
    let child = borrow_at(&cfg, text, "&!(*parent)");
    let grandchild = borrow_at(&cfg, text, "&!(*child)");
    through(&cfg, child, parent);
    through(&cfg, grandchild, child);
    let expected = [grandchild, child, parent];
    let writes = cfg
        .points
        .iter()
        .filter(|point| matches!(point.kind, CfgPointKind::AssignThrough { .. }))
        .collect::<Vec<_>>();
    assert_eq!(writes.len(), 3);
    for (write, origin_point) in writes.iter().zip(expected) {
        let CfgPointKind::AssignThrough {
            candidates,
            previous_origins,
            previous_loans,
            pointer_point,
            ..
        } = &write.kind
        else {
            unreachable!()
        };
        assert!(previous_origins.is_empty());
        assert!(previous_loans.is_empty());
        assert_eq!(candidates.len(), 1);
        assert_eq!(candidates[0].origin, sole_origin(origin_point));
        let loan = evaluated_borrow::accepted_loan(&cfg, origin_point.id);
        assert_eq!(candidates[0].loan, *loan);
        assert_eq!(
            sole_origin(&cfg.points[pointer_point.0]),
            sole_origin(origin_point)
        );
    }
    roundtrip(&cfg);
}

fn repeated(layout: ScalarTargetLayout, project: bool) {
    let text = "%%start\nunit() run = fn { u32 item = 1; *!u32 parent = &!item; *!u32 first = &!(*parent); *first = 2; *!u32 second = &!(*parent); *second = 3; *parent = 4; };\nrun();\n%%end";
    let (cfg, _) = evaluated_cfg(text, layout, project);
    let parent = borrow_at(&cfg, text, "&!item");
    let children = cfg
        .points
        .iter()
        .filter(|point| {
            matches!(
                point.kind,
                CfgPointKind::Borrow {
                    request: ReferenceBorrowRequest::Through { .. },
                }
            )
        })
        .collect::<Vec<_>>();
    assert_eq!(children.len(), 2);
    for child in &children {
        through(&cfg, child, parent);
    }
    let producers = children
        .iter()
        .map(|child| {
            let CfgPointKind::Borrow {
                request: ReferenceBorrowRequest::Through { pointer_point, .. },
            } = &child.kind
            else {
                unreachable!()
            };
            *pointer_point
        })
        .collect::<Vec<_>>();
    assert_ne!(producers[0], producers[1]);
    assert!(producers[0] < children[0].id);
    assert!(children[0].id < producers[1]);
    roundtrip(&cfg);
}

fn joined(layout: ScalarTargetLayout, project: bool) {
    let text = "%%start\nunit(bool) run = fn(flag) { u32 left = 1; u32 right = 2; *!u32 first = &!left; *!u32 second = &!right; *!u32 parent = if (flag) { first } else { second }; *!u32 child = &!(*parent); *child = 3; };\nrun(true);\n%%end";
    let (cfg, _) = evaluated_cfg(text, layout, project);
    let first = borrow_at(&cfg, text, "&!left");
    let second = borrow_at(&cfg, text, "&!right");
    let children = cfg
        .points
        .iter()
        .filter(|point| {
            matches!(
                point.kind,
                CfgPointKind::Borrow {
                    request: ReferenceBorrowRequest::Through { .. },
                }
            )
        })
        .collect::<Vec<_>>();
    assert_eq!(children.len(), 1);
    let child = children[0];
    assert_eq!(
        child.source_span,
        SourceSpan::new(source(), ByteSpan::new(177, 188))
    );
    assert_eq!(cfg.initial_points[child.id.0].kind, child.kind);
    let cases = evaluated_borrow::accepted_cases(&cfg, child.id);
    assert_eq!(cases.len(), 2);
    let parents = cases
        .iter()
        .map(|case| {
            let ReferenceBorrowCase::Accepted {
                loan,
                access:
                    ReferenceBorrowAccess::Through {
                        pointer_point,
                        parent_origin,
                    },
                origin,
                ..
            } = case
            else {
                unreachable!()
            };
            let pointer = &cfg.points[pointer_point.0];
            assert!(pointer
                .possible_origins
                .as_ref()
                .unwrap()
                .contains(parent_origin));
            let parent_loan = match &cfg.facts[parent_origin.0] {
                ReferenceOrigin::Fresh { loan, .. } => loan,
                _ => panic!("evaluated selected parent"),
            };
            assert_eq!(loan.parent.as_deref(), Some(parent_loan));
            assert_eq!(loan.creation_source, source());
            assert_eq!(loan.creation_span, ByteSpan::new(177, 188));
            let ReferenceOrigin::Fresh {
                allocation: parent_allocation,
                ..
            } = &cfg.facts[parent_origin.0]
            else {
                unreachable!()
            };
            let ReferenceOrigin::Fresh {
                allocation: child_allocation,
                loan: child_loan,
            } = &cfg.facts[origin.0]
            else {
                unreachable!()
            };
            assert_eq!(parent_allocation, child_allocation);
            assert_eq!(child_loan, loan);
            *parent_origin
        })
        .collect::<BTreeSet<_>>();
    assert_eq!(
        parents,
        BTreeSet::from([sole_origin(first), sole_origin(second)])
    );
    let evaluations = cfg
        .borrow_evaluations
        .iter()
        .filter(|evaluation| evaluation.point == child.id)
        .collect::<Vec<_>>();
    assert_eq!(evaluations.len(), 2);
    for evaluation in evaluations {
        assert_eq!(evaluation.cases.len(), 1);
        assert_eq!(evaluation.output.len(), 1);
        let ReferenceBorrowInput::Through { origins, .. } = &evaluation.input else {
            unreachable!()
        };
        assert_eq!(origins.len(), 1);
        let ReferenceBorrowCase::Accepted {
            origin,
            access: ReferenceBorrowAccess::Through { parent_origin, .. },
            ..
        } = &evaluation.cases[0]
        else {
            unreachable!()
        };
        assert_eq!(origins, &BTreeSet::from([*parent_origin]));
        assert_eq!(evaluation.output, BTreeSet::from([*origin]));
        let installs = cfg.checked_install_evaluations.iter().filter(|install| matches!(install.input, ReferenceCheckedInstallInput::Evaluated { value_point } if value_point == child.id)).collect::<Vec<_>>();
        assert_eq!(installs.len(), 2);
        let install = installs
            .iter()
            .find(|install| install.accepted_origins == evaluation.output)
            .unwrap();
        assert_eq!(
            install.disposition,
            ReferenceCheckedInstallDisposition::Written
        );
        assert!(install.previous_origins.is_empty());
        assert!(matches!(&install.target.place, ScalarPlace::Name { name, .. } if name == "child"));
        assert!(parents.contains(parent_origin));
        assert!(cases.iter().any(|case| matches!(case, ReferenceBorrowCase::Accepted { origin: other, access: ReferenceBorrowAccess::Through { parent_origin: other_parent, .. }, .. } if other_parent != parent_origin && !origins.contains(other_parent) && !evaluation.output.contains(other))));
    }
    roundtrip(&cfg);
}

#[test]
fn d3_reborrow_state_evaluated_borrow_exact_native_program() {
    exact(ScalarTargetLayout::NATIVE64, false);
}
#[test]
fn d3_reborrow_state_evaluated_borrow_exact_native_project() {
    exact(ScalarTargetLayout::NATIVE64, true);
}
#[test]
fn d3_reborrow_state_evaluated_borrow_exact_wasm_program() {
    exact(ScalarTargetLayout::WASM32, false);
}
#[test]
fn d3_reborrow_state_evaluated_borrow_exact_wasm_project() {
    exact(ScalarTargetLayout::WASM32, true);
}
#[test]
fn d3_reborrow_state_evaluated_borrow_grandchild_native_program() {
    grandchild(ScalarTargetLayout::NATIVE64, false);
}
#[test]
fn d3_reborrow_state_evaluated_borrow_grandchild_native_project() {
    grandchild(ScalarTargetLayout::NATIVE64, true);
}
#[test]
fn d3_reborrow_state_evaluated_borrow_grandchild_wasm_program() {
    grandchild(ScalarTargetLayout::WASM32, false);
}
#[test]
fn d3_reborrow_state_evaluated_borrow_grandchild_wasm_project() {
    grandchild(ScalarTargetLayout::WASM32, true);
}
#[test]
fn d3_reborrow_state_evaluated_borrow_repeated_native_program() {
    repeated(ScalarTargetLayout::NATIVE64, false);
}
#[test]
fn d3_reborrow_state_evaluated_borrow_repeated_native_project() {
    repeated(ScalarTargetLayout::NATIVE64, true);
}
#[test]
fn d3_reborrow_state_evaluated_borrow_repeated_wasm_program() {
    repeated(ScalarTargetLayout::WASM32, false);
}
#[test]
fn d3_reborrow_state_evaluated_borrow_repeated_wasm_project() {
    repeated(ScalarTargetLayout::WASM32, true);
}
#[test]
fn d3_reborrow_state_evaluated_borrow_joined_native_program() {
    joined(ScalarTargetLayout::NATIVE64, false);
}
#[test]
fn d3_reborrow_state_evaluated_borrow_joined_native_project() {
    joined(ScalarTargetLayout::NATIVE64, true);
}
#[test]
fn d3_reborrow_state_evaluated_borrow_joined_wasm_program() {
    joined(ScalarTargetLayout::WASM32, false);
}
#[test]
fn d3_reborrow_state_evaluated_borrow_joined_wasm_project() {
    joined(ScalarTargetLayout::WASM32, true);
}

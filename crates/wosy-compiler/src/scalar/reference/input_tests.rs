use super::*;
use crate::parse_source;

#[cfg(test)]
mod project_origins;

mod output_contracts;

mod specialization;

mod call_resolution;

mod call_projection;

mod binding_outputs;

mod block_outputs;

mod assignment_outputs;

mod assignment_destinations;

mod direct_destinations;

mod dereference_points;

mod projected_reads;

mod address_creation;

mod core_provenance;

mod initializer_identities;

mod complete_results;

mod callable_boundary;

mod plain_names;

mod lexical_members;

#[test]
fn origin_index_preserves_first_ids_and_exact_identity_within_span_buckets() {
    let span = ByteSpan::new(3, 4);
    let source = SourceIdentity::new("project".into(), "package".into(), "src/main.w".into(), "r1".into());
    let binding = ReferenceBindingId {
        source: source.clone(),
        function_span: span,
        declaration_span: span,
        block_span: span,
        kind: ReferenceBindingKind::Declared,
    };
    let borrowed = |mode| ReferenceOrigin::BorrowedFrom {
        parameter: 0,
        loan: ReferenceLoanId {
            mode,
            origin_place: ReferencePlaceId {
                source: source.clone(),
                function_span: span,
                declaration_span: span,
                block_span: span,
                binding: binding.clone(),
                projections: Vec::new(),
                place: ScalarPlace::Name { name: "slot".into(), span },
            },
            creation_span: span,
            parent: None,
        },
    };
    let shared = borrowed(ScalarReferenceMutability::Shared);
    let mutable = borrowed(ScalarReferenceMutability::Mutable);
    let null = ReferenceOrigin::Null { span };
    let invalid = ReferenceOrigin::Invalid { origin_span: span, conflict_span: span };
    let mut facts = Vec::new();
    let mut index = ReferenceOriginIndex::default();
    assert_eq!(index.intern(&mut facts, shared.clone()), ReferenceOriginId(0));
    assert_eq!(index.intern(&mut facts, shared.clone()), ReferenceOriginId(0));
    assert_eq!(index.intern(&mut facts, null.clone()), ReferenceOriginId(1));
    assert_eq!(index.intern(&mut facts, mutable.clone()), ReferenceOriginId(2));
    assert_eq!(index.intern(&mut facts, shared.clone()), ReferenceOriginId(0));
    assert_eq!(index.intern(&mut facts, mutable.clone()), ReferenceOriginId(2));
    assert_eq!(index.intern(&mut facts, invalid.clone()), ReferenceOriginId(3));
    assert_eq!(index.intern(&mut facts, invalid.clone()), ReferenceOriginId(3));
    assert_eq!(facts, vec![shared, null, mutable, invalid]);
}

fn assert_project_reference_results(
    left: &ProjectReferenceResults,
    right: &ProjectReferenceResults,
) {
    fn assert_function(left: &FunctionReferenceResult, right: &FunctionReferenceResult) {
        assert_eq!(left.cfg, right.cfg);
        assert_eq!(left.arms.len(), right.arms.len());
        for (left, right) in left.arms.iter().zip(&right.arms) {
            assert_function(left, right);
        }
    }
    assert_eq!(left.errors, right.errors);
    assert_eq!(left.updates.len(), right.updates.len());
    for ((left_module, left), (right_module, right)) in left.updates.iter().zip(&right.updates) {
        assert_eq!(left_module, right_module);
        assert_eq!(left.len(), right.len());
        for ((left_item, left), (right_item, right)) in left.iter().zip(right) {
            assert_eq!(left_item, right_item);
            assert_function(left, right);
        }
    }
}

const TEXT: &str = "%%start\nstruct copy Point { u8 value; }\nenum Marker { ready; }\npick = overload { u8(u8) => fn(input) { u8 next = input; next }; u32(u32) => fn(input) { u32 next = input; next }; };\nu8(Point) read = fn(input) { next = input; pick(next.value) };\n%%end";

fn source() -> SourceIdentity {
    SourceIdentity::new(
        "project".into(),
        "package".into(),
        "inputs.w".into(),
        "r1".into(),
    )
}

#[test]
fn real_constructor_borrows_nonempty_inventories_and_prepared_call_rows_by_identity() {
    let parsed = parse_source(source(), TEXT.into(), &[]);
    assert!(parsed.diagnostics.is_empty(), "{:?}", parsed.diagnostics);
    let extracted = derive_scalar_declarations_from_cst(parsed.result.canonical_cst());
    let (typed, occurrences) =
        typecheck_scalar_program_with_occurrences(extracted, ScalarTargetLayout::WASM32);
    assert!(typed.diagnostics.is_empty(), "{:?}", typed.diagnostics);
    let program = typed.program;
    let call_sites = resolve_program_call_sites_with_occurrences(&program, occurrences).unwrap();
    let module_names = HashSet::from(["pick".to_owned(), "read".to_owned()]);
    let ScalarItem::Function(function) = &program.items[1] else {
        panic!("read")
    };
    let call_sites = &call_sites.functions[&1];
    assert!(!call_sites.sites.is_empty());
    assert!(!program.structs.is_empty());
    assert!(!program.enums.is_empty());
    assert!(!program.resolved_assignment_outputs.is_empty());
    let scope = layout::observations::Scope::new();
    let builder = ReferenceFlowBuilder::new(
        &program.source,
        function,
        &module_names,
        call_sites,
        &program.structs,
        &program.enums,
    );
    assert!(std::ptr::eq(builder.module_names, &module_names));
    assert!(std::ptr::eq(builder.call_sites, call_sites));
    assert!(std::ptr::eq(builder.structs, program.structs.as_slice()));
    assert!(std::ptr::eq(builder.enums, program.enums.as_slice()));
    assert_eq!(scope.counts(), layout::observations::Counts::default());
}

#[test]
fn actual_function_and_arm_consumers_copy_zero_aggregates_and_match_public_cfgs() {
    for target in [ScalarTargetLayout::WASM32, ScalarTargetLayout::NATIVE64] {
        let parsed = parse_source(source(), TEXT.into(), &[]);
        assert!(parsed.diagnostics.is_empty(), "{:?}", parsed.diagnostics);
        let extracted = derive_scalar_declarations_from_cst(parsed.result.canonical_cst());
        let (typed, occurrences) = typecheck_scalar_program_with_occurrences(extracted, target);
        assert!(typed.diagnostics.is_empty(), "{:?}", typed.diagnostics);
        let mut program = typed.program;
        let call_sites =
            resolve_program_call_sites_with_occurrences(&program, occurrences).unwrap();
        let module_names = HashSet::from(["pick".to_owned(), "read".to_owned()]);
        let public = derive_scalar_program_with_layout(&parsed.result, target);
        assert_eq!(public.diagnostics, typed.diagnostics);
        assert_eq!(program.items.len(), 2);
        for (index, (item, expected)) in program
            .items
            .iter_mut()
            .zip(&public.program.items)
            .enumerate()
        {
            let (ScalarItem::Function(function), ScalarItem::Function(expected)) = (item, expected)
            else {
                panic!("function")
            };
            let scope = layout::observations::Scope::new();
            let errors = record_function_reference_origins(
                &program.source,
                function,
                &module_names,
                &call_sites.functions[&index],
                &program.structs,
                &program.enums,
            );
            assert_eq!(scope.counts(), layout::observations::Counts::default());
            drop(scope);
            assert!(errors.is_empty(), "{errors:?}");
            assert_eq!(function, expected);
            if function.name == "pick" {
                assert_eq!(function.overload_arms.len(), 2);
                for arm in &function.overload_arms {
                    assert!(!arm.reference_cfg.points.is_empty());
                    assert!(!arm.reference_cfg.initial_points.is_empty());
                }
            } else {
                assert!(function
                    .reference_cfg
                    .points
                    .iter()
                    .any(|point| matches!(point.kind, CfgPointKind::Call { .. })));
            }
        }
        assert_eq!(program, public.program);
    }
}

#[test]
fn immutable_producer_preserves_preexisting_cfg_and_entire_input_on_both_targets() {
    for target in [ScalarTargetLayout::WASM32, ScalarTargetLayout::NATIVE64] {
        let parsed = parse_source(source(), TEXT.into(), &[]);
        assert!(parsed.diagnostics.is_empty(), "{:?}", parsed.diagnostics);
        let (typed, occurrences) = typecheck_scalar_program_with_occurrences(
            derive_scalar_declarations_from_cst(parsed.result.canonical_cst()),
            target,
        );
        assert!(typed.diagnostics.is_empty(), "{:?}", typed.diagnostics);
        let mut program = typed.program;
        let call_sites =
            resolve_program_call_sites_with_occurrences(&program, occurrences).unwrap();
        let results = analyze_program_references(&program, &call_sites);
        assert!(results.install(&mut program).is_empty());
        let before = program.clone();
        let module_names = HashSet::from(["pick".to_owned(), "read".to_owned()]);
        for (index, item) in program.items.iter().enumerate() {
            let ScalarItem::Function(function) = item else {
                panic!("function")
            };
            let calls = &call_sites.functions[&index];
            for (callable, calls) in std::iter::once((function, calls))
                .chain(function.overload_arms.iter().zip(&calls.arms))
            {
                assert!(!callable.reference_cfg.points.is_empty());
                assert!(!callable.reference_cfg.initial_points.is_empty());
                let scope = layout::observations::Scope::new();
                let (cfg, errors) = derive_function_reference_cfg(
                    &program.source,
                    callable,
                    &module_names,
                    calls,
                    &program.structs,
                    &program.enums,
                    true,
                );
                assert_eq!(scope.counts(), layout::observations::Counts::default());
                drop(scope);
                assert!(errors.is_empty(), "{errors:?}");
                assert_eq!(cfg, callable.reference_cfg);
                assert_eq!(program, before);
            }
        }
    }
}

#[test]
fn child_errors_keep_source_order_partial_cfgs_and_suppress_parent_initial_transfer() {
    let text = "%%start\npick = overload { u8(u8) => fn(input) { next = input; next }; u32(u32) => fn(input) { next = input; next }; };\n%%end";
    for target in [ScalarTargetLayout::WASM32, ScalarTargetLayout::NATIVE64] {
        let parsed = parse_source(source(), text.into(), &[]);
        assert!(parsed.diagnostics.is_empty(), "{:?}", parsed.diagnostics);
        let extracted = derive_scalar_declarations_from_cst(parsed.result.canonical_cst());
        let typed = typecheck_scalar_program(extracted, target);
        assert!(typed.diagnostics.is_empty(), "{:?}", typed.diagnostics);
        let mut program = typed.program;
        program.resolved_assignment_outputs = ScalarAssignmentTyping::default();
        let calls = resolve_program_call_sites(&program).unwrap();
        let ScalarItem::Function(parent) = &program.items[0] else {
            panic!("overload")
        };
        assert_eq!(parent.overload_arms.len(), 2);
        let before = parent.clone();
        let module_names = HashSet::from(["pick".to_owned()]);
        let expected_errors = parent
            .overload_arms
            .iter()
            .map(|arm| {
                let ScalarBlockItem::Assignment(assignment) = &arm.body.items[0] else {
                    panic!("assignment")
                };
                ReferenceAnalysisError::MissingContext {
                    source_span: SourceSpan::new(source(), assignment.targets[0].target_span),
                    reason: ReferenceAnalysisErrorReason::MissingResolvedBinding,
                }
            })
            .collect::<Vec<_>>();
        let mut child_cfgs = Vec::new();
        for (position, (arm, expected)) in parent
            .overload_arms
            .iter()
            .zip(&expected_errors)
            .enumerate()
        {
            let (cfg, errors) = derive_function_reference_cfg(
                &program.source,
                arm,
                &module_names,
                &calls.functions[&0].arms[position],
                &program.structs,
                &program.enums,
                true,
            );
            assert_eq!(errors, vec![expected.clone()]);
            assert!(!cfg.points.is_empty());
            assert!(matches!(cfg.points[0].kind, CfgPointKind::Entry));
            assert!(cfg.initial_points.is_empty());
            child_cfgs.push(cfg);
        }
        let (gated_parent, errors) = derive_function_reference_cfg(
            &program.source,
            parent,
            &module_names,
            &calls.functions[&0],
            &program.structs,
            &program.enums,
            false,
        );
        assert!(errors.is_empty(), "{errors:?}");
        assert!(!gated_parent.points.is_empty());
        assert!(gated_parent.initial_points.is_empty());
        let (successful_parent, errors) = derive_function_reference_cfg(
            &program.source,
            parent,
            &module_names,
            &calls.functions[&0],
            &program.structs,
            &program.enums,
            true,
        );
        assert!(errors.is_empty(), "{errors:?}");
        assert!(!successful_parent.initial_points.is_empty());
        assert_eq!(parent, &before);
        let mut installed = parent.clone();
        installed.reference_cfg = successful_parent.clone();
        for arm in &mut installed.overload_arms {
            arm.reference_cfg = successful_parent.clone();
        }
        let errors = record_function_reference_origins(
            &program.source,
            &mut installed,
            &module_names,
            &calls.functions[&0],
            &program.structs,
            &program.enums,
        );
        assert_eq!(errors, expected_errors);
        assert_eq!(installed.reference_cfg, gated_parent);
        assert!(installed.reference_cfg.initial_points.is_empty());
        let mut expected = before;
        expected.reference_cfg = gated_parent;
        for (arm, cfg) in expected.overload_arms.iter_mut().zip(child_cfgs) {
            arm.reference_cfg = cfg;
        }
        assert_eq!(installed, expected);
    }
}

#[test]
fn projected_reborrow_authority_requires_a_canonical_live_parent_on_each_edge() {
    let text = r#"%%start
struct copy Pair { u8 left; u8 right; }
unit(Pair) inspect = fn(owner) {
    *Pair parent = &owner;
    *Pair child = &(*parent);
    Pair observed = *child;
    observed.left;
};
%%end"#;
    let parsed = parse_source(source(), text.into(), &[]);
    assert!(parsed.diagnostics.is_empty(), "{:?}", parsed.diagnostics);
    let validated = crate::derive_scalar_program(&parsed.result);
    assert!(validated.diagnostics.is_empty(), "{:?}", validated.diagnostics);
    let ScalarItem::Function(function) = &validated.program.items[0] else { panic!("typed callable") };
    let cfg = &function.reference_cfg;
    let (origin, parent) = cfg.facts.iter().enumerate().find_map(|(index, fact)| match fact {
        ReferenceOrigin::Fresh { loan, .. } if loan.parent.is_none() => Some((ReferenceOriginId(index), loan)),
        _ => None,
    }).expect("ordinary canonical whole owner borrow");
    let read = cfg.initial_points.iter().find(|point| {
        matches!(point.kind, CfgPointKind::Read { .. })
            && point.source_span.range.start == text.rfind("parent)").unwrap() as u32
    }).expect("ordinary canonical parent read");
    let CfgPointKind::Read { binding } = &read.kind else { unreachable!() };
    let borrow = cfg.initial_points.iter().find(|point| matches!(&point.kind,
        CfgPointKind::Borrow { loan, .. } if loan.parent.as_deref() == Some(parent)))
        .expect("ordinary child borrow to use as a transfer topology");
    let holder = cfg.initial_points.iter().find(|point| point.id > borrow.id
        && matches!(point.kind, CfgPointKind::Bind { .. })).expect("ordinary child holder");
    let child_read = cfg.initial_points.iter().find(|point| {
        matches!(&point.kind, CfgPointKind::ReadPlace { place: ScalarPlace::Dereference { .. }, .. })
            && point.source_span.range.start == text.rfind("*child;").unwrap() as u32
    }).expect("ordinary direct checked child read");
    let child_id = *borrow.possible_origins.as_ref().and_then(|origins| origins.first())
        .expect("ordinary child fact");
    let owner = &parent.origin_place;
    let span = read.source_span.range;
    let field = ScalarFieldReference::Resolved(ScalarStructFieldId {
        structure: validated.program.structs[0].id.clone(),
        index: 0,
    });
    let mut child = parent.clone();
    child.parent = Some(Box::new(parent.clone()));
    child.origin_place.projections = vec![
        ReferencePlaceProjection::Dereference(Box::new(parent.clone())),
        ReferencePlaceProjection::Field(field.clone()),
    ];
    child.origin_place.place = ScalarPlace::Field {
        base: Box::new(ScalarPlace::Dereference {
            pointer: Box::new(ScalarExpression::Name { name: "parent".into(), span }),
            span,
        }),
        field,
        span,
    };
    let authority = super::super::control::NamedReborrowParent {
        pointer: read.id,
        borrow: borrow.id,
        origin,
    };
    let origins = BTreeSet::from([origin]);
    let mut present = ReferenceEdgeState::default();
    present.bindings.insert(binding.clone(), origins.clone());
    present.point_values = ReferencePointValues(vec![(read.id, origins)]);
    let allowed = |state: &ReferenceEdgeState, child: &ReferenceLoanId| {
        exact_projected_reborrow_parent_on_edge(state, &cfg.initial_points, &cfg.facts, &authority, child)
    };
    assert!(allowed(&present, &child));
    let absent = ReferenceEdgeState { point_values: ReferencePointValues::default(), ..present.clone() };
    let mut ambiguous = present.clone();
    ambiguous.point_values.0[0].1.insert(ReferenceOriginId(cfg.facts.len()));
    let mut replaced = present.clone();
    replaced.bindings.insert(binding.clone(), BTreeSet::from([ReferenceOriginId(cfg.facts.len())]));
    let mut released = present.clone();
    released.released.insert(owner.binding.clone());
    let mut mismatched = child.clone();
    mismatched.origin_place.binding = binding.clone();
    let mut unproven_field = child.clone();
    unproven_field.origin_place.projections.pop();
    assert!(!allowed(&absent, &child));
    assert!(!allowed(&ambiguous, &child));
    assert!(!allowed(&replaced, &child));
    assert!(!allowed(&released, &child));
    assert!(!allowed(&present, &mismatched));
    assert!(!allowed(&present, &unproven_field));

    let mut projected = cfg.clone();
    projected.initial_points[borrow.id.0].kind = CfgPointKind::Borrow {
        place: child.origin_place.clone(),
        loan: child.clone(),
    };
    let ReferenceOrigin::Fresh { loan, .. } = &mut projected.facts[child_id.0] else {
        panic!("actual whole reborrow fact");
    };
    *loan = child;
    projected.points = projected.initial_points.clone();
    for proven in [true, false] {
        let mut cfg = projected.clone();
        let mut proof = super::super::control::CallablePointFacts::default();
        proof.checked_binding_initializers.push((holder.id, borrow.id));
        proof.checked_pointee_reads.push(child_read.id);
        if proven {
            proof.named_reborrow_parents.push(super::super::control::NamedReborrowParent {
                borrow: borrow.id,
                pointer: read.id,
                origin,
            });
        }
        let errors = transfer_reference_facts(
            ReferenceTransferCfg::new(&mut cfg, Some(&proof)),
            &BTreeMap::new(), &HashMap::new(), None, None,
        );
        if proven {
            assert!(errors.is_empty(), "{errors:?}");
            for id in [borrow.id, holder.id] {
                assert!(cfg.points[id.0].possible_origins.as_ref().is_some_and(|set| {
                    set.len() == 1 && set.contains(&child_id)
                }), "{:?}", cfg.points[id.0]);
            }
        } else {
            assert!(errors.iter().any(|error| matches!(error,
                ReferenceAnalysisError::MissingContext { source_span,
                    reason: ReferenceAnalysisErrorReason::MissingRequiredDereferenceOrigin }
                    if source_span == &borrow.source_span)), "{errors:?}");
            for id in [borrow.id, holder.id] {
                assert!(cfg.points[id.0].possible_origins.as_ref().is_none_or(BTreeSet::is_empty),
                    "{:?}", cfg.points[id.0]);
            }
        }
    }
}

#[test]
fn projected_reborrow_authority_does_not_enable_mutable_source_syntax() {
    let text = r#"%%start
struct copy Pair { u8 left; u8 right; }
u8(Pair) inspect = fn(owner) {
    *!Pair parent = &!owner;
    *!u8 child = &!(*parent).left;
    *child
};
%%end"#;
    let parsed = parse_source(source(), text.into(), &[]);
    assert!(parsed.diagnostics.is_empty(), "{:?}", parsed.diagnostics);
    let validated = crate::derive_scalar_program(&parsed.result);
    assert!(validated.diagnostics.iter().any(|diagnostic| diagnostic.code == "B0003"
        && diagnostic.message == "checked address requires a storage name or direct storage field"),
        "{:?}", validated.diagnostics);
}

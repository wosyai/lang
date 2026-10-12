use super::*;

fn source_rows(
    layout: ScalarTargetLayout,
    project: bool,
) -> (crate::scalar::ScalarReferenceCfg, ScalarAssignment) {
    let source = wosy_syntax::SourceIdentity::new(
        "saved-length-proof".into(),
        "app".into(),
        "src/saved_length.w".into(),
        "r1".into(),
    );
    let parsed = crate::parse_source(source.clone(), "%%start\nu8[]() inspect = fn { u64 old_count = 2; u64 next_count = 5; u8[old_count] items; u8[next_count] replacement; items = replacement };\ninspect();\n%%end\n".to_owned(), &[]);
    assert!(parsed.diagnostics.is_empty(), "{:?}", parsed.diagnostics);
    let validation = crate::derive_scalar_program_with_layout(&parsed.result, layout);
    assert!(
        validation.diagnostics.is_empty(),
        "{:?}",
        validation.diagnostics
    );
    let items = if project {
        let validation = crate::validate_scalar_project(crate::ScalarProject::new(
            vec![crate::ScalarModule::from_program(
                validation.program,
                Vec::new(),
            )],
            vec![source],
        ));
        assert!(
            validation.diagnostics.is_empty(),
            "{:?}",
            validation.diagnostics
        );
        validation.project.modules[0].items.clone()
    } else {
        validation.program.items
    };
    let function = items
        .iter()
        .find_map(|item| match item {
            ScalarItem::Function(function) if function.name == "inspect" => Some(function),
            _ => None,
        })
        .expect("actual source function");
    let ScalarExpression::Assignment(assignment) = &function.body.final_output_values[0].value
    else {
        panic!("actual assignment");
    };
    (function.reference_cfg.clone(), assignment.as_ref().clone())
}

enum Proof {
    Lifecycle,
    Duplicate,
    DuplicateBatch,
    Missing,
    WrongId,
    WrongShape,
    WrongWidth,
    UnsupportedThrough,
}

fn proof(layout: ScalarTargetLayout, project: bool, proof: Proof) {
    let (cfg, assignment) = source_rows(layout, project);
    let context = Context::create();
    let builder = context.create_builder();
    let functions = BTreeMap::new();
    let call_targets = BTreeMap::new();
    let signatures = BTreeMap::new();
    let mut dynamic_owner_outputs = BTreeMap::new();
    dynamic_owner_outputs.insert("consume".to_owned(), BTreeSet::new());
    let literal_signatures = std::collections::HashMap::new();
    let literal_captures = std::collections::HashMap::new();
    let literal_reference_cfgs = std::collections::HashMap::new();
    let module = context.create_module("saved_length");
    let function =
        module.add_function("saved_length", context.i64_type().fn_type(&[], false), None);
    builder.position_at_end(context.append_basic_block(function, "entry"));
    let mut state = EmitState {
        builder: &builder,
        module: &module,
        functions: &functions,
        call_targets: &call_targets,
        signatures: &signatures,
        dynamic_owner_outputs: &dynamic_owner_outputs,
        literal_signatures: &literal_signatures,
        literal_captures: &literal_captures,
        literal_reference_cfgs: &literal_reference_cfgs,
        covered_literals: std::collections::HashSet::new(),
        cyclic_literals: Vec::new(),
        counted_fixed_cells: BTreeSet::new(),
        top_level_remainder_cells: Vec::new(),
        counted_cell_backings: Vec::new(),
        resolved_calls: BTreeMap::new(),
        source: assignment.occurrence.source_span.source.clone(),
        capture_cells: BTreeMap::new(),
        backing: validated_backing_maps(&cfg),
        holder_events: holder_events::HolderEvents::new(&cfg).expect("actual events"),
        escaping: BTreeMap::new(),
        backing_values: BTreeMap::new(),
        backing_slots: BTreeMap::new(),
        imported_cell_slots: BTreeMap::new(),
        backing_captured: BTreeSet::new(),
        values: BTreeMap::new(),
        storage: BTreeMap::new(),
        fresh_return_outputs: BTreeSet::new(),
        branch_return_outputs: BTreeMap::new(),
        joined_return_bindings: BTreeMap::new(),
        joined_assignment_owners: BTreeMap::new(),
        return_output_types: Vec::new(),
        return_body_span: None,
        checked_call_owners: BTreeMap::new(),
        checked_owner_flags: BTreeMap::new(),
        call_owner_flags: BTreeMap::new(),
        return_owner_flags: BTreeMap::new(),
        return_owner_slots: BTreeMap::new(),
        return_length_slots: BTreeMap::new(),
        call_length_slots: BTreeMap::new(),
        runtime_array_owners: BTreeMap::new(),
        runtime_array_allocations: BTreeMap::new(),
        runtime_array_lengths: BTreeMap::new(),
        runtime_array_live: BTreeMap::new(),
        mutable_runtime_allocations: BTreeSet::new(),
        invalidations: BTreeMap::new(),
        automatic_return_result_owners: BTreeMap::new(),
        globals: BTreeMap::new(),
        all_globals: BTreeMap::new(),
        structs: &[],
        target_layout: layout,
        next_literal: 0,
        next_block: 0,
    };
    let (plan, evaluation) = state
        .holder_events
        .assignment_evaluation(&assignment)
        .expect("actual rows");
    let ty = &plan.targets[0].ty;
    let address = builder
        .build_alloca(
            storage_type(&context, ty, &[], layout).expect("actual runtime storage type"),
            "items",
        )
        .expect("storage");
    let old_backing = builder
        .build_alloca(context.i8_type().array_type(2), "old_backing")
        .expect("old backing");
    let replacement = builder
        .build_alloca(context.i8_type().array_type(5), "replacement_backing")
        .expect("replacement backing");
    builder
        .build_store(address, old_backing)
        .expect("old payload");
    let length_slot = builder
        .build_alloca(context.i64_type(), "items_length")
        .expect("length slot");
    builder
        .build_store(length_slot, context.i64_type().const_int(2, false))
        .expect("old length");
    state.storage.insert("items".into(), (address, ty.clone()));
    state
        .runtime_array_lengths
        .insert("items".into(), length_slot);
    let frozen = freeze_assignment_destination(&context, &mut state, &assignment, 0, ty, None)
        .expect("actual freeze");
    let destinations = [frozen];
    if matches!(proof, Proof::UnsupportedThrough) {
        let mut altered = evaluation.clone();
        altered.destinations[0].storage = EvaluatedDestinationStorage::Through {
            pointer: evaluation.rhs[0].value.clone(),
            cases: Vec::new(),
        };
        let error =
            snapshot_assignment_outputs(&context, &mut state, &plan, &altered, &destinations)
                .err()
                .expect("unsupported Through producer");
        assert!(
            error.contains("missing evaluated-length for unsupported storage")
                && error.contains("src/saved_length.w"),
            "{error}"
        );
        assert!(state
            .holder_events
            .saved_assignment_output(&evaluation.results[0].value)
            .is_err());
        return;
    }
    if matches!(proof, Proof::Missing) {
        let error = state
            .holder_events
            .assignment_runtime_length(&assignment, 0)
            .expect_err("missing row");
        assert!(
            error.contains("missing unique emitted saved assignment evaluated-length")
                && error.contains("src/saved_length.w"),
            "{error}"
        );
        let (fact, shape) = state
            .holder_events
            .assignment_value(&evaluation.results[0].value)
            .expect("actual saved fact");
        let saved = SavedAssignmentOutput {
            snapshot: evaluation.previous[0].clone(),
            result: evaluation.results[0].clone(),
            shape: shape.clone(),
            fact: fact.clone(),
            payload: EmitValue::Basic(
                builder
                    .build_load(
                        storage_type(&context, ty, &[], layout).expect("storage type"),
                        address,
                        "saved_without_length",
                    )
                    .expect("saved payload"),
            ),
        };
        state
            .holder_events
            .record_saved_assignment_output(saved)
            .expect("protected payload interface");
        let prepared = PreparedAssignmentOutputs {
            operation: plan.operation.clone(),
            outputs: vec![evaluation.results[0].value.clone()],
        };
        let error = finish_assignment_outputs(&context, &mut state, &plan, &evaluation, prepared)
            .err()
            .expect("finish requires old length");
        assert!(
            error.contains("missing unique emitted saved assignment evaluated-length"),
            "{error}"
        );
        state.runtime_array_lengths.remove("items");
        let error =
            snapshot_assignment_outputs(&context, &mut state, &plan, &evaluation, &destinations)
                .err()
                .expect("missing producer slot");
        assert!(
            error.contains("missing evaluated-length slot") && error.contains("src/saved_length.w"),
            "{error}"
        );
        return;
    }
    if matches!(proof, Proof::DuplicateBatch) {
        let first = snapshot_runtime_length(
            &context,
            &state,
            &plan,
            &evaluation,
            &evaluation.previous[0],
            &evaluation.results[0],
            &destinations[0],
        )
        .expect("first actual row");
        let second = snapshot_runtime_length(
            &context,
            &state,
            &plan,
            &evaluation,
            &evaluation.previous[0],
            &evaluation.results[0],
            &destinations[0],
        )
        .expect("duplicate actual incidence");
        let error = state
            .holder_events
            .record_saved_assignment_runtime_lengths(vec![first, second])
            .expect_err("batch preflight");
        assert!(
            error.contains("duplicate emitted saved assignment runtime length"),
            "{error}"
        );
        assert!(state
            .holder_events
            .assignment_runtime_length(&assignment, 0)
            .expect_err("batch remains unpublished")
            .contains("missing unique emitted saved assignment evaluated-length"));
        return;
    }
    if matches!(proof, Proof::WrongShape) {
        let mut altered = cfg.clone();
        let shape = altered
            .evaluated_values
            .shapes
            .iter_mut()
            .find(|shape| shape.id == evaluation.destinations[0].shape)
            .expect("actual shape");
        shape.ty = ScalarType::I64;
        let events = holder_events::HolderEvents::new(&altered).expect("malformed test events");
        let SnapshotState::Saved(snapshot) = &evaluation.previous[0].state else {
            panic!("snapshot");
        };
        let row = SavedAssignmentRuntimeLength {
            snapshot: snapshot.clone(),
            result: evaluation.results[0].value.clone(),
            destination: evaluation.destinations[0].id.clone(),
            storage: evaluation.destinations[0].storage.clone(),
            length: context.i64_type().const_int(2, false),
        };
        let error = events
            .validate_saved_assignment_runtime_lengths(&[row])
            .expect_err("wrong shape");
        assert!(
            error.contains("shape/storage/i64 incidence") && error.contains("src/saved_length.w"),
            "{error}"
        );
        return;
    }
    let prepared =
        snapshot_assignment_outputs(&context, &mut state, &plan, &evaluation, &destinations)
            .expect("actual snapshot");
    let result = &evaluation.results[0];
    let row = state
        .holder_events
        .saved_assignment_runtime_length(&result.value)
        .expect("precommit row");
    let captured = row.length;
    assert_eq!(row.destination, evaluation.destinations[0].id);
    assert_eq!(row.storage, evaluation.destinations[0].storage);
    assert_eq!(row.result, result.value);
    assert_ne!(row.snapshot, row.result);
    assert_eq!(row.snapshot.point, result.snapshot);
    let mut malformed = SavedAssignmentRuntimeLength {
        snapshot: row.snapshot.clone(),
        result: row.result.clone(),
        destination: row.destination.clone(),
        storage: row.storage.clone(),
        length: row.length,
    };
    match proof {
        Proof::Duplicate => {
            let error = state
                .holder_events
                .record_saved_assignment_runtime_lengths(vec![malformed])
                .expect_err("duplicate row");
            assert!(
                error.contains("duplicate emitted saved assignment runtime length"),
                "{error}"
            );
            assert_eq!(
                state
                    .holder_events
                    .assignment_runtime_length(&assignment, 0)
                    .expect("original remains"),
                captured
            );
            return;
        }
        Proof::WrongId => {
            malformed.result = malformed.snapshot.clone();
            let error = state
                .holder_events
                .validate_saved_assignment_runtime_lengths(&[malformed])
                .expect_err("wrong result ID");
            assert!(
                error.contains("evaluated-length result") && error.contains("src/saved_length.w"),
                "{error}"
            );
            return;
        }
        Proof::WrongWidth => {
            malformed.length = context.i32_type().const_int(2, false);
            let error = state
                .holder_events
                .validate_saved_assignment_runtime_lengths(&[malformed])
                .expect_err("wrong i64 width");
            assert!(error.contains("shape/storage/i64 incidence"), "{error}");
            return;
        }
        Proof::Lifecycle => {}
        _ => panic!("handled proof"),
    }
    builder
        .build_store(address, replacement)
        .expect("replacement commit");
    builder
        .build_store(length_slot, context.i64_type().const_int(5, false))
        .expect("replacement length");
    drop_backing_entry(&mut state, "items");
    assert!(!state.runtime_array_lengths.contains_key("items"));
    let source = ScalarExpression::Assignment(Box::new(assignment));
    assert_eq!(
        runtime_length_for_source(&context, &state, &source, 0).expect("saved consumer"),
        captured
    );
    bind_runtime_length(&context, &mut state, "received", ty, &source, 0)
        .expect("productive bind consumer");
    assert_eq!(
        call_argument_lengths(
            &context,
            &state,
            "consume",
            &[source.clone()],
            &[ty.clone()]
        )
        .expect("productive call consumer"),
        vec![Some(captured)]
    );
    let return_slot = builder
        .build_alloca(context.i64_type(), "returned_length")
        .expect("return metadata");
    state.return_length_slots.insert(0, return_slot);
    record_return_length(&context, &mut state, ty, &source, 0).expect("productive return consumer");
    let payload = finish_assignment_outputs(&context, &mut state, &plan, &evaluation, prepared)
        .expect("finish verifies saved length");
    let EmitValue::Basic(bits) = payload else {
        panic!("saved backing payload");
    };
    assert!(bits.is_pointer_value());
    builder
        .build_return(Some(&captured))
        .expect("return captured length");
    module.verify().expect("verified lifecycle IR");
    let ir = module.print_to_string().to_string();
    let load = ir
        .find("%assignment_previous_length = load i64, ptr %items_length")
        .expect("exact old slot load");
    let commit = ir
        .find("store ptr %replacement_backing, ptr %items")
        .expect("replacement store");
    assert!(load < commit, "{ir}");
    assert_eq!(
        ir.matches("assignment_previous_length = load").count(),
        1,
        "{ir}"
    );
    assert!(
        ir.contains("store i64 %assignment_previous_length, ptr %received_length"),
        "{ir}"
    );
    assert!(
        ir.contains("store i64 %assignment_previous_length, ptr %returned_length"),
        "{ir}"
    );
}

#[test]
fn assignment_saved_length_program_native64() {
    proof(ScalarTargetLayout::NATIVE64, false, Proof::Lifecycle);
}
#[test]
fn assignment_saved_length_program_wasm32() {
    proof(ScalarTargetLayout::WASM32, false, Proof::Lifecycle);
}
#[test]
fn assignment_saved_length_project_native64() {
    proof(ScalarTargetLayout::NATIVE64, true, Proof::Lifecycle);
}
#[test]
fn assignment_saved_length_project_wasm32() {
    proof(ScalarTargetLayout::WASM32, true, Proof::Lifecycle);
}
#[test]
fn assignment_saved_length_rejects_duplicate() {
    proof(ScalarTargetLayout::NATIVE64, false, Proof::Duplicate);
}
#[test]
fn assignment_saved_length_rejects_duplicate_batch() {
    proof(ScalarTargetLayout::NATIVE64, false, Proof::DuplicateBatch);
}
#[test]
fn assignment_saved_length_rejects_missing() {
    proof(ScalarTargetLayout::NATIVE64, false, Proof::Missing);
}
#[test]
fn assignment_saved_length_rejects_wrong_id() {
    proof(ScalarTargetLayout::NATIVE64, false, Proof::WrongId);
}
#[test]
fn assignment_saved_length_rejects_wrong_shape() {
    proof(ScalarTargetLayout::NATIVE64, false, Proof::WrongShape);
}
#[test]
fn assignment_saved_length_rejects_wrong_width() {
    proof(ScalarTargetLayout::NATIVE64, false, Proof::WrongWidth);
}
#[test]
fn assignment_saved_length_rejects_unsupported_through() {
    proof(
        ScalarTargetLayout::NATIVE64,
        false,
        Proof::UnsupportedThrough,
    );
}

use super::*;

fn source_rows(
    layout: ScalarTargetLayout,
    project: bool,
) -> (crate::scalar::ScalarReferenceCfg, ScalarAssignment) {
    let source = wosy_syntax::SourceIdentity::new(
        "registration-proof".into(),
        "app".into(),
        "src/registration.w".into(),
        "r1".into(),
    );
    let parsed = crate::parse_source(source.clone(), "%%start\n(i32, i32)() inspect = fn { i32 left = 11; i32 right = 22; left, right = 33, 44 };\ninspect();\n%%end".to_owned(), &[]);
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
        .expect("source function");
    let ScalarExpression::Assignment(assignment) = &function.body.final_output_values[0].value
    else {
        panic!("actual final assignment");
    };
    (function.reference_cfg.clone(), assignment.as_ref().clone())
}

fn lifecycle(layout: ScalarTargetLayout, project: bool) {
    let (cfg, assignment) = source_rows(layout, project);
    let context = Context::create();
    let builder = context.create_builder();
    let functions = BTreeMap::new();
    let call_targets = BTreeMap::new();
    let signatures = BTreeMap::new();
    let dynamic_owner_outputs = BTreeMap::new();
    let literal_signatures = std::collections::HashMap::new();
    let literal_captures = std::collections::HashMap::new();
    let literal_reference_cfgs = std::collections::HashMap::new();
    let module = context.create_module("registration");
    let pair_type = context.struct_type(
        &[context.i32_type().into(), context.i32_type().into()],
        false,
    );
    let function = module.add_function("registration", pair_type.fn_type(&[], false), None);
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
        return_output_types: vec![ScalarType::I32, ScalarType::I32],
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
    let mut destinations = Vec::new();
    for (name, old) in [("left", 11), ("right", 22)] {
        let address = builder
            .build_alloca(context.i32_type(), name)
            .expect("actual destination");
        builder
            .build_store(address, context.i32_type().const_int(old, false))
            .expect("old bits");
        destinations.push(FrozenAssignmentDestination::Storage {
            address,
            selection: FrozenAssignmentSelection::Ordinary,
        });
    }
    let prepared =
        snapshot_assignment_outputs(&context, &mut state, &plan, &evaluation, &destinations)
            .expect("actual preparation");
    assert_eq!(prepared.outputs.len(), 2);
    let mut old_payloads = Vec::new();
    for result in &evaluation.results {
        let saved = state
            .holder_events
            .saved_assignment_output(&result.value)
            .expect("registered before commits");
        assert_eq!(saved.result, *result);
        assert_eq!(saved.fact.id, result.value);
        let SnapshotState::Saved(snapshot_id) = &saved.snapshot.state else {
            panic!("saved snapshot");
        };
        assert_ne!(*snapshot_id, result.value);
        let EmitValue::Basic(bits) = saved.payload else {
            panic!("actual numeric bits");
        };
        old_payloads.push(bits);
    }
    assert_ne!(old_payloads[0], old_payloads[1]);
    let error =
        snapshot_assignment_outputs(&context, &mut state, &plan, &evaluation, &destinations)
            .err()
            .expect("repeated creation fails");
    assert!(
        error.contains("duplicate emitted saved assignment result"),
        "{error}"
    );
    for (destination, replacement) in destinations.iter().zip([33, 44]) {
        let FrozenAssignmentDestination::Storage { address, .. } = destination else {
            panic!("storage");
        };
        builder
            .build_store(*address, context.i32_type().const_int(replacement, false))
            .expect("current replacement");
    }
    for (result, bits) in evaluation.results.iter().zip(old_payloads) {
        let saved = state
            .holder_events
            .saved_assignment_output(&result.value)
            .expect("saved after replacement");
        let EmitValue::Basic(payload) = saved.payload else {
            panic!("saved bits");
        };
        assert_eq!(payload, bits);
    }
    let output = finish_assignment_outputs(&context, &mut state, &plan, &evaluation, prepared)
        .expect("reader performs zero registration");
    let EmitValue::Aggregate { value, .. } = output else {
        panic!("ordered two outputs");
    };
    builder.build_return(Some(&value)).expect("return old pair");
    module
        .verify()
        .expect("verified actual preparation and consumption");
}

#[test]
fn assignment_precommit_registration_program_native64() {
    lifecycle(ScalarTargetLayout::NATIVE64, false);
}
#[test]
fn assignment_precommit_registration_program_wasm32() {
    lifecycle(ScalarTargetLayout::WASM32, false);
}
#[test]
fn assignment_precommit_registration_project_native64() {
    lifecycle(ScalarTargetLayout::NATIVE64, true);
}
#[test]
fn assignment_precommit_registration_project_wasm32() {
    lifecycle(ScalarTargetLayout::WASM32, true);
}

fn manifest_rows() -> (
    ResolvedAssignmentPlan,
    ScalarAssignmentEvaluation,
    PreparedAssignmentOutputs,
) {
    let (cfg, assignment) = source_rows(ScalarTargetLayout::NATIVE64, false);
    let events = holder_events::HolderEvents::new(&cfg).expect("actual events");
    let (plan, evaluation) = events
        .assignment_evaluation(&assignment)
        .expect("actual rows");
    let prepared = PreparedAssignmentOutputs {
        operation: plan.operation.clone(),
        outputs: evaluation
            .results
            .iter()
            .map(|result| result.value.clone())
            .collect(),
    };
    (plan, evaluation, prepared)
}

#[test]
fn assignment_precommit_registration_rejects_wrong_operation() {
    let (plan, evaluation, mut prepared) = manifest_rows();
    prepared.operation.occurrence.source_span.source.path = "src/foreign.w".into();
    let error =
        validate_prepared_outputs(&plan, &evaluation, &prepared).expect_err("operation mismatch");
    assert!(error.contains("operation differs"), "{error}");
    assert!(error.contains("src/registration.w"), "{error}");
}

#[test]
fn assignment_precommit_registration_rejects_wrong_order() {
    let (plan, evaluation, mut prepared) = manifest_rows();
    prepared.outputs.swap(0, 1);
    let error =
        validate_prepared_outputs(&plan, &evaluation, &prepared).expect_err("order mismatch");
    assert!(error.contains("output order/ID differs"), "{error}");
}

#[test]
fn assignment_precommit_registration_rejects_foreign_source_id() {
    let (plan, evaluation, mut prepared) = manifest_rows();
    prepared.outputs[0].resolved.occurrence.source.path = "src/foreign.w".into();
    let error =
        validate_prepared_outputs(&plan, &evaluation, &prepared).expect_err("foreign result ID");
    assert!(error.contains("output order/ID differs"), "{error}");
}

#[test]
fn assignment_precommit_registration_rejects_duplicate_manifest() {
    let (plan, mut evaluation, mut prepared) = manifest_rows();
    prepared.outputs[1] = prepared.outputs[0].clone();
    evaluation.results[1].value = prepared.outputs[0].clone();
    let error =
        validate_prepared_outputs(&plan, &evaluation, &prepared).expect_err("duplicate entry");
    assert!(
        error.contains("duplicate emitted saved assignment result"),
        "{error}"
    );
}

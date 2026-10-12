use super::*;
use crate::parse_source;

const SOURCE: &str = "%%start\nstruct copy Pair { u8 left; u8 right; }\nu8()() make = fn { u8() inner = fn { 7 }; inner };\nunit() exercise = fn { u64 first, u64 second = 4; Pair pair = { .left = 1; .right = 2; }; u8() local = fn { 11 }; first; second; pair; local(); };\n%%end";

const REPEATED: &str = "%%start\nunit() repeat = fn { u8() left, u8() middle, u8() right = fn { 7 }; left(); middle(); right(); };\nunit() stable = fn { u8() original = fn { 9 }; u8() left, u8() right = original; left(); right(); original(); };\nunit() captured = fn { u8 value = 1; u8() left, u8() right = fn { value = value + 1; value }; left(); right(); };\n%%end";

const AGGREGATE_POLICIES: &str = "%%start\nstruct copy Reader { u8() read; }\nstruct Moving { u8() read; }\nunit() policies = fn { Reader original = { .read = fn { 7 }; }; Moving moved = { .read = fn { 9 }; }; Reader first, Reader second = original; Reader[1] fixed_copy = [original]; Moving[1] fixed_move = [moved]; first.read(); second.read(); fixed_copy; fixed_move; };\n%%end";

const PROJECTED_AGGREGATES: &str = "%%start\nstruct copy Reader { u8() read; }\nstruct copy Shelf { Reader inner; Reader sibling; }\nunit(Shelf) projected = fn(shelf) { Reader first, Reader second = shelf.inner; first; second; };\nReader() make = fn { Reader result = { .read = fn { 7 }; }; result };\nunit() consume = fn { Reader first, Reader second = make(); first.read(); second.read(); };\n%%end";

const MIXED_LITERALS: &str = "%%start\nstruct copy Numeric { u8() read; u8 count; }\nstruct copy RawMix { *?u8 address; u8() read; }\nstruct Storage { u8[] data; }\nstruct RuntimeMix { u8[] data; u8() read; }\nstruct NestedMix { Storage stored; u8() read; u8 count; }\nunit(u64) mixed = fn(count) { Numeric numeric = { .read = fn { 7 }; .count = 2; }; Numeric copied_numeric = numeric; *?u8 address = null; RawMix raw = { .address = address; .read = fn { 9 }; }; RawMix copied_raw = raw; u8[count] first; RuntimeMix runtime = { .data = first; .read = fn { 11 }; }; u8[count] second; Storage storage = { .data = second; }; NestedMix nested = { .stored = storage; .read = fn { 13 }; .count = 4; }; numeric; copied_numeric; raw; copied_raw; runtime; nested; };\nunit() assignment = fn { (u8())[1] source = [fn { 1 }]; (u8())[1] target = [fn { 2 }]; target = source; target[0](); };\n%%end";

const MODULE_MEMBERS: &str = "%%start\nstruct Checked { *u8 address; u8() read; u8 tag; }\nstruct Storage { u8[] data; }\nstruct DirectRuntime { u8[] data; }\nstruct NestedRuntime { Storage stored; }\nstruct copy CopyMixed { u8() read; u8 tag; }\nstruct copy ModuleRaw { *?u8 address; u8() read; }\n*u8(*u8) echo = fn(pointer) { pointer };\nu8[](u64) make_data = fn(count) { u8[count] data; data };\nStorage(u64) make_storage = fn(count) { u8[count] data; Storage result = { .data = data; }; result };\nu8 cell = 1;\n*u8 pointer = &cell;\nChecked named = { .address = pointer; .read = fn { 7 }; .tag = 1; };\nChecked addressed = { .address = &cell; .read = fn { 9 }; .tag = 2; };\nChecked called = { .address = echo(pointer); .read = fn { 11 }; .tag = 3; };\nu64 length = 2;\nu8[length] nested_data;\nStorage storage = { .data = nested_data; };\nu8[length] direct_data;\nDirectRuntime direct_named = { .data = direct_data; };\nDirectRuntime direct_called = { .data = make_data(3); };\nNestedRuntime nested_named = { .stored = storage; };\nNestedRuntime nested_called = { .stored = make_storage(4); };\nCopyMixed first, CopyMixed second = { .read = fn { 13 }; .tag = 5; };\n*?u8 raw = null;\nModuleRaw raw_mixed = { .address = raw; .read = fn { 15 }; };\n%%end";

fn prove_module_member_installations(layout: ScalarTargetLayout, project: bool) {
    let source = SourceIdentity::new("direct-module-members".into(), "app".into(), "src/module.w".into(), "r1".into());
    let parsed = parse_source(source.clone(), MODULE_MEMBERS.to_owned(), &[]);
    assert!(parsed.diagnostics.is_empty(), "{:#?}", parsed.diagnostics);
    let (validation, trace) = derive_scalar_program_observed_test(&parsed.result, layout);
    assert!(validation.diagnostics.is_empty(), "{:#?}", validation.diagnostics);
    let (module, trace) = if project {
        let (validation, trace) = validate_scalar_project_observed_test(ScalarProject::new(
            vec![ScalarModule::from_program(validation.program, Vec::new())], vec![source.clone()]));
        assert!(validation.diagnostics.is_empty(), "{:#?}", validation.diagnostics);
        (validation.project.modules.into_iter().next().unwrap(), trace)
    } else { (ScalarModule::from_program(validation.program, Vec::new()), trace) };
    let cfg = &module.top_level_reference_cfg;
    let pass = trace.passes.iter().rfind(|pass| pass.owner == cfg.producer_owner).unwrap();
    evaluated_aggregate_receiving::validate(cfg).unwrap();
    let mut counts = [0; 4];
    let mut checked_sources = [0; 3];
    let mut backing_sources = [0; 2];
    let mut projected_sources = [0; 2];
    for input in &cfg.evaluated_values.binding_inputs {
        if !matches!(&cfg.initial_points[input.source.point.0].kind, CfgPointKind::Evaluate(ScalarExpression::StructLiteral { .. }))
            || binding_holder::first(cfg, input).is_some() { continue; }
        counts[0] += 1;
        let installed = evaluated_aggregate_receiving::installations(cfg, input, input.consumer).unwrap();
        assert_eq!(installed[0], input.consumer);
        let CfgPointKind::Bind { binding } = &cfg.initial_points[input.consumer.0].kind else { unreachable!(); };
        let later = cfg.evaluated_values.binding_inputs.iter().filter(|candidate|
            candidate.source == input.source && candidate.occurrence == input.occurrence && candidate.consumer != input.consumer).collect::<Vec<_>>();
        let diagnostic = if later.is_empty() { input.consumer } else {
            let [later] = later.as_slice() else { panic!("actual first/additional module fixture"); };
            let copies = cfg.binding_points.iter().filter(|row| row.occurrence == later.occurrence && row.output == later.output
                && row.role == ScalarBindingPointRole::CopyCallableFields).collect::<Vec<_>>();
            let [copy] = copies.as_slice() else { panic!("actual additional module copy"); };
            let receipt = evaluated_aggregate_receiving::plan(cfg, copy.point).unwrap();
            assert_eq!(receipt.source, input.source);
            assert_eq!(receipt.first_receipt, *binding);
            assert_eq!(receipt.installations, installed);
            assert_ne!(later.output, input.source.resolved.output);
            copy.point
        };
        for installation in installed.into_iter().skip(1) {
            match &cfg.initial_points[installation.0].kind {
                CfgPointKind::Assign { rhs_point, target, .. } => {
                    counts[1] += 1;
                    let field = target.projections.iter().find_map(|projection| match projection {
                        ReferencePlaceProjection::Field(ScalarFieldReference::Resolved(field)) => Some(field), _ => None,
                    }).unwrap();
                    let declaration = module.structs.iter().find(|row| row.id == field.structure).unwrap();
                    if matches!(declaration.fields[field.index].ty, ScalarType::CheckedReference { .. }) {
                        let index = match cfg.initial_points[rhs_point.0].kind {
                            CfgPointKind::Read { .. } => 0,
                            CfgPointKind::Borrow { .. } => 1,
                            CfgPointKind::Call { .. } => 2,
                            _ => panic!("actual module checked Name/Address/Call source"),
                        };
                        checked_sources[index] += 1;
                    }
                }
                CfgPointKind::BackingPlace { transfer: RuntimeBackingTransfer::Move { source }, .. } => {
                    counts[2] += 1;
                    backing_sources[usize::from(matches!(source.kind, ReferenceBindingKind::CallOutput { .. }))] += 1;
                }
                CfgPointKind::CopyProjectedBackings { source, .. } => {
                    counts[3] += 1;
                    projected_sources[usize::from(matches!(source.kind, ReferenceBindingKind::CallOutput { .. }))] += 1;
                }
                _ => panic!("actual module member installation"),
            }
            for defect in 0..5 {
                let mut malformed = cfg.clone();
                match defect {
                    0 => {
                        malformed.initial_points[installation.0].kind = CfgPointKind::Evaluate(ScalarExpression::Integer {
                            value: BigInt::from(0), span: cfg.initial_points[installation.0].source_span.range,
                        });
                        malformed.points[installation.0].kind = malformed.initial_points[installation.0].kind.clone();
                    }
                    1 => {
                        if let Some(row) = cfg.binding_points.iter().find(|row| row.point == installation) {
                            malformed.binding_points.push(row.clone());
                        } else {
                            malformed.initial_points.push(cfg.initial_points[installation.0].clone());
                            malformed.points.push(cfg.points[installation.0].clone());
                        }
                    }
                    2 | 3 => {
                        match &mut malformed.initial_points[installation.0].kind {
                            CfgPointKind::Assign { target, rhs_point, .. } => {
                                if defect == 2 { *rhs_point = input.source.point; } else { target.projections.clear(); }
                            }
                            CfgPointKind::BackingPlace { target, transfer: RuntimeBackingTransfer::Move { source }, .. }
                                | CfgPointKind::CopyProjectedBackings { source, target } => {
                                if defect == 2 { *source = binding.clone(); } else { target.projections.clear(); }
                            }
                            _ => unreachable!(),
                        }
                        malformed.points[installation.0].kind = malformed.initial_points[installation.0].kind.clone();
                    }
                    4 => malformed.points[installation.0].kind = CfgPointKind::CopyCallableFields {
                        source: CallableFieldSource::Assignment(binding.clone()), target: binding.clone(),
                    },
                    _ => unreachable!(),
                }
                let before = malformed.clone();
                let mut negative_trace = ReferenceValidationTestTrace::new();
                let errors = transfer_reference_facts(&mut malformed, &pass.context.deferred_policies,
                    &pass.context.contracts, &pass.context.literal_contracts, &pass.context.call_exits,
                    &pass.context.capture_input,
                    pass.role.clone(), &mut negative_trace);
                let expected = if defect == 4 { installation } else { diagnostic };
                assert_eq!(errors, vec![ReferenceAnalysisError::MissingContext {
                    source_span: cfg.initial_points[expected.0].source_span.clone(), reason: ReferenceAnalysisErrorReason::InvalidAssignmentCorrespondence,
                }]);
                assert_eq!(malformed, before);
                assert_eq!(negative_trace.passes.len(), 1);
                assert!(negative_trace.passes[0].observations.points.is_empty());
                assert!(negative_trace.passes[0].observations.borrows.is_empty());
            }
        }
    }
    assert_eq!(counts, [10, 8, 3, 2]);
    assert_eq!(checked_sources, [1, 1, 1]);
    assert_eq!(backing_sources, [2, 1]);
    assert_eq!(projected_sources, [1, 1]);
}

fn prove_mixed_literal_installations(layout: ScalarTargetLayout, project: bool) {
    let source = SourceIdentity::new("direct-mixed-proof".into(), "app".into(), "src/mixed.w".into(), "r1".into());
    let parsed = parse_source(source.clone(), MIXED_LITERALS.to_owned(), &[]);
    assert!(parsed.diagnostics.is_empty(), "{:#?}", parsed.diagnostics);
    let (validation, trace) = derive_scalar_program_observed_test(&parsed.result, layout);
    assert!(validation.diagnostics.is_empty(), "{:#?}", validation.diagnostics);
    let (module, trace) = if project {
        let (validation, trace) = validate_scalar_project_observed_test(ScalarProject::new(
            vec![ScalarModule::from_program(validation.program, Vec::new())], vec![source.clone()]));
        assert!(validation.diagnostics.is_empty(), "{:#?}", validation.diagnostics);
        (validation.project.modules.into_iter().next().unwrap(), trace)
    } else { (ScalarModule::from_program(validation.program, Vec::new()), trace) };
    let function = module.items.iter().find_map(|item| match item {
        ScalarItem::Function(function) if function.name == "mixed" => Some(function), _ => None,
    }).unwrap();
    let cfg = &function.reference_cfg;
    let pass = trace.passes.iter().rfind(|pass| pass.owner == cfg.producer_owner).unwrap();
    evaluated_aggregate_receiving::validate(cfg).unwrap();
    let mut counts = [0; 4];
    for input in &cfg.evaluated_values.binding_inputs {
        if !matches!(&cfg.initial_points[input.source.point.0].kind, CfgPointKind::Evaluate(ScalarExpression::StructLiteral { .. })) { continue; }
        counts[0] += 1;
        let installed = evaluated_aggregate_receiving::installations(cfg, input, input.consumer).unwrap();
        assert_eq!(installed[0], input.consumer);
        let CfgPointKind::Bind { binding } = &cfg.initial_points[input.consumer.0].kind else { unreachable!(); };
        let literal = cfg.evaluated_values.source_literal_members.iter().find(|row| row.value == input.source).unwrap();
        let members = evaluated_aggregate_receiving::members(cfg, &input.source).unwrap();
        assert!(!members.is_empty());
        for member in &literal.members {
            let fact = cfg.evaluated_values.values.iter().find(|fact| fact.id == member.value).unwrap();
            let shape = cfg.evaluated_values.shapes.iter().find(|shape| shape.id == fact.shape).unwrap();
            if matches!(shape.ty, ScalarType::RawPointer(_) | ScalarType::U8) {
                let [ValueProjection::Field(field)] = member.projection.as_slice() else { unreachable!(); };
                assert!(!cfg.binding_points.iter().any(|row| row.occurrence == input.occurrence && row.output == input.output
                    && row.role == ScalarBindingPointRole::FieldAssign(field.clone())));
            }
        }
        for installation in installed.into_iter().skip(1) {
            match cfg.initial_points[installation.0].kind {
                CfgPointKind::Assign { .. } => counts[1] += 1,
                CfgPointKind::BackingPlace { .. } => counts[2] += 1,
                CfgPointKind::CopyProjectedBackings { .. } => counts[3] += 1,
                _ => panic!("actual mixed literal installation"),
            }
            for defect in 0..3 {
                let mut malformed = cfg.clone();
                match defect {
                    0 => {
                        malformed.initial_points[installation.0].kind = CfgPointKind::Evaluate(ScalarExpression::Integer {
                            value: BigInt::from(0),
                            span: cfg.initial_points[installation.0].source_span.range,
                        });
                        malformed.points[installation.0].kind = malformed.initial_points[installation.0].kind.clone();
                    }
                    1 => {
                        let kind = &mut malformed.initial_points[installation.0].kind;
                        match kind {
                            CfgPointKind::Assign { rhs_point, .. } => *rhs_point = input.source.point,
                            CfgPointKind::BackingPlace { transfer: RuntimeBackingTransfer::Move { source }, .. }
                                | CfgPointKind::CopyProjectedBackings { source, .. } => *source = binding.clone(),
                            _ => panic!("actual required source protocol"),
                        }
                        malformed.points[installation.0].kind = malformed.initial_points[installation.0].kind.clone();
                    }
                    2 => {
                        let kind = &mut malformed.initial_points[installation.0].kind;
                        match kind {
                            CfgPointKind::Assign { target, .. } | CfgPointKind::BackingPlace { target, .. }
                                | CfgPointKind::CopyProjectedBackings { target, .. } => target.projections.clear(),
                            _ => unreachable!(),
                        }
                        malformed.points[installation.0].kind = malformed.initial_points[installation.0].kind.clone();
                    }
                    _ => unreachable!(),
                }
                let before = malformed.clone();
                let mut negative_trace = ReferenceValidationTestTrace::new();
                let errors = transfer_reference_facts(&mut malformed, &pass.context.deferred_policies,
                    &pass.context.contracts, &pass.context.literal_contracts, &pass.context.call_exits,
                    &pass.context.capture_input,
                    pass.role.clone(), &mut negative_trace);
                assert_eq!(errors, vec![ReferenceAnalysisError::MissingContext {
                    source_span: cfg.initial_points[input.consumer.0].source_span.clone(),
                    reason: ReferenceAnalysisErrorReason::InvalidAssignmentCorrespondence,
                }]);
                assert_eq!(malformed, before);
                assert_eq!(negative_trace.passes.len(), 1);
                assert!(negative_trace.passes[0].observations.points.is_empty());
                assert!(negative_trace.passes[0].observations.borrows.is_empty());
            }
        }
    }
    assert_eq!(counts, [5, 4, 2, 1]);
    let assignment = module.items.iter().find_map(|item| match item {
        ScalarItem::Function(function) if function.name == "assignment" => Some(function), _ => None,
    }).unwrap();
    let cfg = &assignment.reference_cfg;
    let pass = trace.passes.iter().rfind(|pass| pass.owner == cfg.producer_owner).unwrap();
    evaluated_aggregate_receiving::validate(cfg).unwrap();
    let copies = cfg.initial_points.iter().filter(|point| matches!(&point.kind,
        CfgPointKind::CopyCallableFields { source: CallableFieldSource::Assignment(_), .. })).collect::<Vec<_>>();
    assert_eq!(copies.len(), 1);
    for copy in copies {
        let CfgPointKind::CopyCallableFields { source: CallableFieldSource::Assignment(source), target } = &copy.kind else { unreachable!(); };
        assert_ne!(source, target);
        for defect in 0..3 {
            let mut malformed = cfg.clone();
            let CfgPointKind::CopyCallableFields { source: actual, target: receiving } = &mut malformed.points[copy.id.0].kind else { unreachable!(); };
            match defect {
                0 => *actual = CallableFieldSource::Evaluated(cfg.evaluated_values.binding_inputs[0].source.clone()),
                1 => *actual = CallableFieldSource::Assignment(target.clone()),
                2 => *receiving = source.clone(),
                _ => unreachable!(),
            }
            let before = malformed.clone();
            let mut negative_trace = ReferenceValidationTestTrace::new();
            let errors = transfer_reference_facts(&mut malformed, &pass.context.deferred_policies,
                &pass.context.contracts, &pass.context.literal_contracts, &pass.context.call_exits,
                &pass.context.capture_input,
                pass.role.clone(), &mut negative_trace);
            assert_eq!(errors, vec![ReferenceAnalysisError::MissingContext {
                source_span: copy.source_span.clone(), reason: ReferenceAnalysisErrorReason::InvalidAssignmentCorrespondence,
            }]);
            assert_eq!(malformed, before);
            assert_eq!(negative_trace.passes.len(), 1);
            assert!(negative_trace.passes[0].observations.points.is_empty());
            assert!(negative_trace.passes[0].observations.borrows.is_empty());
        }
    }
}

fn prove_projected_aggregates(layout: ScalarTargetLayout, project: bool) {
    let source = SourceIdentity::new("direct-projected-proof".into(), "app".into(), "src/projected.w".into(), "r1".into());
    let parsed = parse_source(source.clone(), PROJECTED_AGGREGATES.to_owned(), &[]);
    assert!(parsed.diagnostics.is_empty(), "{:#?}", parsed.diagnostics);
    let (validation, trace) = derive_scalar_program_observed_test(&parsed.result, layout);
    assert!(validation.diagnostics.is_empty(), "{:#?}", validation.diagnostics);
    let (module, trace) = if project {
        let (validation, trace) = validate_scalar_project_observed_test(ScalarProject::new(
            vec![ScalarModule::from_program(validation.program, Vec::new())], vec![source.clone()]));
        assert!(validation.diagnostics.is_empty(), "{:#?}", validation.diagnostics);
        (validation.project.modules.into_iter().next().unwrap(), trace)
    } else { (ScalarModule::from_program(validation.program, Vec::new()), trace) };
    let mut projected = 0;
    let mut consumed = 0;
    for function in module.items.iter().filter_map(|item| match item { ScalarItem::Function(function) => Some(function), _ => None }) {
        let cfg = &function.reference_cfg;
        let pass = trace.passes.iter().rfind(|pass| pass.owner == cfg.producer_owner).unwrap();
        for copy in cfg.binding_points.iter().filter(|row| row.role == ScalarBindingPointRole::CopyCallableFields) {
            let input = binding_holder::field_input(cfg, copy.point).unwrap().unwrap();
            let receipt = evaluated_aggregate_receiving::plan(cfg, copy.point).unwrap();
            let prefix = binding_holder::field_prefix(cfg, copy.point, Some(&receipt)).unwrap();
            let CfgPointKind::CopyCallableFields { source: CallableFieldSource::Evaluated(evaluated), target } = &cfg.initial_points[copy.point.0].kind else { panic!("actual field receiving event"); };
            assert_eq!(*evaluated, input.source);
            assert_eq!(receipt.source, input.source);
            assert_eq!(receipt.target, *target);
            let original_binding = match &cfg.initial_points[evaluated.point.0].kind {
                CfgPointKind::ReadPlace { candidates, .. } => {
                    let [place] = candidates.as_slice() else { panic!("original projected fixture source"); };
                    place.binding.clone()
                }
                CfgPointKind::Call { output, .. } => ReferenceBindingId {
                    source: cfg.initial_points[evaluated.point.0].source_span.source.clone(),
                    function_span: target.function_span,
                    declaration_span: cfg.initial_points[evaluated.point.0].source_span.range,
                    block_span: target.function_span,
                    kind: ReferenceBindingKind::CallOutput { point: evaluated.point, output: *output },
                },
                _ => panic!("original projected/Call source duties"),
            };
            let original = &original_binding;
            for defect in 0..8 {
                let mut malformed = cfg.clone();
                match defect {
                    0 => malformed.binding_points.retain(|row| row.point != copy.point || row.role != ScalarBindingPointRole::CopyCallableFields),
                    1 => malformed.binding_points.push(copy.clone()),
                    2 => {
                        let row = malformed.binding_points.iter_mut().find(|row| row.point == copy.point
                            && row.role == ScalarBindingPointRole::CopyCallableFields).unwrap();
                        match &mut row.source {
                            ScalarBindingOutputSource::Direct { output, .. } | ScalarBindingOutputSource::Call { output, .. } => *output += 1,
                            _ => panic!("actual field source role"),
                        }
                    }
                    3 => {
                        let CfgPointKind::CopyCallableFields { target, .. } = &mut malformed.points[copy.point.0].kind else { unreachable!(); };
                        *target = original.clone();
                    }
                    4 => {
                        let CfgPointKind::CopyCallableFields { source, .. } = &mut malformed.points[copy.point.0].kind else { unreachable!(); };
                        *source = CallableFieldSource::Assignment(target.clone());
                    }
                    5 => {
                        let CfgPointKind::CopyCallableFields { source: CallableFieldSource::Evaluated(source), .. } = &mut malformed.points[copy.point.0].kind else { unreachable!(); };
                        source.resolved.producer = ProducerCfgOwnerId::Module(target.source.clone());
                    }
                    6 => {
                        let CfgPointKind::CopyCallableFields { source: CallableFieldSource::Evaluated(source), .. } = &mut malformed.points[copy.point.0].kind else { unreachable!(); };
                        source.resolved.output += 1;
                    }
                    7 => {
                        let CfgPointKind::CopyCallableFields { source: CallableFieldSource::Evaluated(source), .. } = &mut malformed.points[copy.point.0].kind else { unreachable!(); };
                        source.point = input.consumer;
                    }
                    _ => unreachable!(),
                }
                let before = malformed.clone();
                let mut negative_trace = ReferenceValidationTestTrace::new();
                let errors = transfer_reference_facts(&mut malformed, &pass.context.deferred_policies,
                    &pass.context.contracts, &pass.context.literal_contracts, &pass.context.call_exits,
                    &pass.context.capture_input,
                    pass.role.clone(), &mut negative_trace);
                assert_eq!(errors, vec![ReferenceAnalysisError::MissingContext {
                    source_span: cfg.initial_points[copy.point.0].source_span.clone(), reason: ReferenceAnalysisErrorReason::InvalidAssignmentCorrespondence,
                }]);
                assert_eq!(malformed, before);
                assert_eq!(negative_trace.passes.len(), 1);
                assert!(negative_trace.passes[0].observations.points.is_empty());
                assert!(negative_trace.passes[0].observations.borrows.is_empty());
            }
            if !prefix.is_empty() {
                projected += 1;
                assert_eq!(prefix.len(), 1);
                let CfgPointKind::ReadPlace { candidates, .. } = &cfg.initial_points[input.source.point.0].kind else { panic!("actual aggregate selected source"); };
                let [candidate] = candidates.as_slice() else { panic!("actual selected aggregate place singleton"); };
                assert_eq!(prefix, candidate.projections);
                let fact = cfg.evaluated_values.values.iter().find(|fact| fact.id == input.source).unwrap();
                let shape = cfg.evaluated_values.shapes.iter().find(|shape| shape.id == fact.shape).unwrap();
                let ScalarType::Struct(reader) = &shape.ty else { panic!("actual selected source aggregate type"); };
                let declaration = module.structs.iter().find(|item| item.id == *reader).unwrap();
                let mut member = candidate.clone();
                member.projections.push(ReferencePlaceProjection::Field(ScalarFieldReference::Resolved(declaration.fields[0].id.clone())));
                let relative = binding_holder::field_relative(&member, original, &prefix).unwrap();
                assert_eq!(relative.projections, member.projections[1..]);
                let shelf = module.structs.iter().find(|item| item.fields.iter().any(|field| matches!(&prefix[0],
                    ReferencePlaceProjection::Field(ScalarFieldReference::Resolved(actual)) if actual == &field.id))).unwrap();
                let sibling = shelf.fields.iter().find(|field| !matches!(&prefix[0],
                    ReferencePlaceProjection::Field(ScalarFieldReference::Resolved(actual)) if actual == &field.id)).unwrap();
                let mut crossed = member.clone();
                crossed.projections[0] = ReferencePlaceProjection::Field(ScalarFieldReference::Resolved(sibling.id.clone()));
                assert!(binding_holder::field_relative(&crossed, original, &prefix).is_none());
                let mut malformed = cfg.clone();
                let CfgPointKind::CopyCallableFields { source, .. } = &mut malformed.points[copy.point.0].kind else { unreachable!(); };
                *source = CallableFieldSource::Assignment(target.clone());
                let before = malformed.clone();
                let mut negative_trace = ReferenceValidationTestTrace::new();
                let errors = transfer_reference_facts(&mut malformed, &pass.context.deferred_policies,
                    &pass.context.contracts, &pass.context.literal_contracts, &pass.context.call_exits,
                    &pass.context.capture_input,
                    pass.role.clone(), &mut negative_trace);
                assert_eq!(errors, vec![ReferenceAnalysisError::MissingContext { source_span: cfg.initial_points[copy.point.0].source_span.clone(),
                    reason: ReferenceAnalysisErrorReason::InvalidAssignmentCorrespondence }]);
                assert_eq!(malformed, before);
                assert_eq!(negative_trace.passes.len(), 1);
                assert!(negative_trace.passes[0].observations.points.is_empty());
                assert!(negative_trace.passes[0].observations.borrows.is_empty());
            }
            let Some(first) = binding_holder::field_first(cfg, copy.point, Some(&receipt)).unwrap() else { continue; };
            consumed += 1;
            assert!(matches!(original.kind, ReferenceBindingKind::CallOutput { .. }));
            let snapshots = pass.observations.points.iter().filter(|snapshot| snapshot.point == copy.point
                && snapshot.phase == ReferenceTransferPhase::BeforeInputCleanup).collect::<Vec<_>>();
            assert!(!snapshots.is_empty());
            for snapshot in snapshots {
                let mut fields = 0;
                for relation in &snapshot.state.holder_point_values[input.source.point.0] {
                    for holder in &relation.holders {
                        let CallableHolderPlace::Projection(place) = holder else { continue; };
                        let Some(relative) = binding_holder::field_relative(place, original, &prefix) else { continue; };
                        fields += 1;
                        let donor = binding_holder::field_donor(&cfg.initial_points, copy.point, Some(first), &snapshot.state, place, &relative, relation).unwrap();
                        assert_eq!(snapshot.point, copy.point);
                        assert!(!pass.observations.points.is_empty());
                        for defect in 0..2 {
                            let mut missing = snapshot.state.clone();
                            match defect {
                                0 => missing.holder_provenance.remove(&relation.environment, &donor),
                                1 => {
                                    for live in &mut missing.holder_relations {
                                        if live.environment == relation.environment {
                                            live.holders.retain(|holder| !holder_place_same(holder, &donor));
                                        }
                                    }
                                }
                                _ => unreachable!(),
                            }
                            assert_eq!(binding_holder::field_donor(&cfg.initial_points, copy.point, Some(first), &missing, place, &relative, relation),
                                Err(ReferenceAnalysisError::MissingContext { source_span: cfg.initial_points[copy.point.0].source_span.clone(),
                                    reason: ReferenceAnalysisErrorReason::MissingGlobalCallableProducer }));
                            assert_eq!(snapshot.point, copy.point);
                        }
                        let chain = snapshot.state.holder_provenance.lookup(&relation.environment, &donor).unwrap();
                        let HolderProducerRef::Forward { point, action, .. } = chain else { panic!("actual first field Move Forward"); };
                        assert_eq!(*point, first);
                        assert!(matches!(action, CallableHolderAction::Move { environment, from, to, .. }
                            if environment == &relation.environment && holder_place_same(from, holder) && holder_place_same(to, &donor)));
                        assert!(snapshot.state.holder_provenance.lookup(&relation.environment, holder).is_none());
                        let mut received = relative;
                        received.binding = target.clone(); received.source = target.source.clone();
                        received.function_span = target.function_span; received.declaration_span = target.declaration_span; received.block_span = target.block_span;
                        let received = CallableHolderPlace::Projection(received);
                        let HolderProducerRef::Forward { point, action, previous, .. } = snapshot.state.holder_provenance.lookup(&relation.environment, &received).unwrap()
                            else { panic!("actual later field Retain Forward"); };
                        assert_eq!(*point, copy.point);
                        assert_eq!(previous.as_ref(), chain);
                        assert!(matches!(action, CallableHolderAction::Retain { environment, from, to, .. }
                            if environment == &relation.environment && holder_place_same(from, &donor) && holder_place_same(to, &received)));
                    }
                }
                assert!(fields > 0);
            }
        }
    }
    assert_eq!((projected, consumed), (2, 1));
}

fn prove_aggregate_policies(layout: ScalarTargetLayout, project: bool) {
    let source = SourceIdentity::new("direct-aggregate-policy-proof".into(), "app".into(), "src/policies.w".into(), "r1".into());
    let parsed = parse_source(source.clone(), AGGREGATE_POLICIES.to_owned(), &[]);
    assert!(parsed.diagnostics.is_empty(), "{:#?}", parsed.diagnostics);
    let (validation, trace) = derive_scalar_program_observed_test(&parsed.result, layout);
    assert!(validation.diagnostics.is_empty(), "{:#?}", validation.diagnostics);
    let (module, trace) = if project {
        let (validation, trace) = validate_scalar_project_observed_test(ScalarProject::new(
            vec![ScalarModule::from_program(validation.program, Vec::new())], vec![source.clone()]));
        assert!(validation.diagnostics.is_empty(), "{:#?}", validation.diagnostics);
        (validation.project.modules.into_iter().next().unwrap(), trace)
    } else { (ScalarModule::from_program(validation.program, Vec::new()), trace) };
    let function = module.items.iter().find_map(|item| match item { ScalarItem::Function(function) => Some(function), _ => None }).unwrap();
    let cfg = &function.reference_cfg;
    let pass = trace.passes.iter().rfind(|pass| pass.owner == cfg.producer_owner).unwrap();
    let mut counts = [0; 4];
    for input in &cfg.evaluated_values.binding_inputs {
        let CfgPointKind::Bind { binding } = &cfg.initial_points[input.consumer.0].kind else { panic!("actual aggregate receiver"); };
        let disposition = binding_disposition(&cfg.evaluated_values, input, binding).unwrap();
        let fact = cfg.evaluated_values.values.iter().find(|fact| fact.id == input.source).unwrap();
        let shape = cfg.evaluated_values.shapes.iter().find(|shape| shape.id == fact.shape).unwrap();
        let structure = match &shape.ty {
            ScalarType::Struct(structure) => (structure, false),
            ScalarType::Array { element, .. } => {
                let ScalarType::Struct(structure) = element.as_ref() else { panic!("actual aggregate element declaration"); };
                (structure, true)
            }
            _ => panic!("actual aggregate policy source"),
        };
        let declaration = module.structs.iter().find(|declaration| declaration.id == *structure.0).unwrap();
        assert_eq!(disposition.policy, declaration.copy_policy);
        let index = match (structure.1, disposition.policy) {
            (false, CopyPolicy::Copy) => 0,
            (false, CopyPolicy::Move) => 1,
            (true, CopyPolicy::Copy) => 2,
            (true, CopyPolicy::Move) => 3,
            _ => panic!("actual concrete declaration policy"),
        };
        counts[index] += 1;
        if let CfgPointKind::Read { binding: original } = &cfg.initial_points[input.source.point.0].kind {
            let copies = cfg.binding_points.iter().filter(|point| point.occurrence == input.occurrence
                && point.output == input.output && point.role == ScalarBindingPointRole::CopyCallableFields).collect::<Vec<_>>();
            let [copy] = copies.as_slice() else { panic!("actual aggregate callable-field copy singleton"); };
            assert!(matches!(&cfg.initial_points[copy.point.0].kind, CfgPointKind::CopyCallableFields { source, target }
                if *source == CallableFieldSource::Evaluated(input.source.clone()) && target == binding));
            let snapshots = pass.observations.points.iter().filter(|snapshot| snapshot.point == copy.point
                && snapshot.phase == ReferenceTransferPhase::BeforeInputCleanup).collect::<Vec<_>>();
            assert!(!snapshots.is_empty());
            for snapshot in snapshots {
                let mut fields = 0;
                for relation in &snapshot.state.holder_relations {
                    for holder in &relation.holders {
                        let CallableHolderPlace::Projection(place) = holder else { continue; };
                        if place.binding != *original { continue; }
                        fields += 1;
                        let mut received = place.clone();
                        received.source = binding.source.clone();
                        received.function_span = binding.function_span;
                        received.declaration_span = binding.declaration_span;
                        received.block_span = binding.block_span;
                        received.binding = binding.clone();
                        let received = CallableHolderPlace::Projection(received);
                        assert!(relation.holders.iter().any(|known| holder_place_same(known, &received)));
                        let previous = snapshot.state.holder_provenance.lookup(&relation.environment, holder).unwrap();
                        let HolderProducerRef::Forward { point, action, previous: actual, .. } =
                            snapshot.state.holder_provenance.lookup(&relation.environment, &received).unwrap() else { panic!("actual aggregate Retain Forward"); };
                        assert_eq!(*point, copy.point);
                        assert_eq!(actual.as_ref(), previous);
                        assert!(matches!(action, CallableHolderAction::Retain { environment, from, to, .. }
                            if environment == &relation.environment && holder_place_same(from, holder) && holder_place_same(to, &received)));
                    }
                }
                assert!(fields > 0);
            }
        }
        if let Some(first) = binding_holder::first(cfg, input) {
            assert_eq!(input.source, first.source);
            let mut malformed = cfg.clone();
            malformed.evaluated_values.disposition_inputs.iter_mut().find(|row| row.consumer == disposition.consumer).unwrap().policy = CopyPolicy::Move;
            let before = malformed.clone();
            let mut negative_trace = ReferenceValidationTestTrace::new();
            let errors = transfer_reference_facts(&mut malformed, &pass.context.deferred_policies,
                &pass.context.contracts, &pass.context.literal_contracts, &pass.context.call_exits,
                &pass.context.capture_input,
                pass.role.clone(), &mut negative_trace);
            assert_eq!(errors, vec![ReferenceAnalysisError::MissingContext {
                source_span: input.occurrence.source_span.clone(), reason: ReferenceAnalysisErrorReason::InvalidAssignmentCorrespondence,
            }]);
            assert_eq!(malformed, before);
            assert_eq!(negative_trace.passes.len(), 1);
            assert!(negative_trace.passes[0].observations.points.is_empty());
            assert!(negative_trace.passes[0].observations.borrows.is_empty());
        }
    }
    assert_eq!(counts, [3, 1, 1, 1]);
}

fn prove_repeated(layout: ScalarTargetLayout, project: bool) {
    let source = SourceIdentity::new("direct-repeated-proof".into(), "app".into(), "src/repeated.w".into(), "r1".into());
    let parsed = parse_source(source.clone(), REPEATED.to_owned(), &[]);
    assert!(parsed.diagnostics.is_empty(), "{:#?}", parsed.diagnostics);
    let (validation, trace) = derive_scalar_program_observed_test(&parsed.result, layout);
    assert!(validation.diagnostics.is_empty(), "{:#?}", validation.diagnostics);
    let (module, trace) = if project {
        let (validation, trace) = validate_scalar_project_observed_test(ScalarProject::new(
            vec![ScalarModule::from_program(validation.program, Vec::new())], vec![source.clone()]));
        assert!(validation.diagnostics.is_empty(), "{:#?}", validation.diagnostics);
        (validation.project.modules.into_iter().next().unwrap(), trace)
    } else { (ScalarModule::from_program(validation.program, Vec::new()), trace) };
    let mut subsequent = 0;
    for function in module.items.iter().filter_map(|item| match item { ScalarItem::Function(function) => Some(function), _ => None }) {
        let cfg = &function.reference_cfg;
        let pass = trace.passes.iter().rfind(|pass| pass.owner == cfg.producer_owner).unwrap();
        assert!(pass.errors.is_empty(), "{:#?}", pass.errors);
        for input in &cfg.evaluated_values.binding_inputs {
            let Some(first) = binding_holder::first(cfg, input) else { continue; };
            subsequent += 1;
            assert_eq!(input.source, first.source);
            let CfgPointKind::Bind { binding: receiver } = &cfg.initial_points[input.consumer.0].kind else { panic!("actual receiver"); };
            let CfgPointKind::Bind { binding: first_receiver } = &cfg.initial_points[first.consumer.0].kind else { panic!("actual first receiver"); };
            let original = match &cfg.initial_points[input.source.point.0].kind {
                CfgPointKind::Capture { .. } => CallableHolderPlace::Temporary(input.source.point),
                CfgPointKind::Read { binding } => CallableHolderPlace::Binding(binding.clone()),
                _ => panic!("actual repeated callable source"),
            };
            let consumed = matches!(original, CallableHolderPlace::Temporary(_));
            let designated = if consumed { CallableHolderPlace::Binding(first_receiver.clone()) } else { original.clone() };
            for defect in 0..3 {
                let mut malformed = cfg.clone();
                match defect {
                    0 => malformed.binding_points.retain(|row| row.point != first.consumer || row.role != ScalarBindingPointRole::Bind),
                    1 => {
                        for row in &mut malformed.binding_points {
                            if row.occurrence == input.occurrence && row.role == ScalarBindingPointRole::Bind {
                                if row.point == first.consumer { row.output = input.output; }
                                if row.point == input.consumer { row.output = first.output; }
                            }
                        }
                        for row in &mut malformed.evaluated_values.binding_inputs {
                            if row.consumer == first.consumer { row.output = input.output; }
                            if row.consumer == input.consumer { row.output = first.output; }
                        }
                    }
                    2 => malformed.points[first.consumer.0].successors.clear(),
                    _ => unreachable!(),
                }
                let before = malformed.clone();
                let mut negative_trace = ReferenceValidationTestTrace::new();
                let errors = transfer_reference_facts(&mut malformed, &pass.context.deferred_policies,
                    &pass.context.contracts, &pass.context.literal_contracts, &pass.context.call_exits,
                    &pass.context.capture_input,
                    pass.role.clone(), &mut negative_trace);
                assert_eq!(errors, vec![ReferenceAnalysisError::MissingContext {
                    source_span: input.occurrence.source_span.clone(), reason: ReferenceAnalysisErrorReason::InvalidAssignmentCorrespondence,
                }]);
                assert_eq!(malformed, before);
                assert_eq!(negative_trace.passes.len(), 1);
                assert!(negative_trace.passes[0].observations.points.is_empty());
                assert!(negative_trace.passes[0].observations.borrows.is_empty());
            }
            let snapshots = pass.observations.points.iter().filter(|snapshot| snapshot.point == input.consumer
                && snapshot.phase == ReferenceTransferPhase::BeforeInputCleanup).collect::<Vec<_>>();
            assert!(!snapshots.is_empty());
            for snapshot in snapshots {
                let environments = &snapshot.state.callable_point_values[input.source.point.0];
                assert!(!environments.is_empty());
                assert_eq!(&snapshot.state.callable_bindings[receiver], environments);
                for environment in environments {
                    let donor_chain = snapshot.state.holder_provenance.lookup(environment, &designated).unwrap();
                    if consumed {
                        let HolderProducerRef::Forward { point, action, previous, .. } = donor_chain else { panic!("first Move Forward"); };
                        assert_eq!(*point, first.consumer);
                        assert!(matches!(action, CallableHolderAction::Move { environment: actual, from, to, .. }
                            if actual == environment && from == &original && to == &designated));
                        assert!(matches!(previous.as_ref(), HolderProducerRef::CaptureCreation { point, environment: actual }
                            if *point == input.source.point && actual == environment));
                        assert!(snapshot.state.holder_provenance.lookup(environment, &original).is_none());
                        assert!(!snapshot.state.holder_relations.iter().any(|relation| relation.holders.contains(&original)));
                    }
                    let receiver_holder = CallableHolderPlace::Binding(receiver.clone());
                    let HolderProducerRef::Forward { point, action, previous, .. } = snapshot.state.holder_provenance.lookup(environment, &receiver_holder).unwrap()
                        else { panic!("subsequent Retain Forward"); };
                    assert_eq!(*point, input.consumer);
                    assert_eq!(previous.as_ref(), donor_chain);
                    assert!(matches!(action, CallableHolderAction::Retain { environment: actual, from, to, .. }
                        if actual == environment && from == &designated && to == &receiver_holder));
                    assert!(snapshot.state.holder_relations.iter().any(|relation| relation.environment == *environment
                        && relation.holders.contains(&designated) && relation.holders.contains(&receiver_holder)));
                }
            }
            let calls = cfg.points.iter().filter(|point| matches!(&point.kind,
                CfgPointKind::Call { target: ResolvedCallTarget::LocalCallable(actual), output: 0, .. } if actual == receiver)).collect::<Vec<_>>();
            assert_eq!(calls.len(), 1);
            assert_eq!(calls[0].possible_callee_environments, cfg.points[input.consumer.0].possible_environments);
        }
    }
    assert_eq!(subsequent, 4);
}

fn prove(layout: ScalarTargetLayout, project: bool) {
    let source = SourceIdentity::new("direct-binding-proof".into(), "app".into(), "src/direct.w".into(), "r1".into());
    let parsed = parse_source(source.clone(), SOURCE.to_owned(), &[]);
    assert!(parsed.diagnostics.is_empty(), "{:#?}", parsed.diagnostics);
    let (validation, trace) = derive_scalar_program_observed_test(&parsed.result, layout);
    assert!(validation.diagnostics.is_empty(), "{:#?}", validation.diagnostics);
    let (module, trace) = if project {
        let (validation, trace) = validate_scalar_project_observed_test(ScalarProject::new(
            vec![ScalarModule::from_program(validation.program, Vec::new())], vec![source.clone()]));
        assert!(validation.diagnostics.is_empty(), "{:#?}", validation.diagnostics);
        (validation.project.modules.into_iter().next().unwrap(), trace)
    } else {
        (ScalarModule::from_program(validation.program, Vec::new()), trace)
    };
    let mut captures = 0;
    let mut aggregates = 0;
    let mut repeated = 0;
    for function in module.items.iter().filter_map(|item| match item {
        ScalarItem::Function(function) => Some(function),
        _ => None,
    }) {
        let cfg = &function.reference_cfg;
        let pass = trace.passes.iter().rfind(|pass| pass.owner == cfg.producer_owner).unwrap();
        for binding in cfg.binding_points.iter().filter(|binding| binding.role == ScalarBindingPointRole::Bind) {
            let ScalarBindingOutputSource::Direct { occurrence, output_projection, output } = &binding.source else { panic!("actual Direct fixture binding"); };
            let inputs = cfg.evaluated_values.binding_inputs.iter().filter(|input| input.consumer == binding.point).collect::<Vec<_>>();
            let [input] = inputs.as_slice() else { panic!("actual Direct singleton"); };
            assert_eq!(input.producer, cfg.producer_owner);
            assert_eq!(input.occurrence, binding.occurrence);
            assert_eq!(input.output, binding.output);
            assert_eq!(input.source.resolved.occurrence, *occurrence);
            assert_eq!(input.source.resolved.output_projection, *output_projection);
            assert_eq!(input.source.resolved.output, *output);
            assert_eq!(*output, 0);
            assert_eq!(retained_source_value(cfg, &binding.source, &binding.occurrence.source_span).unwrap(), &input.source);
            assert!(!cfg.evaluated_values.productions.iter().any(|production| production.outputs.contains(&input.source)));
            let fact = cfg.evaluated_values.values.iter().find(|fact| fact.id == input.source).unwrap();
            let CfgPointKind::Bind { binding: receiver } = &cfg.initial_points[input.consumer.0].kind else { panic!("actual receiver event"); };
            let disposition = binding_disposition(&cfg.evaluated_values, input, receiver).unwrap();
            assert_eq!(disposition.source, input.source);
            assert_eq!(disposition.policy, CopyPolicy::Copy);
            assert_eq!(fact.shape.producer, cfg.producer_owner);
            assert_eq!(cfg.initial_points[input.source.point.0].source_span, *occurrence);
            match &cfg.initial_points[input.source.point.0].kind {
                CfgPointKind::Capture { .. } => captures += 1,
                CfgPointKind::Evaluate(ScalarExpression::StructLiteral { .. }) => aggregates += 1,
                CfgPointKind::Evaluate(ScalarExpression::Integer { .. }) => {
                    let receivers = cfg.evaluated_values.binding_inputs.iter().filter(|other| other.source == input.source).collect::<Vec<_>>();
                    assert_eq!(receivers.len(), 2);
                    assert_eq!(receivers[0].output, 0);
                    assert_eq!(receivers[1].output, 1);
                    repeated += 1;
                }
                _ => panic!("actual Direct source event"),
            }
            for mutation in 0..13 {
                if mutation == 12 && !matches!(cfg.initial_points[input.source.point.0].kind, CfgPointKind::Capture { .. }) { continue; }
                let mut malformed = cfg.clone();
                let index = malformed.evaluated_values.binding_inputs.iter().position(|row| row.consumer == input.consumer).unwrap();
                let policy_index = malformed.evaluated_values.disposition_inputs.iter().position(|row| row.consumer == disposition.consumer).unwrap();
                match mutation {
                    0 => { malformed.evaluated_values.binding_inputs.remove(index); }
                    1 => malformed.evaluated_values.binding_inputs.push((*input).clone()),
                    2 => malformed.evaluated_values.binding_inputs[index].source.resolved.output = 1,
                    3 => malformed.evaluated_values.binding_inputs[index].source.resolved.producer = ProducerCfgOwnerId::Module(source.clone()),
                    4 => { malformed.evaluated_values.disposition_inputs.remove(policy_index); }
                    5 => malformed.evaluated_values.disposition_inputs.push(disposition.clone()),
                    6 => malformed.evaluated_values.disposition_inputs[policy_index].source.resolved.output = 1,
                    7 => malformed.evaluated_values.disposition_inputs[policy_index].consumer.producer = ProducerCfgOwnerId::Module(source.clone()),
                    8 => {
                        let ValueConsumerKind::Binding { projection, .. } = &mut malformed.evaluated_values.disposition_inputs[policy_index].consumer.kind else { unreachable!(); };
                        projection.push(ValueProjection::FixedIndex(0));
                    }
                    9 => {
                        let shape = cfg.evaluated_values.shapes.iter().find(|shape| shape.id == fact.shape).unwrap();
                        malformed.evaluated_values.shapes.push(shape.clone());
                    }
                    10 => {
                        malformed.evaluated_values.disposition_inputs[policy_index].consumer.point = CfgPointId(cfg.points.len());
                    }
                    11 => malformed.evaluated_values.shapes.retain(|shape| shape.id != fact.shape),
                    12 => {
                        let crossed = cfg.evaluated_values.shapes.iter().find(|shape| shape.ty == ScalarType::Unit).unwrap();
                        malformed.evaluated_values.values.iter_mut().find(|value| value.id == input.source).unwrap().shape = crossed.id.clone();
                    }
                    _ => unreachable!(),
                }
                let before = malformed.clone();
                let mut negative_trace = ReferenceValidationTestTrace::new();
                let errors = transfer_reference_facts(&mut malformed, &pass.context.deferred_policies,
                    &pass.context.contracts, &pass.context.literal_contracts, &pass.context.call_exits,
                    &pass.context.capture_input,
                    pass.role.clone(), &mut negative_trace);
                assert_eq!(errors, vec![ReferenceAnalysisError::MissingContext {
                    source_span: binding.occurrence.source_span.clone(),
                    reason: ReferenceAnalysisErrorReason::InvalidAssignmentCorrespondence,
                }]);
                assert_eq!(malformed, before);
                assert_eq!(negative_trace.passes.len(), 1);
                assert!(negative_trace.passes[0].observations.points.is_empty());
                assert!(negative_trace.passes[0].observations.borrows.is_empty());
            }
        }
    }
    assert_eq!((captures, aggregates, repeated), (2, 1, 2));
    prove_repeated(layout, project);
    prove_aggregate_policies(layout, project);
    prove_projected_aggregates(layout, project);
    prove_mixed_literal_installations(layout, project);
    prove_module_member_installations(layout, project);
}

#[test]
fn direct_binding_inputs_program_native64() { prove(ScalarTargetLayout::NATIVE64, false); }
#[test]
fn direct_binding_inputs_program_wasm32() { prove(ScalarTargetLayout::WASM32, false); }
#[test]
fn direct_binding_inputs_project_native64() { prove(ScalarTargetLayout::NATIVE64, true); }
#[test]
fn direct_binding_inputs_project_wasm32() { prove(ScalarTargetLayout::WASM32, true); }

use super::*;

const TUPLE: &str = "%%start\nenv = extern wasm \"helper\" { (i32, u64)() read; };\n(i32, u64)() pair = fn { env.read() };\ni32 first, u64 second = pair();\n%%end";
const EFFECTS: &str = "%%start\nenv = extern wasm \"helper\" { (i32, u64)(i32) read; u64() index; };\n(*i32, u64)(*i32) borrow_pair = fn(pointer) { pointer, 0 };\nunit() exercise = fn {\n\ti32[2] values = [1, 2];\n\ti32 first = 0;\n\tu64 second = 0;\n\ti32 got, u64 wide = env.read(values[env.index()]);\n\tgot; wide;\n\tfirst, second = env.read(values[env.index()]);\n\tfirst; second;\n\ti32 target = 3;\n\t*i32 received, u64 tag = borrow_pair(&target);\n\treceived; tag;\n};\n%%end";
const LITERAL: &str = "%%start\nenv = extern wasm \"helper\" { (i32, u64)() read; };\n(i32, u64)() pair = fn { env.read() };\ni32 first, u64 second = pair();\nstruct Readers { (i32, u64)() read; }\nReaders readers = { .read = fn { i32 a, u64 b = env.read(); a, b }; };\n%%end";

const CALLABLE_RECEIVERS: &str = "%%start\n(u8(), u64)() pair = fn { fn { 7 }, 8 };\nu8()() single = fn { fn { 11 } };\nunit() exercise = fn {\n\tu8() read, u64 tag = pair();\n\tread(); tag;\n\tu64 count, u8() independent = 4, single();\n\tcount; independent();\n};\n%%end";

pub(super) fn prove_callable_receivers(layout: ScalarTargetLayout, project: bool) {
    let source = source("src/callable_first_receiver.w", "r1");
    let parsed = parse_source(source.clone(), CALLABLE_RECEIVERS.to_owned(), &[]);
    assert!(parsed.diagnostics.is_empty(), "{:#?}", parsed.diagnostics);
    let (validation, trace) = derive_scalar_program_observed_test(&parsed.result, layout);
    assert!(
        validation.diagnostics.is_empty(),
        "{:#?}",
        validation.diagnostics
    );
    let (module, trace) = if project {
        let (validation, trace) = validate_scalar_project_observed_test(ScalarProject::new(
            vec![ScalarModule::from_program(validation.program, Vec::new())],
            vec![source.clone()],
        ));
        assert!(
            validation.diagnostics.is_empty(),
            "{:#?}",
            validation.diagnostics
        );
        (
            validation.project.modules.into_iter().next().unwrap(),
            trace,
        )
    } else {
        (
            ScalarModule::from_program(validation.program, Vec::new()),
            trace,
        )
    };
    let exercise = module
        .items
        .iter()
        .find_map(|item| match item {
            ScalarItem::Function(function) if function.name == "exercise" => Some(function),
            _ => None,
        })
        .expect("actual exercise");
    let cfg = &exercise.reference_cfg;
    assert_calls(cfg, &source);
    let bindings = exercise
        .body
        .items
        .iter()
        .filter_map(|item| match item {
            ScalarBlockItem::LocalBinding(binding) => Some(binding),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(bindings.len(), 2);
    let tuple = &bindings[0];
    let independent = &bindings[1];
    assert_eq!(
        tuple.output_origin,
        ScalarBindingOutputOrigin::SingleExpression
    );
    assert_eq!(
        independent.output_origin,
        ScalarBindingOutputOrigin::IndependentExpressions
    );
    assert_eq!(cfg.evaluated_values.binding_inputs.len(), 4);
    for (binding, receiver, source_output) in [(tuple, 0, 0), (tuple, 1, 1), (independent, 1, 0)] {
        let rows = cfg
            .evaluated_values
            .binding_inputs
            .iter()
            .filter(|row| row.occurrence == *binding.occurrence && row.output == receiver)
            .collect::<Vec<_>>();
        assert_eq!(rows.len(), 1);
        let row = rows[0];
        assert_eq!(row.producer, cfg.producer_owner);
        assert_eq!(row.source.resolved.output, source_output);
        assert_eq!(row.source.resolved.occurrence.source, source);
        assert_eq!(row.occurrence.source_span.source, source);
        let retained = cfg
            .binding_points
            .iter()
            .filter(|point| {
                point.point == row.consumer && point.role == ScalarBindingPointRole::Bind
            })
            .collect::<Vec<_>>();
        assert_eq!(retained.len(), 1);
        assert_eq!(retained[0].occurrence, row.occurrence);
        assert_eq!(retained[0].output, receiver);
        assert_eq!(
            retained[0].source,
            ScalarBindingOutputSource::Call {
                occurrence: ScalarCallOccurrence {
                    source_span: row.source.resolved.occurrence.clone(),
                    output_projection: row.source.resolved.output_projection.clone(),
                },
                output: source_output,
            }
        );
        let fact = cfg
            .evaluated_values
            .values
            .iter()
            .find(|fact| fact.id == row.source)
            .unwrap();
        assert!(fact.cases.is_empty());
        let shape = cfg
            .evaluated_values
            .shapes
            .iter()
            .find(|shape| shape.id == fact.shape)
            .unwrap();
        assert_eq!(shape.ty, binding.receivers[receiver].ty);
        let production = cfg
            .evaluated_values
            .productions
            .iter()
            .find(|production| production.outputs.contains(&row.source))
            .unwrap();
        assert!(row.consumer > production.outputs.last().unwrap().point);
        assert!(matches!(&cfg.initial_points[row.consumer.0].kind,
            CfgPointKind::Bind { binding: actual } if actual.declaration_span == binding.receivers[receiver].name_span));
    }
    let expression = cfg
        .binding_points
        .iter()
        .find(|point| {
            point.occurrence == *independent.occurrence
                && point.output == 0
                && point.role == ScalarBindingPointRole::Bind
        })
        .expect("actual independent expression receiver");
    assert_eq!(
        expression.source,
        ScalarBindingOutputSource::Direct {
            occurrence: SourceSpan::new(
                source.clone(),
                expression_span(&independent.output_values[0].value)
            ),
            output_projection: Vec::new(),
            output: 0,
        }
    );
    let inputs = cfg
        .evaluated_values
        .binding_inputs
        .iter()
        .filter(|input| input.consumer == expression.point)
        .collect::<Vec<_>>();
    let [input] = inputs.as_slice() else { panic!("actual Direct singleton"); };
    assert_eq!(input.producer, cfg.producer_owner);
    assert_eq!(input.occurrence, *independent.occurrence);
    assert_eq!(input.output, 0);
    assert_eq!(input.source.resolved.output, 0);
    assert_eq!(input.source.resolved.producer, cfg.producer_owner);
    assert_eq!(input.source.resolved.occurrence, SourceSpan::new(source.clone(), expression_span(&independent.output_values[0].value)));
    assert!(input.source.resolved.output_projection.is_empty());
    let fact = cfg.evaluated_values.values.iter().find(|fact| fact.id == input.source).unwrap();
    let shape = cfg.evaluated_values.shapes.iter().find(|shape| shape.id == fact.shape).unwrap();
    assert_eq!(shape.ty, independent.receivers[0].ty);
    assert!(matches!(&cfg.initial_points[input.source.point.0].kind,
        CfgPointKind::Evaluate(ScalarExpression::Integer { .. })));
    assert!(matches!(&cfg.initial_points[input.consumer.0].kind,
        CfgPointKind::Bind { binding } if binding.declaration_span == independent.receivers[0].name_span));
    reference::assert_call_binding_channels(cfg, &trace);
}

fn source(path: &str, revision: &str) -> SourceIdentity {
    SourceIdentity::new(
        "return-output-proof".into(),
        "app".into(),
        path.into(),
        revision.into(),
    )
}

fn derive(
    text: &str,
    source: SourceIdentity,
    layout: ScalarTargetLayout,
    project: bool,
) -> ScalarModule {
    let parsed = parse_source(source.clone(), text.to_owned(), &[]);
    assert!(parsed.diagnostics.is_empty(), "{:#?}", parsed.diagnostics);
    let validated = derive_scalar_program_with_layout(&parsed.result, layout);
    assert!(
        validated.diagnostics.is_empty(),
        "{:#?}",
        validated.diagnostics
    );
    if project {
        let validated = validate_scalar_project(ScalarProject::new(
            vec![ScalarModule::from_program(validated.program, Vec::new())],
            vec![source],
        ));
        assert!(
            validated.diagnostics.is_empty(),
            "{:#?}",
            validated.diagnostics
        );
        validated
            .project
            .modules
            .into_iter()
            .next()
            .expect("actual single module")
    } else {
        ScalarModule::from_program(validated.program, Vec::new())
    }
}

fn assert_calls(cfg: &ScalarReferenceCfg, source: &SourceIdentity) {
    for production in &cfg.evaluated_values.productions {
        let OutputProductionSource::Call {
            occurrence,
            projections,
        } = &production.source
        else {
            panic!("R1b source role")
        };
        assert_eq!(&occurrence.source_span.source, source);
        assert_eq!(production.producer, cfg.producer_owner);
        assert_eq!(production.occurrence, occurrence.source_span);
        assert_eq!(production.output_projection, occurrence.output_projection);
        let calls = cfg
            .call_points
            .iter()
            .filter(|call| call.occurrence == *occurrence)
            .collect::<Vec<_>>();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].evaluated_point, production.evaluation_point);
        assert_eq!(&calls[0].projections, projections);
        assert_eq!(projections.len(), production.outputs.len());
        let CfgPointKind::Call {
            target,
            argument_points,
            expression,
            output: 0,
        } = &cfg.initial_points[production.evaluation_point.0].kind
        else {
            panic!("genuine invocation anchor")
        };
        let ScalarExpression::Call {
            occurrence: actual, ..
        } = expression
        else {
            unreachable!()
        };
        assert_eq!(actual.as_ref(), occurrence);
        for (position, (projection, value)) in
            projections.iter().zip(&production.outputs).enumerate()
        {
            assert_eq!(projection.output, position);
            assert_eq!(value.point, projection.point);
            assert_eq!(value.resolved.output, position);
            assert_eq!(value.resolved.producer, cfg.producer_owner);
            assert_eq!(value.resolved.occurrence, occurrence.source_span);
            assert_eq!(
                value.resolved.output_projection,
                occurrence.output_projection
            );
            let CfgPointKind::Call {
                expression: projected,
                target: projected_target,
                argument_points: frozen,
                output,
            } = &cfg.initial_points[projection.point.0].kind
            else {
                panic!("actual result projection")
            };
            assert_eq!(*output, position);
            assert_eq!(projected, expression);
            assert_eq!(projected_target, target);
            assert_eq!(frozen, argument_points);
            if position > 0 {
                assert_ne!(projection.point, production.evaluation_point);
                assert!(cfg.initial_points[projections[position - 1].point.0]
                    .successors
                    .contains(&projection.point));
            }
            let facts = cfg
                .evaluated_values
                .values
                .iter()
                .filter(|fact| fact.id == *value)
                .collect::<Vec<_>>();
            assert_eq!(facts.len(), 1);
            assert!(facts[0].cases.is_empty());
            let shape = cfg
                .evaluated_values
                .shapes
                .iter()
                .find(|shape| shape.id == facts[0].shape)
                .expect("source shape");
            assert_eq!(shape.id.producer, cfg.producer_owner);
        }
    }
    assert_eq!(
        cfg.call_points.len(),
        cfg.evaluated_values.productions.len()
    );
    for input in &cfg.evaluated_values.final_inputs {
        assert_eq!(input.consumer.producer, cfg.producer_owner);
        let ValueConsumerKind::FinalOutput { output, .. } = &input.consumer.kind else {
            panic!("actual final consumer")
        };
        assert!(matches!(&cfg.initial_points[input.consumer.point.0].kind,
            CfgPointKind::ReturnOutput { output: actual, value_point, .. }
                if actual == output && *value_point == input.source.point));
        assert_eq!(
            cfg.initial_points[input.consumer.point.0].scope,
            ReferenceScopeId(0)
        );
    }
}

pub(super) fn prove_calls(layout: ScalarTargetLayout, project: bool) {
    let module = derive(TUPLE, source("src/call_tuple.w", "r1"), layout, project);
    assert_calls(&module.top_level_reference_cfg, &module.source);
    let pair = module
        .items
        .iter()
        .find_map(|item| match item {
            ScalarItem::Function(function) if function.name == "pair" => Some(function),
            _ => None,
        })
        .expect("actual pair");
    let cfg = &pair.reference_cfg;
    assert_calls(cfg, &module.source);
    assert_eq!(cfg.evaluated_values.productions.len(), 1);
    let production = &cfg.evaluated_values.productions[0];
    assert_eq!(production.outputs.len(), 2);
    assert_eq!(cfg.evaluated_values.final_inputs.len(), 2);
    assert_eq!(
        cfg.initial_points
            .iter()
            .filter(|point| matches!(point.kind, CfgPointKind::ReturnOutput { .. }))
            .count(),
        2
    );
    for (position, expected) in [ScalarType::I32, ScalarType::U64].iter().enumerate() {
        let value = &production.outputs[position];
        let fact = cfg
            .evaluated_values
            .values
            .iter()
            .find(|fact| fact.id == *value)
            .unwrap();
        let shape = cfg
            .evaluated_values
            .shapes
            .iter()
            .find(|shape| shape.id == fact.shape)
            .unwrap();
        assert_eq!(&shape.ty, expected);
        assert_eq!(cfg.evaluated_values.final_inputs[position].source, *value);
        assert_eq!(
            &cfg.evaluated_values.final_inputs[position].receiving_type,
            expected
        );
    }
    assert!(
        matches!(&cfg.initial_points[production.evaluation_point.0].kind,
        CfgPointKind::Call { target: ResolvedCallTarget::ExternCallable { source: actual, .. }, argument_points, .. }
            if actual == &module.source && argument_points.is_empty())
    );
    let effects = derive(EFFECTS, source("src/call_effects.w", "r1"), layout, project);
    let exercise = effects
        .items
        .iter()
        .find_map(|item| match item {
            ScalarItem::Function(function) if function.name == "exercise" => Some(function),
            _ => None,
        })
        .unwrap();
    let cfg = &exercise.reference_cfg;
    assert_calls(cfg, &effects.source);
    let reads = cfg.evaluated_values.productions.iter().filter(|production| matches!(&cfg.initial_points[production.evaluation_point.0].kind,
        CfgPointKind::Call { expression: ScalarExpression::Call { name, .. }, .. } if name == "read")).collect::<Vec<_>>();
    assert_eq!(reads.len(), 2);
    let indices = cfg.call_points.iter().filter(|call| matches!(&cfg.initial_points[call.evaluated_point.0].kind,
        CfgPointKind::Call { expression: ScalarExpression::Call { name, .. }, .. } if name == "index")).collect::<Vec<_>>();
    assert_eq!(
        indices.len(),
        2,
        "one actual selector invocation per independent source read"
    );
    for (read, index) in reads.iter().zip(indices) {
        let CfgPointKind::Call {
            argument_points, ..
        } = &cfg.initial_points[read.evaluation_point.0].kind
        else {
            unreachable!()
        };
        assert_eq!(argument_points.len(), 1);
        assert!(index.evaluated_point < argument_points[0]);
        assert!(argument_points[0] < read.evaluation_point);
        let CfgPointKind::ReadPlace { .. } = cfg.initial_points[argument_points[0].0].kind else {
            panic!("actual indexed argument")
        };
        let indexed = cfg
            .read_points
            .iter()
            .find(|point| point.point == argument_points[0])
            .expect("actual selector correspondence");
        assert_eq!(indexed.indices.len(), 1);
        assert_eq!(indexed.indices[0].point, index.projections[0].point);
    }
    let rhs = cfg
        .evaluated_values
        .assignment_evaluations
        .iter()
        .find(|evaluation| evaluation.rhs.len() == 2)
        .expect("actual tuple assignment RHS");
    assert_eq!(rhs.rhs[0].value, reads[1].outputs[0]);
    assert_eq!(rhs.rhs[1].value, reads[1].outputs[1]);
    let borrowed = cfg.evaluated_values.productions.iter().find(|production| matches!(&cfg.initial_points[production.evaluation_point.0].kind,
        CfgPointKind::Call { expression: ScalarExpression::Call { name, .. }, .. } if name == "borrow_pair")).unwrap();
    assert_eq!(borrowed.outputs.len(), 2);
    assert!(cfg.checked_installs.iter().any(|install| matches!(install.input,
        ReferenceCheckedInstallInput::Evaluated { value_point } if value_point == borrowed.outputs[0].point)));
    assert!(cfg
        .binding_points
        .iter()
        .all(|point| point.point > reads[0].outputs[1].point
            || point.occurrence.source_span.range.end < reads[0].occurrence.range.start));
}

fn literal_module(revision: &str) -> ScalarModule {
    derive(
        LITERAL,
        source("src/call_tuple.w", revision),
        ScalarTargetLayout::NATIVE64,
        false,
    )
}

fn actual_cfgs(module: &ScalarModule) -> Vec<&ScalarReferenceCfg> {
    let mut cfgs = vec![&module.top_level_reference_cfg];
    cfgs.extend(module.items.iter().filter_map(|item| match item {
        ScalarItem::Function(function) => Some(&function.reference_cfg),
        _ => None,
    }));
    cfgs.extend(
        module
            .top_level_reference_cfg
            .literal_cfgs
            .iter()
            .map(|literal| literal.reference_cfg.as_ref()),
    );
    cfgs
}

pub(super) fn prove_roundtrip() {
    let module = literal_module("r1");
    let cfgs = actual_cfgs(&module);
    assert!(cfgs
        .iter()
        .any(|cfg| matches!(cfg.producer_owner, ProducerCfgOwnerId::Literal { .. })));
    assert!(cfgs.iter().any(|cfg| !matches!(
        cfg.producer_owner,
        ProducerCfgOwnerId::Literal { .. }
    ) && !cfg.evaluated_values.binding_inputs.is_empty()));
    assert!(cfgs.iter().any(|cfg| matches!(
        cfg.producer_owner,
        ProducerCfgOwnerId::Literal { .. }
    ) && !cfg.evaluated_values.binding_inputs.is_empty()));
    for cfg in cfgs {
        assert_calls(cfg, &module.source);
        let bytes =
            serde_json::to_vec(&cfg.evaluated_values).expect("real qualified tables serialize");
        let restored: EvaluatedValueTables =
            serde_json::from_slice(&bytes).expect("real qualified tables deserialize");
        assert_eq!(restored, cfg.evaluated_values);
        assert_eq!(restored.binding_inputs, cfg.evaluated_values.binding_inputs);
        let bytes = serde_json::to_vec(cfg).expect("actual retained-origin CFG serializes");
        let restored: ScalarReferenceCfg =
            serde_json::from_slice(&bytes).expect("actual retained-origin CFG deserializes");
        assert_eq!(restored, *cfg);
        assert_eq!(restored.binding_points, cfg.binding_points);
    }
}

pub(super) fn prove_remap() {
    let original = literal_module("r1");
    let target = literal_module("r2");
    let originals = actual_cfgs(&original);
    let targets = actual_cfgs(&target);
    assert_eq!(originals.len(), targets.len());
    assert!(originals.iter().any(|cfg| matches!(
        cfg.producer_owner,
        ProducerCfgOwnerId::Literal { .. }
    ) && !cfg.evaluated_values.binding_inputs.is_empty()));
    for (old, new) in originals.into_iter().zip(targets) {
        assert_ne!(old.producer_owner, new.producer_owner);
        let points = old
            .initial_points
            .iter()
            .map(|point| {
                let matches = new
                    .initial_points
                    .iter()
                    .filter(|candidate| {
                        candidate.source_span.range == point.source_span.range
                            && std::mem::discriminant(&candidate.kind)
                                == std::mem::discriminant(&point.kind)
                            && match (&point.kind, &candidate.kind) {
                                (
                                    CfgPointKind::Call { output: a, .. },
                                    CfgPointKind::Call { output: b, .. },
                                )
                                | (
                                    CfgPointKind::ReturnOutput { output: a, .. },
                                    CfgPointKind::ReturnOutput { output: b, .. },
                                ) => a == b,
                                _ => true,
                            }
                    })
                    .collect::<Vec<_>>();
                assert_eq!(matches.len(), 1, "actual event-role/source correspondence");
                ValuePointRemap {
                    producer: old.producer_owner.clone(),
                    point: point.id,
                    imported: matches[0].id,
                }
            })
            .collect::<Vec<_>>();
        assert_eq!(original.source.project, target.source.project);
        assert_eq!(original.source.package, target.source.package);
        assert_eq!(original.source.path, target.source.path);
        assert_eq!(original.source.revision, "r1");
        assert_eq!(target.source.revision, "r2");
        assert_ne!(original.source, target.source);
        assert_eq!(original.structs.len(), target.structs.len());
        assert_eq!(original.enums.len(), target.enums.len());
        let structures = original.structs.iter().map(|declaration| {
            assert_eq!(declaration.id.source, original.source);
            let matches = target.structs.iter().filter(|candidate|
                candidate.id.source == target.source && candidate.span == declaration.span
                    && candidate.name_span == declaration.name_span && candidate.name == declaration.name).collect::<Vec<_>>();
            let [receiving] = matches.as_slice() else { panic!("unique source-qualified struct declaration"); };
            assert_eq!(declaration.copy_policy, receiving.copy_policy);
            assert_eq!(declaration.copy_modifier_span, receiving.copy_modifier_span);
            assert_eq!(declaration.fields.len(), receiving.fields.len());
            (declaration.id.clone(), receiving.id.clone())
        }).collect::<Vec<_>>();
        let enumerations = original.enums.iter().map(|declaration| {
            assert_eq!(declaration.id.source, original.source);
            let matches = target.enums.iter().filter(|candidate|
                candidate.id.source == target.source && candidate.span == declaration.span
                    && candidate.name_span == declaration.name_span && candidate.name == declaration.name).collect::<Vec<_>>();
            let [receiving] = matches.as_slice() else { panic!("unique source-qualified enum declaration"); };
            assert_eq!(declaration.copy_policy, receiving.copy_policy);
            assert_eq!(declaration.copy_modifier_span, receiving.copy_modifier_span);
            assert_eq!(declaration.variants, receiving.variants);
            (declaration.id.clone(), receiving.id.clone())
        }).collect::<Vec<_>>();
        for declaration in &target.structs {
            assert_eq!(structures.iter().filter(|(_, receiving)| receiving == &declaration.id).count(), 1);
        }
        for declaration in &target.enums {
            assert_eq!(enumerations.iter().filter(|(_, receiving)| receiving == &declaration.id).count(), 1);
        }
        for declaration in &original.structs {
            assert_eq!(structures.iter().filter(|(source, _)| source == &declaration.id).count(), 1);
        }
        for declaration in &original.enums {
            assert_eq!(enumerations.iter().filter(|(source, _)| source == &declaration.id).count(), 1);
        }
        fn corresponding_type(
            before: &ScalarType,
            after: &ScalarType,
            structures: &[(ScalarStructId, ScalarStructId)],
            enumerations: &[(ScalarEnumId, ScalarEnumId)],
        ) -> bool {
            match (before, after) {
                (ScalarType::Unit | ScalarType::Bool | ScalarType::I8 | ScalarType::I16
                    | ScalarType::I32 | ScalarType::I64 | ScalarType::I128 | ScalarType::U8
                    | ScalarType::U16 | ScalarType::U32 | ScalarType::U64 | ScalarType::U128
                    | ScalarType::F32 | ScalarType::F64 | ScalarType::Char | ScalarType::ArtifactId, _) => before == after,
                (ScalarType::Struct(before), ScalarType::Struct(after)) =>
                    structures.iter().filter(|(source, receiving)| source == before && receiving == after).count() == 1,
                (ScalarType::Enum(before), ScalarType::Enum(after)) =>
                    enumerations.iter().filter(|(source, receiving)| source == before && receiving == after).count() == 1,
                (ScalarType::RawPointer(before), ScalarType::RawPointer(after)) =>
                    corresponding_type(before, after, structures, enumerations),
                (ScalarType::CheckedReference { mutability: before_mode, inner: before },
                    ScalarType::CheckedReference { mutability: after_mode, inner: after }) =>
                    before_mode == after_mode && corresponding_type(before, after, structures, enumerations),
                (ScalarType::Array { element: before, length: before_length, length_span: before_length_span, span: before_span },
                    ScalarType::Array { element: after, length: after_length, length_span: after_length_span, span: after_span }) =>
                    before_length == after_length && before_length_span == after_length_span && before_span == after_span
                        && corresponding_type(before, after, structures, enumerations),
                (ScalarType::RuntimeArray { element: before, span: before_span },
                    ScalarType::RuntimeArray { element: after, span: after_span }) =>
                    before_span == after_span && corresponding_type(before, after, structures, enumerations),
                (ScalarType::Callable { outputs: before_outputs, parameters: before_parameters },
                    ScalarType::Callable { outputs: after_outputs, parameters: after_parameters }) =>
                    before_outputs.span == after_outputs.span
                        && before_outputs.outputs.len() == after_outputs.outputs.len()
                        && before_parameters.len() == after_parameters.len()
                        && before_outputs.outputs.iter().zip(&after_outputs.outputs).all(|(before, after)|
                            before.span == after.span && corresponding_type(&before.ty, &after.ty, structures, enumerations))
                        && before_parameters.iter().zip(after_parameters).all(|(before, after)|
                            corresponding_type(before, after, structures, enumerations)),
                _ => false,
            }
        }
        let mut fields = Vec::new();
        for (source, receiving) in &structures {
            let old_declarations = original.structs.iter().filter(|declaration| declaration.id == *source).collect::<Vec<_>>();
            let [old_declaration] = old_declarations.as_slice() else { panic!("unique original struct registry entry"); };
            let new_declarations = target.structs.iter().filter(|declaration| declaration.id == *receiving).collect::<Vec<_>>();
            let [new_declaration] = new_declarations.as_slice() else { panic!("unique destination struct registry entry"); };
            for (ordinal, (before, after)) in old_declaration.fields.iter().zip(&new_declaration.fields).enumerate() {
                assert_eq!(before.id.structure, *source);
                assert_eq!(after.id.structure, *receiving);
                assert_eq!(before.id.index, ordinal);
                assert_eq!(after.id.index, ordinal);
                assert_eq!(before.declaration_index, after.declaration_index);
                assert_eq!(before.name, after.name);
                assert_eq!(before.name_span, after.name_span);
                assert!(corresponding_type(&before.ty, &after.ty, &structures, &enumerations));
                fields.push((before.id.clone(), after.id.clone()));
            }
        }
        assert_eq!(old.evaluated_values.shapes.len(), new.evaluated_values.shapes.len());
        let shapes = old
            .evaluated_values
            .shapes
            .iter()
            .map(|shape| {
                assert_eq!(shape.id.producer, old.producer_owner);
                let matches = new
                    .evaluated_values
                    .shapes
                    .iter()
                    .filter(|candidate| candidate.id.producer == new.producer_owner
                        && corresponding_type(&shape.ty, &candidate.ty, &structures, &enumerations))
                    .collect::<Vec<_>>();
                let [matched] = matches.as_slice() else { panic!("unique actual resolved shape bijection"); };
                (shape.id.clone(), matched.id.clone())
            })
            .collect::<Vec<_>>();
        for shape in &old.evaluated_values.shapes {
            assert_eq!(shapes.iter().filter(|(source, _)| source == &shape.id).count(), 1);
        }
        for shape in &new.evaluated_values.shapes {
            assert_eq!(shape.id.producer, new.producer_owner);
            assert_eq!(shapes.iter().filter(|(_, receiving)| receiving == &shape.id).count(), 1);
        }
        let corresponding_shape = |before: &ValueShapeId, after: &ValueShapeId|
            shapes.iter().filter(|(source, receiving)| source == before && receiving == after).count() == 1;
        for before in &old.evaluated_values.shapes {
            let pairs = shapes.iter().filter(|(source, _)| source == &before.id).collect::<Vec<_>>();
            let [pair] = pairs.as_slice() else { panic!("unique source shape pair"); };
            let targets = new.evaluated_values.shapes.iter().filter(|shape| shape.id == pair.1).collect::<Vec<_>>();
            let [after] = targets.as_slice() else { panic!("unique receiving shape entry"); };
            match (&before.payload, &after.payload) {
                (ValuePayloadShape::Plain(before), ValuePayloadShape::Plain(after)) => assert_eq!(before, after),
                (ValuePayloadShape::Checked { mutability: before_mode, referent: before },
                    ValuePayloadShape::Checked { mutability: after_mode, referent: after }) => {
                    assert_eq!(before_mode, after_mode);
                    assert!(corresponding_shape(before, after));
                }
                (ValuePayloadShape::Raw { pointee: before }, ValuePayloadShape::Raw { pointee: after }) =>
                    assert!(corresponding_shape(before, after)),
                (ValuePayloadShape::Callable { signature: before }, ValuePayloadShape::Callable { signature: after }) =>
                    assert!(corresponding_type(before, after, &structures, &enumerations)),
                (ValuePayloadShape::Struct { structure: before_declaration, fields: before_fields },
                    ValuePayloadShape::Struct { structure: after_declaration, fields: after_fields }) => {
                    assert_eq!(structures.iter().filter(|(source, receiving)|
                        source == before_declaration && receiving == after_declaration).count(), 1);
                    assert_eq!(before_fields.len(), after_fields.len());
                    let declarations = original.structs.iter().filter(|declaration| declaration.id == *before_declaration).collect::<Vec<_>>();
                    let [declaration] = declarations.as_slice() else { panic!("actual payload struct declaration"); };
                    assert_eq!(before_fields.len(), declaration.fields.len());
                    for (ordinal, (before, after)) in before_fields.iter().zip(after_fields).enumerate() {
                        assert_eq!(before.field, declaration.fields[ordinal].id);
                        assert_eq!(fields.iter().filter(|(source, receiving)| source == &before.field && receiving == &after.field).count(), 1);
                        assert!(corresponding_shape(&before.shape, &after.shape));
                    }
                }
                (ValuePayloadShape::Enum { enumeration: before_declaration, variants: before_variants },
                    ValuePayloadShape::Enum { enumeration: after_declaration, variants: after_variants }) => {
                    assert_eq!(enumerations.iter().filter(|(source, receiving)|
                        source == before_declaration && receiving == after_declaration).count(), 1);
                    assert_eq!(before_variants.len(), after_variants.len());
                    let declarations = original.enums.iter().filter(|declaration| declaration.id == *before_declaration).collect::<Vec<_>>();
                    let [declaration] = declarations.as_slice() else { panic!("actual payload enum declaration"); };
                    assert_eq!(before_variants.len(), declaration.variants.len());
                    for (ordinal, (before, after)) in before_variants.iter().zip(after_variants).enumerate() {
                        assert_eq!(before.ordinal, declaration.variants[ordinal].tag as usize);
                        assert_eq!(before.ordinal, after.ordinal);
                        assert_eq!(before.fields.len(), after.fields.len());
                        for (before, after) in before.fields.iter().zip(&after.fields) {
                            assert!(corresponding_shape(before, after));
                        }
                    }
                }
                (ValuePayloadShape::FixedArray { element: before, length: before_length },
                    ValuePayloadShape::FixedArray { element: after, length: after_length }) => {
                    assert_eq!(before_length, after_length);
                    assert!(corresponding_shape(before, after));
                }
                (ValuePayloadShape::RuntimeArray { element: before }, ValuePayloadShape::RuntimeArray { element: after }) =>
                    assert!(corresponding_shape(before, after)),
                _ => panic!("crossed actual payload shape role"),
            }
        }
        let mut receiving_bindings = Vec::new();
        for disposition in &old.evaluated_values.disposition_inputs {
            let ValueConsumerKind::Binding { binding, projection } = &disposition.consumer.kind else { continue; };
            assert_eq!(disposition.consumer.producer, old.producer_owner);
            assert!(projection.is_empty());
            let old_events = old.initial_points.iter().filter(|point| point.id == disposition.consumer.point).collect::<Vec<_>>();
            let [old_event] = old_events.as_slice() else { panic!("unique original Binding event"); };
            assert!(matches!(&old_event.kind, CfgPointKind::Bind { binding: actual } if actual == binding));
            let old_inputs = old.evaluated_values.binding_inputs.iter().filter(|input| input.consumer == old_event.id).collect::<Vec<_>>();
            let [old_input] = old_inputs.as_slice() else { panic!("unique original Binding input"); };
            assert_eq!(old_input.producer, old.producer_owner);
            assert_eq!(old_input.source, disposition.source);
            let old_rows = old.binding_points.iter().filter(|row| row.point == old_event.id && row.role == ScalarBindingPointRole::Bind).collect::<Vec<_>>();
            let [old_row] = old_rows.as_slice() else { panic!("unique original receiving role"); };
            assert_eq!(old_row.occurrence, old_input.occurrence);
            assert_eq!(old_row.output, old_input.output);
            let receiving_points = points.iter().filter(|point| point.producer == old.producer_owner && point.point == old_event.id).collect::<Vec<_>>();
            let [receiving_point] = receiving_points.as_slice() else { panic!("unique mapped receiving point"); };
            let new_events = new.initial_points.iter().filter(|point| point.id == receiving_point.imported).collect::<Vec<_>>();
            let [new_event] = new_events.as_slice() else { panic!("unique destination Binding event"); };
            let CfgPointKind::Bind { binding: receiving } = &new_event.kind else { panic!("actual destination Bind"); };
            assert_eq!(old_event.source_span.range, new_event.source_span.range);
            assert_eq!(binding.function_span, receiving.function_span);
            assert_eq!(binding.declaration_span, receiving.declaration_span);
            assert_eq!(binding.block_span, receiving.block_span);
            assert_eq!(binding.kind, receiving.kind);
            assert_eq!(binding.source, original.source);
            assert_eq!(receiving.source, target.source);
            assert_ne!(binding, receiving);
            let new_inputs = new.evaluated_values.binding_inputs.iter().filter(|input| input.consumer == new_event.id).collect::<Vec<_>>();
            let [new_input] = new_inputs.as_slice() else { panic!("unique destination Binding input"); };
            assert_eq!(new_input.producer, new.producer_owner);
            let new_rows = new.binding_points.iter().filter(|row| row.point == new_event.id && row.role == ScalarBindingPointRole::Bind).collect::<Vec<_>>();
            let [new_row] = new_rows.as_slice() else { panic!("unique destination receiving role"); };
            assert_eq!(new_row.occurrence, new_input.occurrence);
            assert_eq!(new_row.output, new_input.output);
            assert_eq!(old_input.occurrence.source_span.range, new_input.occurrence.source_span.range);
            assert_eq!(old_input.occurrence.output_projection, new_input.occurrence.output_projection);
            assert_eq!(old_input.output, new_input.output);
            assert_eq!(old_input.source.resolved.occurrence.range, new_input.source.resolved.occurrence.range);
            assert_eq!(old_input.source.resolved.output_projection, new_input.source.resolved.output_projection);
            assert_eq!(old_input.source.resolved.output, new_input.source.resolved.output);
            let source_points = points.iter().filter(|point| point.producer == old.producer_owner && point.point == old_input.source.point).collect::<Vec<_>>();
            let [source_point] = source_points.as_slice() else { panic!("unique mapped source point"); };
            assert_eq!(source_point.imported, new_input.source.point);
            assert!(!receiving_bindings.iter().any(|(before, after)|
                (before == binding && after != receiving) || (before != binding && after == receiving)));
            let pair = (binding.clone(), receiving.clone());
            if !receiving_bindings.contains(&pair) { receiving_bindings.push(pair); }
        }
        let remap = graph_correspondence::complete(&original, &target, old, new, points, shapes, receiving_bindings,
            &|before, after| corresponding_type(before, after, &structures, &enumerations));
        let context = graph_correspondence::context(&original, old);
        let sources = capture_table_sources(old, &context).expect("actual original Call closure");
        let forest = graph_correspondence::forest(&original);
        let mapped_graph = remap
            .tables(&sources, &forest)
            .expect("complete real projection remap");
        let mapped = graph_correspondence::mapped_root(&old.producer_owner, &sources, &mapped_graph);
        assert_eq!(mapped.disposition_inputs.len(), old.evaluated_values.disposition_inputs.len());
        for before in &old.evaluated_values.disposition_inputs {
            let consumer = remap.consumer(&before.consumer).unwrap();
            let rows = mapped.disposition_inputs.iter().filter(|row| row.consumer == consumer).collect::<Vec<_>>();
            let [after] = rows.as_slice() else { panic!("unique complete mapped disposition"); };
            assert_eq!(after.source, remap.value(&before.source).unwrap());
            assert_eq!(after.consumer, consumer);
            assert_eq!(after.policy, before.policy);
            assert_eq!(after.source.resolved.occurrence, before.source.resolved.occurrence);
            assert_eq!(after.source.resolved.output_projection, before.source.resolved.output_projection);
            assert_eq!(after.source.resolved.output, before.source.resolved.output);
            assert_eq!(after.source.resolved.producer, new.producer_owner);
            assert_eq!(after.source.point, remap.point(&old.producer_owner, before.source.point).unwrap());
            let before_facts = old.evaluated_values.values.iter().filter(|fact| fact.id == before.source).collect::<Vec<_>>();
            let [before_fact] = before_facts.as_slice() else { panic!("unique original disposition source fact"); };
            let after_facts = mapped.values.iter().filter(|fact| fact.id == after.source).collect::<Vec<_>>();
            let [after_fact] = after_facts.as_slice() else { panic!("unique mapped disposition source fact"); };
            assert_eq!(after_fact.shape, remap.shape(&before_fact.shape).unwrap());
            let before_shapes = old.evaluated_values.shapes.iter().filter(|shape| shape.id == before_fact.shape).collect::<Vec<_>>();
            let [before_shape] = before_shapes.as_slice() else { panic!("unique original disposition shape"); };
            let after_shapes = mapped.shapes.iter().filter(|shape| shape.id == after_fact.shape).collect::<Vec<_>>();
            let [after_shape] = after_shapes.as_slice() else { panic!("unique mapped disposition shape"); };
            assert_eq!(after_shape.ty, before_shape.ty);
            match &after.consumer.kind {
                ValueConsumerKind::Binding { binding, projection } => {
                    assert!(projection.is_empty());
                    assert_eq!(after.consumer.producer, new.producer_owner);
                    assert!(matches!(&new.initial_points[after.consumer.point.0].kind, CfgPointKind::Bind { binding: actual } if actual == binding));
                    let before_inputs = old.evaluated_values.binding_inputs.iter().filter(|input| input.consumer == before.consumer.point).collect::<Vec<_>>();
                    let [before_input] = before_inputs.as_slice() else { panic!("unique original disposition receiving input"); };
                    let after_inputs = mapped.binding_inputs.iter().filter(|input| input.consumer == after.consumer.point).collect::<Vec<_>>();
                    let [after_input] = after_inputs.as_slice() else { panic!("unique mapped disposition receiving input"); };
                    assert_eq!(after_input.source, after.source);
                    assert_eq!(after_input.occurrence, before_input.occurrence);
                    assert_eq!(after_input.output, before_input.output);
                    let receiving_inputs = new.evaluated_values.binding_inputs.iter().filter(|input| input.consumer == after.consumer.point).collect::<Vec<_>>();
                    let [receiving_input] = receiving_inputs.as_slice() else { panic!("unique actual destination receiving input"); };
                    assert_eq!(receiving_input.source.point, after_input.source.point);
                    assert_eq!(receiving_input.output, after_input.output);
                    assert_eq!(receiving_input.source.resolved.output, after_input.source.resolved.output);
                    assert_eq!(receiving_input.source.resolved.output_projection, after_input.source.resolved.output_projection);
                    assert_eq!(receiving_input.source.resolved.occurrence.range, after_input.source.resolved.occurrence.range);
                    let receiving_facts = new.evaluated_values.values.iter().filter(|fact| fact.id == receiving_input.source).collect::<Vec<_>>();
                    let [receiving_fact] = receiving_facts.as_slice() else { panic!("unique actual destination source fact"); };
                    assert_eq!(receiving_fact.shape, after_fact.shape);
                    assert_eq!(new.initial_points[after.consumer.point.0].source_span.range, old.initial_points[before.consumer.point.0].source_span.range);
                }
                ValueConsumerKind::FinalOutput { .. } => {
                    let inputs = mapped.final_inputs.iter().filter(|input| input.consumer == after.consumer).collect::<Vec<_>>();
                    let [input] = inputs.as_slice() else { panic!("unique preserved final disposition input"); };
                    assert_eq!(input.source, after.source);
                }
                _ => panic!("unchanged remap fixture disposition protocol"),
            }
        }
        let bindings = old
            .binding_points
            .iter()
            .map(|binding| {
                remap
                    .binding_point(&old.producer_owner, binding)
                    .expect("actual retained binding point remap")
            })
            .collect::<Vec<_>>();
        assert_eq!(bindings.len(), old.binding_points.len());
        for (before, after) in old.binding_points.iter().zip(&bindings) {
            assert_eq!(after.occurrence, before.occurrence);
            assert_eq!(after.output, before.output);
            assert_eq!(after.role, before.role);
            assert_eq!(after.source, before.source);
            assert_eq!(
                after.point,
                remap.point(&old.producer_owner, before.point).unwrap()
            );
            match &after.source {
                ScalarBindingOutputSource::Direct { occurrence, output_projection, output } => {
                    let source_facts = old.evaluated_values.values.iter().filter(|fact|
                        fact.id.resolved.occurrence == *occurrence
                            && fact.id.resolved.output_projection == *output_projection
                            && fact.id.resolved.output == *output).collect::<Vec<_>>();
                    let [source_fact] = source_facts.as_slice() else { panic!("actual Direct source fact"); };
                    let value = remap.value(&source_fact.id).unwrap();
                    let facts = mapped.values.iter().filter(|fact| fact.id == value).collect::<Vec<_>>();
                    let [fact] = facts.as_slice() else { panic!("mapped Direct value singleton"); };
                    assert_eq!(fact.shape, remap.shape(&source_fact.shape).unwrap());
                    assert_eq!(value.resolved.producer, new.producer_owner);
                    assert_eq!(value.resolved.occurrence, *occurrence);
                    assert_eq!(value.resolved.output_projection, *output_projection);
                    assert_eq!(value.resolved.output, *output);
                    assert_eq!(value.point, remap.point(&old.producer_owner, source_fact.id.point).unwrap());
                    if after.role == ScalarBindingPointRole::Bind {
                        let inputs = mapped.binding_inputs.iter().filter(|input| input.consumer == after.point).collect::<Vec<_>>();
                        let [input] = inputs.as_slice() else { panic!("mapped Direct input singleton"); };
                        assert_eq!(input.source, value);
                        assert_eq!(input.producer, new.producer_owner);
                        assert_eq!(input.occurrence, after.occurrence);
                        assert_eq!(input.output, after.output);
                    }
                }
                ScalarBindingOutputSource::Call { occurrence, output }
                    if after.role == ScalarBindingPointRole::Bind =>
                {
                    let input = mapped
                        .binding_inputs
                        .iter()
                        .find(|input| input.consumer == after.point)
                        .expect("mapped actual Call input");
                    assert_eq!(input.source.resolved.occurrence, occurrence.source_span);
                    assert_eq!(
                        input.source.resolved.output_projection,
                        occurrence.output_projection
                    );
                    assert_eq!(input.source.resolved.output, *output);
                    assert_eq!(input.source.resolved.producer, new.producer_owner);
                }
                ScalarBindingOutputSource::Expression { .. } => assert!(!mapped
                    .binding_inputs
                    .iter()
                    .any(|input| input.consumer == after.point)),
                ScalarBindingOutputSource::Call { .. } => {}
                ScalarBindingOutputSource::Conditional { .. } => {
                    panic!("unchanged Call remap fixture contains unexpected Conditional binding origin")
                }
                ScalarBindingOutputSource::Block { .. } => {
                    panic!("unchanged Call remap fixture contains unexpected Block binding origin")
                }
            }
        }
        assert_eq!(
            mapped.productions.len(),
            old.evaluated_values.productions.len()
        );
        for (before, after) in old
            .evaluated_values
            .productions
            .iter()
            .zip(&mapped.productions)
        {
            assert_eq!(after.producer, new.producer_owner);
            assert_eq!(after.occurrence, before.occurrence);
            assert_eq!(after.output_projection, before.output_projection);
            assert_eq!(
                after.evaluation_point,
                remap
                    .point(&before.producer, before.evaluation_point)
                    .unwrap()
            );
            let OutputProductionSource::Call { projections, .. } = &after.source else {
                unreachable!()
            };
            for (value, projection) in after.outputs.iter().zip(projections) {
                assert_eq!(value.resolved.producer, new.producer_owner);
                assert_eq!(value.point, projection.point);
                assert_eq!(value.resolved.output, projection.output);
                assert!(mapped
                    .values
                    .iter()
                    .any(|fact| fact.id == *value && fact.shape.producer == new.producer_owner));
            }
        }
        for (before, after) in old
            .evaluated_values
            .final_inputs
            .iter()
            .zip(&mapped.final_inputs)
        {
            assert_eq!(after.consumer, remap.consumer(&before.consumer).unwrap());
            assert_eq!(after.source, remap.value(&before.source).unwrap());
            assert_eq!(after.receiving_type, before.receiving_type);
        }
        assert_eq!(
            mapped.binding_inputs.len(),
            old.evaluated_values.binding_inputs.len()
        );
        for (before, after) in old
            .evaluated_values
            .binding_inputs
            .iter()
            .zip(&mapped.binding_inputs)
        {
            assert_eq!(after.producer, new.producer_owner);
            assert_eq!(
                after.consumer,
                remap.point(&before.producer, before.consumer).unwrap()
            );
            assert_eq!(after.occurrence, before.occurrence);
            assert_eq!(after.output, before.output);
            assert_eq!(after.source, remap.value(&before.source).unwrap());
            assert!(mapped.values.iter().any(|fact| fact.id == after.source));
        }
    }
}

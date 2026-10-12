use super::*;
use crate::scalar::reference::conditional_output_production;
use crate::scalar::reference::{assert_control_binding_channels, assert_control_binding_rejection, validate_binding_inputs};

const NESTED: &str = "%%start\n(i32, bool)() pair = fn { if (true) { if (true) { 1, true } else { 2, false } } else { 3, true } };\n(u8())() make_a = fn { u8() inner = fn { 1 }; inner };\n(u8())() make_b = fn { u8() inner = fn { 2 }; inner };\nunit(bool) receive = fn(flag) { u8() target, u8 tag = if (flag) { make_a(), 7 } else { make_b(), 8 }; target(); tag; i32 first, i32 second = if (flag) { 3 } else { 4 }, if (flag) { 5 } else { 6 }; first; second; i32 left, i32 right = if (flag) { 9 } else { 10 }; left; right; };\ni32 module_first, i32 module_second = if (true) { 11 } else { 12 };\n%%end";
const BLOCK: &str = "%%start\n(i32, bool)() pair = fn { { 1, true } };\n(u8())() make_a = fn { u8() inner = fn { 1 }; inner };\nunit() receive = fn { u8() target, u8 tag = { make_a(), 7 }; target(); tag; i32 first, i32 second = { 3 }, { 4 }; first; second; i32 left, i32 right = { 9 }; left; right; };\ni32 module_first, i32 module_second = { 11 };\n%%end";

fn actual_cfgs(module: &ScalarModule) -> Vec<&ScalarReferenceCfg> {
    let mut cfgs = vec![&module.top_level_reference_cfg];
    cfgs.extend(module.items.iter().filter_map(|item| match item {
        ScalarItem::Function(function) => Some(&function.reference_cfg),
        _ => None,
    }));
    let mut index = 0;
    while index < cfgs.len() {
        let cfg = cfgs[index];
        cfgs.extend(cfg.literal_cfgs.iter().map(|literal| literal.reference_cfg.as_ref()));
        index += 1;
    }
    cfgs
}

fn derive(layout: ScalarTargetLayout, project: bool, block: bool, revision: &str) {
    let family = if block { "block_tuple" } else { "nested_conditional_tuple" };
    let source = SourceIdentity::new(
        "return-output-proof".into(), "app".into(), format!("src/{family}.w"), revision.into(),
    );
    let text = if block { BLOCK } else { NESTED };
    let parsed = parse_source(source.clone(), text.to_owned(), &[]);
    assert!(parsed.diagnostics.is_empty(), "{:#?}", parsed.diagnostics);
    let (validation, trace) = derive_scalar_program_observed_test(&parsed.result, layout);
    assert!(validation.diagnostics.is_empty(), "{:#?}", validation.diagnostics);
    let (module, trace) = if project {
        let (validation, trace) = validate_scalar_project_observed_test(ScalarProject::new(
            vec![ScalarModule::from_program(validation.program, Vec::new())], vec![source.clone()],
        ));
        assert!(validation.diagnostics.is_empty(), "{:#?}", validation.diagnostics);
        (validation.project.modules.into_iter().next().unwrap(), trace)
    } else {
        (ScalarModule::from_program(validation.program, Vec::new()), trace)
    };
    let pair = module.items.iter().find_map(|item| match item {
        ScalarItem::Function(function) if function.name == "pair" => Some(function),
        _ => None,
    }).unwrap();
    let cfg = &pair.reference_cfg;
    let productions = cfg.evaluated_values.productions.iter().filter(|production| {
        matches!(production.source, OutputProductionSource::Conditional { .. } | OutputProductionSource::Block { .. })
    }).collect::<Vec<_>>();
    assert_eq!(productions.len(), if block { 1 } else { 2 });
    assert_eq!(cfg.initial_points.iter().filter(|point| matches!(point.kind,
        CfgPointKind::ValueOutputProjection { .. })).count(), productions.len() * 2);
    for production in productions {
        assert_eq!(production.outputs.len(), 2);
        for (position, ty) in [ScalarType::I32, ScalarType::Bool].iter().enumerate() {
            let fact = cfg.evaluated_values.values.iter().find(|fact| fact.id == production.outputs[position]).unwrap();
            assert_eq!(&cfg.evaluated_values.shapes.iter().find(|shape| shape.id == fact.shape).unwrap().ty, ty);
            assert!(trace.construction_fact(cfg, fact).unwrap().cases.is_empty());
            trace.selected_plain_fact(cfg, fact).unwrap();
        }
        let inputs = match &production.source {
            OutputProductionSource::Conditional { inputs, .. } => {
                assert_eq!(inputs.len(), 2);
                inputs.iter().map(|input| (input.predecessor, &input.outputs)).collect::<Vec<_>>()
            }
            OutputProductionSource::Block { scope_exit, inputs, .. } => vec![(*scope_exit, inputs)],
            _ => unreachable!(),
        };
        for (predecessor, inputs) in inputs {
            let transported = conditional_output_production::edge_outputs(
                &cfg.evaluated_values, &cfg.producer_owner, predecessor, production.evaluation_point,
            ).unwrap();
            assert_eq!(transported, inputs.iter().cloned().zip(production.outputs.iter().cloned()).collect::<Vec<_>>());
            let contributors = cfg.evaluated_values.source_contributors.iter().filter(|tuple| {
                tuple.event.point == production.evaluation_point
                    && tuple.children.iter().any(|child| child.event.point == predecessor)
            }).collect::<Vec<_>>();
            assert!(!contributors.is_empty());
            for contributor in contributors {
                let equation = cfg.evaluated_values.source_port_equations.iter().find(|equation| equation.contributor == contributor.id).unwrap();
                let monitor = cfg.evaluated_values.source_port_monitors.iter().find(|monitor| monitor.id == equation.output).unwrap();
                for (position, output) in production.outputs.iter().enumerate() {
                    let port = monitor.ports.iter().find(|port| port.observation.case.value == *output).unwrap();
                    assert!(port.sources.iter().any(|source| source.case.value == inputs[position]));
                }
            }
        }
    }
    let returns = cfg.evaluated_values.final_inputs.iter().collect::<Vec<_>>();
    assert_eq!(returns.len(), 2);
    for (input, ty) in returns.iter().zip([ScalarType::I32, ScalarType::Bool]) {
        assert_eq!(input.receiving_type, ty);
        let fact = cfg.evaluated_values.values.iter().find(|fact| fact.id == input.source).unwrap();
        assert_eq!(cfg.evaluated_values.shapes.iter().find(|shape| shape.id == fact.shape).unwrap().ty, ty);
    }
    if !block { assert!(!cfg.evaluated_values.block_output_aliases.is_empty()); }
    for alias in &cfg.evaluated_values.block_output_aliases {
        let mut crossed = cfg.clone();
        crossed.evaluated_values.block_output_aliases.iter_mut().find(|row| row.selected == alias.selected).unwrap().source.source_span.source = SourceIdentity::new(
            "foreign-source".into(), "app".into(), "src/crossed.w".into(), "r1".into(),
        );
        assert!(conditional_output_production::validate(&crossed).is_err());
    }
    for production in cfg.evaluated_values.productions.iter().filter(|production| matches!(production.source,
        OutputProductionSource::Conditional { .. } | OutputProductionSource::Block { .. })) {
        let mut missing = cfg.clone();
        missing.evaluated_values.productions.iter_mut().find(|row| row.evaluation_point == production.evaluation_point).unwrap().outputs.clear();
        assert!(conditional_output_production::validate(&missing).is_err());
        let mut crossed = cfg.clone();
        let row = crossed.evaluated_values.productions.iter_mut().find(|row| row.evaluation_point == production.evaluation_point).unwrap();
        match &mut row.source {
            OutputProductionSource::Conditional { inputs, .. } => inputs[0].outputs.swap(0, 1),
            OutputProductionSource::Block { inputs, .. } => inputs.swap(0, 1),
            _ => unreachable!(),
        }
        assert!(conditional_output_production::validate(&crossed).is_err());
    }
    let cfgs = actual_cfgs(&module);
    let mut retained = 0;
    let mut independent = 0;
    let mut callable = 0;
    let receiver = module.items.iter().find_map(|item| match item {
        ScalarItem::Function(function) if function.name == "receive" => Some(function),
        _ => None,
    }).unwrap();
    assert_control_binding_channels(&receiver.reference_cfg, &trace);
    let imported_source = SourceIdentity::new("return-output-proof".into(), "app".into(), format!("src/{family}.w"), "r2".into());
    let imported_parsed = parse_source(imported_source.clone(), text.to_owned(), &[]);
    assert!(imported_parsed.diagnostics.is_empty(), "{:#?}", imported_parsed.diagnostics);
    let (imported_validation, _) = derive_scalar_program_observed_test(&imported_parsed.result, layout);
    assert!(imported_validation.diagnostics.is_empty(), "{:#?}", imported_validation.diagnostics);
    let imported_module = if project {
        let (validation, _) = validate_scalar_project_observed_test(ScalarProject::new(
            vec![ScalarModule::from_program(imported_validation.program, Vec::new())], vec![imported_source],
        ));
        assert!(validation.diagnostics.is_empty(), "{:#?}", validation.diagnostics);
        validation.project.modules.into_iter().next().unwrap()
    } else { ScalarModule::from_program(imported_validation.program, Vec::new()) };
    let imported_cfgs = actual_cfgs(&imported_module);
    assert_eq!(cfgs.len(), imported_cfgs.len());
    let pairs = cfgs.into_iter().zip(imported_cfgs).collect::<Vec<_>>();
    for &(cfg, imported) in &pairs {
        conditional_output_production::validate(cfg).unwrap();
        validate_binding_inputs(cfg).unwrap();
        for configuration in cfg.evaluated_values.source_configurations.iter().filter(|row| row.id.owner == cfg.producer_owner) {
            for producer in &configuration.pending {
                assert!(cfg.evaluated_values.source_use_graphs.iter().filter(|graph| graph.id.owner == cfg.producer_owner).any(|graph| {
                    graph.coverage.iter().any(|row| matches!(&row.reason,
                        SourceUseCoverageReason::PendingProducer { configuration: actual, producer: pending }
                            if *actual == configuration.id && pending == producer))
                }));
            }
        }
        let bytes = serde_json::to_vec(cfg).unwrap();
        let restored: ScalarReferenceCfg = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(restored, *cfg);
        let remap = ValueSchemaRemap {
            producers: pairs.iter().map(|(cfg, imported)| (cfg.producer_owner.clone(), imported.producer_owner.clone())).collect(),
            points: pairs.iter().flat_map(|(cfg, imported)| cfg.initial_points.iter().map(|point| {
                let matches = imported.initial_points.iter().filter(|candidate| candidate.source_span.range == point.source_span.range
                    && std::mem::discriminant(&candidate.kind) == std::mem::discriminant(&point.kind)
                    && match (&point.kind, &candidate.kind) {
                        (CfgPointKind::ValueOutputProjection { output: a, .. }, CfgPointKind::ValueOutputProjection { output: b, .. })
                        | (CfgPointKind::Call { output: a, .. }, CfgPointKind::Call { output: b, .. })
                        | (CfgPointKind::ReturnOutput { output: a, .. }, CfgPointKind::ReturnOutput { output: b, .. }) => a == b,
                        _ => true,
                    }).collect::<Vec<_>>();
                let [candidate] = matches.as_slice() else { panic!("total actual event correspondence"); };
                ValuePointRemap { producer: cfg.producer_owner.clone(), point: point.id, imported: candidate.id }
            }).collect::<Vec<_>>()).collect(),
            shapes: pairs.iter().flat_map(|(cfg, imported)| cfg.evaluated_values.shapes.iter().map(|shape| {
                let matches = imported.evaluated_values.shapes.iter().filter(|candidate| candidate.ty == shape.ty).collect::<Vec<_>>();
                let [candidate] = matches.as_slice() else { panic!("actual canonical shape correspondence"); };
                (shape.id.clone(), candidate.id.clone())
            }).collect::<Vec<_>>()).collect(),
            imports: pairs.iter().map(|(cfg, imported)| ValueImportCorrespondence {
                producer: cfg.producer_owner.clone(),
                bindings: cfg.evaluated_values.source_binding_shapes.iter().map(|row| {
                    let matches = imported.evaluated_values.source_binding_shapes.iter().filter(|candidate| {
                        candidate.binding.declaration_span == row.binding.declaration_span
                            && candidate.binding.function_span == row.binding.function_span
                            && candidate.binding.block_span == row.binding.block_span
                            && candidate.binding.kind == row.binding.kind
                    }).collect::<Vec<_>>();
                    let [candidate] = matches.as_slice() else { panic!("actual qualified binding import"); };
                    (row.binding.clone(), candidate.binding.clone())
                }).collect(),
                places: cfg.evaluated_values.source_move_inputs.iter().map(|input| {
                    let matches = imported.evaluated_values.source_move_inputs.iter().filter(|candidate| {
                        candidate.event.point == input.event.point
                            && candidate.source.resolved.occurrence.range == input.source.resolved.occurrence.range
                            && candidate.source.resolved.output_projection == input.source.resolved.output_projection
                            && candidate.source.resolved.output == input.source.resolved.output
                            && candidate.place.projections == input.place.projections
                    }).collect::<Vec<_>>();
                    let [candidate] = matches.as_slice() else { panic!("actual qualified source move place import"); };
                    (input.place.clone(), candidate.place.clone())
                }).collect(),
                origins: Vec::new(), loans: Vec::new(), runtime_values: Vec::new(),
                environments: cfg.environments.iter().enumerate().map(|(index, environment)| {
                    let matches = imported.environments.iter().enumerate().filter(|(_, candidate)| {
                        candidate.literal.range == environment.literal.range && match (&environment.instance, &candidate.instance) {
                            (CallableEnvironmentInstance::Capture, CallableEnvironmentInstance::Capture) => true,
                            (CallableEnvironmentInstance::CallOutput { call: a, output: x, function_span: f },
                                CallableEnvironmentInstance::CallOutput { call: b, output: y, function_span: g }) => a.range == b.range && x == y && f == g,
                            _ => false,
                        }
                    }).collect::<Vec<_>>();
                    let [(target_index, candidate)] = matches.as_slice() else { panic!("actual environment instance import"); };
                    (CallableEnvironmentId { literal: environment.literal.clone(), index },
                        CallableEnvironmentId { literal: candidate.literal.clone(), index: *target_index })
                }).collect(),
                backings: Vec::new(),
                holders: cfg.evaluated_values.values.iter().flat_map(|fact| &fact.cases).flat_map(|case| match &case.payload {
                    EvaluatedPayloadFact::Callable { producers } => producers.as_slice(),
                    _ => &[],
                }).map(|holder| {
                    let mut matches = imported.evaluated_values.values.iter().flat_map(|fact| &fact.cases).flat_map(|case| match &case.payload {
                        EvaluatedPayloadFact::Callable { producers } => producers.as_slice(),
                        _ => &[],
                    }).filter(|candidate| candidate.environment.literal.range == holder.environment.literal.range
                        && candidate.environment.index == holder.environment.index
                        && match (&holder.holder, &candidate.holder) {
                            (CallableHolderPlace::Temporary(a), CallableHolderPlace::Temporary(b)) => a == b,
                            (CallableHolderPlace::Binding(a), CallableHolderPlace::Binding(b)) => a.declaration_span == b.declaration_span
                                && a.function_span == b.function_span && a.block_span == b.block_span && a.kind == b.kind,
                            (CallableHolderPlace::Output(a), CallableHolderPlace::Output(b)) => a == b,
                            _ => false,
                        }).collect::<Vec<_>>();
                    matches.dedup();
                    let [candidate] = matches.as_slice() else { panic!("actual live connected holder import"); };
                    (holder.clone(), (*candidate).clone())
                }).collect(),
            }).collect(),
        };
        for &(old, new) in &pairs {
            match (&old.producer_owner, &new.producer_owner) {
                (ProducerCfgOwnerId::Module(_), ProducerCfgOwnerId::Module(_)) => {}
                (ProducerCfgOwnerId::Function { declaration: a, body: x, .. }, ProducerCfgOwnerId::Function { declaration: b, body: y, .. }) => {
                    assert_eq!(a, b); assert_eq!(x, y);
                }
                (ProducerCfgOwnerId::Literal { enclosing: a, capture_point: x, literal: f },
                    ProducerCfgOwnerId::Literal { enclosing: b, capture_point: y, literal: g }) => {
                    assert_eq!(remap.producer(a).unwrap(), **b);
                    assert_eq!(remap.point(a, *x).unwrap(), *y);
                    assert_eq!(f.range, g.range);
                }
                _ => panic!("actual owner role correspondence"),
            }
        }
        assert_ne!(cfg.producer_owner, imported.producer_owner);
        let context = graph_correspondence::context(&module, cfg);
        let sources = capture_table_sources(cfg, &context).expect("actual original control closure");
        let forest = graph_correspondence::forest(&module);
        let mapped_graph = remap.tables(&sources, &forest).unwrap();
        let mapped = graph_correspondence::mapped_root(&cfg.producer_owner, &sources, &mapped_graph);
        assert_eq!(mapped.values.len(), cfg.evaluated_values.values.len());
        assert_eq!(mapped.block_output_aliases, cfg.evaluated_values.block_output_aliases);
        for (before, after) in cfg.evaluated_values.values.iter().zip(&mapped.values) {
            assert_eq!(after.id, remap.value(&before.id).unwrap());
            assert_eq!(after.id.resolved.producer, imported.producer_owner);
            assert_eq!(after.id.resolved.occurrence, before.id.resolved.occurrence);
            assert_eq!(after.id.resolved.output_projection, before.id.resolved.output_projection);
        }
        for (before, after) in cfg.evaluated_values.source_use_graphs.iter().zip(&mapped.source_use_graphs) {
            assert_eq!(after.id.owner, imported.producer_owner);
            assert_eq!(after.nodes.len(), before.nodes.len());
            assert_eq!(after.coverage.len(), before.coverage.len());
        }
        for binding in cfg.binding_points.iter().filter(|binding| binding.role == ScalarBindingPointRole::Bind) {
            let output = match &binding.source {
                ScalarBindingOutputSource::Conditional { output, .. } if !block => *output,
                ScalarBindingOutputSource::Block { output, .. } if block => *output,
                _ => continue,
            };
            retained += 1;
            assert_control_binding_rejection(cfg, &trace, binding);
            let mapped_binding = remap.binding_point(&cfg.producer_owner, binding).unwrap();
            assert_eq!(mapped_binding.source, binding.source);
            assert_eq!(mapped_binding.point, remap.point(&cfg.producer_owner, binding.point).unwrap());
            let mapped_input = mapped.binding_inputs.iter().find(|input| input.consumer == mapped_binding.point).unwrap();
            assert_eq!(mapped_input.producer, imported.producer_owner);
            assert_eq!(mapped_input.source.resolved.producer, imported.producer_owner);
            let inputs = cfg.evaluated_values.binding_inputs.iter().filter(|input| input.consumer == binding.point).collect::<Vec<_>>();
            let [input] = inputs.as_slice() else { panic!("exact actual retained input"); };
            assert_eq!(input.source.resolved.output, output);
            if binding.output == 1 && output == 0 { independent += 1; }
            let fact = cfg.evaluated_values.values.iter().find(|fact| fact.id == input.source).unwrap();
            if matches!(cfg.evaluated_values.shapes.iter().find(|shape| shape.id == fact.shape).unwrap().ty, ScalarType::Callable { .. }) { callable += 1; }
            let mut missing = cfg.clone();
            missing.evaluated_values.binding_inputs.retain(|known| known.consumer != binding.point);
            assert!(matches!(validate_binding_inputs(&missing), Err(ReferenceAnalysisError::MissingContext {
                source_span, reason: ReferenceAnalysisErrorReason::InvalidAssignmentCorrespondence,
            }) if source_span == binding.occurrence.source_span));
            let mut crossed = cfg.clone();
            crossed.evaluated_values.binding_inputs.iter_mut().find(|known| known.consumer == binding.point).unwrap().source.resolved.producer = ProducerCfgOwnerId::Module(source.clone());
            assert!(matches!(validate_binding_inputs(&crossed), Err(ReferenceAnalysisError::MissingContext {
                source_span, reason: ReferenceAnalysisErrorReason::InvalidAssignmentCorrespondence,
            }) if source_span == binding.occurrence.source_span));
            let mut absent_shape = cfg.clone();
            absent_shape.evaluated_values.shapes.retain(|shape| shape.id != fact.shape);
            assert!(matches!(validate_binding_inputs(&absent_shape), Err(ReferenceAnalysisError::MissingContext {
                source_span, reason: ReferenceAnalysisErrorReason::InvalidAssignmentCorrespondence,
            }) if source_span == binding.occurrence.source_span));
        }
        let mut missing_event = cfg.clone();
        if block { missing_event.block_scope_exits.clear(); } else { missing_event.conditional_points.clear(); }
        if cfg.evaluated_values.productions.iter().any(|production| matches!(production.source,
            OutputProductionSource::Conditional { .. } | OutputProductionSource::Block { .. })) {
            assert!(conditional_output_production::validate(&missing_event).is_err());
        }
    }
    assert_eq!(retained, 8);
    assert_eq!(independent, 3);
    assert_eq!(callable, 1);
}

#[test]
fn return_ordered_output_correspondence_nested_conditional_tuple_scalar_program_native64() { derive(ScalarTargetLayout::NATIVE64, false, false, "r1"); }
#[test]
fn return_ordered_output_correspondence_nested_conditional_tuple_scalar_program_wasm32() { derive(ScalarTargetLayout::WASM32, false, false, "r1"); }
#[test]
fn return_ordered_output_correspondence_nested_conditional_tuple_scalar_project_native64() { derive(ScalarTargetLayout::NATIVE64, true, false, "r1"); }
#[test]
fn return_ordered_output_correspondence_nested_conditional_tuple_scalar_project_wasm32() { derive(ScalarTargetLayout::WASM32, true, false, "r1"); }
#[test]
fn return_ordered_output_correspondence_block_tuple_scalar_program_native64() { derive(ScalarTargetLayout::NATIVE64, false, true, "r1"); }
#[test]
fn return_ordered_output_correspondence_block_tuple_scalar_program_wasm32() { derive(ScalarTargetLayout::WASM32, false, true, "r1"); }
#[test]
fn return_ordered_output_correspondence_block_tuple_scalar_project_native64() { derive(ScalarTargetLayout::NATIVE64, true, true, "r1"); }
#[test]
fn return_ordered_output_correspondence_block_tuple_scalar_project_wasm32() { derive(ScalarTargetLayout::WASM32, true, true, "r1"); }

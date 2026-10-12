use super::*;

fn prove(layout: ScalarTargetLayout, project: bool) {
    let source = SourceIdentity::new(
        "return-producer-proof".into(), "app".into(), "src/simple.w".into(), "r1".into(),
    );
    let parsed = parse_source(source.clone(), SIMPLE.to_owned(), &[]);
    assert!(parsed.diagnostics.is_empty(), "{:#?}", parsed.diagnostics);
    let (validation, trace) = derive_scalar_program_observed_test(&parsed.result, layout);
    assert!(validation.diagnostics.is_empty(), "{:#?}", validation.diagnostics);
    let (items, trace) = if project {
        let (validation, trace) = validate_scalar_project_observed_test(ScalarProject::new(
            vec![ScalarModule::from_program(validation.program, Vec::new())], vec![source],
        ));
        assert!(validation.diagnostics.is_empty(), "{:#?}", validation.diagnostics);
        (validation.project.modules.into_iter().next().expect("actual module").items, trace)
    } else {
        (validation.program.items, trace)
    };
    let functions = items.iter().filter_map(|item| match item {
        ScalarItem::Function(function) if function.name == "choose" => Some(function),
        _ => None,
    }).collect::<Vec<_>>();
    let [function] = functions.as_slice() else { panic!("exact actual choose source"); };
    let cfg = &function.reference_cfg;
    assert_eq!(cfg.evaluated_values.values.len(), 3);
    let mut integers = 0;
    let mut joins = 0;
    for fact in &cfg.evaluated_values.values {
        let construction = trace.construction_fact(cfg, fact).expect("qualified initial source");
        assert!(construction.cases.is_empty());
        trace.assert_construction_rejections(cfg, fact);
        trace.selected_plain_fact(cfg, fact).expect("complete actual selected Plain incidence");
        let point = &cfg.initial_points[fact.id.point.0];
        match &point.kind {
            CfgPointKind::Evaluate(ScalarExpression::Integer { .. }) => integers += 1,
            CfgPointKind::Join => {
                joins += 1;
                let productions = cfg.evaluated_values.productions.iter().filter(|production| {
                    production.evaluation_point == point.id && production.outputs == vec![fact.id.clone()]
                }).collect::<Vec<_>>();
                let [production] = productions.as_slice() else { panic!("actual unique Join production"); };
                let OutputProductionSource::Conditional { inputs, .. } = &production.source else {
                    panic!("actual Conditional source");
                };
                assert_eq!(inputs.len(), 2);
                assert_ne!(inputs[0].predecessor, inputs[1].predecessor);
                for input in inputs {
                    assert_eq!(input.outputs.len(), 1);
                    let contributors = cfg.evaluated_values.source_contributors.iter().filter(|tuple| {
                        tuple.event.point == point.id && tuple.outputs.contains(&fact.id)
                            && tuple.children.iter().any(|child| child.event.point == input.predecessor)
                    }).collect::<Vec<_>>();
                    assert!(!contributors.is_empty(), "qualified actual predecessor {input:?}");
                    for contributor in contributors {
                        let equations = cfg.evaluated_values.source_port_equations.iter().filter(|equation| {
                            equation.contributor == contributor.id
                        }).collect::<Vec<_>>();
                        let [equation] = equations.as_slice() else { panic!("actual Join equation"); };
                        let monitors = cfg.evaluated_values.source_port_monitors.iter().filter(|monitor| {
                            monitor.id == equation.output
                        }).collect::<Vec<_>>();
                        let [monitor] = monitors.as_slice() else { panic!("actual Join monitor"); };
                        let ports = monitor.ports.iter().filter(|port| port.observation.case.value == fact.id).collect::<Vec<_>>();
                        let [port] = ports.as_slice() else { panic!("actual ordered output port"); };
                        assert!(port.sources.iter().any(|source| source.case.value == input.outputs[0]));
                        assert!(port.sources.iter().filter(|source| source.case.value != fact.id)
                            .all(|source| source.case.value == input.outputs[0]));
                    }
                }
            }
            _ => panic!("original SIMPLE source event"),
        }
        let mut missing_case = cfg.clone();
        missing_case.evaluated_values.values.iter_mut().find(|row| row.id == fact.id).unwrap().cases.clear();
        let malformed = missing_case.evaluated_values.values.iter().find(|row| row.id == fact.id).unwrap();
        assert!(trace.selected_plain_fact(&missing_case, malformed).is_err());
        let mut missing_monitor = cfg.clone();
        missing_monitor.evaluated_values.source_port_monitors.clear();
        assert!(trace.selected_plain_fact(&missing_monitor, fact).is_err());
        let mut crossed = fact.clone();
        crossed.shape.producer = ProducerCfgOwnerId::Module(fact.id.resolved.occurrence.source.clone());
        assert!(trace.selected_plain_fact(cfg, &crossed).is_err());
    }
    assert_eq!(integers, 2);
    assert_eq!(joins, 1);
}

#[test]
fn construction_to_selected_case_program_native64() {
    prove(ScalarTargetLayout::NATIVE64, false);
}
#[test]
fn construction_to_selected_case_program_wasm32() {
    prove(ScalarTargetLayout::WASM32, false);
}
#[test]
fn construction_to_selected_case_project_native64() {
    prove(ScalarTargetLayout::NATIVE64, true);
}
#[test]
fn construction_to_selected_case_project_wasm32() {
    prove(ScalarTargetLayout::WASM32, true);
}

use super::*;
use crate::parse_source;

const SOURCE: &str = "%%start\n(i32, i32)() exercise = fn { i32 left = 1; i32 right = 2; left, right = (left = 3), 4 };\nunit() discard = fn { i32 value = 1; value = 2; };\n%%end";

fn prove(layout: ScalarTargetLayout, project: bool) {
    let source = SourceIdentity::new(
        "assignment-occurrence-phase-proof".into(),
        "app".into(),
        "src/phases.w".into(),
        "r1".into(),
    );
    let parsed = parse_source(source.clone(), SOURCE.to_owned(), &[]);
    assert!(parsed.diagnostics.is_empty(), "{:#?}", parsed.diagnostics);
    let (validation, trace) = derive_scalar_program_observed_test(&parsed.result, layout);
    assert!(
        validation.diagnostics.is_empty(),
        "{:#?}",
        validation.diagnostics
    );
    let (items, trace) = if project {
        let (validation, trace) = validate_scalar_project_observed_test(ScalarProject::new(
            vec![ScalarModule::from_program(validation.program, Vec::new())],
            vec![source],
        ));
        assert!(
            validation.diagnostics.is_empty(),
            "{:#?}",
            validation.diagnostics
        );
        (
            validation
                .project
                .modules
                .into_iter()
                .next()
                .expect("source module")
                .items,
            trace,
        )
    } else {
        (validation.program.items, trace)
    };
    let mut functions = 0;
    for item in &items {
        let ScalarItem::Function(function) = item else {
            continue;
        };
        functions += 1;
        let cfg = &function.reference_cfg;
        let protocol = assignment_lifecycle::FrozenAssignmentProtocol::new(cfg).unwrap();
        let pass = trace
            .passes
            .iter()
            .rfind(|pass| {
                pass.owner == cfg.producer_owner && pass.role == ReferenceTransferTestRole::Function
            })
            .expect("actual function transfer");
        assert!(pass.errors.is_empty(), "{:#?}", pass.errors);
        let destinations = match function.name.as_str() {
            "exercise" => 3,
            "discard" => 1,
            _ => panic!("actual source function"),
        };
        assert_eq!(protocol.destinations.len(), destinations);
        let mut phases = 0;
        for destination in &protocol.destinations {
            for consumer in &destination.consumers {
                let point = consumer_point(consumer);
                let before = pass
                    .observations
                    .points
                    .iter()
                    .find(|observation| {
                        observation.point == point
                            && observation.phase == ReferenceTransferPhase::BeforeEvaluation
                    })
                    .expect("actual pending consumer input");
                let context = phase_context(&before.state, &protocol, point)
                    .unwrap()
                    .unwrap();
                assert_eq!(context.usage.consumer, *consumer);
                assert_eq!(context.usage.selection.destination, destination.destination);
                assert_eq!(
                    context.usage.selection.application.frame,
                    before.state.self_frame
                );
                assert_eq!(before.state.self_frame.owner, cfg.producer_owner);
                assert_eq!(
                    before.state.self_frame.variable,
                    SupportedInstanceVariable(0)
                );
                let [successor] = cfg.initial_points[point.0].successors.as_slice() else {
                    panic!("actual straight-line consumer successor")
                };
                let after = pass
                    .observations
                    .points
                    .iter()
                    .find(|observation| {
                        observation.point == *successor
                            && observation.phase == ReferenceTransferPhase::BeforeEvaluation
                    })
                    .expect("actual next consumer input");
                let mut expected = before.state.assignment_applications.clone();
                for application in &mut expected {
                    application
                        .remaining
                        .retain(|usage| usage != &context.usage);
                }
                expected.retain(|application| !application.remaining.is_empty());
                assert_eq!(after.state.assignment_applications, expected);
                let remains = expected
                    .iter()
                    .flat_map(|application| &application.remaining)
                    .any(|usage| usage.selection == context.usage.selection);
                assert_eq!(
                    after
                        .state
                        .assignment_destinations
                        .contains_key(&destination.point),
                    remains
                );
                for application in &expected {
                    for usage in &application.remaining {
                        let row = protocol
                            .destinations
                            .iter()
                            .find(|row| row.destination == usage.selection.destination)
                            .expect("remaining registry destination");
                        assert!(after.state.assignment_destinations[&row.point]
                            .iter()
                            .all(|frozen| frozen.selection == usage.selection));
                    }
                }
                if matches!(consumer, FrozenConsumerRole::Snapshot(_)) {
                    assert!(expected
                        .iter()
                        .flat_map(|application| &application.remaining)
                        .any(|usage| usage.selection == context.usage.selection
                            && matches!(usage.consumer, FrozenConsumerRole::Commit(_))));
                }
                phases += 1;
            }
        }
        assert_eq!(phases, destinations * 3);
    }
    assert_eq!(functions, 2);
}

#[test]
fn assignment_occurrence_phase_program_native64() {
    prove(ScalarTargetLayout::NATIVE64, false);
}
#[test]
fn assignment_occurrence_phase_program_wasm32() {
    prove(ScalarTargetLayout::WASM32, false);
}
#[test]
fn assignment_occurrence_phase_project_native64() {
    prove(ScalarTargetLayout::NATIVE64, true);
}
#[test]
fn assignment_occurrence_phase_project_wasm32() {
    prove(ScalarTargetLayout::WASM32, true);
}

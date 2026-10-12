use super::*;
use crate::parse_source;

const SOURCE: &str = "%%start\nunit(u64) exercise = fn(count) { (u8())[count] items; u64 index = 0; index; items[0] = fn { 1 }; items[1] = fn { 4 }; items[index], items[(index = 1) + 1] = fn { 2 }, fn { 3 }; items; };\n%%end";

fn prove(layout: ScalarTargetLayout, project: bool) {
    let source = SourceIdentity::new(
        "assignment-frozen-context-proof".into(),
        "app".into(),
        "src/frozen.w".into(),
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
                .expect("actual source module")
                .items,
            trace,
        )
    } else {
        (validation.program.items, trace)
    };
    let function = items
        .iter()
        .find_map(|item| match item {
            ScalarItem::Function(function) if function.name == "exercise" => Some(function),
            _ => None,
        })
        .expect("actual source function");
    let cfg = &function.reference_cfg;
    let evaluation = cfg
        .evaluated_values
        .assignment_evaluations
        .iter()
        .find(|evaluation| evaluation.destinations.len() == 2)
        .expect("actual two-destination operation");
    let pass = trace
        .passes
        .iter()
        .rfind(|pass| {
            pass.owner == cfg.producer_owner && pass.role == ReferenceTransferTestRole::Function
        })
        .expect("actual function transfer");
    assert!(pass.errors.is_empty(), "{:#?}", pass.errors);
    let first = &evaluation.destinations[0];
    let second = &evaluation.destinations[1];
    let [input] = first.selector_inputs.as_slice() else {
        panic!("actual first index input")
    };
    let frozen = pass
        .observations
        .points
        .iter()
        .find(|observation| {
            observation.point == first.point
                && observation.phase == ReferenceTransferPhase::BeforeInputCleanup
        })
        .expect("actual frozen first destination");
    let snapshot = pass
        .observations
        .points
        .iter()
        .find(|observation| {
            observation.point == evaluation.previous[0].point
                && observation.phase == ReferenceTransferPhase::BeforeEvaluation
        })
        .expect("actual common snapshot phase");
    assert!(first.point.0 < second.point.0);
    assert!(second.point.0 < snapshot.point.0);
    assert!(evaluation.previous.iter().all(|previous| evaluation
        .writes
        .iter()
        .all(|write| previous.point.0 < write.point.0)));
    let index_change = cfg
        .evaluated_values
        .assignment_evaluations
        .iter()
        .find(|nested| {
            nested.operation.occurrence.source_span.range.start >= first.source_span.range.end
                && nested.operation.occurrence.source_span.range.end <= second.source_span.range.end
        })
        .expect("actual later selector index assignment");
    assert!(index_change
        .writes
        .iter()
        .all(|write| first.point.0 < write.point.0 && write.point.0 < second.point.0));
    let before_rows = &frozen.state.assignment_destinations[&first.point];
    let after_rows = &snapshot.state.assignment_destinations[&first.point];
    assert_eq!(before_rows, after_rows);
    assert!(!after_rows.is_empty());
    for row in after_rows {
        assert_eq!(row.destination, first.id);
        assert_eq!(row.point, first.point);
        let FrozenAssignmentAccess::Indexed { holder, key, guard } = &row.access else {
            panic!("actual owned indexed destination")
        };
        assert_eq!(holder.index_point, input.point);
        assert_eq!(holder.occurrence, input.resolved.occurrence);
        assert_eq!(&holder.place, &row.place);
        assert_eq!(
            key,
            &CallableIndexKeyFact::Evaluated {
                point: input.point,
                occurrence: input.resolved.occurrence.clone(),
                function_span: row.place.function_span,
            }
        );
        let backings = &snapshot.state.backing_bindings[&row.place.binding];
        assert!(backings.iter().any(|backing| backing.value == holder.value
            && backing.owner == holder.owner
            && backing.length == holder.length));
        for partition in backings
            .iter()
            .filter(|backing| {
                backing.value == holder.value
                    && backing.owner == holder.owner
                    && backing.length == holder.length
            })
            .flat_map(|backing| &backing.callable_family)
            .flat_map(|family| &family.partitions)
        {
            assert!(
                guard.iter().all(|relation| partition
                    .representatives
                    .iter()
                    .any(|representative| representative.key == relation.left)
                    && partition
                        .representatives
                        .iter()
                        .any(|representative| representative.key == relation.right)),
                "frozen guard requires an exact later-state key correspondence"
            );
        }
    }
}

#[test]
fn assignment_frozen_context_program_native64() {
    prove(ScalarTargetLayout::NATIVE64, false);
}
#[test]
fn assignment_frozen_context_program_wasm32() {
    prove(ScalarTargetLayout::WASM32, false);
}
#[test]
fn assignment_frozen_context_project_native64() {
    prove(ScalarTargetLayout::NATIVE64, true);
}
#[test]
fn assignment_frozen_context_project_wasm32() {
    prove(ScalarTargetLayout::WASM32, true);
}

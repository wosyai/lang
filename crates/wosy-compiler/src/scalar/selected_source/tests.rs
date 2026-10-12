use super::*;
use crate::parse_source;

const SOURCE: &str = "%%start\nstruct Pair { u64 left; u64 right; }\nPair() make = fn { u64 left = 1; u64 right = 2; Pair pair = { .left = left; .right = right; }; pair };\n%%end";

fn cfg(layout: ScalarTargetLayout, project: bool) -> ScalarReferenceCfg {
    source_cfg(SOURCE, "make", layout, project)
}

pub(super) fn source_cfg(
    text: &str,
    name: &str,
    layout: ScalarTargetLayout,
    project: bool,
) -> ScalarReferenceCfg {
    let source = SourceIdentity::new(
        "selected-source-proof".into(),
        "app".into(),
        "src/main.w".into(),
        "r1".into(),
    );
    let parsed = parse_source(source.clone(), text.to_owned(), &[]);
    assert!(parsed.diagnostics.is_empty(), "{:#?}", parsed.diagnostics);
    let validation = derive_scalar_program_with_layout(&parsed.result, layout);
    assert!(
        validation.diagnostics.is_empty(),
        "{:#?}",
        validation.diagnostics
    );
    let items = if project {
        let validation = validate_scalar_project(ScalarProject::new(
            vec![ScalarModule::from_program(validation.program, Vec::new())],
            vec![source],
        ));
        assert!(
            validation.diagnostics.is_empty(),
            "{:#?}",
            validation.diagnostics
        );
        validation
            .project
            .modules
            .into_iter()
            .next()
            .expect("actual module")
            .items
    } else {
        validation.program.items
    };
    items
        .into_iter()
        .find_map(|item| match item {
            ScalarItem::Function(function) if function.name == name => Some(function.reference_cfg),
            _ => None,
        })
        .expect("actual source function")
}

fn prove(layout: ScalarTargetLayout, project: bool) {
    let cfg = cfg(layout, project);
    validate(&cfg).expect("selected source correspondence");
    assert!(!cfg.evaluated_values.source_configurations.is_empty());
    assert!(cfg
        .evaluated_values
        .source_contributors
        .iter()
        .any(|row| matches!(row.kind, JointProducerRuleKind::Root)));
    assert!(cfg.evaluated_values.values.iter().any(|value| value
        .cases
        .iter()
        .any(|case| matches!(case.payload, EvaluatedPayloadFact::Plain))));
    let aggregates = cfg
        .evaluated_values
        .values
        .iter()
        .flat_map(|value| &value.cases)
        .filter(|case| matches!(case.payload, EvaluatedPayloadFact::FixedAggregate { .. }))
        .collect::<Vec<_>>();
    assert!(!aggregates.is_empty());
    for case in aggregates {
        let path = &cfg.evaluated_values.source_paths[case.path.ordinal];
        let tuple = &cfg.evaluated_values.source_contributors[path.contributor.index];
        assert_eq!(tuple.event, path.event);
        assert_eq!(tuple.output, path.configuration);
        let EvaluatedPayloadFact::FixedAggregate { members } = &case.payload else {
            unreachable!()
        };
        assert_eq!(members.len(), 2);
        let configuration = &cfg.evaluated_values.source_configurations[path.configuration.index];
        for member in members {
            selected_slot(
                &cfg.evaluated_values,
                configuration,
                &SourceCoordinateRole::Evaluated(member.value.clone()),
            )
            .expect("same contributor child");
        }
    }
    assert!(cfg
        .evaluated_values
        .source_configurations
        .iter()
        .any(|row| !row.owns.is_empty()));
}

#[test]
fn selected_source_program_native64() {
    prove(ScalarTargetLayout::NATIVE64, false);
}
#[test]
fn selected_source_program_wasm32() {
    prove(ScalarTargetLayout::WASM32, false);
}
#[test]
fn selected_source_project_native64() {
    prove(ScalarTargetLayout::NATIVE64, true);
}
#[test]
fn selected_source_project_wasm32() {
    prove(ScalarTargetLayout::WASM32, true);
}

#[test]
fn malformed_selected_path_configuration_is_rejected() {
    let mut cfg = cfg(ScalarTargetLayout::NATIVE64, false);
    cfg.evaluated_values.source_paths[0].configuration.index = usize::MAX;
    assert!(validate(&cfg).is_err());
}
#[test]
fn malformed_selected_owner_instance_is_rejected() {
    let mut cfg = cfg(ScalarTargetLayout::NATIVE64, false);
    let row = cfg
        .evaluated_values
        .source_configurations
        .iter_mut()
        .find(|row| !row.owns.is_empty())
        .expect("actual ownership");
    row.owns[0].owner_instance = row.owns[0].payload.clone();
    assert!(validate(&cfg).is_err());
}
#[test]
fn crossed_selected_contributor_child_is_rejected() {
    let mut cfg = cfg(ScalarTargetLayout::NATIVE64, false);
    let row = cfg
        .evaluated_values
        .source_contributors
        .iter_mut()
        .find(|row| !row.children.is_empty())
        .expect("actual step");
    row.children[0].configuration.index = usize::MAX;
    assert!(validate(&cfg).is_err());
}

fn prove_raw(layout: ScalarTargetLayout, project: bool) {
    let cfg = source_cfg("%%start\nstruct Packet { *?u8 data; u64 length; }\nunit() inspect = fn { u8[2] bytes = [4, 5]; *?u8 data = null; unsafe { data = &?bytes[0]; }; Packet packet = { .data = data; .length = 2; }; packet; };\n%%end", "inspect", layout, project);
    validate(&cfg).expect("actual raw selected tuple");
    let cases = cfg
        .evaluated_values
        .values
        .iter()
        .flat_map(|value| &value.cases)
        .filter(|case| {
            matches!(
                case.payload,
                EvaluatedPayloadFact::Raw {
                    provenance: RawValueProvenance::CompilerStorage { .. }
                }
            )
        })
        .collect::<Vec<_>>();
    assert!(!cases.is_empty());
    for case in cases {
        assert!(!case.dependencies.is_empty());
        let path = &cfg.evaluated_values.source_paths[case.path.ordinal];
        let configuration = &cfg.evaluated_values.source_configurations[path.configuration.index];
        assert!(configuration
            .owns
            .iter()
            .any(|owns| owns.state == SourceObligationState::Outstanding));
        assert!(case
            .dependencies
            .iter()
            .any(|dependency| matches!(dependency.action, DependencyAction::BorrowFixed { .. })));
    }
    let cases = cfg
        .evaluated_values
        .values
        .iter()
        .flat_map(|value| &value.cases)
        .filter(|case| {
            matches!(case.payload, EvaluatedPayloadFact::FixedAggregate { .. })
                && !case.dependencies.is_empty()
        })
        .collect::<Vec<_>>();
    assert!(!cases.is_empty());
    let member = cfg
        .evaluated_values
        .source_literal_members
        .iter()
        .flat_map(|row| &row.members)
        .find(|member| {
            let value = cfg
                .evaluated_values
                .values
                .iter()
                .find(|value| value.id == member.value)
                .expect("actual member value");
            matches!(
                cfg.evaluated_values
                    .shapes
                    .iter()
                    .find(|shape| shape.id == value.shape)
                    .expect("actual member shape")
                    .payload,
                ValuePayloadShape::Raw { .. }
            )
        })
        .expect("declared Packet raw member");
    assert!(cases.iter().all(|case| case
        .dependencies
        .iter()
        .all(|dependency| dependency.projection.starts_with(&member.projection))));
}

#[test]
fn selected_raw_aggregate_program_native64() {
    prove_raw(ScalarTargetLayout::NATIVE64, false);
}
#[test]
fn selected_raw_aggregate_program_wasm32() {
    prove_raw(ScalarTargetLayout::WASM32, false);
}
#[test]
fn selected_raw_aggregate_project_native64() {
    prove_raw(ScalarTargetLayout::NATIVE64, true);
}
#[test]
fn selected_raw_aggregate_project_wasm32() {
    prove_raw(ScalarTargetLayout::WASM32, true);
}

#[test]
fn crossed_selected_aggregate_member_view_is_rejected() {
    let mut cfg = cfg(ScalarTargetLayout::NATIVE64, false);
    let row = cfg
        .evaluated_values
        .source_views
        .iter_mut()
        .find(|row| matches!(row.payload, SourcePayloadView::Aggregate(_)))
        .expect("actual aggregate");
    let SourcePayloadView::Aggregate(members) = &mut row.payload else {
        unreachable!()
    };
    members[0].view.index = usize::MAX;
    assert!(validate(&cfg).is_err());
}

#[test]
fn selected_relation_serde_preserves_whole_configuration_and_contributor() {
    let cfg = cfg(ScalarTargetLayout::NATIVE64, false);
    let serialized =
        serde_json::to_string(&cfg.evaluated_values).expect("serialize actual selected arenas");
    let decoded: EvaluatedValueTables =
        serde_json::from_str(&serialized).expect("deserialize actual selected arenas");
    assert_eq!(decoded, cfg.evaluated_values);
    let mut restored = cfg;
    restored.evaluated_values = decoded;
    validate(&restored).expect("restored whole selected tuple");
}

#[test]
fn simultaneous_fresh_supported_classes_require_distinct_variables() {
    let mut cfg = cfg(ScalarTargetLayout::NATIVE64, false);
    let row = cfg
        .evaluated_values
        .source_configurations
        .iter()
        .find(|row| row.instances.len() >= 2)
        .expect("simultaneous source instances");
    let SourceInstanceRef::Supported(first) = &row.instances[0] else {
        unreachable!()
    };
    let SourceInstanceRef::Supported(second) = &row.instances[1] else {
        unreachable!()
    };
    let first = first.index;
    let second = second.index;
    assert_ne!(
        cfg.evaluated_values.source_instances[first].variable,
        cfg.evaluated_values.source_instances[second].variable
    );
    cfg.evaluated_values.source_instances[second].variable =
        cfg.evaluated_values.source_instances[first].variable;
    assert!(validate(&cfg).is_err());
}

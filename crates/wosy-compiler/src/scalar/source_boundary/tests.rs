use super::*;
use crate::parse_source;

const SOURCE: &str = "%%start\nu8(u8) identity = fn(value) { value };\nu8() build = fn { u8 offset = 1; u8(u8) reader = fn(index) { offset + index }; identity(reader(2)) };\n%%end";

fn prove(layout: ScalarTargetLayout, project: bool) {
    let source = SourceIdentity::new(
        "boundary-source-ports".into(),
        "app".into(),
        "src/ports.w".into(),
        "r1".into(),
    );
    let parsed = parse_source(source.clone(), SOURCE.to_owned(), &[]);
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
    let functions = items
        .iter()
        .filter_map(|item| match item {
            ScalarItem::Function(function) => Some(function),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(functions.len(), 2);
    let identity = functions
        .iter()
        .find(|function| function.name == "identity")
        .expect("identity source");
    let cfg = &identity.reference_cfg;
    let [signature] = cfg.evaluated_values.boundary_signatures.as_slice() else {
        panic!("actual declaration signature");
    };
    assert_eq!(signature.owner, cfg.producer_owner);
    assert_eq!(signature.frame.owner, cfg.producer_owner);
    assert!(matches!(
        cfg.initial_points[signature.entry.point.0].kind,
        CfgPointKind::Entry
    ));
    assert_eq!(signature.ports.len(), 2);
    assert_eq!(
        signature.ports[0].role,
        BoundaryPortRole::Parameter {
            ordinal: 0,
            binding: cfg.reference_parameters[0].clone(),
        }
    );
    assert_eq!(signature.ports[1].role, BoundaryPortRole::Output(0));
    validate_signatures(cfg).expect("actual formal signature");
    let mut wrong = cfg.clone();
    wrong.evaluated_values.source_slots[0].id.owner =
        ProducerCfgOwnerId::Module(identity.reference_cfg.points[0].source_span.source.clone());
    assert!(validate_signatures(&wrong).is_err());
    let mut wrong = cfg.clone();
    wrong.evaluated_values.boundary_signatures[0].ports[0].coordinate =
        signature.ports[1].coordinate.clone();
    assert!(validate_signatures(&wrong).is_err());
    let build = functions
        .iter()
        .find(|function| function.name == "build")
        .expect("build source");
    let cfg = &build.reference_cfg;
    assert_eq!(cfg.evaluated_values.call_argument_inputs.len(), 2);
    let mut actual_sources = Vec::new();
    for input in &cfg.evaluated_values.call_argument_inputs {
        validate_call_arguments(cfg, input).expect("source-derived argument input");
        let [argument] = input.arguments.as_slice() else {
            panic!("actual ordered argument");
        };
        assert_eq!(argument.ordinal, 0);
        let value = cfg
            .evaluated_values
            .values
            .iter()
            .find(|value| value.id == argument.source)
            .expect("actual evaluated input");
        let shape = cfg
            .evaluated_values
            .shapes
            .iter()
            .find(|shape| shape.id == argument.shape)
            .expect("actual input shape");
        assert_eq!(value.shape, shape.id);
        assert_eq!(shape.ty, ScalarType::U8);
        actual_sources.push(argument.source.clone());
        let mut wrong = input.clone();
        wrong.arguments[0].ordinal = 1;
        assert!(validate_call_arguments(cfg, &wrong).is_err());
        let mut wrong = input.clone();
        wrong.arguments[0].source.point = input.event.point;
        assert!(validate_call_arguments(cfg, &wrong).is_err());
    }
    assert_ne!(actual_sources[0], actual_sources[1]);
    assert!(matches!(
        cfg.initial_points[actual_sources[0].point.0].kind,
        CfgPointKind::Evaluate(ScalarExpression::Integer { .. })
    ));
    assert!(matches!(
        cfg.initial_points[actual_sources[1].point.0].kind,
        CfgPointKind::Call { output: 0, .. }
    ));
    let [literal] = cfg.literal_cfgs.as_slice() else {
        panic!("actual capture child");
    };
    let child = &literal.reference_cfg;
    let [signature] = child.evaluated_values.boundary_signatures.as_slice() else {
        panic!("actual literal boundary");
    };
    assert_eq!(signature.ports.len(), 3);
    assert_eq!(
        signature.ports[0].role,
        BoundaryPortRole::Parameter {
            ordinal: 0,
            binding: child.reference_parameters[0].clone(),
        }
    );
    let [capture] = literal.captures.as_slice() else {
        panic!("actual offset capture");
    };
    assert_eq!(
        signature.ports[1].role,
        BoundaryPortRole::Capture {
            ordinal: 0,
            binding: capture.binding.clone(),
        }
    );
    assert_eq!(signature.ports[2].role, BoundaryPortRole::Output(0));
    validate_signatures(child).expect("literal boundary signature");
    let encoded = serde_json::to_string(cfg).expect("boundary CFG serialization");
    let decoded: ScalarReferenceCfg =
        serde_json::from_str(&encoded).expect("boundary CFG deserialization");
    assert_eq!(decoded, *cfg);
}

#[test]
fn source_boundary_ports_program_native64() {
    prove(ScalarTargetLayout::NATIVE64, false);
}
#[test]
fn source_boundary_ports_program_wasm32() {
    prove(ScalarTargetLayout::WASM32, false);
}
#[test]
fn source_boundary_ports_project_native64() {
    prove(ScalarTargetLayout::NATIVE64, true);
}
#[test]
fn source_boundary_ports_project_wasm32() {
    prove(ScalarTargetLayout::WASM32, true);
}

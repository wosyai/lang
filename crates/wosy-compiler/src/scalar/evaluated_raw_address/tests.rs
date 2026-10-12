use super::*;
use crate::parse_source;

const THROUGH_SOURCE: &str = "%%start\n*?u8(*?u8) relay = fn(pointer) { *?u8 output = null; unsafe { output = &?*pointer; }; output };\n%%end";

fn prove_through(layout: ScalarTargetLayout, project: bool) {
    let source = SourceIdentity::new(
        "raw-through-pointer-source".into(),
        "app".into(),
        "src/through.w".into(),
        "r1".into(),
    );
    let parsed = parse_source(source.clone(), THROUGH_SOURCE.to_owned(), &[]);
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
            ScalarItem::Function(function) if function.name == "relay" => Some(function),
            _ => None,
        })
        .collect::<Vec<_>>();
    let [function] = functions.as_slice() else {
        panic!("actual through producer");
    };
    let cfg = &function.reference_cfg;
    let [row] = cfg.evaluated_values.raw_address_productions.as_slice() else {
        panic!("actual raw source category");
    };
    assert_eq!(
        production(cfg, row.value.point).expect("total raw source dispatch"),
        row
    );
    let RawAddressInputSource::ThroughPointer { pointer, place } = &row.source else {
        panic!("actual pointer input");
    };
    let ScalarPlace::Dereference {
        pointer: expression,
        ..
    } = reference::reference_root(place)
    else {
        panic!("actual pointer root");
    };
    assert_eq!(
        pointer.resolved.occurrence,
        SourceSpan::new(
            row.value.resolved.occurrence.source.clone(),
            expression_span(expression)
        )
    );
    assert_ne!(pointer, &row.value);
    assert!(pointer.point < row.value.point);
    assert!(cfg
        .evaluated_values
        .values
        .iter()
        .any(|fact| fact.id == *pointer));
    assert!(cfg.evaluated_values.raw_address_inputs.is_empty());
    let mut wrong = row.clone();
    wrong.source = RawAddressInputSource::CompilerStorage;
    assert!(validate_production(cfg, &wrong).is_err());
    let mut wrong = row.clone();
    let RawAddressInputSource::ThroughPointer { pointer, .. } = &mut wrong.source else {
        unreachable!();
    };
    *pointer = row.value.clone();
    assert!(validate_production(cfg, &wrong).is_err());
    let mut duplicate = cfg.clone();
    duplicate
        .evaluated_values
        .raw_address_productions
        .push(row.clone());
    assert!(production(&duplicate, row.value.point).is_err());
    let mut missing = cfg.clone();
    missing.evaluated_values.raw_address_productions.clear();
    assert!(production(&missing, row.value.point).is_err());
    let encoded = serde_json::to_string(cfg).expect("source category serialize");
    let decoded: ScalarReferenceCfg =
        serde_json::from_str(&encoded).expect("source category deserialize");
    assert_eq!(decoded, *cfg);
}

#[test]
fn evaluated_raw_address_through_program_native64() {
    prove_through(ScalarTargetLayout::NATIVE64, false);
}
#[test]
fn evaluated_raw_address_through_program_wasm32() {
    prove_through(ScalarTargetLayout::WASM32, false);
}
#[test]
fn evaluated_raw_address_through_project_native64() {
    prove_through(ScalarTargetLayout::NATIVE64, true);
}
#[test]
fn evaluated_raw_address_through_project_wasm32() {
    prove_through(ScalarTargetLayout::WASM32, true);
}

const SOURCE: &str = "%%start\nstruct Packet { *?u8 data; u64 length; }\n*Packet() make_packet = fn { u64 length = 1; u8[length] bytes; bytes[0] = 4; *?u8 data = null; unsafe { data = &?bytes[0]; }; Packet result = { .data = data; .length = length; }; &result };\nunit() release = fn { u64 length = 1; u8[length] temporary; temporary[0] = 9; };\n%%end";

fn prove(layout: ScalarTargetLayout, project: bool) {
    let source = SourceIdentity::new(
        "raw-address-source-input-proof".into(),
        "app".into(),
        "src/packet.w".into(),
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
            ScalarItem::Function(function) if function.name == "make_packet" => Some(function),
            _ => None,
        })
        .collect::<Vec<_>>();
    let [function] = functions.as_slice() else {
        panic!("actual packet producer");
    };
    let cfg = &function.reference_cfg;
    let [input] = cfg.evaluated_values.raw_address_inputs.as_slice() else {
        panic!("actual raw address input");
    };
    assert_eq!(
        source_input(cfg, input.value.point).expect("connected input"),
        input
    );
    assert_eq!(input.value.resolved.producer, cfg.producer_owner);
    assert_eq!(
        cfg.initial_points[input.read.0].successors,
        vec![input.value.point]
    );
    assert_eq!(
        cfg.initial_points[input.read.0].source_span,
        input.value.resolved.occurrence
    );
    assert_eq!(input.place.source, input.place.binding.source);
    assert_eq!(input.place.projections.len(), 1);
    assert!(
        matches!(&input.place.projections[0], ReferencePlaceProjection::Index(ScalarExpression::Integer { value, .. })
        if value == &BigInt::from(0))
    );
    let pointee = cfg
        .evaluated_values
        .shapes
        .iter()
        .find(|shape| shape.id == input.pointee)
        .expect("actual pointee");
    assert_eq!(pointee.ty, ScalarType::U8);
    let evaluations = cfg
        .evaluated_values
        .assignment_evaluations
        .iter()
        .filter(|row| row.rhs.iter().any(|rhs| rhs.value == input.value))
        .collect::<Vec<_>>();
    let [assignment] = evaluations.as_slice() else {
        panic!("actual replacement consuming raw address");
    };
    assert_eq!(assignment.rhs.len(), 1);
    assert_eq!(assignment.rhs[0].value, input.value);
    let serialized = serde_json::to_string(cfg).expect("full CFG serialize");
    let decoded: ScalarReferenceCfg =
        serde_json::from_str(&serialized).expect("full CFG deserialize");
    assert_eq!(decoded, *cfg);
    let mut missing = cfg.clone();
    missing.evaluated_values.raw_address_inputs.clear();
    assert_eq!(
        source_input(&missing, input.value.point),
        Err(invalid(&input.value.resolved.occurrence))
    );
    let mut duplicate = cfg.clone();
    duplicate
        .evaluated_values
        .raw_address_inputs
        .push(input.clone());
    assert_eq!(
        source_input(&duplicate, input.value.point),
        Err(invalid(&input.value.resolved.occurrence))
    );
    let mut foreign = input.clone();
    foreign.value.resolved.occurrence.source.revision = "foreign".into();
    assert_eq!(
        validate_input(cfg, &foreign),
        Err(invalid(&foreign.value.resolved.occurrence))
    );
    let mut wrong = input.clone();
    wrong.read = input.value.point;
    assert_eq!(
        validate_input(cfg, &wrong),
        Err(invalid(&input.value.resolved.occurrence))
    );
    let mut wrong = input.clone();
    wrong.pointee = cfg
        .evaluated_values
        .values
        .iter()
        .find(|value| value.id == input.value)
        .expect("actual raw value")
        .shape
        .clone();
    assert_eq!(
        validate_input(cfg, &wrong),
        Err(invalid(&input.value.resolved.occurrence))
    );
}

#[test]
fn evaluated_raw_address_input_program_native64() {
    prove(ScalarTargetLayout::NATIVE64, false);
}
#[test]
fn evaluated_raw_address_input_program_wasm32() {
    prove(ScalarTargetLayout::WASM32, false);
}
#[test]
fn evaluated_raw_address_input_project_native64() {
    prove(ScalarTargetLayout::NATIVE64, true);
}
#[test]
fn evaluated_raw_address_input_project_wasm32() {
    prove(ScalarTargetLayout::WASM32, true);
}

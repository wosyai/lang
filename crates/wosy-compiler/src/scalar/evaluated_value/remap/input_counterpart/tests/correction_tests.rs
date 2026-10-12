use super::*;

fn conflict(layout: ScalarTargetLayout, project: bool) {
    let text = "%%start\nstruct copy Pair { u32 item; }\nunit(*?Pair) run = fn(pointer) { pointer; };\n%%end";
    let (source, forest) = original(text, "demand-source", layout, project);
    let (target, target_forest) = original(text, "first-actual", layout, project);
    let (incoming, incoming_forest) = original(text, "incoming-actual", layout, project);
    let first_map = keys(&source, &target);
    let incoming_map = keys(&source, &incoming);
    let headers = source.iter().map(BoundaryTableHeaders::extract).collect::<Vec<_>>();
    let first = first_map.checked_qualified_header_shapes(&headers, &target.iter().map(BoundaryTableHeaders::extract).collect::<Vec<_>>(), &header_counterpart::BoundaryHeaderRoleInput::Independent { source: &source, source_forest: &forest, destination: &target, destination_forest: &target_forest }).expect("first genuine original finite nominal proof");
    let second = incoming_map.checked_qualified_header_shapes(&headers, &incoming.iter().map(BoundaryTableHeaders::extract).collect::<Vec<_>>(), &header_counterpart::BoundaryHeaderRoleInput::Independent { source: &source, source_forest: &forest, destination: &incoming, destination_forest: &incoming_forest }).expect("incoming independently original finite nominal proof");
    let (BoundaryInputTypeEvidence::Independent(first), BoundaryInputTypeEvidence::Independent(second)) = (&first, &second) else { unreachable!(); };
    let nominal = source[0].policy_context.structs.iter().find(|row| row.name == "Pair").expect("actual demanding declaration");
    let shape = source[0].tables.shapes.iter().find(|shape| shape.ty == ScalarType::RawPointer(Box::new(ScalarType::Struct(nominal.id.clone())))).expect("genuine original raw-pointer demand leading to nominal");
    let demands = second.nominal_demands.iter().filter(|demand| demand.root == shape.id).collect::<Vec<_>>();
    let [demand] = demands.as_slice() else { panic!("exact incoming demanding raw shape occurrence"); };
    assert!(second.nominal_demands.iter().any(|other| other.root != demand.root && matches!(&other.counterpart, BoundaryNominalCounterpart::Struct { source, .. } if source.id == nominal.id)));
    let unrelated = first.shapes.iter().find(|row| row.source.ty == ScalarType::Unit).expect("actual unrelated retained owner shape");
    assert_ne!(unrelated.source.id, demand.root);
    let before = (source.clone(), forest.clone(), target.clone(), target_forest.clone(), incoming.clone(), incoming_forest.clone(), first_map.clone(), incoming_map.clone(), first.clone(), second.clone());
    let mut merged = first.clone();
    assert_eq!(types::merge(&mut merged, &[(*demand).clone()], &[], &[], &[unrelated.clone()]), Err(ValueSchemaRemapError::MissingShape(shape.id.clone())));
    assert_eq!(merged, *first);
    assert_eq!((source, forest, target, target_forest, incoming, incoming_forest, first_map, incoming_map, first.clone(), second.clone()), before);
}

#[test]
fn incoming_nominal_demand_native64_program() { conflict(ScalarTargetLayout::NATIVE64, false); }
#[test]
fn incoming_nominal_demand_native64_project() { conflict(ScalarTargetLayout::NATIVE64, true); }
#[test]
fn incoming_nominal_demand_wasm32_program() { conflict(ScalarTargetLayout::WASM32, false); }
#[test]
fn incoming_nominal_demand_wasm32_project() { conflict(ScalarTargetLayout::WASM32, true); }

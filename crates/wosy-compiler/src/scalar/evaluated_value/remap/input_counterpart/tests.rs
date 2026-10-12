use super::*;

fn original(text: &str, unit: &str, layout: ScalarTargetLayout, project: bool) -> (Vec<ValueTableSource>, SourceCaptureConstructionForest) {
    let identity = SourceIdentity::new("u3-i1".into(), unit.into(), "main.w".into(), "r1".into());
    let parsed = crate::parse_source(identity.clone(), text.into(), &[]);
    assert!(parsed.diagnostics.is_empty(), "{:?}", parsed.diagnostics);
    let (validation, _) = derive_scalar_program_observed_test(&parsed.result, layout);
    assert!(validation.diagnostics.is_empty(), "{:?}", validation.diagnostics);
    let module = if project {
        let validation = validate_scalar_project(ScalarProject::new(vec![ScalarModule::from_program(validation.program, Vec::new())], vec![identity]));
        assert!(validation.diagnostics.is_empty(), "{:?}", validation.diagnostics);
        validation.project.modules[0].clone()
    } else { ScalarModule::from_program(validation.program, Vec::new()) };
    let forest = source_boundary::module_preflight_forest(&module.source, &module.items, &module.top_level_reference_cfg, &module.structs, &module.enums, SourceCaptureConstructionPhase::Complete).expect("genuine original whole forest");
    let function = module.items.iter().find_map(|item| match item { ScalarItem::Function(function) if function.name == "run" => Some(function), _ => None }).expect("actual declared root");
    let input = source_boundary::preflight_input(&function.reference_cfg.producer_owner, &forest, BTreeMap::new()).expect("original qualified context");
    (capture_table_sources(&function.reference_cfg, &input.policy_context).expect("original selected header closure"), forest)
}

fn keys(source: &[ValueTableSource], target: &[ValueTableSource]) -> ValueSchemaRemap {
    let mut remap = ValueSchemaRemap { producers: Vec::new(), points: Vec::new(), shapes: Vec::new(), imports: Vec::new() };
    for original in source {
        let targets = target.iter().filter(|row| row.initial_points[0].source_span.range == original.initial_points[0].source_span.range && std::mem::discriminant(&row.producer) == std::mem::discriminant(&original.producer)).collect::<Vec<_>>();
        let [actual] = targets.as_slice() else { panic!("unique actual original declared owner"); };
        remap.producers.push((original.producer.clone(), actual.producer.clone()));
        for point in &original.initial_points {
            let rows = actual.initial_points.iter().filter(|row| row.source_span.range == point.source_span.range && std::mem::discriminant(&row.kind) == std::mem::discriminant(&point.kind)).collect::<Vec<_>>();
            let [target] = rows.as_slice() else { panic!("unique actual point key only;productive constructor checks follow"); };
            remap.points.push(ValuePointRemap { producer: original.producer.clone(), point: point.id, imported: target.id });
        }
        for shape in &original.tables.shapes {
            let rows = actual.tables.shapes.iter().filter(|row| type_key(&shape.ty, &row.ty, &original.policy_context, &actual.policy_context)).collect::<Vec<_>>();
            let [target] = rows.as_slice() else { panic!("unique actual original typed shape key;graph proof mandatory"); };
            remap.shapes.push((shape.id.clone(), target.id.clone()));
        }
        let bindings = original.tables.source_slots.iter().filter_map(|slot| {
            let SourceCoordinateRole::Binding(binding) = &slot.role else { return None; };
            let rows = actual.tables.source_slots.iter().filter_map(|slot| match &slot.role { SourceCoordinateRole::Binding(target) if binding.function_span == target.function_span && binding.declaration_span == target.declaration_span && binding.block_span == target.block_span && binding.kind == target.kind => Some(target), _ => None }).collect::<Vec<_>>();
            let [target] = rows.as_slice() else { panic!("unique original actual binding key"); };
            Some((binding.clone(), (*target).clone()))
        }).collect();
        remap.imports.push(ValueImportCorrespondence { producer: original.producer.clone(), bindings, places: Vec::new(), origins: Vec::new(), loans: Vec::new(), runtime_values: Vec::new(), environments: Vec::new(), backings: Vec::new(), holders: Vec::new() });
    }
    remap
}

fn type_key(a: &ScalarType, b: &ScalarType, source: &SourceCapturePolicyContext, target: &SourceCapturePolicyContext) -> bool {
    match (a, b) {
        (ScalarType::Struct(a), ScalarType::Struct(b)) => source.structs.iter().any(|x| x.id == *a && target.structs.iter().any(|y| y.id == *b && x.name == y.name && x.span == y.span)),
        (ScalarType::Enum(a), ScalarType::Enum(b)) => source.enums.iter().any(|x| x.id == *a && target.enums.iter().any(|y| y.id == *b && x.name == y.name && x.span == y.span)),
        (ScalarType::RawPointer(a), ScalarType::RawPointer(b)) => type_key(a, b, source, target),
        (ScalarType::CheckedReference { mutability: a, inner: x }, ScalarType::CheckedReference { mutability: b, inner: y }) => a == b && type_key(x, y, source, target),
        (ScalarType::Callable { outputs: a, parameters: x }, ScalarType::Callable { outputs: b, parameters: y }) => a.span == b.span && a.outputs.len() == b.outputs.len() && x.len() == y.len() && a.outputs.iter().zip(&b.outputs).all(|(a, b)| a.span == b.span && type_key(&a.ty, &b.ty, source, target)) && x.iter().zip(y).all(|(a, b)| type_key(a, b, source, target)),
        _ => a == b,
    }
}

fn matrix(layout: ScalarTargetLayout, project: bool) {
    let text = "%%start\nstruct copy Pair { u32 item; }\nunit() run = fn { Pair pair = { .item = 1; }; pair.item; };\n%%end";
    let (left, mut forest) = original(text, "left", layout, project);
    let (right, right_forest) = original(text, "right", layout, project);
    let (target, target_forest) = original(text, "target", layout, project);
    forest.roots.extend(right_forest.roots);
    forest.sources.extend(right_forest.sources);
    let mut source = left;
    source.extend(right);
    let remap = keys(&source, &target);
    let before = (source.clone(), forest.clone(), target.clone(), target_forest.clone(), remap.clone());
    let correspondence = remap.boundary_arena_correspondence(&source, &forest, &target, &target_forest).expect("actual L/R to T header core consumes ONE finite declaration/shape proof");
    let BoundaryInputTypeEvidence::Independent(checked) = &correspondence.input_types else { panic!("mandatory original proof"); };
    assert_eq!(checked.source_forest, forest);
    assert_eq!(checked.destination_forest, target_forest);
    assert_eq!(checked.shapes.len(), source.iter().map(|row| row.tables.shapes.len()).sum::<usize>());
    for row in &source {
        assert!(checked.nominals.iter().any(|pair| matches!(pair, BoundaryNominalCounterpart::Struct { owner, .. } if *owner == row.producer)));
        assert!(checked.originals.iter().any(|record| record.owner == row.producer && record.construction == row.resource_construction && record.policy_context == row.policy_context));
        for shape in &row.tables.shapes {
            let actual = target[0].tables.shapes.iter().find(|actual| actual.id == remap.boundary_shape(&shape.id).expect("qualified original shape key")).expect("actual registered shape");
            assert!(checked.type_pair(&row.producer, &shape.ty, &actual.ty));
            assert_eq!(checked.extend_original_types(&remap, &row.producer, &[shape.ty.clone()], &[actual.ty.clone()], &shape.id).expect("mandatory finite extension retains checked prefix and registered root"), *checked);
        }
    }
    let snapshots = source.iter().map(|row| row.resource_construction.clone()).collect::<Vec<_>>();
    let actuals = target.iter().map(|row| row.resource_construction.clone()).collect::<Vec<_>>();
    let retained = retain_creator_types(&remap, &correspondence.input_types, &snapshots, &actuals, &forest, &target_forest).expect("immutable header prefix retained across demanded creator contexts");
    assert_eq!(retained, correspondence.input_types);
    assert_eq!((source, forest, target, target_forest, remap), before);
    let (source, forest) = original("%%start\nunit() run = fn { };\n%%end", "owned", layout, project);
    let remap = keys(&source, &source);
    let full = remap.tables_into(&source, &forest, &source, &forest).expect("actual identity Independent owned full result");
    let witness = remap.independent_boundary_destination(&source, &forest).expect("actual original target witness");
    let (_, correspondence) = witness.checked_headers(&remap, &source, &forest).expect("same checked retained graph result");
    for row in &source {
        let uses = remap.source_use_graphs_into(&row.producer, &row.tables, &source, &forest, &witness).expect("owned source-use evidence with empty capture input");
        let stages = remap.staged_loan_queries_into(&row.producer, &row.tables, &source, &forest, &witness).expect("owned staged evidence with empty capture input");
        let mapped = full.iter().find(|mapped| mapped.source == row.producer).expect("full owned row");
        for resources in [&mapped.resources, &uses.resources, &stages.resources] {
            postvalidate(&remap, &source, &correspondence, resources).expect("original evidence chain repeated on every owned cut");
            assert!(matches!(resources.input_types(), BoundaryInputTypeEvidence::Independent(_)));
        }
        assert_eq!(mapped.tables, row.tables);
    }
    let mut corrupt = correspondence.clone();
    let BoundaryInputTypeEvidence::Independent(checked) = &mut corrupt.input_types else { unreachable!(); };
    checked.types.clear();
    assert_eq!(postvalidate(&remap, &source, &corrupt, &full[0].resources), Err(ValueSchemaRemapError::EmptyTransportSources));
    let copied = remap.tables(&source, &forest).expect("original strict Copied evidence branch");
    assert!(copied.iter().all(|row| *row.resources.input_types() == BoundaryInputTypeEvidence::Transported));
}

#[test]
fn finite_original_type_evidence_native64_program() { matrix(ScalarTargetLayout::NATIVE64, false); }
#[test]
fn finite_original_type_evidence_native64_project() { matrix(ScalarTargetLayout::NATIVE64, true); }
#[test]
fn finite_original_type_evidence_wasm32_program() { matrix(ScalarTargetLayout::WASM32, false); }
#[test]
fn finite_original_type_evidence_wasm32_project() { matrix(ScalarTargetLayout::WASM32, true); }

fn declarations(layout: ScalarTargetLayout, project: bool) {
    for text in [
        "%%start\nstruct Node { *?Node next; }\nunit(*?Node) run = fn(node) { node; };\n%%end",
        "%%start\nstruct copy Pair { u32 item; }\nunit() run = fn { Pair pair = { .item = 1; }; unit() read = fn { pair.item; }; read(); };\n%%end",
    ] {
        let (source, forest) = original(text, "original", layout, project);
        let (target, target_forest) = original(text, "actual", layout, project);
        let remap = keys(&source, &target);
        let evidence = remap.checked_qualified_header_shapes(&source.iter().map(BoundaryTableHeaders::extract).collect::<Vec<_>>(), &target.iter().map(BoundaryTableHeaders::extract).collect::<Vec<_>>(), &header_counterpart::BoundaryHeaderRoleInput::Independent { source: &source, source_forest: &forest, destination: &target, destination_forest: &target_forest }).expect("actual cyclic declaration/shape or Capture place-type roots fully checked before seal");
        let BoundaryInputTypeEvidence::Independent(checked) = &evidence else { unreachable!(); };
        assert!(!checked.nominals.is_empty());
        assert!(!checked.fields.is_empty());
        for row in &source {
            for production in &row.resource_construction.original().evaluated_values.source_capture_productions {
                let actual = target.iter().flat_map(|row| &row.resource_construction.original().evaluated_values.source_capture_productions).find(|actual| actual.event == remap.boundary_point(&production.event).expect("genuine mapped Capture event")).expect("actual original Capture");
                assert!(checked.type_pair(&row.producer, &production.signature, &actual.signature));
                for (source, actual) in production.inputs.iter().zip(&actual.inputs) {
                    assert!(checked.type_pair(&row.producer, &source.capture.ty, &actual.capture.ty));
                    assert!(checked.type_pair(&row.producer, &source.capture.place_type, &actual.capture.place_type));
                }
            }
        }
        for defect in 0..10 {
            let mut candidate = target.clone();
            for row in &mut candidate {
                let declaration = row.policy_context.structs.first_mut().expect("actual nominal declaration");
                match defect {
                    0 => declaration.name.push('x'),
                    1 => declaration.name_span = ByteSpan::new(0, 0),
                    2 => declaration.span = ByteSpan::new(0, 0),
                    3 => declaration.copy_policy = if declaration.copy_policy == CopyPolicy::Move { CopyPolicy::Copy } else { CopyPolicy::Move },
                    4 => declaration.copy_modifier_span = Some(ByteSpan::new(0, 0)),
                    5 => declaration.layout = Some(ScalarLayout { size: u64::MAX, alignment: 1 }),
                    6 => declaration.fields[0].name.push('x'),
                    7 => declaration.fields[0].declaration_index = usize::MAX,
                    8 => declaration.fields[0].offset = Some(u64::MAX),
                    9 => declaration.fields[0].ty = ScalarType::Error,
                    _ => unreachable!(),
                }
            }
            let mut candidate_forest = target_forest.clone();
            for row in &candidate { candidate_forest.sources.iter_mut().find(|actual| actual.producer == row.producer).expect("unique actual candidate original context carrier").policy_context = row.policy_context.clone(); }
            let before = (source.clone(), forest.clone(), target.clone(), target_forest.clone(), candidate.clone(), candidate_forest.clone(), remap.clone());
            let error = remap.checked_qualified_header_shapes(&source.iter().map(BoundaryTableHeaders::extract).collect::<Vec<_>>(), &candidate.iter().map(BoundaryTableHeaders::extract).collect::<Vec<_>>(), &header_counterpart::BoundaryHeaderRoleInput::Independent { source: &source, source_forest: &forest, destination: &candidate, destination_forest: &candidate_forest }).expect_err("all finite original declaration local/edge obligations before seal");
            assert!(matches!(error, ValueSchemaRemapError::MissingShape(ref shape) if source.iter().any(|row| row.producer == shape.producer)));
            assert_eq!((source.clone(), forest.clone(), target.clone(), target_forest.clone(), candidate, candidate_forest, remap.clone()), before);
        }
    }
}

#[test]
fn finite_original_declaration_edges_native64_program() { declarations(ScalarTargetLayout::NATIVE64, false); }
#[test]
fn finite_original_declaration_edges_native64_project() { declarations(ScalarTargetLayout::NATIVE64, true); }
#[test]
fn finite_original_declaration_edges_wasm32_program() { declarations(ScalarTargetLayout::WASM32, false); }
#[test]
fn finite_original_declaration_edges_wasm32_project() { declarations(ScalarTargetLayout::WASM32, true); }

fn enumerations(layout: ScalarTargetLayout, project: bool) {
    let text = "%%start\nenum copy Status { ready; done; }\nunit(Status) run = fn(status) { status; };\n%%end";
    let (source, forest) = original(text, "enum-original", layout, project);
    let (target, target_forest) = original(text, "enum-actual", layout, project);
    let remap = keys(&source, &target);
    let source_headers = source.iter().map(BoundaryTableHeaders::extract).collect::<Vec<_>>();
    let target_headers = target.iter().map(BoundaryTableHeaders::extract).collect::<Vec<_>>();
    let evidence = remap.checked_qualified_header_shapes(&source_headers, &target_headers, &header_counterpart::BoundaryHeaderRoleInput::Independent { source: &source, source_forest: &forest, destination: &target, destination_forest: &target_forest }).expect("original enum complete variants retained by same productive graph algorithm");
    let BoundaryInputTypeEvidence::Independent(checked) = &evidence else { unreachable!(); };
    assert!(checked.nominals.iter().any(|row| matches!(row, BoundaryNominalCounterpart::Enum { source, destination, .. } if source.variants == destination.variants)));
    assert!(checked.fields.is_empty());
    for defect in 0..5 {
        let mut candidate = target.clone();
        for row in &mut candidate {
            let actual = &mut row.policy_context.enums[0];
            match defect {
                0 => actual.variants[0].tag = actual.variants[1].tag,
                1 => actual.variants[0].name.push('x'),
                2 => actual.variants[0].name_span = ByteSpan::new(0, 0),
                3 => actual.variants.reverse(),
                4 => actual.id.source = SourceIdentity::new("foreign".into(), "foreign".into(), "foreign.w".into(), "foreign".into()),
                _ => unreachable!(),
            }
        }
        let mut current_forest = target_forest.clone();
        for row in &candidate { current_forest.sources.iter_mut().find(|actual| actual.producer == row.producer).expect("exact original carrier context").policy_context = row.policy_context.clone(); }
        let before = (source.clone(), forest.clone(), target.clone(), target_forest.clone(), candidate.clone(), current_forest.clone(), remap.clone());
        let result = remap.checked_qualified_header_shapes(&source_headers, &candidate.iter().map(BoundaryTableHeaders::extract).collect::<Vec<_>>(), &header_counterpart::BoundaryHeaderRoleInput::Independent { source: &source, source_forest: &forest, destination: &candidate, destination_forest: &current_forest });
        assert!(matches!(result, Err(ValueSchemaRemapError::MissingShape(ref shape)) if source.iter().any(|row| row.producer == shape.producer)));
        assert_eq!((source.clone(), forest.clone(), target.clone(), target_forest.clone(), candidate, current_forest, remap.clone()), before);
    }
    let mut stale = target_forest.clone();
    stale.roots.clear();
    assert!(remap.checked_qualified_header_shapes(&source_headers, &target_headers, &header_counterpart::BoundaryHeaderRoleInput::Independent { source: &source, source_forest: &forest, destination: &target, destination_forest: &stale }).is_err());
    assert!(remap.validate_qualified_shapes(&source, &source).is_err());
}

#[test]
fn finite_original_enum_evidence_native64_program() { enumerations(ScalarTargetLayout::NATIVE64, false); }
#[test]
fn finite_original_enum_evidence_native64_project() { enumerations(ScalarTargetLayout::NATIVE64, true); }
#[test]
fn finite_original_enum_evidence_wasm32_program() { enumerations(ScalarTargetLayout::WASM32, false); }
#[test]
fn finite_original_enum_evidence_wasm32_project() { enumerations(ScalarTargetLayout::WASM32, true); }

fn templates(layout: ScalarTargetLayout, project: bool) {
    let text = "%%start\ngeneric T;\nunit(T) run = fn(item) { unit() read = fn { item; }; read(); };\n%%end";
    let (source, forest) = original(text, "template", layout, project);
    let remap = keys(&source, &source);
    let headers = source.iter().map(BoundaryTableHeaders::extract).collect::<Vec<_>>();
    let evidence = remap.checked_qualified_header_shapes(&headers, &headers, &header_counterpart::BoundaryHeaderRoleInput::Independent { source: &source, source_forest: &forest, destination: &source, destination_forest: &forest }).expect("actual original declared template scope is retained exactly");
    let BoundaryInputTypeEvidence::Independent(checked) = evidence else { unreachable!(); };
    assert!(checked.types.iter().any(|row| matches!(row.source, ScalarType::Named { .. }) && row.source == row.destination));
    let (foreign, foreign_forest) = original(text, "foreign-template", layout, project);
    let foreign_remap = keys(&source, &foreign);
    let before = (source.clone(), forest.clone(), foreign.clone(), foreign_forest.clone(), foreign_remap.clone());
    let result = foreign_remap.checked_qualified_header_shapes(&headers, &foreign.iter().map(BoundaryTableHeaders::extract).collect::<Vec<_>>(), &header_counterpart::BoundaryHeaderRoleInput::Independent { source: &source, source_forest: &forest, destination: &foreign, destination_forest: &foreign_forest });
    assert!(matches!(result, Err(ValueSchemaRemapError::MissingShape(ref shape)) if source.iter().any(|row| row.producer == shape.producer)));
    assert_eq!((source, forest, foreign, foreign_forest, foreign_remap), before);
}

#[test]
fn finite_original_template_evidence_native64_program() { templates(ScalarTargetLayout::NATIVE64, false); }
#[test]
fn finite_original_template_evidence_native64_project() { templates(ScalarTargetLayout::NATIVE64, true); }
#[test]
fn finite_original_template_evidence_wasm32_program() { templates(ScalarTargetLayout::WASM32, false); }
#[test]
fn finite_original_template_evidence_wasm32_project() { templates(ScalarTargetLayout::WASM32, true); }

mod correction_tests;

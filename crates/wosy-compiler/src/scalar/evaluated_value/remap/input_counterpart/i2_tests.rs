use super::*;

fn original(text: &str, unit: &str, layout: ScalarTargetLayout, project: bool) -> (Vec<ValueTableSource>, SourceCaptureConstructionForest) {
    let identity = SourceIdentity::new("i2-original-input".into(), unit.into(), "main.w".into(), "r1".into());
    let parsed = crate::parse_source(identity.clone(), text.into(), &[]);
    assert!(parsed.diagnostics.is_empty(), "{:?}", parsed.diagnostics);
    let (validation, _) = derive_scalar_program_observed_test(&parsed.result, layout);
    assert!(validation.diagnostics.is_empty(), "{:?}", validation.diagnostics);
    let module = if project {
        let validation = validate_scalar_project(ScalarProject::new(vec![ScalarModule::from_program(validation.program, Vec::new())], vec![identity]));
        assert!(validation.diagnostics.is_empty(), "{:?}", validation.diagnostics);
        validation.project.modules[0].clone()
    } else { ScalarModule::from_program(validation.program, Vec::new()) };
    let forest = source_boundary::module_preflight_forest(&module.source, &module.items, &module.top_level_reference_cfg, &module.structs, &module.enums, SourceCaptureConstructionPhase::Complete).expect("whole genuine original module/function/literal forest");
    let root = module.items.iter().find_map(|item| match item { ScalarItem::Function(function) if function.name == "run" => Some(function), _ => None }).expect("real selected original caller");
    let input = source_boundary::preflight_input(&root.reference_cfg.producer_owner, &forest, BTreeMap::new()).expect("actual original qualified caller input");
    (capture_table_sources(&root.reference_cfg, &input.policy_context).expect("real selected Capture tree"), forest)
}

fn keys(source: &SourceCaptureConstructionForest, target: &SourceCaptureConstructionForest) -> ValueSchemaRemap {
    let mut remap = ValueSchemaRemap { producers: Vec::new(), points: Vec::new(), shapes: Vec::new(), imports: Vec::new() };
    for row in &source.sources {
        let candidates = target.sources.iter().filter(|candidate| std::mem::discriminant(&candidate.producer) == std::mem::discriminant(&row.producer)
            && match (&row.producer, &candidate.producer) {
                (ProducerCfgOwnerId::Module(_), ProducerCfgOwnerId::Module(_)) => true,
                (ProducerCfgOwnerId::Function { declaration: a, body: x, .. }, ProducerCfgOwnerId::Function { declaration: b, body: y, .. }) => a == b && x == y,
                (ProducerCfgOwnerId::Literal { literal: a, capture_point: x, .. }, ProducerCfgOwnerId::Literal { literal: b, capture_point: y, .. }) => a.range == b.range && x == y,
                _ => false,
            }).collect::<Vec<_>>();
        let [actual] = candidates.as_slice() else { panic!("one actual independently parsed provisional owner key"); };
        remap.producers.push((row.producer.clone(), actual.producer.clone()));
        for point in &row.initial_points {
            let candidates = actual.initial_points.iter().filter(|candidate| candidate.source_span.range == point.source_span.range && std::mem::discriminant(&candidate.kind) == std::mem::discriminant(&point.kind)).collect::<Vec<_>>();
            let [actual] = candidates.as_slice() else { panic!("one actual provisional point key"); };
            remap.points.push(ValuePointRemap { producer: row.producer.clone(), point: point.id, imported: actual.id });
        }
        for shape in &row.tables.shapes {
            let candidates = actual.tables.shapes.iter().filter(|candidate| c3_key_builders::type_key(&shape.ty, &candidate.ty, &row.policy_context, &actual.policy_context)).collect::<Vec<_>>();
            let [actual] = candidates.as_slice() else { panic!("one actual provisional whole type key"); };
            remap.shapes.push((shape.id.clone(), actual.id.clone()));
        }
        remap.imports.push(ValueImportCorrespondence { producer: row.producer.clone(), bindings: Vec::new(), places: Vec::new(), origins: Vec::new(), loans: Vec::new(), runtime_values: Vec::new(), environments: Vec::new(), backings: Vec::new(), holders: Vec::new() });
    }
    let source_catalog = original::catalog(&source.sources).expect("all original provisional resource keys");
    let target_catalog = original::catalog(&target.sources).expect("all actual original provisional resource keys");
    for import in &mut remap.imports {
        let actual_owner = remap.producers.iter().find(|pair| pair.0 == import.producer).expect("explicit original provisional owner pair").1.clone();
        let sources = source_catalog.vertices.iter().filter(|vertex| vertex.reference.owner == import.producer).collect::<Vec<_>>();
        let targets = target_catalog.vertices.iter().filter(|vertex| vertex.reference.owner == actual_owner).collect::<Vec<_>>();
        assert_eq!(sources.len(), targets.len());
        for (source, target) in sources.iter().zip(targets) {
            match (&source.reference.key, &target.reference.key) {
                (BoundaryResourceKey::Origin(a), BoundaryResourceKey::Origin(b)) => { assert_eq!(a.0, import.origins.len()); import.origins.push(*b); }
                (BoundaryResourceKey::Binding(a), BoundaryResourceKey::Binding(b)) => import.bindings.push((a.clone(), b.clone())),
                (BoundaryResourceKey::Place(a), BoundaryResourceKey::Place(b)) => import.places.push((a.clone(), b.clone())),
                (BoundaryResourceKey::Loan(a), BoundaryResourceKey::Loan(b)) => import.loans.push((a.clone(), b.clone())),
                (BoundaryResourceKey::Runtime(a), BoundaryResourceKey::Runtime(b)) => import.runtime_values.push((a.clone(), b.clone())),
                (BoundaryResourceKey::Environment(a), BoundaryResourceKey::Environment(b)) => import.environments.push((a.clone(), b.clone())),
                (BoundaryResourceKey::Backing(a), BoundaryResourceKey::Backing(b)) => import.backings.push((a.clone(), b.clone())),
                (BoundaryResourceKey::Holder(a), BoundaryResourceKey::Holder(b)) => import.holders.push((a.clone(), b.clone())),
                _ => panic!("provisional typed catalogs have distinct constructor kinds"),
            }
        }
    }
    remap
}

fn direct(text: &str, layout: ScalarTargetLayout, project: bool) {
    let (left, mut source_forest) = original(text, "left", layout, project);
    let (right, right_forest) = original(text, "right", layout, project);
    let (target, target_forest) = original(text, "target", layout, project);
    source_forest.roots.extend(right_forest.roots);
    source_forest.sources.extend(right_forest.sources);
    let mut source = left; source.extend(right);
    let remap = keys(&source_forest, &target_forest);
    let before = (source.clone(), source_forest.clone(), target.clone(), target_forest.clone(), remap.clone());
    let correspondence = remap.boundary_arena_correspondence(&source, &source_forest, &target, &target_forest).expect("original L/R to actual target typed AST roots and Effect header factory");
    let source_catalog = original::catalog(&source_forest.sources).expect("real source catalog");
    let target_catalog = original::catalog(&target_forest.sources).expect("real target catalog");
    let sources = reservation::ResourceReservations::original(&source_catalog);
    let targets = reservation::ResourceReservations::original(&target_catalog);
    let mut associations = Vec::new();
    for row in &sources.vertices {
        let actual = allocation::proposed(&remap, &row.source).expect("actual provisional resource key through complete real imports");
        associations.push(BoundaryResourceAssociation { source: row.source.clone(), destination: actual.clone(), source_vertex: row.source_vertex.clone(), destination_vertex: targets.destination(&actual).expect("actual original target catalog key") });
    }
    let source_originals = source_forest.sources.iter().map(|row| row.resource_construction.clone()).collect::<Vec<_>>();
    let actual_originals = target_forest.sources.iter().map(|row| row.resource_construction.clone()).collect::<Vec<_>>();
    let types = input_counterpart::retain_creator_types(&remap, &correspondence.input_types, &source_originals, &actual_originals, &source_forest, &target_forest).expect("all demanded genuine original creator/type/AST/signature extensions");
    let tuples = original_capture_tuples(&source).expect("whole genuine original source tuples");
    let actual_tuples = original_capture_tuples(&target).expect("whole genuine original actual tuples");
    for source in &tuples {
        let transported = allocation::mapped_tuple(&remap, source, &correspondence, &associations, true).expect("unchanged canonical tuple constructor");
        let candidates = actual_tuples.iter().filter(|actual| actual.production.event == transported.production.event).collect::<Vec<_>>();
        let [actual] = candidates.as_slice() else { panic!("one genuine actual Capture tuple"); };
        capture_counterpart::validate(&remap, source, &transported, actual, &correspondence, &source_originals, &actual_originals, &types, &associations).expect("productive original Capture input visitor with complete mandatory original evidence");
        assert_ne!(source.production.output.resolved.occurrence.source, actual.production.output.resolved.occurrence.source);
        assert_eq!(transported.production.output.resolved.occurrence, source.production.output.resolved.occurrence);
        assert_eq!(transported.production.signature, source.production.signature);
        for (source, transported) in source.production.inputs.iter().zip(&transported.production.inputs) { assert_eq!(source.capture.use_span, transported.capture.use_span); assert_eq!(source.capture.place, transported.capture.place); }
        for defect in 0..7 {
            let mut candidate = (*actual).clone();
            match defect {
                0 => candidate.production.signature = ScalarType::Error,
                1 => candidate.production.output.resolved.occurrence.range = ByteSpan::new(0, 0),
                2 => candidate.production.child = source.production.child.clone(),
                3 if !candidate.production.inputs.is_empty() => candidate.production.inputs[0].capture.use_span.range = ByteSpan::new(0, 0),
                4 if !candidate.production.inputs.is_empty() => candidate.production.inputs[0].capture.place_type = ScalarType::Error,
                5 if !candidate.production.inputs.is_empty() => candidate.production.inputs[0].capture.ty = ScalarType::Error,
                6 if !candidate.production.inputs.is_empty() => candidate.production.inputs[0].formal.port = usize::MAX,
                _ => continue,
            }
            let saved = (source.clone(), transported.clone(), (*actual).clone(), candidate.clone(), types.clone(), associations.clone());
            let error = ValueSchemaRemapError::MissingPoint { producer: source.production.event.owner.clone(), point: source.production.event.point };
            assert_eq!(capture_counterpart::validate(&remap, source, &transported, &candidate, &correspondence, &source_originals, &actual_originals, &types, &associations), Err(error));
            assert_eq!((source.clone(), transported.clone(), (*actual).clone(), candidate, types.clone(), associations.clone()), saved);
        }
    }
    assert_eq!((source, source_forest, target, target_forest, remap), before);
}

fn owned(text: &str, layout: ScalarTargetLayout, project: bool) {
    let (source, forest) = original(text, "owned", layout, project);
    let remap = keys(&forest, &forest);
    let before = (source.clone(), forest.clone(), remap.clone());
    let witness = remap.independent_boundary_destination(&source, &forest).expect("genuine original owned witness");
    let (_, correspondence) = witness.checked_headers(&remap, &source, &forest).expect("actual same-source checked header evidence");
    let full = remap.tables_into(&source, &forest, &source, &forest).expect("full original canonical production and owned AST input evidence");
    for row in &source {
        let uses = remap.source_use_graphs_into(&row.producer, &row.tables, &source, &forest, &witness).expect("source-use owned input evidence");
        let stages = remap.staged_loan_queries_into(&row.producer, &row.tables, &source, &forest, &witness).expect("staged owned input evidence");
        for resources in [&full[0].resources, &uses.resources, &stages.resources] {
            header_counterpart::postvalidate(&remap, &source, &correspondence, resources).expect("ALL owned cuts reconstruct same original syntax/output/signature roots");
            postvalidation::capture_inputs(&remap, &source, &correspondence, resources).expect("ALL owned cuts consume same complete Capture helper");
            let input_counterpart::BoundaryInputTypeEvidence::Independent(types) = resources.input_types() else { panic!("mandatory owned original evidence"); };
            let (annotations, errors, incidences, _) = types.ast_evidence_counts();
            assert!(annotations >= errors);
            if text != LEXICAL && text != EFFECT && text != PROJECTED_EFFECT { assert!(errors > 0); }
            assert!(incidences > 0);
            for corrupted in types.ast_proof_candidates() {
                let mut candidate = resources.clone(); candidate.input_types = input_counterpart::BoundaryInputTypeEvidence::Independent(corrupted);
                let saved = candidate.clone();
                assert_eq!(header_counterpart::postvalidate(&remap, &source, &correspondence, &candidate), Err(ValueSchemaRemapError::EmptyTransportSources));
                assert_eq!(candidate, saved);
            }
            let mut candidate = resources.clone(); candidate.input_types = input_counterpart::BoundaryInputTypeEvidence::Transported;
            assert!(header_counterpart::postvalidate(&remap, &source, &correspondence, &candidate).is_err());
            assert_eq!(&full[0].resources.input_types, &uses.resources.input_types);
            assert_eq!(&uses.resources.input_types, &stages.resources.input_types);
        }
    }
    let mut empty = source.clone();
    let mut empty_forest = forest.clone();
    for row in &mut empty {
        row.tables.source_use_graphs.clear(); row.tables.staged_loan_queries.clear();
        empty_forest.sources.iter_mut().find(|actual| actual.producer == row.producer).expect("exact unique cloned candidate carrier").tables = row.tables.clone();
    }
    let empty_witness = remap.independent_boundary_destination(&empty, &empty_forest).expect("genuine original snapshots around empty current outputs");
    let (_, empty_correspondence) = empty_witness.checked_headers(&remap, &empty, &empty_forest).expect("mandatory empty cut original headers");
    for row in &empty {
        let uses = remap.source_use_graphs_into(&row.producer, &row.tables, &empty, &empty_forest, &empty_witness).expect("empty source-use cut preserves input proof");
        let stages = remap.staged_loan_queries_into(&row.producer, &row.tables, &empty, &empty_forest, &empty_witness).expect("empty staged cut preserves input proof");
        assert!(uses.graphs.is_empty()); assert!(stages.queries.is_empty());
        for resources in [&uses.resources, &stages.resources] {
            header_counterpart::postvalidate(&remap, &empty, &empty_correspondence, resources).expect("empty owned proof reconstructs whole AST/output/signature evidence");
            postvalidation::capture_inputs(&remap, &empty, &empty_correspondence, resources).expect("empty owned output still retains complete original tuple relation");
            assert_eq!(resources.capture_inputs, full[0].resources.capture_inputs);
        }
    }
    assert_eq!((source, forest, remap), before);
}

fn original_negatives(layout: ScalarTargetLayout, project: bool) {
    for text in [MAKE, BRANCH, DEPTH2, CONTROL, BLOCK, MULTI_OUTPUT, ALIASES, UNSAFE, FIXED_RUNTIME] {
        let (source, forest) = original(text, "original-negative", layout, project);
        let remap = keys(&forest, &forest);
        remap.boundary_arena_correspondence(&source, &forest, &source, &forest).expect("true original annotation/output/signature baseline before isolated original input corruption");
        let root = &source[0];
        let baseline = root.resource_construction.original().clone();
        let capture = baseline.initial_points.iter().find(|point| matches!(&point.kind, CfgPointKind::Capture { body, .. } if !body.final_output_values.is_empty())).expect("genuine original stored syntax-output constructor");
        let event = SourceEventId { owner: root.producer.clone(), point: capture.id };
        for defect in 0..9 {
            let mut cfg = baseline.clone();
            let expected = if defect < 5 {
                let point = cfg.initial_points.iter_mut().find(|point| point.id == event.point).expect("real unique original Capture");
                let CfgPointKind::Capture { body, .. } = &mut point.kind else { unreachable!(); };
                match defect {
                    0 => body.final_output_values[0].ty = ScalarType::Unit,
                    1 => body.final_output_values[0].position = usize::MAX,
                    2 => body.final_output_values[0].span = ByteSpan::new(0, 0),
                    3 => body.final_output_values[0].value = ScalarExpression::Boolean { value: false, span: ByteSpan::new(0, 0) },
                    4 => body.unsafe_context = !body.unsafe_context,
                    _ => unreachable!(),
                }
                let kind = point.kind.clone();
                cfg.points.iter_mut().find(|point| point.id == event.point).expect("real evaluated original Capture counterpart").kind = kind;
                ValueSchemaRemapError::MissingPoint { producer: event.owner.clone(), point: event.point }
            } else {
                let child = cfg.literal_cfgs.iter_mut().find(|child| child.capture_point == event.point).expect("genuine original Capture-created child");
                let values = &mut child.reference_cfg.evaluated_values;
                let Some(input) = values.final_inputs.first_mut() else { continue; };
                match defect {
                    5 => input.receiving_type = ScalarType::Error,
                    6 => input.consumer.point = CfgPointId(usize::MAX),
                    7 => input.source.point = CfgPointId(usize::MAX),
                    8 => {
                        let fact = values.values.iter().find(|fact| fact.id == input.source).expect("genuine original independently evaluated output fact");
                        let shape = values.shapes.iter_mut().find(|shape| shape.id == fact.shape).expect("exact actual original source shape");
                        shape.ty = ScalarType::Error;
                        let expected = ValueSchemaRemapError::MissingShape(shape.id.clone());
                        let actual = capture_table_sources(&cfg, &root.policy_context).expect("isolated child shape corruption snapshot");
                        let mut actual_forest = forest.clone();
                        for row in &actual { *actual_forest.sources.iter_mut().find(|original| original.producer == row.producer).expect("unique actual original carrier") = row.clone(); }
                        let saved = (actual.clone(), actual_forest.clone());
                        assert_eq!(remap.boundary_arena_correspondence(&source, &forest, &actual, &actual_forest), Err(expected));
                        assert_eq!((actual, actual_forest), saved);
                        continue;
                    }
                    _ => unreachable!(),
                }
                ValueSchemaRemapError::MissingPoint { producer: event.owner.clone(), point: event.point }
            };
            let actual = capture_table_sources(&cfg, &root.policy_context).expect("isolated corrupt original snapshot retains genuine unchanged producer topology");
            let mut actual_forest = forest.clone();
            for row in &actual { *actual_forest.sources.iter_mut().find(|original| original.producer == row.producer).expect("unique original forest carrier") = row.clone(); }
            let saved = (source.clone(), forest.clone(), actual.clone(), actual_forest.clone(), remap.clone());
            assert_eq!(remap.boundary_arena_correspondence(&source, &forest, &actual, &actual_forest), Err(expected));
            assert_eq!((source.clone(), forest.clone(), actual, actual_forest, remap.clone()), saved);
        }
    }
}

const LEXICAL: &str = "%%start\nunit() run = fn { u32 item = 1; unit() read = fn { item; }; read(); };\n%%end";
const PROJECTED: &str = "%%start\nstruct copy Pair { u32 item; }\nunit() run = fn { Pair pair = { .item = 1; }; unit() read = fn { pair.item; }; read(); };\n%%end";
const MAKE: &str = "%%start\n(u8())() make = fn { u8() inner = fn { 1 }; inner };\nunit() run = fn { u8() first = make(); u8() probe = fn { first() }; probe(); };\n%%end";
const BRANCH: &str = "%%start\n(u8())() make_a = fn { u8() inner = fn { 1 }; inner };\n(u8())() make_b = fn { u8() inner = fn { 2 }; inner };\nunit(bool) run = fn(flag) { u8() target = if (flag) { make_a() } else { make_b() }; u8 tag = 7; tag; u8() probe = fn { target(); tag }; probe(); };\n%%end";
const DEPTH2: &str = "%%start\nu64() select = fn { 0 };\nunit() run = fn { u8[2] slots = [1, 2]; unit() outer = fn { u8() inner = fn { slots[select()] }; inner(); }; outer(); };\n%%end";
const CONTROL: &str = "%%start\nunit(bool) run = fn(flag) { u8() read = fn { if (flag) { 1 } else { 2 } }; read(); };\n%%end";
const BLOCK: &str = "%%start\nunit() run = fn { u8() read = fn { { 1 } }; read(); };\n%%end";
const MULTI_OUTPUT: &str = "%%start\nunit() run = fn { (i32, bool)() pair = fn { 1, true }; pair(); };\n%%end";
const ALIASES: &str = "%%start\nunit() run = fn { (i32, bool)() pair = fn { if (true) { 1 } else { 2 }, if (false) { true } else { false } }; pair(); };\n%%end";
const UNSAFE: &str = "%%start\nunit() run = fn { u8() read = fn { unsafe { 1 } }; read(); };\n%%end";
const FIXED_RUNTIME: &str = "%%start\nunit() run = fn { u8[](u8[2]) widen = fn(fixed) { fixed }; u8[2] fixed = [4, 5]; u8[] output = widen(fixed); output[0]; };\n%%end";
const EFFECT: &str = "%%start\nu32 item = 1;\nunit() run = fn { unit() change = fn { item = 2; }; change(); };\n%%end";
const PROJECTED_EFFECT: &str = "%%start\nstruct copy Pair { u32 item; }\nPair value = { .item = 1; };\nunit() run = fn { unit() change = fn { value.item = 2; }; change(); };\n%%end";
const OVERLOAD: &str = "%%start\nidentity = overload { generic T; T(T) => fn(value) { value }; };\nunit() run = fn { u8() read = fn { identity(1) }; read(); };\n%%end";
const FIELD_CALL: &str = "%%start\nstruct Factory { u8() read; }\nunit() run = fn { Factory factory = { .read = fn { 1 }; }; u8() read = fn { factory.read() }; read(); };\n%%end";
const CORE_CALL: &str = "%%start\nunit() run = fn { u8 item = 1; u64() read = fn { core.int_extend<u64>(item) }; read(); };\n%%end";

fn direct_matrix(layout: ScalarTargetLayout, project: bool) { for text in [LEXICAL, PROJECTED, DEPTH2, EFFECT, PROJECTED_EFFECT] { direct(text, layout, project); } }
fn owned_matrix(layout: ScalarTargetLayout, project: bool) { for text in [LEXICAL, MAKE, BRANCH, DEPTH2, CONTROL, BLOCK, MULTI_OUTPUT, ALIASES, UNSAFE, FIXED_RUNTIME, EFFECT, PROJECTED_EFFECT, OVERLOAD, FIELD_CALL, CORE_CALL] { owned(text, layout, project); } }

#[test]
fn original_input_ast_native64_program() { direct_matrix(ScalarTargetLayout::NATIVE64, false); }
#[test]
fn original_input_ast_native64_project() { direct_matrix(ScalarTargetLayout::NATIVE64, true); }
#[test]
fn original_input_ast_wasm32_program() { direct_matrix(ScalarTargetLayout::WASM32, false); }
#[test]
fn original_input_ast_wasm32_project() { direct_matrix(ScalarTargetLayout::WASM32, true); }
#[test]
fn original_owned_ast_native64_program() { owned_matrix(ScalarTargetLayout::NATIVE64, false); }
#[test]
fn original_owned_ast_native64_project() { owned_matrix(ScalarTargetLayout::NATIVE64, true); }
#[test]
fn original_owned_ast_wasm32_program() { owned_matrix(ScalarTargetLayout::WASM32, false); }
#[test]
fn original_owned_ast_wasm32_project() { owned_matrix(ScalarTargetLayout::WASM32, true); }
#[test]
fn original_annotation_corruption_native64_program() { original_negatives(ScalarTargetLayout::NATIVE64, false); }
#[test]
fn original_annotation_corruption_native64_project() { original_negatives(ScalarTargetLayout::NATIVE64, true); }
#[test]
fn original_annotation_corruption_wasm32_program() { original_negatives(ScalarTargetLayout::WASM32, false); }
#[test]
fn original_annotation_corruption_wasm32_project() { original_negatives(ScalarTargetLayout::WASM32, true); }

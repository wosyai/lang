use super::*;

const SOURCE: &str = "%%start\nunit() run = fn { u32 item = 1; unit() write = fn { item = 2; }; write(); };\n%%end";

fn original(text: &str, identity: SourceIdentity, layout: ScalarTargetLayout, project: bool) -> (Vec<ValueTableSource>, SourceCaptureConstructionForest) {
    let parsed = crate::parse_source(identity.clone(), text.into(), &[]);
    assert!(parsed.diagnostics.is_empty(), "{:?}", parsed.diagnostics);
    let (validation, _) = derive_scalar_program_observed_test(&parsed.result, layout);
    assert!(validation.diagnostics.is_empty(), "{:?}", validation.diagnostics);
    let module = if project {
        let validated = validate_scalar_project(ScalarProject::new(vec![ScalarModule::from_program(validation.program, Vec::new())], vec![identity]));
        assert!(validated.diagnostics.is_empty(), "{:?}", validated.diagnostics);
        validated.project.modules[0].clone()
    } else { ScalarModule::from_program(validation.program, Vec::new()) };
    let forest = source_boundary::module_preflight_forest(&module.source, &module.items, &module.top_level_reference_cfg, &module.structs, &module.enums, SourceCaptureConstructionPhase::Complete).expect("actual independently parsed complete forest");
    let function = module.items.iter().find_map(|item| match item { ScalarItem::Function(function) if function.name == "run" => Some(function), _ => None }).expect("actual run");
    let input = source_boundary::preflight_input(&function.reference_cfg.producer_owner, &forest, BTreeMap::new()).expect("actual original input");
    source_boundary::validate_capture_preflight(&function.reference_cfg, &input).expect("actual original populated preflight");
    (capture_table_sources(&function.reference_cfg, &input.policy_context).expect("actual selected literal closure"), forest)
}

fn associations(source: &[ValueTableSource], forest: &SourceCaptureConstructionForest, target: &[ValueTableSource]) -> ValueSchemaRemap {
    let source_catalog = original::catalog(source).expect("actual source constructors");
    let target_catalog = original::catalog(target).expect("actual destination constructors");
    let mut remap = ValueSchemaRemap { producers: Vec::new(), points: Vec::new(), shapes: Vec::new(), imports: Vec::new() };
    for row in source {
        let targets = target.iter().filter(|actual| actual.initial_points[0].source_span.range == row.initial_points[0].source_span.range && std::mem::discriminant(&actual.producer) == std::mem::discriminant(&row.producer)).collect::<Vec<_>>();
        let [actual] = targets.as_slice() else { panic!("unique independently original declared owner"); };
        assert_eq!(row.capture_inventory.required.len(), actual.capture_inventory.required.len());
        remap.producers.push((row.producer.clone(), actual.producer.clone()));
        for point in &row.initial_points {
            let counterparts = actual.initial_points.iter().filter(|counterpart| counterpart.source_span.range == point.source_span.range && std::mem::discriminant(&counterpart.kind) == std::mem::discriminant(&point.kind)).collect::<Vec<_>>();
            let [counterpart] = counterparts.as_slice() else { panic!("unique actual typed point occurrence"); };
            remap.points.push(ValuePointRemap { producer: row.producer.clone(), point: point.id, imported: counterpart.id });
        }
        for shape in &row.tables.shapes {
            let counterparts = actual.resource_construction.original().evaluated_values.shapes.iter().filter(|counterpart| c3_key_builders::type_key(&shape.ty, &counterpart.ty, &row.policy_context, &actual.policy_context)).collect::<Vec<_>>();
            let [counterpart] = counterparts.as_slice() else { panic!("unique whole canonical type/layout/template target shape"); };
            remap.shapes.push((shape.id.clone(), counterpart.id.clone()));
        }
        let import = ValueImportCorrespondence { producer: row.producer.clone(), bindings: Vec::new(), places: Vec::new(), origins: Vec::new(), loans: Vec::new(), runtime_values: Vec::new(), environments: Vec::new(), backings: Vec::new(), holders: Vec::new() };
        assert_eq!(row.resource_construction.original().facts.len(), actual.resource_construction.original().facts.len());
        remap.imports.push(import);
    }
    remap.validate_qualified_shapes(source, target).expect("ALL genuine shape key pairs installed before qualified type/payload/graph-edge validation");
    c3_key_builders::install(&mut remap, &source_catalog, &target_catalog, forest);
    remap
}

fn matrix(layout: ScalarTargetLayout, project: bool) {
    for text in [SOURCE,
        "%%start\nunit() run = fn { unit() empty = fn { }; empty(); };\n%%end",
        "%%start\nunit() run = fn { u32 item = 1; unit() outer = fn { unit() inner = fn { item = 2; }; inner(); }; outer(); };\n%%end",
        "%%start\nunit() run = fn { u32 item = 1; *!u32 parent = &!item; unit() write = fn { *!u32 child = &!(*parent); *child = 2; }; write(); };\n%%end",
        "%%start\nunit(u64) run = fn(count) { u8[count] slots; u8() read = fn { slots[0] }; read(); };\n%%end",
        "%%start\nunit() run = fn { u8() first = fn { 1 }; u8() second = fn { first() }; second(); };\n%%end",
    ] {
        let (left, mut forest) = original(text, SourceIdentity::new("c3".into(), "left".into(), "main.w".into(), "r1".into()), layout, project);
        let (right, right_forest) = original(text, SourceIdentity::new("c3".into(), "right".into(), "main.w".into(), "r1".into()), layout, project);
        let (target, target_forest) = original(text, SourceIdentity::new("c3".into(), "target".into(), "main.w".into(), "r1".into()), layout, project);
        forest.roots.extend(right_forest.roots); forest.sources.extend(right_forest.sources);
        let mut source = left; source.extend(right);
        let remap = associations(&source, &forest, &target);
        let witness = remap.independent_boundary_destination(&target, &target_forest).expect("genuine Independent target witness");
        let saved = (source.clone(), forest.clone(), target.clone(), target_forest.clone(), remap.clone(), witness.clone());
        let full = remap.tables_into(&source, &forest, &target, &target_forest).expect("whole populated many-source one-target transport");
        assert_eq!(full.len(), source.len());
        let tuples = original_capture_tuples(&source).expect("all independently original source tuples");
        let actual_tuples = original_capture_tuples(&target).expect("all genuine target tuples");
        for row in &source {
            let matches = full.iter().filter(|mapped| mapped.source == row.producer).collect::<Vec<_>>();
            let [mapped] = matches.as_slice() else { panic!("exact source-tagged full result"); };
            assert_eq!(mapped.capture_inventory, remap.capture_inventory(&row.capture_inventory).expect("canonical original occurrence retained"));
            for production in &row.tables.source_capture_productions {
                let canonical = remap.value(&production.output).expect("canonical transported output");
                assert!(mapped.tables.source_capture_productions.iter().any(|actual| actual.output == canonical));
                assert_eq!(canonical.resolved.occurrence, production.output.resolved.occurrence);
            }
            let uses = remap.source_use_graphs_into(&row.producer, &row.tables, &source, &forest, &witness).expect("whole populated aliased selective uses");
            let stages = remap.staged_loan_queries_into(&row.producer, &row.tables, &source, &forest, &witness).expect("whole populated aliased selective stages");
            for resources in [&mapped.resources, &uses.resources, &stages.resources] {
                assert_eq!(resources.capture_inputs.iter().map(|pair| pair.source.clone()).collect::<Vec<_>>(), tuples);
                for pair in &resources.capture_inputs {
                    assert!(actual_tuples.contains(&pair.destination));
                    assert_ne!(pair.source.production.output.resolved.occurrence.source, pair.destination.production.output.resolved.occurrence.source);
                    assert_eq!(pair.source.production.output.resolved.occurrence.range, pair.destination.production.output.resolved.occurrence.range);
                    assert!(resources.capture_inputs.iter().any(|other| other.source.production.event.owner != pair.source.production.event.owner && other.destination == pair.destination));
                }
            }
        }
        assert_eq!((source.clone(), forest.clone(), target.clone(), target_forest.clone(), remap.clone(), witness.clone()), saved);
        assert!(remap.stage_boundary_sources(&source, &forest).is_err(), "Copied owner collision remains explicit");
        let (headers, correspondence) = witness.checked_headers(&remap, &source, &forest).expect("exact original complete headers");
        let resources = &full[0].resources;
        for pair in &resources.capture_inputs {
            let transported = allocation::mapped_tuple(&remap, &pair.source, &correspondence, &resources.associations, true).expect("complete checked counterpart transport");
            capture_counterpart::validate_owned(&remap, &pair.source, &transported, &pair.destination, &correspondence, resources).expect("same paired relation used by full and selective guards");
            let error = ValueSchemaRemapError::MissingPoint { producer: pair.source.production.event.owner.clone(), point: pair.source.production.event.point };
            for defect in 0..17 {
                if defect >= 10 && pair.destination.production.inputs.is_empty() { continue; }
                let mut candidate = pair.destination.clone();
                match defect {
                    0 => candidate.production.output.resolved.occurrence.source = pair.source.production.output.resolved.occurrence.source.clone(),
                    1 => candidate.production.output.resolved.occurrence.range = ByteSpan::new(0, 0),
                    2 => candidate.production.event.point = CfgPointId(usize::MAX),
                    3 => candidate.production.output.point = CfgPointId(usize::MAX),
                    4 => candidate.production.output.resolved.output_projection.push(usize::MAX),
                    5 => candidate.production.output.resolved.output += 1,
                    6 => candidate.production.child = pair.source.production.child.clone(),
                    7 => candidate.production.signature = ScalarType::U32,
                    8 => candidate.production.output.resolved.producer = pair.source.production.event.owner.clone(),
                    9 => candidate.production.event.owner = pair.source.production.event.owner.clone(),
                    10 => candidate.production.inputs[0].formal.port = usize::MAX,
                    11 => candidate.production.inputs[0].coordinate.index = usize::MAX,
                    12 => candidate.production.inputs[0].shape.index = usize::MAX,
                    13 => candidate.production.inputs[0].capture.binding.source = copied::lexical(&pair.source.production.event.owner).clone(),
                    14 => candidate.production.inputs[0].policy = CopyPolicy::Deferred,
                    15 => candidate.production.inputs[0].capture.ty = ScalarType::Error,
                    16 => candidate.production.inputs.pop().map(|_| ()).expect("populated actual input"),
                    _ => unreachable!(),
                }
                let saved = (pair.clone(), transported.clone(), candidate.clone());
                assert_eq!(capture_counterpart::validate_owned(&remap, &pair.source, &transported, &candidate, &correspondence, resources), Err(error.clone()), "defect={defect}");
                let mut corrupt = resources.clone();
                let association = corrupt.capture_inputs.iter_mut().find(|association| association.source == pair.source).expect("exact source-tagged retained tuple");
                association.destination = candidate.clone();
                let before = (source.clone(), forest.clone(), target.clone(), target_forest.clone(), full.clone(), corrupt.clone(), remap.clone(), witness.clone());
                assert_eq!(postvalidation::capture_inputs(&remap, &source, &correspondence, &corrupt), Err(error.clone()));
                assert_eq!(postvalidation::full(&remap, &source, &full, &correspondence, &corrupt), Err(error.clone()));
                let root = &source[0].producer;
                let mapped = full.iter().find(|row| row.source == *root).expect("retained full root");
                for cut in [BoundaryResourceDemandCut::SourceUses { root: root.clone() }, BoundaryResourceDemandCut::StagedLoans { root: root.clone() }] {
                    assert_eq!(postvalidation::selective(&remap, root, &source, &mapped.tables, headers, &correspondence, &cut, &corrupt), Err(error.clone()));
                }
                assert_eq!((source.clone(), forest.clone(), target.clone(), target_forest.clone(), full.clone(), corrupt, remap.clone(), witness.clone()), before);
                assert_eq!((pair.clone(), transported.clone(), candidate), saved);
            }
            if !pair.destination.paths.is_empty() {
                let mut candidate = pair.destination.clone();
                candidate.paths.push(candidate.paths[0].clone());
                let holder = pair.source.paths.iter().flat_map(|path| &path.inputs).find_map(|input| match &input.origin { CaptureInputOriginFact::Callable { holders } => holders.first(), CaptureInputOriginFact::Value => None });
                let expected = match holder { Some(holder) => ValueSchemaRemapError::MissingHolder(holder.clone()), None => error.clone() };
                let saved = (pair.clone(), transported.clone(), candidate.clone());
                assert_eq!(capture_counterpart::validate_owned(&remap, &pair.source, &transported, &candidate, &correspondence, resources), Err(expected));
                assert_eq!((pair.clone(), transported, candidate), saved);
            }
        }
    }
}

#[test]
fn actual_populated_counterparts_native64_program() { matrix(ScalarTargetLayout::NATIVE64, false); }
#[test]
fn actual_populated_counterparts_native64_project() { matrix(ScalarTargetLayout::NATIVE64, true); }
#[test]
fn actual_populated_counterparts_wasm32_program() { matrix(ScalarTargetLayout::WASM32, false); }
#[test]
fn actual_populated_counterparts_wasm32_project() { matrix(ScalarTargetLayout::WASM32, true); }

fn reindexed(layout: ScalarTargetLayout, project: bool) {
    let before = "%%start\nunit() run = fn { u32 item = 1;        unit() write = fn { item = 2; }; write(); };\n%%end";
    let after = "%%start\nunit() run = fn { u32 item = 1; true;  unit() write = fn { item = 2; }; write(); };\n%%end";
    assert_eq!(before.len(), after.len());
    let (source, forest) = original(before, SourceIdentity::new("c3".into(), "reindex-source".into(), "main.w".into(), "r1".into()), layout, project);
    let (target, target_forest) = original(after, SourceIdentity::new("c3".into(), "reindex-target".into(), "main.w".into(), "r1".into()), layout, project);
    let remap = associations(&source, &forest, &target);
    assert!(remap.points.iter().any(|point| point.point != point.imported));
    let witness = remap.independent_boundary_destination(&target, &target_forest).expect("genuinely allocated nonidentity target headers");
    let saved = (source.clone(), forest.clone(), target.clone(), target_forest.clone(), remap.clone(), witness.clone());
    let full = remap.tables_into(&source, &forest, &target, &target_forest).expect("genuine nonidentity original destination graph");
    let (headers, correspondence) = witness.checked_headers(&remap, &source, &forest).expect("independently checked target coordinate and formal incidence");
    assert!(source.iter().flat_map(|row| &row.tables.source_slots).any(|slot| correspondence.slot(&slot.id).expect("actual mapped coordinate").index != slot.id.index));
    for row in &source {
        let uses = remap.source_use_graphs_into(&row.producer, &row.tables, &source, &forest, &witness).expect("nonidentity selective uses");
        let stages = remap.staged_loan_queries_into(&row.producer, &row.tables, &source, &forest, &witness).expect("nonidentity selective stages");
        for resource in [&full[0].resources, &uses.resources, &stages.resources] {
            for tuple in &resource.capture_inputs {
                assert_ne!(tuple.source.production.event, tuple.destination.production.event);
                assert_eq!(tuple.source.production.output.resolved.occurrence.range, tuple.destination.production.output.resolved.occurrence.range);
            }
        }
    }
    let root = &source[0].producer;
    let copied = remap.stage_boundary_sources(&source, &forest).expect("matching allocated Copied headers");
    let mut changed = remap.clone();
    changed.producers.retain(|(owner, _)| owner != root);
    let saved_changed = (source.clone(), forest.clone(), target.clone(), target_forest.clone(), changed.clone(), copied.clone(), witness.clone());
    let error = ValueSchemaRemapError::MissingProducer(root.clone());
    assert_eq!(changed.tables_into(&source, &forest, &target, &target_forest), Err(error.clone()));
    for destination in [&copied, &witness] {
        assert_eq!(changed.source_use_graphs_into(root, &source[0].tables, &source, &forest, destination), Err(error.clone()));
        assert_eq!(changed.staged_loan_queries_into(root, &source[0].tables, &source, &forest, destination), Err(error.clone()));
    }
    assert_eq!((source.clone(), forest.clone(), target.clone(), target_forest.clone(), changed, copied, witness.clone()), saved_changed);
    assert!(!headers.is_empty());
    assert_eq!((source, forest, target, target_forest, remap, witness), saved);
}

#[test]
fn actual_reindexed_counterpart_native64_program() { reindexed(ScalarTargetLayout::NATIVE64, false); }
#[test]
fn actual_reindexed_counterpart_native64_project() { reindexed(ScalarTargetLayout::NATIVE64, true); }
#[test]
fn actual_reindexed_counterpart_wasm32_program() { reindexed(ScalarTargetLayout::WASM32, false); }
#[test]
fn actual_reindexed_counterpart_wasm32_project() { reindexed(ScalarTargetLayout::WASM32, true); }

fn header_boundary(layout: ScalarTargetLayout, project: bool) {
    let (source, forest) = original(SOURCE, SourceIdentity::new("c3".into(), "header-source".into(), "main.w".into(), "r1".into()), layout, project);
    let (target, target_forest) = original(SOURCE, SourceIdentity::new("c3".into(), "header-target".into(), "main.w".into(), "r1".into()), layout, project);
    let remap = associations(&source, &forest, &target);
    let witness = remap.independent_boundary_destination(&target, &target_forest).expect("actual target original witness before checked header pairing");
    let slot = source[0].tables.source_slots.iter().find(|slot| matches!(&slot.role, SourceCoordinateRole::Evaluated(_))).expect("genuine integer Evaluate coordinate");
    let mapped = remap.source_role(&source[0].producer, &slot.role).expect("original canonical output role transport");
    assert!(!target[0].tables.source_slots.iter().any(|slot| slot.role == mapped));
    let mut candidate = target.clone();
    let owner = remap.producer(&source[0].producer).expect("actual target lexical owner");
    let current = candidate.iter_mut().find(|row| row.producer == owner).expect("unique selected target");
    let actual = current.tables.source_slots.iter_mut().find(|slot| matches!(&slot.role, SourceCoordinateRole::Evaluated(_))).expect("actual independently original Evaluated header slot");
    let SourceCoordinateRole::Evaluated(value) = &mut actual.role else { unreachable!(); };
    value.resolved.occurrence.range = ByteSpan::new(0, 0);
    let mut candidate_forest = target_forest.clone();
    let carriers = candidate_forest.sources.iter_mut().filter(|row| row.producer == owner).collect::<Vec<_>>();
    let [carrier] = carriers.as_slice() else { panic!("unique candidate forest current-table carrier"); };
    assert_eq!(carrier.resource_construction, current.resource_construction);
    candidate_forest.sources.iter_mut().find(|row| row.producer == owner).expect("unique bounded candidate carrier").tables = current.tables.clone();
    assert_eq!(candidate_forest.sources.iter().find(|row| row.producer == owner).expect("retained original CFG").resource_construction.original(), target.iter().find(|row| row.producer == owner).expect("genuine original target").resource_construction.original());
    let candidate_witness = remap.independent_boundary_destination(&candidate, &candidate_forest).expect("actual target original witness before checked header pairing");
    let before_candidate = (candidate.clone(), candidate_forest.clone());
    let error = ValueSchemaRemapError::MissingBoundarySlot(slot.id.clone());
    let saved = (source.clone(), forest.clone(), target.clone(), target_forest.clone(), remap.clone(), witness.clone());
    assert_eq!(candidate_witness.checked_headers(&remap, &source, &forest), Err(error.clone()));
    assert_eq!(remap.tables_into(&source, &forest, &candidate, &candidate_forest), Err(error.clone()));
    assert_eq!(remap.source_use_graphs_into(&source[0].producer, &source[0].tables, &source, &forest, &candidate_witness), Err(error.clone()));
    assert_eq!(remap.staged_loan_queries_into(&source[0].producer, &source[0].tables, &source, &forest, &candidate_witness), Err(error));
    assert_eq!((candidate, candidate_forest), before_candidate);
    for duplicate in [false, true] {
        let mut candidate = target.clone();
        let production = candidate[0].tables.source_capture_productions[0].clone();
        if duplicate { candidate[0].tables.source_capture_productions.push(production.clone()); }
        else { candidate[0].tables.source_capture_productions.clear(); }
        let mut candidate_forest = target_forest.clone();
        candidate_forest.sources.iter_mut().find(|row| row.producer == candidate[0].producer).expect("exact original destination carrier").tables = candidate[0].tables.clone();
        let expected = ValueSchemaRemapError::MissingPoint { producer: production.event.owner.clone(), point: production.event.point };
        let saved_candidate = (source.clone(), forest.clone(), target.clone(), target_forest.clone(), candidate.clone(), candidate_forest.clone(), remap.clone(), witness.clone());
        assert_eq!(remap.independent_boundary_destination(&candidate, &candidate_forest), Err(expected.clone()));
        assert_eq!(remap.stage_boundary_sources(&candidate, &candidate_forest), Err(expected.clone()));
        assert_eq!(remap.tables_into(&source, &forest, &candidate, &candidate_forest), Err(expected));
        assert_eq!((source.clone(), forest.clone(), target.clone(), target_forest.clone(), candidate, candidate_forest, remap.clone(), witness.clone()), saved_candidate);
    }
    assert_eq!((source, forest, target, target_forest, remap, witness), saved);
}

#[test]
fn actual_evaluated_header_boundary_native64_program() { header_boundary(ScalarTargetLayout::NATIVE64, false); }
#[test]
fn actual_evaluated_header_boundary_native64_project() { header_boundary(ScalarTargetLayout::NATIVE64, true); }
#[test]
fn actual_evaluated_header_boundary_wasm32_program() { header_boundary(ScalarTargetLayout::WASM32, false); }
#[test]
fn actual_evaluated_header_boundary_wasm32_project() { header_boundary(ScalarTargetLayout::WASM32, true); }

mod header_tests;

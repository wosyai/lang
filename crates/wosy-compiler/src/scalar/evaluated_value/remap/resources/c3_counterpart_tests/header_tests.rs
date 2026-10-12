use super::*;

fn candidate(original: &[ValueTableSource], forest: &SourceCaptureConstructionForest, owner: &ProducerCfgOwnerId, tables: EvaluatedValueTables) -> (Vec<ValueTableSource>, SourceCaptureConstructionForest) {
    let mut source = original.to_vec();
    let mut rows = source.iter_mut().filter(|row| row.producer == *owner).collect::<Vec<_>>();
    let [row] = rows.as_mut_slice() else { panic!("unique genuine selected target"); };
    row.tables = tables;
    let mut current = forest.clone();
    let mut rows = current.sources.iter_mut().filter(|row| row.producer == *owner).collect::<Vec<_>>();
    let [row] = rows.as_mut_slice() else { panic!("unique original target forest carrier"); };
    row.tables = source.iter().find(|row| row.producer == *owner).expect("matching current candidate tables").tables.clone();
    (source, current)
}

fn matrix(layout: ScalarTargetLayout, project: bool) {
    let text = "%%start\nunit() run = fn { u32 item = 1; unit() write = fn { u32 old = (item = 2); old; }; write(); };\n%%end";
    let (source, forest) = original(text, SourceIdentity::new("c3".into(), "header-original".into(), "main.w".into(), "r1".into()), layout, project);
    let (target, target_forest) = original(text, SourceIdentity::new("c3".into(), "header-target".into(), "main.w".into(), "r1".into()), layout, project);
    let remap = associations(&source, &forest, &target);
    let witness = remap.independent_boundary_destination(&target, &target_forest).expect("actual original target forest witness");
    let full = remap.tables_into(&source, &forest, &target, &target_forest).expect("genuine Saved/Evaluated productive full graph transport");
    let (headers, correspondence) = witness.checked_headers(&remap, &source, &forest).expect("ONE checked shared header core");
    assert!(correspondence.header_inputs.iter().any(|pair| matches!(pair.source().role, SourceCoordinateRole::Saved(_))));
    assert!(correspondence.header_inputs.iter().any(|pair| matches!(pair.source().role, SourceCoordinateRole::Evaluated(_))));
    let before = (source.clone(), forest.clone(), target.clone(), target_forest.clone(), remap.clone(), witness.clone());
    for row in &source {
        let uses = remap.source_use_graphs_into(&row.producer, &row.tables, &source, &forest, &witness).expect("Saved/Evaluated association retained even empty selective graph cut");
        let stages = remap.staged_loan_queries_into(&row.producer, &row.tables, &source, &forest, &witness).expect("Saved/Evaluated association retained even empty selective query cut");
        let mapped = full.iter().find(|mapped| mapped.source == row.producer).expect("each source-tagged full row");
        assert_eq!(mapped.tables.source_slots, target.iter().find(|actual| actual.producer == mapped.destination).expect("real allocated target headers").tables.source_slots);
        for resources in [&mapped.resources, &uses.resources, &stages.resources] {
            assert_eq!(resources.header_inputs(), correspondence.header_inputs);
            assert!(resources.source_originals().iter().any(|snapshot| snapshot == &row.resource_construction));
            let BoundaryResourceProvenance::Independent { originals } = resources.provenance() else { panic!("actual Independent provenance"); };
            assert!(target.iter().all(|row| originals.contains(&row.resource_construction)));
            header_counterpart::postvalidate(&remap, &source, &correspondence, resources).expect("same retained whole association relation on all owned cuts");
        }
    }
    assert_eq!((source.clone(), forest.clone(), target.clone(), target_forest.clone(), remap.clone(), witness.clone()), before);
    for association in &correspondence.header_inputs {
        for defect in 0..9 {
            if !matches!(association.source().role, SourceCoordinateRole::Evaluated(_) | SourceCoordinateRole::Saved(_)) { continue; }
            let owner = &association.destination().id.owner;
            let actual = target.iter().find(|row| row.producer == *owner).expect("exact target header constructor");
            let mut tables = actual.tables.clone();
            let role = &mut tables.source_slots[association.destination().id.index].role;
            match role {
                SourceCoordinateRole::Evaluated(value) => match defect {
                    0 => value.resolved.occurrence.source = copied::lexical(&association.source().id.owner).clone(),
                    1 => value.resolved.occurrence.range = ByteSpan::new(0, 0),
                    2 => value.point = CfgPointId(usize::MAX),
                    3 => value.resolved.output += 1,
                    4 => value.resolved.output_projection.push(usize::MAX),
                    5 => value.resolved.producer = association.source().id.owner.clone(),
                    _ => continue,
                },
                SourceCoordinateRole::Saved(selection) => match defect {
                    0 => selection.application.operation.occurrence.source_span.source = copied::lexical(&association.source().id.owner).clone(),
                    1 => selection.application.operation.occurrence.source_span.range = ByteSpan::new(0, 0),
                    2 => selection.destination.operation.occurrence.source_span.source = copied::lexical(&association.source().id.owner).clone(),
                    3 => selection.destination.operation.occurrence.source_span.range = ByteSpan::new(0, 0),
                    4 => selection.application.frame.variable.0 += 1,
                    5 => selection.destination.target += 1,
                    6 => selection.application.operation.producer = association.source().id.owner.clone(),
                    7 => selection.destination.operation.occurrence.output_projection.push(usize::MAX),
                    8 => selection.application.frame.owner = association.source().id.owner.clone(),
                    _ => unreachable!(),
                },
                _ => unreachable!(),
            }
            let (current, current_forest) = candidate(&target, &target_forest, owner, tables);
            assert!(current.iter().zip(&target).all(|(a, b)| a.resource_construction == b.resource_construction));
            let current_witness = remap.independent_boundary_destination(&current, &current_forest).expect("bounded malformed CURRENT header with genuine CFG snapshots");
            let saved = (source.clone(), forest.clone(), target.clone(), target_forest.clone(), current.clone(), current_forest.clone(), remap.clone(), current_witness.clone());
            let error = ValueSchemaRemapError::MissingBoundarySlot(association.source().id.clone());
            assert_eq!(current_witness.checked_headers(&remap, &source, &forest), Err(error.clone()));
            assert_eq!(remap.tables_into(&source, &forest, &current, &current_forest), Err(error.clone()));
            for row in &source {
                assert_eq!(remap.source_use_graphs_into(&row.producer, &row.tables, &source, &forest, &current_witness), Err(error.clone()));
                assert_eq!(remap.staged_loan_queries_into(&row.producer, &row.tables, &source, &forest, &current_witness), Err(error.clone()));
            }
            assert_eq!((source.clone(), forest.clone(), target.clone(), target_forest.clone(), current, current_forest, remap.clone(), current_witness), saved);
        }
    }
    let mut corrupt = full[0].resources.clone();
    corrupt.header_inputs.pop().expect("populated association vector");
    assert_eq!(header_counterpart::postvalidate(&remap, &source, &correspondence, &corrupt), Err(ValueSchemaRemapError::EmptyTransportSources));
    assert_eq!(postvalidation::full(&remap, &source, &full, &correspondence, &corrupt), Err(ValueSchemaRemapError::EmptyTransportSources));
    for row in &source {
        let mapped = full.iter().find(|actual| actual.source == row.producer).expect("actual canonical mapped topology");
        assert_eq!(postvalidation::selective(&remap, &row.producer, &source, &mapped.tables, headers, &correspondence, &BoundaryResourceDemandCut::StagedLoans { root: row.producer.clone() }, &corrupt), Err(ValueSchemaRemapError::EmptyTransportSources));
    }
}

#[test]
fn actual_saved_evaluated_header_native64_program() { matrix(ScalarTargetLayout::NATIVE64, false); }
#[test]
fn actual_saved_evaluated_header_native64_project() { matrix(ScalarTargetLayout::NATIVE64, true); }
#[test]
fn actual_saved_evaluated_header_wasm32_program() { matrix(ScalarTargetLayout::WASM32, false); }
#[test]
fn actual_saved_evaluated_header_wasm32_project() { matrix(ScalarTargetLayout::WASM32, true); }

fn identity_families(layout: ScalarTargetLayout, project: bool) {
    for text in [SOURCE,
        "%%start\nunit() run = fn { unit() empty = fn { }; empty(); };\n%%end",
        "%%start\nunit() run = fn { u32 item = 1; unit() outer = fn { unit() inner = fn { item = 2; }; inner(); }; outer(); };\n%%end",
        "%%start\nstruct copy Pair { u32 item; }\nunit() run = fn { Pair pair = { .item = 1; }; unit() read = fn { pair.item; }; read(); };\n%%end",
        "%%start\nunit() run = fn { u32 item = 1; *!u32 parent = &!item; unit() write = fn { *!u32 child = &!(*parent); *child = 2; }; write(); };\n%%end",
        "%%start\nunit(u64) run = fn(count) { u8[count] slots; u8() read = fn { slots[0] }; read(); };\n%%end",
        "%%start\nunit() run = fn { u8() first = fn { 1 }; u8() second = fn { first() }; second(); };\n%%end",
    ] {
        let (source, forest) = original(text, SourceIdentity::new("c3".into(), "identity-families".into(), "main.w".into(), "r1".into()), layout, project);
        let remap = associations(&source, &forest, &source);
        let copied = remap.stage_boundary_sources(&source, &forest).expect("strict original Copied shared allocator");
        let independent = remap.independent_boundary_destination(&source, &forest).expect("same-source independently original snapshot witness");
        let full = remap.tables(&source, &forest).expect("whole original identity families Copied result");
        let actual = remap.tables_into(&source, &forest, &source, &forest).expect("whole original identity families Independent result");
        let before = (source.clone(), forest.clone(), remap.clone(), copied.clone(), independent.clone());
        for row in &source {
            for witness in [&copied, &independent] {
                let uses = remap.source_use_graphs_into(&row.producer, &row.tables, &source, &forest, witness).expect("all actual identity selective source uses");
                let stages = remap.staged_loan_queries_into(&row.producer, &row.tables, &source, &forest, witness).expect("all actual identity selective staged queries");
                assert_eq!(uses.graphs, row.tables.source_use_graphs);
                assert_eq!(stages.queries, row.tables.staged_loan_queries);
                let (_, correspondence) = witness.checked_headers(&remap, &source, &forest).expect("same strict shared header core");
                assert_eq!(uses.resources.header_inputs(), correspondence.header_inputs);
                assert_eq!(stages.resources.header_inputs(), correspondence.header_inputs);
                assert!(uses.resources.source_originals().iter().any(|original| original == &row.resource_construction));
                assert!(stages.resources.source_originals().iter().any(|original| original == &row.resource_construction));
            }
            for rows in [&full, &actual] {
                let mapped = rows.iter().find(|mapped| mapped.source == row.producer).expect("all owned rows retained");
                assert_eq!(mapped.tables, row.tables);
                assert_eq!(mapped.capture_inventory, row.capture_inventory);
                assert_eq!(mapped.children, row.children);
                assert_eq!(mapped.policy_context, row.policy_context);
            }
        }
        assert_eq!((source, forest, remap, copied, independent), before);
    }
}

#[test]
fn actual_identity_families_native64_program() { identity_families(ScalarTargetLayout::NATIVE64, false); }
#[test]
fn actual_identity_families_native64_project() { identity_families(ScalarTargetLayout::NATIVE64, true); }
#[test]
fn actual_identity_families_wasm32_program() { identity_families(ScalarTargetLayout::WASM32, false); }
#[test]
fn actual_identity_families_wasm32_project() { identity_families(ScalarTargetLayout::WASM32, true); }

fn capture_use_span_boundary(layout: ScalarTargetLayout, project: bool) {
    let (source, forest) = original(SOURCE, SourceIdentity::new("c3".into(), "input-span-source".into(), "main.w".into(), "r1".into()), layout, project);
    let (target, target_forest) = original(SOURCE, SourceIdentity::new("c3".into(), "input-span-target".into(), "main.w".into(), "r1".into()), layout, project);
    let sources = original_capture_tuples(&source).expect("complete real original primitive Capture tuples");
    let targets = original_capture_tuples(&target).expect("complete real independently parsed destination tuples");
    let tuples = sources.iter().filter(|tuple| !tuple.production.inputs.is_empty()).collect::<Vec<_>>();
    assert!(!tuples.is_empty());
    for tuple in tuples {
        let actuals = targets.iter().filter(|actual| actual.production.output.resolved.occurrence.range == tuple.production.output.resolved.occurrence.range).collect::<Vec<_>>();
        let [actual] = actuals.as_slice() else { panic!("actual original primitive Capture at same constructor range"); };
        let bindings = tuple.production.inputs.iter().zip(&actual.production.inputs).map(|(a, b)| (a.capture.binding.clone(), b.capture.binding.clone())).collect();
        let remap = ValueSchemaRemap { producers: vec![(tuple.production.event.owner.clone(), actual.production.event.owner.clone())], points: vec![ValuePointRemap { producer: tuple.production.event.owner.clone(), point: tuple.production.event.point, imported: actual.production.event.point }], shapes: Vec::new(), imports: vec![ValueImportCorrespondence { producer: tuple.production.event.owner.clone(), bindings, places: Vec::new(), origins: Vec::new(), loans: Vec::new(), runtime_values: Vec::new(), environments: Vec::new(), backings: Vec::new(), holders: Vec::new() }] };
        let before = (source.clone(), forest.clone(), target.clone(), target_forest.clone(), remap.clone());
        for (original, actual) in tuple.production.inputs.iter().zip(&actual.production.inputs) {
            let transported = remap.capture_binding(&tuple.production.event.owner, &original.capture).expect("isolated actual qualified binding counterpart only");
            assert_eq!(transported.binding, actual.capture.binding);
            assert_eq!(transported.ty, actual.capture.ty);
            assert_eq!(transported.place_type, actual.capture.place_type);
            assert_eq!(transported.use_span, original.capture.use_span);
            assert_eq!(transported.use_span.range, actual.capture.use_span.range);
            assert_ne!(transported.use_span.source, actual.capture.use_span.source);
            assert_ne!(transported, actual.capture);
        }
        assert_eq!((source.clone(), forest.clone(), target.clone(), target_forest.clone(), remap), before);
    }
}

#[test]
fn actual_capture_input_use_span_boundary_native64_program() { capture_use_span_boundary(ScalarTargetLayout::NATIVE64, false); }
#[test]
fn actual_capture_input_use_span_boundary_native64_project() { capture_use_span_boundary(ScalarTargetLayout::NATIVE64, true); }
#[test]
fn actual_capture_input_use_span_boundary_wasm32_program() { capture_use_span_boundary(ScalarTargetLayout::WASM32, false); }
#[test]
fn actual_capture_input_use_span_boundary_wasm32_project() { capture_use_span_boundary(ScalarTargetLayout::WASM32, true); }

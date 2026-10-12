use super::*;

const EDGE: &str = "%%start\n(u8())() make = fn { u8() inner = fn { 1 }; inner };\nunit() run = fn { u8() first = make(); u8() probe = fn { first() }; probe(); };\n%%end";

fn fixture(layout: ScalarTargetLayout, project: bool, corrupt: bool) {
    let identity = SourceIdentity::new("return-incidence".into(), "resource-correction".into(), "main.w".into(), "r1".into());
    let parsed = crate::parse_source(identity.clone(), EDGE.into(), &[]);
    assert!(parsed.diagnostics.is_empty(), "{:?}", parsed.diagnostics);
    let (validation, _) = derive_scalar_program_observed_test(&parsed.result, layout);
    assert!(validation.diagnostics.is_empty(), "{:?}", validation.diagnostics);
    let module = if project {
        let validated = validate_scalar_project(ScalarProject::new(vec![ScalarModule::from_program(validation.program, Vec::new())], vec![identity]));
        assert!(validated.diagnostics.is_empty(), "{:?}", validated.diagnostics);
        validated.project.modules[0].clone()
    } else { ScalarModule::from_program(validation.program, Vec::new()) };
    let forest = source_boundary::module_preflight_forest(&module.source, &module.items, &module.top_level_reference_cfg, &module.structs, &module.enums, SourceCaptureConstructionPhase::Complete).expect("actual original maker and caller forest");
    let run = module.items.iter().find_map(|item| match item { ScalarItem::Function(function) if function.name == "run" => Some(function), _ => None }).expect("actual run");
    let root = &run.reference_cfg.producer_owner;
    let rows = forest.sources.iter().filter(|row| row.producer == *root).collect::<Vec<_>>();
    let [carrier] = rows.as_slice() else { panic!("unique original run"); };
    let sources = capture_table_sources(carrier.resource_construction.original(), &carrier.policy_context).expect("actual selected caller tree");
    let tuples = original_capture_tuples(&sources).expect("actual original whole caller tuples");
    let demanded = tuples.iter().flat_map(capture_tuple_roots).find(|reference| matches!(&reference.key, BoundaryResourceKey::Holder(_))).expect("actual demanded original Holder root");
    let BoundaryResourceKey::Holder(holder) = &demanded.key else { unreachable!(); };
    let mut producer = &holder.producer;
    while let HolderProducerRef::Forward { previous, .. } = producer { producer = previous; }
    let HolderProducerRef::CallCreation { selection, .. } = producer else { panic!("actual selected CallCreation ancestry"); };
    let makers = forest.sources.iter().filter(|row| row.producer == selection.source.table.owner).collect::<Vec<_>>();
    let [maker] = makers.as_slice() else { panic!("unique actual maker"); };
    let original = maker.resource_construction.original();
    let returns = original.producer_return_cases.iter().filter(|fact| fact.table == selection.source.table && fact.return_point == selection.source.return_point
        && fact.output == selection.source.output && fact.alternative == selection.source.alternative).collect::<Vec<_>>();
    let [returned] = returns.as_slice() else { panic!("exact genuine selected return case"); };
    let output = CallableHolderPlace::Output(returned.output);
    let connected = |before: &[CallableHolderRelation], after: &[CallableHolderRelation]| {
        before.iter().any(|relation| relation.environment == returned.environment && relation.holders.contains(&returned.holder))
            && after.iter().any(|relation| relation.environment == returned.environment && relation.holders.contains(&output))
    };
    let edges = original.holder_edges.iter().filter(|edge| edge.from == returned.return_point && edge.actions.contains(&returned.transfer) && connected(&edge.before, &edge.after)).collect::<Vec<_>>();
    assert!(!edges.is_empty(), "actual parsed edge row holds source BEFORE and output AFTER together");
    assert!(original.points.iter().any(|point| point.id == returned.return_point && matches!(&point.kind, CfgPointKind::ReturnOutput { output, .. } if *output == returned.output) && !point.successors.is_empty()));
    assert!(!original.holder_exits.iter().any(|exit| exit.from == returned.return_point && exit.actions.contains(&returned.transfer)), "current parsed ReturnOutput has a successor; actual terminal row is a different point");
    assert!(creators::return_transfer_connected(original, returned));
    assert!(creators::validate(&demanded.owner, holder, &forest).expect("complete actual maker ancestry").contains(&maker.resource_construction));
    let catalog = original::catalog(&sources).expect("actual original caller resource constructors");
    let mut remap = ValueSchemaRemap {
        producers: sources.iter().map(|row| (row.producer.clone(), row.producer.clone())).collect(),
        points: sources.iter().flat_map(|row| row.initial_points.iter().map(|point| ValuePointRemap { producer: row.producer.clone(), point: point.id, imported: point.id })).collect(),
        shapes: sources.iter().flat_map(|row| row.tables.shapes.iter().map(|shape| (shape.id.clone(), shape.id.clone()))).collect(), imports: Vec::new(),
    };
    for row in &sources {
        let mut import = ValueImportCorrespondence { producer: row.producer.clone(), bindings: Vec::new(), places: Vec::new(), origins: row.resource_construction.original().facts.iter().enumerate().map(|(index, _)| ReferenceOriginId(index)).collect(), loans: Vec::new(), runtime_values: Vec::new(), environments: Vec::new(), backings: Vec::new(), holders: Vec::new() };
        for vertex in catalog.vertices.iter().filter(|vertex| vertex.reference.owner == row.producer) {
            match &vertex.reference.key {
                BoundaryResourceKey::Origin(_) => {}
                BoundaryResourceKey::Binding(id) => import.bindings.push((id.clone(), id.clone())),
                BoundaryResourceKey::Place(id) => import.places.push((id.clone(), id.clone())),
                BoundaryResourceKey::Loan(id) => import.loans.push((id.clone(), id.clone())),
                BoundaryResourceKey::Runtime(id) => import.runtime_values.push((id.clone(), id.clone())),
                BoundaryResourceKey::Environment(id) => import.environments.push((id.clone(), id.clone())),
                BoundaryResourceKey::Backing(id) => import.backings.push((id.clone(), id.clone())),
                BoundaryResourceKey::Holder(id) => import.holders.push((id.clone(), id.clone())),
            }
        }
        remap.imports.push(import);
    }
    for independent in [false, true] {
        let witness = if independent { remap.independent_boundary_destination(&sources, &forest) } else { remap.stage_boundary_sources(&sources, &forest) }.expect("actual original return-incidence witness");
        let saved = (sources.clone(), forest.clone(), remap.clone(), witness.clone());
        let full = if independent { remap.tables_into(&sources, &forest, &sources, &forest) } else { remap.tables(&sources, &forest) }.expect("actual edge-return full resource transport");
        let uses = remap.source_use_graphs_into(root, &sources[0].tables, &sources, &forest, &witness).expect("actual edge-return selective graphs");
        let stages = remap.staged_loan_queries_into(root, &sources[0].tables, &sources, &forest, &witness).expect("actual edge-return selective stages");
        for resources in [&full[0].resources, &uses.resources, &stages.resources] {
            assert!(resources.associations.iter().any(|pair| pair.source == demanded));
            assert_eq!(resources.capture_inputs.iter().map(|tuple| tuple.source.clone()).collect::<Vec<_>>(), tuples);
            assert!(resources.source_originals.contains(&maker.resource_construction));
        }
        assert_eq!((sources.clone(), forest.clone(), remap.clone(), witness.clone()), saved);
    }
    if !corrupt { return; }
    for defect in 0..11 {
        let mut candidate = original.clone();
        let mut matched = 0;
        let mut split = Vec::new();
        for edge in candidate.holder_edges.iter_mut().filter(|edge| edge.from == returned.return_point && edge.actions.contains(&returned.transfer)) {
            matched += 1;
            match defect {
                0 => edge.before.clear(),
                1 => edge.after.clear(),
                2 => for relation in &mut edge.before { relation.holders.retain(|holder| holder != &returned.holder); },
                3 => for relation in &mut edge.after { relation.holders.retain(|holder| holder != &output); },
                4 => for relation in &mut edge.before { if relation.environment == returned.environment { relation.environment.index = usize::MAX; } },
                5 => for relation in &mut edge.after { if relation.environment == returned.environment { relation.environment.index = usize::MAX; } },
                6 => for relation in &mut edge.after { for holder in &mut relation.holders { if *holder == output { *holder = CallableHolderPlace::Output(returned.output + 1); } } },
                7 => edge.actions.retain(|action| action != &returned.transfer),
                8 => for action in &mut edge.actions { if action == &returned.transfer { let CallableHolderAction::ReturnTransfer { from, .. } = action else { unreachable!(); }; *from = CallableHolderPlace::Output(usize::MAX); } },
                9 => {
                    let mut other = edge.clone(); other.before.clear(); split.push(other); edge.after.clear();
                }
                10 => edge.from = CfgPointId(usize::MAX),
                _ => unreachable!(),
            }
        }
        candidate.holder_edges.extend(split);
        assert!(matched > 0);
        assert!(!creators::return_transfer_connected(&candidate, returned), "same-row defect={defect}");
        let candidate_rows = capture_table_sources(&candidate, &maker.policy_context).expect("collector snapshot of explicit corrupted maker candidate, not original parsed evidence");
        let candidates = candidate_rows.iter().filter(|row| row.producer == maker.producer).collect::<Vec<_>>();
        let [candidate_row] = candidates.as_slice() else { panic!("exact corrupted maker carrier"); };
        let mut candidate_forest = forest.clone();
        let row = candidate_forest.sources.iter_mut().find(|row| row.producer == maker.producer).expect("exact maker snapshot slot");
        *row = (*candidate_row).clone();
        assert_eq!(creators::validate(&demanded.owner, holder, &candidate_forest), Err(ValueSchemaRemapError::MissingHolder(holder.clone())), "demanded root error defect={defect}");
        for independent in [false, true] {
            let witness = if independent { remap.independent_boundary_destination(&sources, &forest) } else { remap.stage_boundary_sources(&sources, &candidate_forest) }.expect("fresh source-candidate factory keeps actual selected headers");
            let saved = (sources.clone(), forest.clone(), candidate.clone(), candidate_forest.clone(), remap.clone(), witness.clone());
            let error = ValueSchemaRemapError::MissingHolder(holder.clone());
            assert_eq!(if independent { remap.tables_into(&sources, &candidate_forest, &sources, &forest) } else { remap.tables(&sources, &candidate_forest) }, Err(error.clone()), "source full defect={defect}");
            assert_eq!(remap.source_use_graphs_into(root, &sources[0].tables, &sources, &candidate_forest, &witness), Err(error.clone()), "source selective graphs defect={defect}");
            assert_eq!(remap.staged_loan_queries_into(root, &sources[0].tables, &sources, &candidate_forest, &witness), Err(error), "source selective stages defect={defect}");
            assert_eq!((sources.clone(), forest.clone(), candidate.clone(), candidate_forest.clone(), remap.clone(), witness.clone()), saved);
        }
        let witness = remap.independent_boundary_destination(&sources, &candidate_forest).expect("fresh target-candidate factory keeps actual selected headers");
        let saved = (sources.clone(), forest.clone(), candidate.clone(), candidate_forest.clone(), remap.clone(), witness.clone());
        let error = ValueSchemaRemapError::MissingHolder(holder.clone());
        assert_eq!(remap.tables_into(&sources, &forest, &sources, &candidate_forest), Err(error.clone()), "target full defect={defect}");
        assert_eq!(remap.source_use_graphs_into(root, &sources[0].tables, &sources, &forest, &witness), Err(error.clone()), "target selective graphs defect={defect}");
        assert_eq!(remap.staged_loan_queries_into(root, &sources[0].tables, &sources, &forest, &witness), Err(error), "target selective stages defect={defect}");
        assert_eq!((sources.clone(), forest.clone(), candidate.clone(), candidate_forest.clone(), remap.clone(), witness.clone()), saved);
        assert_eq!(original, maker.resource_construction.original());
    }
}

#[test]
fn edge_return_native64_program() { fixture(ScalarTargetLayout::NATIVE64, false, false); }
#[test]
fn edge_return_native64_project() { fixture(ScalarTargetLayout::NATIVE64, true, false); }
#[test]
fn edge_return_wasm32_program() { fixture(ScalarTargetLayout::WASM32, false, false); }
#[test]
fn edge_return_wasm32_project() { fixture(ScalarTargetLayout::WASM32, true, false); }
#[test]
fn same_row_return_negatives_native64_program() { fixture(ScalarTargetLayout::NATIVE64, false, true); }
#[test]
fn same_row_return_negatives_native64_project() { fixture(ScalarTargetLayout::NATIVE64, true, true); }
#[test]
fn same_row_return_negatives_wasm32_program() { fixture(ScalarTargetLayout::WASM32, false, true); }
#[test]
fn same_row_return_negatives_wasm32_project() { fixture(ScalarTargetLayout::WASM32, true, true); }

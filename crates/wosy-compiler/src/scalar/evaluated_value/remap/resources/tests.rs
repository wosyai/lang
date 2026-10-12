use super::*;

const MAKE: &str = "%%start\n(u8())() make = fn { u8() inner = fn { 1 }; inner };\nunit() run = fn { u8() first = make(); u8() probe = fn { first() }; probe(); };\n%%end";
const BRANCH: &str = "%%start\n(u8())() make_a = fn { u8() inner = fn { 1 }; inner };\n(u8())() make_b = fn { u8() inner = fn { 2 }; inner };\nunit(bool) run = fn(flag) { u8() target = if (flag) { make_a() } else { make_b() }; u8 tag = 7; tag; u8() probe = fn { target(); tag }; probe(); };\n%%end";
const BACKING: &str = "%%start\nunit() run = fn { u8 first = 1; u8()[2] readers = [fn { first }, fn { first }]; u8() probe = fn { readers[0]() }; u8() outer = fn { probe() }; outer(); };\n%%end";
const REBORROW: &str = "%%start\nunit() run = fn { u32 item = 1; *!u32 parent = &!item; unit() write = fn { *!u32 child = &!(*parent); *child = 2; }; write(); };\n%%end";
const CYCLE: &str = "%%start\nunit() run = fn { u8 first = 1; u8()[2] readers = [fn { first }, fn { first }]; u64 slot = 0; slot; bool again = true; while (again) { readers[slot] = fn { readers[1]() }; again = false; }; u8() probe = fn { readers[0]() }; probe(); };\n%%end";

fn cyclic(resources: &CheckedBoundaryResources) -> bool {
    for start in resources.arenas.iter().flat_map(|arena| &arena.vertices) {
        let mut pending = start.dependencies.clone();
        let mut visited = Vec::new();
        while let Some(id) = pending.pop() {
            if id == start.id { return true; }
            if visited.contains(&id) { continue; } visited.push(id.clone());
            let rows = resources.arenas.iter().flat_map(|arena| &arena.vertices).filter(|vertex| vertex.id == id).collect::<Vec<_>>();
            let [vertex] = rows.as_slice() else { panic!("every reserved cycle edge resolves uniquely"); };
            pending.extend(vertex.dependencies.clone());
        }
    }
    false
}

fn fixture(text: &str, layout: ScalarTargetLayout, project: bool) {
    let source_identity = SourceIdentity::new("resource-correction".into(), "tuples".into(), "main.w".into(), "r1".into());
    let parsed = crate::parse_source(source_identity.clone(), text.to_owned(), &[]);
    assert!(parsed.diagnostics.is_empty(), "{:?}", parsed.diagnostics);
    let (validation, _) = derive_scalar_program_observed_test(&parsed.result, layout);
    assert!(validation.diagnostics.is_empty(), "{:?}", validation.diagnostics);
    let module = if project {
        let validated = validate_scalar_project(ScalarProject::new(vec![ScalarModule::from_program(validation.program, Vec::new())], vec![source_identity]));
        assert!(validated.diagnostics.is_empty(), "{:?}", validated.diagnostics);
        validated.project.modules[0].clone()
    } else { ScalarModule::from_program(validation.program, Vec::new()) };
    let forest = source_boundary::module_preflight_forest(&module.source, &module.items, &module.top_level_reference_cfg, &module.structs, &module.enums, SourceCaptureConstructionPhase::Complete).expect("real original module forest");
    let function = module.items.iter().find_map(|item| match item { ScalarItem::Function(function) if function.name == "run" => Some(function), _ => None }).expect("real run declaration");
    let root = &function.reference_cfg.producer_owner;
    let mut owners = vec![root.clone()];
    let mut next = 0;
    while next < owners.len() {
        let rows = forest.sources.iter().filter(|row| row.producer == owners[next]).collect::<Vec<_>>();
        let [row] = rows.as_slice() else { panic!("unique original source"); };
        for child in &row.children { owners.push(child.child.clone()); }
        next += 1;
    }
    let sources = owners.iter().map(|owner| {
        let rows = forest.sources.iter().filter(|row| row.producer == *owner).collect::<Vec<_>>();
        let [row] = rows.as_slice() else { panic!("unique original source"); };
        (*row).clone()
    }).collect::<Vec<_>>();
    let catalog = original::catalog(&sources).expect("independently original resource catalog");
    let mut remap = ValueSchemaRemap {
        producers: sources.iter().map(|row| (row.producer.clone(), row.producer.clone())).collect(),
        points: sources.iter().flat_map(|row| row.initial_points.iter().map(|point| ValuePointRemap { producer: row.producer.clone(), point: point.id, imported: point.id })).collect(),
        shapes: sources.iter().flat_map(|row| row.tables.shapes.iter().map(|shape| (shape.id.clone(), shape.id.clone()))).collect(), imports: Vec::new(),
    };
    for source in &sources {
        let mut import = ValueImportCorrespondence { producer: source.producer.clone(), bindings: Vec::new(), places: Vec::new(), origins: source.resource_construction.original().facts.iter().enumerate().map(|(index, _)| ReferenceOriginId(index)).collect(), loans: Vec::new(), runtime_values: Vec::new(), environments: Vec::new(), backings: Vec::new(), holders: Vec::new() };
        for row in catalog.vertices.iter().filter(|row| row.reference.owner == source.producer) {
            match &row.reference.key {
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
    let originals = original_capture_tuples(&sources).expect("whole original inputs");
    let demanded = originals.iter().flat_map(|tuple| tuple.paths.iter().flat_map(|path| path.inputs.iter().flat_map(|input| match &input.origin {
        CaptureInputOriginFact::Value => Vec::new(), CaptureInputOriginFact::Callable { holders } => holders.clone(),
    }))).collect::<Vec<_>>();
    if text == MAKE || text == BRANCH {
        assert!(!demanded.is_empty());
        assert!(demanded.iter().any(|holder| matches!(&holder.producer, HolderProducerRef::Forward { previous, .. } if matches!(previous.as_ref(), HolderProducerRef::CallCreation { .. }))));
    }
    if text == BRANCH { assert!(originals.iter().any(|tuple| tuple.paths.len() >= 2)); }
    if text == MAKE || text == BRANCH {
        let owner = &originals.iter().find(|tuple| tuple.paths.iter().any(|path| path.inputs.iter().any(|input| matches!(&input.origin, CaptureInputOriginFact::Callable { holders } if !holders.is_empty())))).expect("actual demanded tuple").production.event.owner;
        let saved = (sources.clone(), forest.clone(), remap.clone());
        let holder = demanded[0].clone();
        let maker = creators::validate(owner, &holder, &forest).expect("all independently demanded creator snapshots").into_iter().find(|evidence| !owners.contains(&evidence.original().producer_owner)).expect("actual external creator");
        let mut absent = forest.clone();
        absent.sources.retain(|row| row.producer != maker.original().producer_owner);
        assert_eq!(creators::validate(owner, &holder, &absent), Err(ValueSchemaRemapError::MissingHolder(holder.clone())));
        assert_eq!((sources.clone(), forest.clone(), remap.clone()), saved);
        let mut erased = sources.clone();
        for row in &mut erased { row.tables.source_capture_productions.clear(); }
        let roots = demand::resource_demand_full(&erased).roots;
        for tuple in &originals { for root in capture_tuple_roots(tuple) { assert!(roots.contains(&root), "original tuple survives current production erasure"); } }
        let mut empty = sources.clone();
        let mut empty_forest = forest.clone();
        for row in &mut empty {
            row.tables.source_use_graphs.clear(); row.tables.staged_loan_queries.clear();
            let candidate = empty_forest.sources.iter_mut().find(|candidate| candidate.producer == row.producer).expect("actual retained carrier");
            candidate.tables = row.tables.clone();
            assert_eq!(candidate.resource_construction, row.resource_construction);
        }
        for independent in [false, true] {
        let witness = if independent { remap.independent_boundary_destination(&empty, &empty_forest) } else { remap.stage_boundary_sources(&empty, &empty_forest) }.expect("genuine empty selective boundary");
        let before = (empty.clone(), empty_forest.clone(), witness.clone());
        let uses = remap.source_use_graphs_into(root, &empty[0].tables, &empty, &empty_forest, &witness).expect("empty graph result retains mandatory tuples");
        let stages = remap.staged_loan_queries_into(root, &empty[0].tables, &empty, &empty_forest, &witness).expect("empty staged result retains mandatory tuples");
        assert!(uses.graphs.is_empty()); assert!(stages.queries.is_empty());
        for resources in [&uses.resources, &stages.resources] {
            assert_eq!(resources.capture_inputs.iter().map(|tuple| tuple.source.clone()).collect::<Vec<_>>(), originals);
            assert!(resources.associations.iter().any(|pair| pair.source.key == BoundaryResourceKey::Holder(holder.clone())));
        }
        assert_eq!((empty.clone(), empty_forest.clone(), witness.clone()), before);
        }
    }
    if text == MAKE || text == BRANCH {
        let mut copied_remap = remap.clone();
        for point in &mut copied_remap.points { point.imported = CfgPointId(point.point.0 + 16); }
        for import in &mut copied_remap.imports {
            import.bindings.clear(); import.places.clear(); import.loans.clear(); import.runtime_values.clear();
            import.environments.clear(); import.backings.clear(); import.holders.clear();
        }
        let constructors = copied_remap.clone();
        for row in &catalog.vertices {
            let value = copied::CopyResourceConstruction::new(&constructors, &row.reference.owner, &row.reference.owner, &forest).value(&row.value).expect("original constructor allocation before installing any proposed resource map");
            let import = copied_remap.imports.iter_mut().find(|import| import.producer == row.reference.owner).expect("actual selected owner");
            match (&row.reference.key, allocation::key(&value)) {
                (BoundaryResourceKey::Origin(original), BoundaryResourceKey::Origin(actual)) => assert_eq!(*original, actual),
                (BoundaryResourceKey::Binding(original), BoundaryResourceKey::Binding(actual)) => import.bindings.push((original.clone(), actual)),
                (BoundaryResourceKey::Place(original), BoundaryResourceKey::Place(actual)) => import.places.push((original.clone(), actual)),
                (BoundaryResourceKey::Loan(original), BoundaryResourceKey::Loan(actual)) => import.loans.push((original.clone(), actual)),
                (BoundaryResourceKey::Runtime(original), BoundaryResourceKey::Runtime(actual)) => import.runtime_values.push((original.clone(), actual)),
                (BoundaryResourceKey::Environment(original), BoundaryResourceKey::Environment(actual)) => { assert_eq!(original.index, actual.index); import.environments.push((original.clone(), actual)); }
                (BoundaryResourceKey::Backing(original), BoundaryResourceKey::Backing(actual)) => import.backings.push((original.clone(), actual)),
                (BoundaryResourceKey::Holder(original), BoundaryResourceKey::Holder(actual)) => import.holders.push((original.clone(), actual)),
                _ => panic!("typed copied allocation retains resource kind"),
            }
        }
        let witness = copied_remap.stage_boundary_sources(&sources, &forest).expect("nonidentity Copied point witness");
        let before = (sources.clone(), forest.clone(), copied_remap.clone(), witness.clone());
        let full = copied_remap.tables(&sources, &forest).expect("nonidentity Copied full tuple transport");
        let uses = copied_remap.source_use_graphs_into(root, &sources[0].tables, &sources, &forest, &witness).expect("nonidentity Copied selective uses");
        let stages = copied_remap.staged_loan_queries_into(root, &sources[0].tables, &sources, &forest, &witness).expect("nonidentity Copied selective stages");
        for resources in [&full[0].resources, &uses.resources, &stages.resources] {
            for tuple in &resources.capture_inputs {
                assert_eq!(tuple.destination.production.event.point.0, tuple.source.production.event.point.0 + 16);
                assert_eq!(tuple.destination.paths.len(), tuple.source.paths.len());
                for (original, mapped) in tuple.source.paths.iter().zip(&tuple.destination.paths) {
                    assert_eq!(original.capture_span, mapped.capture_span);
                    assert_eq!(mapped.capture.0, original.capture.0 + 16);
                    assert_eq!(original.inputs.len(), mapped.inputs.len());
                }
            }
            assert!(resources.source_originals.iter().any(|original| !owners.contains(&original.original().producer_owner)));
        }
        assert_eq!((sources.clone(), forest.clone(), copied_remap.clone(), witness.clone()), before);
    }
    if text == REBORROW {
        let mut candidate = sources.clone();
        let cfg = candidate[0].resource_construction.original();
        let origin = cfg.facts.iter().enumerate().find_map(|(index, fact)| match fact { ReferenceOrigin::Fresh { .. } | ReferenceOrigin::BorrowedFrom { .. } => Some(ReferenceOriginId(index)), _ => None }).expect("actual parsed checked origin");
        let point = cfg.initial_points.iter().find(|point| matches!(&point.kind, CfgPointKind::Borrow { .. })).expect("actual original borrow event").clone();
        let coordinate = candidate[0].tables.source_slots.iter().find(|slot| matches!(&slot.role, SourceCoordinateRole::Binding(_))).expect("actual declared source coordinate").id.clone();
        let event = SourceEventId { owner: candidate[0].producer.clone(), point: point.id };
        let state = CheckedValueState::Expired { origin, effect_point: point.id };
        let graph = candidate[0].tables.source_use_graphs.first_mut().expect("actual parsed source-use graph");
        graph.coverage.push(SourceUseCoverage { event, source_span: point.source_span, reason: SourceUseCoverageReason::CheckedState { coordinate, state: state.clone() } });
        let stage = candidate[0].tables.staged_loan_queries.first_mut().expect("actual parsed staged-loan analysis");
        let (index, _) = stage.epoch.roles.iter().enumerate().next().expect("actual original common role");
        let role = CommonLoanRoleId(index);
        let StagedLoanQueryResult::Candidates { pending, .. } = &mut stage.result else { panic!("actual parsed candidate stage"); };
        pending.push(LocalLoanPendingReason::CheckedState { role, state });
        let mut candidate_forest = forest.clone();
        for row in &candidate {
            candidate_forest.sources.iter_mut().find(|original| original.producer == row.producer).expect("retained original forest carrier").tables = row.tables.clone();
        }
        for independent in [false, true] {
            let witness = if independent { remap.independent_boundary_destination(&candidate, &candidate_forest) } else { remap.stage_boundary_sources(&candidate, &candidate_forest) }.expect("original-backed candidate reader headers");
            let saved = (candidate.clone(), candidate_forest.clone(), remap.clone(), witness.clone());
            let uses = remap.source_use_graphs_into(root, &candidate[0].tables, &candidate, &candidate_forest, &witness).expect("expired source-use reader pairs actual original origin");
            let stages = remap.staged_loan_queries_into(root, &candidate[0].tables, &candidate, &candidate_forest, &witness).expect("pending staged reader pairs actual original origin");
            for resources in [&uses.resources, &stages.resources] { assert!(resources.associations.iter().any(|pair| pair.source.owner == *root && pair.source.key == BoundaryResourceKey::Origin(origin))); }
            assert_eq!((candidate.clone(), candidate_forest.clone(), remap.clone(), witness.clone()), saved);
            let mut missing = remap.clone();
            missing.imports.iter_mut().find(|import| import.producer == *root).expect("root import").origins[origin.0] = ReferenceOriginId(usize::MAX);
            let fresh = if independent { missing.independent_boundary_destination(&candidate, &candidate_forest) } else { missing.stage_boundary_sources(&candidate, &candidate_forest) }.expect("fresh invalid-origin reader witness");
            let saved = (candidate.clone(), candidate_forest.clone(), missing.clone(), fresh.clone());
            let error = ValueSchemaRemapError::MissingOrigin { producer: root.clone(), origin };
            assert_eq!(missing.source_use_graphs_into(root, &candidate[0].tables, &candidate, &candidate_forest, &fresh), Err(error.clone()));
            assert_eq!(missing.staged_loan_queries_into(root, &candidate[0].tables, &candidate, &candidate_forest, &fresh), Err(error));
            assert_eq!((candidate.clone(), candidate_forest.clone(), missing.clone(), fresh.clone()), saved);
        }
    }
    for independent in [false, true] {
        let witness = if independent { remap.independent_boundary_destination(&sources, &forest) } else { remap.stage_boundary_sources(&sources, &forest) }.expect("actual resource witness");
        let before = (sources.clone(), forest.clone(), remap.clone(), witness.clone());
        let full = if independent { remap.tables_into(&sources, &forest, &sources, &forest) } else { remap.tables(&sources, &forest) }.expect("full real resource transport");
        let uses = remap.source_use_graphs_into(root, &sources[0].tables, &sources, &forest, &witness).expect("source-use real resource inputs");
        let stages = remap.staged_loan_queries_into(root, &sources[0].tables, &sources, &forest, &witness).expect("staged real resource inputs");
        for resources in full.iter().map(|row| &row.resources).chain([&uses.resources, &stages.resources]) {
            assert_eq!(resources.capture_inputs.iter().map(|tuple| tuple.source.clone()).collect::<Vec<_>>(), originals);
            for tuple in &resources.capture_inputs { assert_eq!(tuple.source, tuple.destination); }
            for holder in &demanded { assert!(resources.arenas.iter().flat_map(|arena| &arena.vertices).any(|vertex| vertex.value == BoundaryResourceValue::Holder(holder.clone()))); }
            for arena in &resources.arenas {
                for vertex in &arena.vertices {
                    for edge in &vertex.dependencies { assert_eq!(resources.arenas.iter().flat_map(|arena| &arena.vertices).filter(|vertex| vertex.id == *edge).count(), 1); }
                }
            }
            if text == MAKE || text == BRANCH {
                assert!(resources.source_originals.iter().any(|original| !owners.contains(&original.original().producer_owner)), "actual external maker snapshot retained");
            }
        }
        if text == BACKING {
            assert!(full.iter().flat_map(|row| &row.resources.arenas).flat_map(|arena| &arena.vertices).any(|vertex| matches!(&vertex.value, BoundaryResourceValue::Backing(backing) if backing.callable_family.iter().any(|family| family.elements.len() == 2 || family.partitions.iter().any(|partition| partition.representatives.len() >= 2)))));
        }
        if text == REBORROW {
            assert!(full.iter().flat_map(|row| &row.resources.arenas).flat_map(|arena| &arena.vertices).any(|vertex| matches!(&vertex.value, BoundaryResourceValue::Loan(loan) if loan.parent.is_some())));
        }
        if text == CYCLE {
            assert!(cyclic(&full[0].resources), "actual original environment/backing cycle");
            for tuple in &full[0].resources.capture_inputs { assert_eq!(tuple.source.paths, tuple.destination.paths); }
        }
        assert_eq!((sources.clone(), forest.clone(), remap.clone(), witness.clone()), before);
        let (_, correspondence) = witness.checked_headers(&remap, &sources, &forest).expect("unchanged constructor header associations");
        let mut kinds = Vec::new();
        for pair in &full[0].resources.associations {
            if !owners.contains(&pair.source.owner) { continue; }
            let kind = match &pair.source.key {
                BoundaryResourceKey::Origin(_) => 0, BoundaryResourceKey::Binding(_) => 1,
                BoundaryResourceKey::Place(_) => 2, BoundaryResourceKey::Loan(_) => 3,
                BoundaryResourceKey::Runtime(_) => 4, BoundaryResourceKey::Environment(_) => 5,
                BoundaryResourceKey::Backing(_) => 6, BoundaryResourceKey::Holder(_) => 7,
            };
            if kinds.contains(&kind) { continue; } kinds.push(kind);
            let mut missing = remap.clone();
            let import = missing.imports.iter_mut().find(|import| import.producer == pair.source.owner).expect("actual qualified import");
            match &pair.source.key {
                BoundaryResourceKey::Origin(origin) => import.origins[origin.0] = ReferenceOriginId(usize::MAX),
                BoundaryResourceKey::Binding(binding) => import.bindings.retain(|(original, _)| original != binding),
                BoundaryResourceKey::Place(place) => import.places.retain(|(original, _)| original != place),
                BoundaryResourceKey::Loan(loan) => import.loans.retain(|(original, _)| original != loan),
                BoundaryResourceKey::Runtime(runtime) => import.runtime_values.retain(|(original, _)| original != runtime),
                BoundaryResourceKey::Environment(environment) => import.environments.retain(|(original, _)| original != environment),
                BoundaryResourceKey::Backing(backing) => import.backings.retain(|(original, _)| original != backing),
                BoundaryResourceKey::Holder(holder) => import.holders.retain(|(original, _)| original != holder),
            }
            let saved = (sources.clone(), forest.clone(), missing.clone(), witness.clone());
            let demand = demand::resource_demand_full(&sources);
            assert_eq!(if independent {
                independent::validate_independent_resources(&missing, &sources, &forest, &sources, &forest, &demand, &correspondence)
            } else { allocation::allocate_copied_resources(&missing, &sources, &forest, &demand, &correspondence) }, Err(original::missing(&pair.source)), "actual demanded resource kind={kind}");
            assert_eq!((sources.clone(), forest.clone(), missing.clone(), witness.clone()), saved);
        }
        if text == BACKING || text == REBORROW || text == CYCLE { continue; }
        let holder = demanded[0].clone();
        let mut missing = remap.clone();
        for import in &mut missing.imports { import.holders.retain(|(original, _)| original != &holder); }
        let fresh = if independent { missing.independent_boundary_destination(&sources, &forest) } else { missing.stage_boundary_sources(&sources, &forest) }.expect("fresh missing-holder witness");
        let saved = (sources.clone(), forest.clone(), missing.clone(), fresh.clone());
        let error = ValueSchemaRemapError::MissingHolder(holder.clone());
        assert_eq!(missing.source_use_graphs_into(root, &sources[0].tables, &sources, &forest, &fresh), Err(error.clone()));
        assert_eq!(missing.staged_loan_queries_into(root, &sources[0].tables, &sources, &forest, &fresh), Err(error.clone()));
        assert_eq!(if independent { missing.tables_into(&sources, &forest, &sources, &forest) } else { missing.tables(&sources, &forest) }, Err(error));
        assert_eq!((sources.clone(), forest.clone(), missing.clone(), fresh.clone()), saved);
        if !independent { assert_eq!(missing.source_use_graphs_into(root, &sources[0].tables, &sources, &forest, &witness), Err(ValueSchemaRemapError::MissingProducer(root.clone()))); }
        for defect in 0..8 {
            let mut crossed = remap.clone();
            let mut target = holder.clone();
            let HolderProducerRef::Forward { point, action, previous, .. } = &mut target.producer else { panic!("actual Forward ancestry"); };
            let HolderProducerRef::CallCreation { selection, .. } = previous.as_mut() else { panic!("actual CallCreation ancestry"); };
            match defect {
                0 => target.environment.index = usize::MAX,
                1 => *point = CfgPointId(usize::MAX),
                2 => match action {
                    CallableHolderAction::Retain { environment, .. } | CallableHolderAction::Move { environment, .. } => environment.index = usize::MAX,
                    _ => panic!("actual binding Forward action"),
                },
                3 => selection.source.return_point = CfgPointId(usize::MAX),
                4 => selection.source.alternative.source_index = usize::MAX,
                5 => selection.source.output += 1,
                6 => *previous = Box::new(HolderProducerRef::CaptureCreation { point: CfgPointId(usize::MAX), environment: holder.environment.clone() }),
                7 => target.holder = CallableHolderPlace::Output(usize::MAX),
                _ => unreachable!(),
            }
            let mut changed = 0;
            for import in &mut crossed.imports {
                for (original, destination) in &mut import.holders {
                    if original == &holder { *destination = target.clone(); changed += 1; }
                }
            }
            assert!(changed > 0);
            let fresh = if independent { crossed.independent_boundary_destination(&sources, &forest) } else { crossed.stage_boundary_sources(&sources, &forest) }.expect("fresh wrong-constructor witness");
            let saved = (sources.clone(), forest.clone(), crossed.clone(), fresh.clone());
            let error = ValueSchemaRemapError::MissingHolder(holder.clone());
            assert_eq!(crossed.source_use_graphs_into(root, &sources[0].tables, &sources, &forest, &fresh), Err(error.clone()), "defect={defect}");
            assert_eq!(crossed.staged_loan_queries_into(root, &sources[0].tables, &sources, &forest, &fresh), Err(error.clone()), "defect={defect}");
            assert_eq!(if independent { crossed.tables_into(&sources, &forest, &sources, &forest) } else { crossed.tables(&sources, &forest) }, Err(error), "defect={defect}");
            assert_eq!((sources.clone(), forest.clone(), crossed.clone(), fresh.clone()), saved);
        }
        if text == BRANCH {
            let other = demanded.iter().find(|candidate| **candidate != holder).expect("genuine other grouped alternative");
            let mut crossed = remap.clone();
            for import in &mut crossed.imports {
                for (original, target) in &mut import.holders { if *original == holder { *target = other.clone(); } }
            }
            let fresh = if independent { crossed.independent_boundary_destination(&sources, &forest) } else { crossed.stage_boundary_sources(&sources, &forest) }.expect("fresh existing crossed-alternative witness");
            let saved = (sources.clone(), forest.clone(), crossed.clone(), fresh.clone());
            let error = ValueSchemaRemapError::MissingHolder(holder.clone());
            assert_eq!(crossed.source_use_graphs_into(root, &sources[0].tables, &sources, &forest, &fresh), Err(error.clone()));
            assert_eq!(crossed.staged_loan_queries_into(root, &sources[0].tables, &sources, &forest, &fresh), Err(error.clone()));
            assert_eq!(if independent { crossed.tables_into(&sources, &forest, &sources, &forest) } else { crossed.tables(&sources, &forest) }, Err(error));
            assert_eq!((sources.clone(), forest.clone(), crossed.clone(), fresh.clone()), saved);
        }
    }
}

#[test]
fn whole_make_native64_program() { fixture(MAKE, ScalarTargetLayout::NATIVE64, false); }
#[test]
fn whole_make_native64_project() { fixture(MAKE, ScalarTargetLayout::NATIVE64, true); }
#[test]
fn whole_make_wasm32_program() { fixture(MAKE, ScalarTargetLayout::WASM32, false); }
#[test]
fn whole_make_wasm32_project() { fixture(MAKE, ScalarTargetLayout::WASM32, true); }
#[test]
fn whole_branch_native64_program() { fixture(BRANCH, ScalarTargetLayout::NATIVE64, false); }
#[test]
fn whole_branch_native64_project() { fixture(BRANCH, ScalarTargetLayout::NATIVE64, true); }
#[test]
fn whole_branch_wasm32_program() { fixture(BRANCH, ScalarTargetLayout::WASM32, false); }
#[test]
fn whole_branch_wasm32_project() { fixture(BRANCH, ScalarTargetLayout::WASM32, true); }

#[test]
fn grouped_backing_native64_program() { fixture(BACKING, ScalarTargetLayout::NATIVE64, false); }
#[test]
fn grouped_backing_native64_project() { fixture(BACKING, ScalarTargetLayout::NATIVE64, true); }
#[test]
fn grouped_backing_wasm32_program() { fixture(BACKING, ScalarTargetLayout::WASM32, false); }
#[test]
fn grouped_backing_wasm32_project() { fixture(BACKING, ScalarTargetLayout::WASM32, true); }
#[test]
fn parent_loan_native64_program() { fixture(REBORROW, ScalarTargetLayout::NATIVE64, false); }
#[test]
fn parent_loan_native64_project() { fixture(REBORROW, ScalarTargetLayout::NATIVE64, true); }
#[test]
fn parent_loan_wasm32_program() { fixture(REBORROW, ScalarTargetLayout::WASM32, false); }
#[test]
fn parent_loan_wasm32_project() { fixture(REBORROW, ScalarTargetLayout::WASM32, true); }
#[test]
fn reserved_cycle_native64_program() { fixture(CYCLE, ScalarTargetLayout::NATIVE64, false); }
#[test]
fn reserved_cycle_native64_project() { fixture(CYCLE, ScalarTargetLayout::NATIVE64, true); }
#[test]
fn reserved_cycle_wasm32_program() { fixture(CYCLE, ScalarTargetLayout::WASM32, false); }
#[test]
fn reserved_cycle_wasm32_project() { fixture(CYCLE, ScalarTargetLayout::WASM32, true); }

fn alias_source(identity: SourceIdentity, layout: ScalarTargetLayout, project: bool) -> (Vec<ValueTableSource>, SourceCaptureConstructionForest) {
    let text = "%%start\nunit() run = fn { u32 item = 1; *!u32 parent = &!item; *!u32 child = &!(*parent); *child = 2; };\n%%end";
    let parsed = crate::parse_source(identity.clone(), text.into(), &[]);
    assert!(parsed.diagnostics.is_empty(), "{:?}", parsed.diagnostics);
    let (validation, _) = derive_scalar_program_observed_test(&parsed.result, layout);
    assert!(validation.diagnostics.is_empty(), "{:?}", validation.diagnostics);
    let module = if project {
        let validated = validate_scalar_project(ScalarProject::new(vec![ScalarModule::from_program(validation.program, Vec::new())], vec![identity]));
        assert!(validated.diagnostics.is_empty(), "{:?}", validated.diagnostics);
        validated.project.modules[0].clone()
    } else { ScalarModule::from_program(validation.program, Vec::new()) };
    let forest = source_boundary::module_preflight_forest(&module.source, &module.items, &module.top_level_reference_cfg, &module.structs, &module.enums, SourceCaptureConstructionPhase::Complete).expect("independent real alias-source forest");
    let function = module.items.iter().find_map(|item| match item { ScalarItem::Function(function) if function.name == "run" => Some(function), _ => None }).expect("actual run constructor");
    let sources = forest.sources.iter().filter(|row| row.producer == function.reference_cfg.producer_owner).cloned().collect::<Vec<_>>();
    assert_eq!(sources.len(), 1);
    assert!(sources[0].children.is_empty());
    (sources, forest)
}

fn independent_aliases(layout: ScalarTargetLayout, project: bool) {
    let (left, mut forest) = alias_source(SourceIdentity::new("resources".into(), "left".into(), "left.w".into(), "r1".into()), layout, project);
    let (right, right_forest) = alias_source(SourceIdentity::new("resources".into(), "right".into(), "right.w".into(), "r1".into()), layout, project);
    let destination_forest = forest.clone();
    let destination = left.clone();
    forest.roots.extend(right_forest.roots);
    forest.sources.extend(right_forest.sources);
    let mut source = left;
    source.extend(right);
    let target = &destination[0];
    let target_catalog = original::catalog(&destination).expect("genuine target constructor catalog");
    let original_catalog = original::catalog(&source).expect("genuine two-source constructor catalog");
    let mut remap = ValueSchemaRemap { producers: source.iter().map(|row| (row.producer.clone(), target.producer.clone())).collect(), points: Vec::new(), shapes: Vec::new(), imports: Vec::new() };
    for row in &source {
        for (original, actual) in row.initial_points.iter().zip(&target.initial_points) {
            assert_eq!(original.source_span.range, actual.source_span.range);
            remap.points.push(ValuePointRemap { producer: row.producer.clone(), point: original.id, imported: actual.id });
        }
        for (original, actual) in row.tables.shapes.iter().zip(&target.tables.shapes) {
            assert_eq!(original.ty, actual.ty);
            remap.shapes.push((original.id.clone(), actual.id.clone()));
        }
        let mut import = ValueImportCorrespondence { producer: row.producer.clone(), bindings: Vec::new(), places: Vec::new(), origins: row.resource_construction.original().facts.iter().enumerate().map(|(index, _)| {
            assert!(target.resource_construction.original().facts.get(index).is_some()); ReferenceOriginId(index)
        }).collect(), loans: Vec::new(), runtime_values: Vec::new(), environments: Vec::new(), backings: Vec::new(), holders: Vec::new() };
        for original in original_catalog.vertices.iter().filter(|vertex| vertex.reference.owner == row.producer) {
            match &original.reference.key {
                BoundaryResourceKey::Origin(_) => {}
                BoundaryResourceKey::Binding(binding) => {
                    let matches = target_catalog.vertices.iter().filter_map(|vertex| match &vertex.reference.key {
                        BoundaryResourceKey::Binding(actual) if actual.function_span == binding.function_span && actual.declaration_span == binding.declaration_span && actual.block_span == binding.block_span && actual.kind == binding.kind => Some(actual), _ => None,
                    }).collect::<Vec<_>>();
                    let [actual] = matches.as_slice() else { panic!("unique actual counterpart binding"); };
                    import.bindings.push((binding.clone(), (*actual).clone()));
                }
                BoundaryResourceKey::Place(place) => {
                    let matches = target_catalog.vertices.iter().filter_map(|vertex| match &vertex.reference.key {
                        BoundaryResourceKey::Place(actual) if actual.function_span == place.function_span && actual.declaration_span == place.declaration_span && actual.block_span == place.block_span && actual.place == place.place && actual.projections.len() == place.projections.len() => Some(actual), _ => None,
                    }).collect::<Vec<_>>();
                    let [actual] = matches.as_slice() else { panic!("unique actual counterpart place"); };
                    import.places.push((place.clone(), (*actual).clone()));
                }
                BoundaryResourceKey::Loan(loan) => {
                    let matches = target_catalog.vertices.iter().filter_map(|vertex| match &vertex.reference.key {
                        BoundaryResourceKey::Loan(actual) if actual.creation_span == loan.creation_span && actual.mode == loan.mode && actual.origin_place.place == loan.origin_place.place => Some(actual), _ => None,
                    }).collect::<Vec<_>>();
                    let [actual] = matches.as_slice() else { panic!("unique actual counterpart loan"); };
                    import.loans.push((loan.clone(), (*actual).clone()));
                }
                BoundaryResourceKey::Runtime(_) | BoundaryResourceKey::Environment(_) | BoundaryResourceKey::Backing(_) | BoundaryResourceKey::Holder(_) => panic!("this actual primitive reborrow constructor has no callable/runtime inputs"),
            }
        }
        remap.imports.push(import);
    }
    let witness = remap.independent_boundary_destination(&destination, &destination_forest).expect("actual many-source target witness");
    let before = (source.clone(), forest.clone(), destination.clone(), destination_forest.clone(), remap.clone(), witness.clone());
    let result = remap.tables_into(&source, &forest, &destination, &destination_forest).expect("actual nonidentity many-source resource pair");
    assert_eq!(result.len(), 2);
    let resources = &result[0].resources;
    assert!(resources.associations.iter().any(|pair| pair.source.owner != pair.destination.owner));
    assert!(resources.associations.iter().any(|left| resources.associations.iter().any(|right| left.source.owner != right.source.owner && left.destination == right.destination && left.destination_vertex == right.destination_vertex)));
    for original in &source {
        let uses = remap.source_use_graphs_into(&original.producer, &original.tables, &source, &forest, &witness).expect("aliased selective source-use resources");
        let stages = remap.staged_loan_queries_into(&original.producer, &original.tables, &source, &forest, &witness).expect("aliased selective staged resources");
        assert!(uses.resources.associations.iter().any(|pair| pair.source.owner == original.producer));
        assert!(stages.resources.associations.iter().any(|pair| pair.source.owner == original.producer));
    }
    assert_eq!((source.clone(), forest.clone(), destination.clone(), destination_forest.clone(), remap.clone(), witness.clone()), before);
}

#[test]
fn nonidentity_aliases_native64_program() { independent_aliases(ScalarTargetLayout::NATIVE64, false); }
#[test]
fn nonidentity_aliases_native64_project() { independent_aliases(ScalarTargetLayout::NATIVE64, true); }
#[test]
fn nonidentity_aliases_wasm32_program() { independent_aliases(ScalarTargetLayout::WASM32, false); }
#[test]
fn nonidentity_aliases_wasm32_project() { independent_aliases(ScalarTargetLayout::WASM32, true); }

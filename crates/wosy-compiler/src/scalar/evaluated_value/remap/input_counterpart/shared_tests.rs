use super::*;

const MAKE: &str = "%%start\n(u8())() make = fn { u8() inner = fn { 1 }; inner };\nunit() run = fn { u8() first = make(); u8() probe = fn { first() }; probe(); };\n%%end";
const BRANCH: &str = "%%start\n(u8())() make_a = fn { u8() inner = fn { 1 }; inner };\n(u8())() make_b = fn { u8() inner = fn { 2 }; inner };\nunit(bool) run = fn(flag) { u8() target = if (flag) { make_a() } else { make_b() }; u8 tag = 7; tag; u8() probe = fn { target(); tag }; probe(); };\n%%end";

fn fixture(text: &str, layout: ScalarTargetLayout, project: bool) {
    let identity = SourceIdentity::new("i1-correction".into(), "shared".into(), "main.w".into(), "r1".into());
    let parsed = crate::parse_source(identity.clone(), text.into(), &[]);
    assert!(parsed.diagnostics.is_empty(), "{:?}", parsed.diagnostics);
    let (validation, _) = derive_scalar_program_observed_test(&parsed.result, layout);
    assert!(validation.diagnostics.is_empty(), "{:?}", validation.diagnostics);
    let module = if project {
        let validation = validate_scalar_project(ScalarProject::new(vec![ScalarModule::from_program(validation.program, Vec::new())], vec![identity]));
        assert!(validation.diagnostics.is_empty(), "{:?}", validation.diagnostics);
        validation.project.modules[0].clone()
    } else { ScalarModule::from_program(validation.program, Vec::new()) };
    let forest = source_boundary::module_preflight_forest(&module.source, &module.items, &module.top_level_reference_cfg, &module.structs, &module.enums, SourceCaptureConstructionPhase::Complete).expect("actual original makers and selected caller forest");
    let function = module.items.iter().find_map(|item| match item { ScalarItem::Function(function) if function.name == "run" => Some(function), _ => None }).expect("original caller selected apart from external makers");
    let input = source_boundary::preflight_input(&function.reference_cfg.producer_owner, &forest, BTreeMap::new()).expect("original caller context");
    let source = capture_table_sources(&function.reference_cfg, &input.policy_context).expect("actual selected caller/literal closure");
    let catalog = original::catalog(&source).expect("genuine provisional original identity resource keys");
    let mut remap = ValueSchemaRemap { producers: source.iter().map(|row| (row.producer.clone(), row.producer.clone())).collect(), points: source.iter().flat_map(|row| row.initial_points.iter().map(|point| ValuePointRemap { producer: row.producer.clone(), point: point.id, imported: point.id })).collect(), shapes: source.iter().flat_map(|row| row.tables.shapes.iter().map(|shape| (shape.id.clone(), shape.id.clone()))).collect(), imports: Vec::new() };
    for row in &source {
        let mut import = ValueImportCorrespondence { producer: row.producer.clone(), bindings: Vec::new(), places: Vec::new(), origins: row.resource_construction.original().facts.iter().enumerate().map(|(index, _)| ReferenceOriginId(index)).collect(), loans: Vec::new(), runtime_values: Vec::new(), environments: Vec::new(), backings: Vec::new(), holders: Vec::new() };
        for vertex in catalog.vertices.iter().filter(|vertex| vertex.reference.owner == row.producer) {
            match &vertex.reference.key {
                BoundaryResourceKey::Origin(_) => {},
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
    let witness = remap.independent_boundary_destination(&source, &forest).expect("actual Independent original caller witness");
    let (_, correspondence) = witness.checked_headers(&remap, &source, &forest).expect("explicit mapped selected headers");
    let before = (source.clone(), forest.clone(), remap.clone(), witness.clone());
    let full = remap.tables_into(&source, &forest, &source, &forest).expect("shared original makers survive productive type extension and strict payload validation");
    let snapshots = full[0].resources.source_originals().iter().filter(|snapshot| !source.iter().any(|row| row.producer == snapshot.original().producer_owner)).collect::<Vec<_>>();
    assert!(!snapshots.is_empty());
    let mut empty_seen = false;
    for row in &source {
        let uses = remap.source_use_graphs_into(&row.producer, &row.tables, &source, &forest, &witness).expect("shared creator evidence in owned source-use selection");
        let stages = remap.staged_loan_queries_into(&row.producer, &row.tables, &source, &forest, &witness).expect("shared creator evidence in owned staged selection");
        empty_seen |= uses.graphs.is_empty() || stages.queries.is_empty();
        for resources in full.iter().map(|row| &row.resources).chain([&uses.resources, &stages.resources]) {
            input_counterpart::postvalidate(&remap, &source, &correspondence, resources).expect("same original mapped/shared qualifier repeated in ALL owned evidence postchecks");
            let input_counterpart::BoundaryInputTypeEvidence::Independent(checked) = resources.input_types() else { panic!("mandatory retained original proof"); };
            for snapshot in &snapshots {
                let cfg = snapshot.original();
                assert!(!remap.producers.iter().any(|pair| pair.0 == cfg.producer_owner));
                assert!(resources.source_originals().contains(snapshot));
                let BoundaryResourceProvenance::Independent { originals } = resources.provenance() else { unreachable!(); };
                assert!(originals.contains(snapshot));
                for shape in &cfg.evaluated_values.shapes {
                    assert!(checked.type_pair(&cfg.producer_owner, &shape.ty, &shape.ty));
                    assert_eq!(checked.extend_original_types(&remap, &cfg.producer_owner, &[shape.ty.clone()], &[shape.ty.clone()], &shape.id).expect("same retained shared original finite extension with exact IDs"), *checked);
                }
            }
            postvalidation::full(&remap, &source, &full, &correspondence, &full[0].resources).expect("full original fields/resource/tuple/header/evidence postcheck");
        }
    }
    assert!(empty_seen);
    let mut empty = source.clone();
    let mut empty_forest = forest.clone();
    for row in &mut empty {
        row.tables.source_use_graphs.clear();
        row.tables.staged_loan_queries.clear();
        empty_forest.sources.iter_mut().find(|actual| actual.producer == row.producer).expect("matching current empty-selection carrier").tables = row.tables.clone();
    }
    let empty_witness = remap.independent_boundary_destination(&empty, &empty_forest).expect("original snapshots remain genuine around empty current selections");
    let (_, empty_correspondence) = empty_witness.checked_headers(&remap, &empty, &empty_forest).expect("exact empty operation forests retained");
    let empty_before = (empty.clone(), empty_forest.clone(), empty_witness.clone());
    for row in &empty {
        let uses = remap.source_use_graphs_into(&row.producer, &row.tables, &empty, &empty_forest, &empty_witness).expect("empty source-use output still demands shared makers");
        let stages = remap.staged_loan_queries_into(&row.producer, &row.tables, &empty, &empty_forest, &empty_witness).expect("empty staged output still demands shared makers");
        assert!(uses.graphs.is_empty());
        assert!(stages.queries.is_empty());
        for resources in [&uses.resources, &stages.resources] {
            input_counterpart::postvalidate(&remap, &empty, &empty_correspondence, resources).expect("empty owned cut repeats actual shared original proof");
            for snapshot in &snapshots { assert!(resources.source_originals().contains(snapshot)); }
        }
    }
    assert_eq!((empty, empty_forest, empty_witness), empty_before);
    assert_eq!((source, forest, remap, witness), before);
}

#[test]
fn shared_make_native64_program() { fixture(MAKE, ScalarTargetLayout::NATIVE64, false); }
#[test]
fn shared_make_native64_project() { fixture(MAKE, ScalarTargetLayout::NATIVE64, true); }
#[test]
fn shared_make_wasm32_program() { fixture(MAKE, ScalarTargetLayout::WASM32, false); }
#[test]
fn shared_make_wasm32_project() { fixture(MAKE, ScalarTargetLayout::WASM32, true); }
#[test]
fn shared_branch_native64_program() { fixture(BRANCH, ScalarTargetLayout::NATIVE64, false); }
#[test]
fn shared_branch_native64_project() { fixture(BRANCH, ScalarTargetLayout::NATIVE64, true); }
#[test]
fn shared_branch_wasm32_program() { fixture(BRANCH, ScalarTargetLayout::WASM32, false); }
#[test]
fn shared_branch_wasm32_project() { fixture(BRANCH, ScalarTargetLayout::WASM32, true); }

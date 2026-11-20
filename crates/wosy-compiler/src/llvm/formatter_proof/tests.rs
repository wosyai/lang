use super::*;
use crate::{
    analyze_scalar_build, derive_scalar_declarations_from_cst, parse_source,
    prepare_scalar_project, ScalarNamespaceBinding, ScalarPreparationInputs, ScalarTargetLayout,
};
use std::cell::Cell;
use wosy_syntax::{ByteSpan, SourceIdentity};

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Withheld {
    LiteralField(Body, usize),
    Formal(Formal),
}

thread_local! {
    static WITHHELD: Cell<Option<Withheld>> = const { Cell::new(None) };
}

#[test]
fn formatter_proof_dominance_rejects_unreachable_and_bypass() {
    use crate::scalar::{ReferenceFlowPoint, ReferenceScopeId};
    use wosy_syntax::SourceSpan;

    let span = SourceSpan::new(
        SourceIdentity::new("p".into(), "pkg".into(), "main.w".into(), "r1".into()),
        ByteSpan::new(0, 0),
    );
    let points = [&[1, 3][..], &[2], &[3], &[3], &[]]
        .into_iter()
        .enumerate()
        .map(|(index, successors)| ReferenceFlowPoint {
            id: CfgPointId(index),
            kind: CfgPointKind::Entry,
            source_span: span.clone(),
            scope: ReferenceScopeId(0),
            possible_origins: None,
            successors: successors.iter().copied().map(CfgPointId).collect(),
        })
        .collect();
    let cfg = ScalarReferenceCfg {
        points,
        facts: Vec::new(),
        initial_points: Vec::new(),
        allocations: BTreeSet::new(),
        specialized_policies: Vec::new(),
        primary_call_points: Default::default(),
    };
    let work = Cell::new(Work::default());
    let summary = AllocationDominance::new(&cfg, CfgPointId(1), &work).unwrap();
    assert!(summary.dominates(CfgPointId(2)));
    for use_point in [4, 1, 3, 5] {
        assert!(
            !summary.dominates(CfgPointId(use_point)),
            "point {use_point}"
        );
    }
    assert_eq!(
        (
            work.get().dominance_walks,
            work.get().points,
            work.get().arcs
        ),
        (2, 6, 8)
    );
    let absent = AllocationDominance::new(&cfg, CfgPointId(4), &work).unwrap();
    assert!(!absent.dominates(CfgPointId(2)));
}

pub(super) fn is_withheld(evidence: Withheld) -> bool {
    WITHHELD.get() == Some(evidence)
}

#[test]
fn formatter_proof_batch_indexes_once_and_preserves_edge_order() {
    let source = |path: &str, revision: &str| {
        SourceIdentity::new("p".into(), "pkg".into(), path.into(), revision.into())
    };
    let main = source("main.w", "r1");
    let other = source("other.w", "r1");
    let revised = source("main.w", "r2");
    let program = "
struct Packet { *?u8 address; u64 length; }
unit(u8[]) compact = fn(buffer) { buffer[0] = 1; };
unit(u8[], u8) digits = fn(buffer, value) {
    if (value == 0) { compact(buffer); } else {
        if (value == 1) { compact(buffer); } else { digits(buffer, value - 1); };
    };
};
Packet(u8) first = fn(value) {
    u8[3] scratch;
    if (value == 0) { digits(scratch, value); } else { digits(scratch, value); };
    *?u8 data = null;
    unsafe { data = &?scratch[0]; };
    Packet result = { .address = data; .length = 1; };
    result
};
Packet(u8) second = fn(value) {
    u8[3] scratch;
    digits(scratch, value);
    *?u8 data = null;
    unsafe { data = &?scratch[0]; };
    Packet result = { .address = data; .length = 1; };
    result
};
";
    let declarations = |source, text| {
        let parsed = parse_source(source, text, &[]);
        assert!(parsed.diagnostics.is_empty(), "{:?}", parsed.diagnostics);
        derive_scalar_declarations_from_cst(parsed.result.canonical_cst())
    };
    for target_layout in [ScalarTargetLayout::NATIVE64, ScalarTargetLayout::WASM32] {
        let mut baseline = None;
        for noise in [0, 64] {
            let unrelated = (0..noise)
                .map(|i| format!("u8() unused_{i} = fn {{ 1 }};\n"))
                .collect::<String>();
            let prepared = prepare_scalar_project(
                vec![
                    (
                        declarations(main.clone(), format!("%%start\n{program}{unrelated}%%end")),
                        Vec::new(),
                    ),
                    (
                        declarations(
                            other.clone(),
                            format!("%%start\nstruct Other {{ u8 value; }}\n{unrelated}%%end"),
                        ),
                        Vec::new(),
                    ),
                ],
                vec![main.clone(), other.clone()],
                ScalarPreparationInputs { target_layout },
            )
            .unwrap_or_else(|error| panic!("{:?}", error.diagnostics));
            let build = analyze_scalar_build(&prepared);
            assert!(build.diagnostics().is_empty(), "{:?}", build.diagnostics());
            let modules = &prepared.project().modules;
            let m = modules
                .iter()
                .position(|module| module.source == main)
                .unwrap();
            let n = modules
                .iter()
                .position(|module| module.source == other)
                .unwrap();
            let find = |name| {
                modules[m]
                    .items
                    .iter()
                    .enumerate()
                    .find_map(|(i, item)| match item {
                        ScalarItem::Function(function) if function.name == name => {
                            Some((i, function.name_span))
                        }
                        _ => None,
                    })
                    .unwrap()
            };
            let (compact, _) = find("compact");
            let (digits, digits_span) = find("digits");
            let id = &modules[m].structs[0].id;
            let other_id = &modules[n].structs[0].id;
            let revised_id = ScalarStructId {
                source: revised.clone(),
                index: id.index,
            };
            let mut batch = ProofBatch::new(&build);
            let initial = batch.work.get();
            assert_eq!(
                initial,
                Work {
                    index_builds: 1,
                    modules: modules.len(),
                    items: modules.iter().map(|module| module.items.len()).sum(),
                    structs: modules.iter().map(|module| module.structs.len()).sum(),
                    ..Work::default()
                }
            );
            assert_eq!(
                (initial.modules, initial.items, initial.structs),
                (2, 4 + noise * 2, 2)
            );

            let mut proofs = Vec::new();
            for (name, forward_count) in [("first", 2), ("second", 1)] {
                let (producer, _) = find(name);
                let before = batch.work.get();
                let proof = batch.prove(m, producer).unwrap();
                let after = batch.work.get();
                let cfg = build.reference_cfg(m, producer, None).unwrap();
                let arcs = cfg
                    .points
                    .iter()
                    .map(|point| point.successors.len())
                    .sum::<usize>();
                assert_eq!(after.dominance_walks - before.dominance_walks, 2);
                assert_eq!(
                    after.dominance_queries - before.dominance_queries,
                    2 + forward_count
                );
                assert!(after.points - before.points <= 2 * cfg.points.len());
                assert!(after.arcs - before.arcs <= 2 * arcs);
                assert_eq!(after.helper_checks - before.helper_checks, 2);
                assert_eq!(
                    after.definition_queries - before.definition_queries,
                    forward_count + 3
                );
                assert_eq!(after.struct_queries - before.struct_queries, 1);
                assert_eq!(
                    (
                        after.index_builds,
                        after.modules,
                        after.items,
                        after.structs
                    ),
                    (
                        initial.index_builds,
                        initial.modules,
                        initial.items,
                        initial.structs
                    )
                );
                let edges = proof
                    .edges
                    .iter()
                    .map(|edge| (edge.from, edge.site, edge.to))
                    .collect::<Vec<_>>();
                let routes = edges
                    .iter()
                    .map(|(from, _, to)| (*from, *to))
                    .collect::<Vec<_>>();
                let mut expected = vec![((m, producer), (m, digits, 0)); forward_count];
                expected.extend([
                    ((m, digits), (m, compact, 0)),
                    ((m, digits), (m, compact, 0)),
                    ((m, digits), (m, digits, 0)),
                ]);
                assert_eq!(routes, expected);
                assert_eq!(
                    edges
                        .iter()
                        .map(|(body, point, _)| (body, point))
                        .collect::<BTreeSet<_>>()
                        .len(),
                    edges.len()
                );
                assert!(cfg.allocations.contains(&proof.owner));
                proofs.push((proof.owner, proof.field, proof.output, edges));
            }
            if noise == 0 {
                baseline = Some(proofs);
            } else {
                assert_eq!(baseline.as_ref(), Some(&proofs));
            }

            assert_eq!(batch.definition(&main, digits_span), Ok((m, digits)));
            assert_eq!(
                batch.definition(&revised, digits_span),
                Err(IndexFailure::Missing)
            );
            batch.insert_definition(&other, digits_span, (n, 0));
            batch.insert_definition(&revised, digits_span, (n, 1));
            assert_eq!(batch.definition(&other, digits_span), Ok((n, 0)));
            assert_eq!(batch.definition(&revised, digits_span), Ok((n, 1)));
            batch.insert_definition(&main, digits_span, (n, 0));
            batch.insert_definition(&main, digits_span, (m, digits));
            assert_eq!(
                batch.definition(&main, digits_span),
                Err(IndexFailure::Ambiguous)
            );
            assert!(
                matches!(batch.prove(m, find("first").0), Err(Unproved::Definition(body, _, IndexFailure::Ambiguous)) if body == (m, find("first").0))
            );
            batch
                .definitions
                .get_mut(&source_key(&main))
                .unwrap()
                .remove(&span_key(digits_span));
            assert!(
                matches!(batch.prove(m, find("first").0), Err(Unproved::Definition(body, _, IndexFailure::Missing)) if body == (m, find("first").0))
            );

            assert_eq!(batch.structure(other_id).unwrap().id, *other_id);
            assert!(matches!(
                batch.structure(&revised_id),
                Err(IndexFailure::Missing)
            ));
            batch.insert_struct(&revised_id, (n, 0));
            assert_eq!(batch.structure(&revised_id).unwrap().id, *other_id);
            batch.insert_struct(id, (m, 0));
            assert!(matches!(batch.structure(id), Err(IndexFailure::Ambiguous)));
            assert_eq!(batch.structure(other_id).unwrap().id, *other_id);
            assert!(
                matches!(batch.prove(m, find("first").0), Err(Unproved::Unsupported(body, "ambiguous result struct")) if body == (m, find("first").0))
            );
            batch
                .structs
                .get_mut(&source_key(&id.source))
                .unwrap()
                .remove(&id.index);
            assert!(
                matches!(batch.prove(m, find("first").0), Err(Unproved::Missing(body, "result struct")) if body == (m, find("first").0))
            );
        }
    }
}

fn prove_withheld(
    build: &ScalarBuildValidation<'_>,
    producer: usize,
    evidence: Withheld,
) -> Result<Proof, Unproved> {
    let previous = WITHHELD.replace(Some(evidence));
    let result = prove(build, 0, producer);
    WITHHELD.set(previous);
    result
}

#[test]
fn formatter_proof_real_u8() {
    for target_layout in [ScalarTargetLayout::NATIVE64, ScalarTargetLayout::WASM32] {
        let source =
            |path: &str| SourceIdentity::new("p".into(), "std".into(), path.into(), "r1".into());
        let bootstrap = source("src/bootstrap.w");
        let preview = source("src/wasi/preview1.w");
        let declarations = |source, text: &str| {
            let parsed = parse_source(source, text.into(), &[]);
            assert!(parsed.diagnostics.is_empty(), "{:?}", parsed.diagnostics);
            derive_scalar_declarations_from_cst(parsed.result.canonical_cst())
        };
        let prepared = prepare_scalar_project(
            vec![
                (
                    declarations(
                        bootstrap.clone(),
                        include_str!("../../../../../stdlib/src/bootstrap.w"),
                    ),
                    vec![ScalarNamespaceBinding {
                        binding: "preview1".into(),
                        target: preview.clone(),
                        span: ByteSpan::new(0, 0),
                    }],
                ),
                (
                    declarations(
                        preview.clone(),
                        include_str!("../../../../../stdlib/src/wasi/preview1.w"),
                    ),
                    Vec::new(),
                ),
            ],
            vec![bootstrap, preview],
            ScalarPreparationInputs { target_layout },
        )
        .unwrap_or_else(|error| panic!("{:?}", error.diagnostics));
        let build = analyze_scalar_build(&prepared);
        assert!(build.diagnostics().is_empty(), "{:?}", build.diagnostics());
        let items = &prepared.project().modules[0].items;
        let find = |name| {
            items
                .iter()
                .position(|item| matches!(item, crate::ScalarItem::Function(f) if f.name == name))
                .unwrap()
        };
        let producer = find("_format_u8");
        let digits = find("_format_digits_u8");
        let compact = find("_format_compact_bytes");
        let proof = prove(&build, 0, producer).expect("typed u8 producer and helper component");
        assert_eq!(proof.output, 0);
        assert_eq!(proof.field.index, 0);
        assert!(build
            .reference_cfg(0, producer, None)
            .unwrap()
            .allocations
            .contains(&proof.owner));
        assert_eq!(
            proof
                .edges
                .iter()
                .map(|edge| (edge.from.1, edge.to.1, edge.to.2))
                .collect::<BTreeSet<_>>(),
            BTreeSet::from([
                (producer, digits, 0),
                (digits, digits, 0),
                (digits, compact, 0),
                (compact, compact, 0)
            ])
        );
        let without_field = prove_withheld(
            &build,
            producer,
            Withheld::LiteralField((0, producer), proof.field.index),
        );
        let without_formal = prove_withheld(&build, producer, Withheld::Formal((0, compact, 0)));
        assert!(
            matches!(without_field, Err(Unproved::Unsupported((0, body), "local allocation trace")) if body == producer),
            "{target_layout:?}: {without_field:?}"
        );
        assert!(
            matches!(without_formal, Err(Unproved::Missing((0, body), "helper formal")) if body == compact),
            "{target_layout:?}: {without_formal:?}"
        );
        for unsupported in ["_format_u128", "_format_i128", "_format_f32", "_format_f64"] {
            assert!(
                prove(&build, 0, find(unsupported)).is_err(),
                "{unsupported}"
            );
        }
        assert!(matches!(
            prove(&build, 0, items.len()),
            Err(Unproved::Missing(..))
        ));
    }
}

#[test]
fn formatter_proof_reaching_definition_and_effect_controls() {
    let cases = [
        ("reordered field and frozen snapshots", "unsafe { data, size = &?scratch[0], size + 1; };", "", true),
        ("disjoint same-spelled locals", "unsafe { u8[3] spare; spare[0] = 8; }; unsafe { u8[3] spare; spare[0] = 9; data = &?scratch[0]; };", "", true),
        ("missing store", "", "", false),
        ("wrong base", "unsafe { data = &?scratch[1]; };", "", false),
        ("dead inner owner", "unsafe { u8[3] inner; data = &?inner[0]; };", "", false),
        ("external backing", "unsafe { data = &?outside[0]; };", "", false),
        ("ambiguous definition", "unsafe { if (input == 0) { data = &?scratch[0]; } else { data = &?scratch[0]; }; };", "", false),
        ("release via helper address", "unsafe { data = &?scratch[0]; };", "unsafe { *?u8 p = &?buffer[0]; core.free(p); };", false),
        ("extern forwarding", "unsafe { data = &?scratch[0]; };", "unsafe { *?u8 p = &?buffer[0]; foreign.publish(p); };", false),
        ("address return from helper", "unsafe { data = &?scratch[0]; };", "unsafe { &?buffer[0] }", false),
    ];
    for target_layout in [ScalarTargetLayout::NATIVE64, ScalarTargetLayout::WASM32] {
        for (case, write, effect, expected) in cases {
            let source = SourceIdentity::new("p".into(), "p".into(), "case.w".into(), "r1".into());
            let helper_output = if case == "address return from helper" {
                "*?u8"
            } else {
                "unit"
            };
            let text = format!(
                "%%start
struct Packet {{ *?u8 address; u64 length; }}
u8[3] outside = [1, 2, 3];
foreign = extern wasm \"foreign\" {{ unit(*?u8) publish; }};
{helper_output}(u8[]) touch = fn(buffer) {{ buffer[0] = 7; {effect} }};
Packet(u8) make = fn(input) {{
    input;
    u8[3] scratch;
    touch(scratch);
    u64 size = 1;
    *?u8 data = null;
    {write}
    Packet result = {{ .length = size; .address = data; }};
    result
}};
%%end"
            );
            let parsed = parse_source(source.clone(), text, &[]);
            assert!(
                parsed.diagnostics.is_empty(),
                "{case}: {:?}",
                parsed.diagnostics
            );
            let prepared = prepare_scalar_project(
                vec![(
                    derive_scalar_declarations_from_cst(parsed.result.canonical_cst()),
                    Vec::new(),
                )],
                vec![source],
                ScalarPreparationInputs { target_layout },
            )
            .unwrap_or_else(|error| panic!("{case}: {:?}", error.diagnostics));
            let build = analyze_scalar_build(&prepared);
            assert!(
                build.diagnostics().is_empty(),
                "{case}: {:?}",
                build.diagnostics()
            );
            let item = prepared.project().modules[0]
                .items
                .iter()
                .position(|item| matches!(item, ScalarItem::Function(f) if f.name == "make"))
                .unwrap();
            let result = prove(&build, 0, item);
            assert_eq!(result.is_ok(), expected, "{case}: {result:?}");
            let stage = match case {
                "release via helper address" => Some("release or indirect storage effect"),
                "extern forwarding" => Some("address-bearing local"),
                "address return from helper" => Some("helper signature"),
                _ => None,
            };
            if let Some(stage) = stage {
                let helper = prepared.project().modules[0]
                    .items
                    .iter()
                    .position(|item| matches!(item, ScalarItem::Function(f) if f.name == "touch"))
                    .unwrap();
                assert!(
                    matches!(result, Err(Unproved::Unsupported((0, body), reason)) if body == helper && reason == stage),
                    "{case}: expected helper-stage {stage:?}, got {result:?}"
                );
            }
        }
    }
}

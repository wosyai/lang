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
    ConversionOperand(Body, CfgPointId),
    ConversionResult(Body, CfgPointId),
    TupleProjection(Body, CfgPointId),
    ReorderedTupleProjection(Body, CfgPointId),
    TupleArm(Body, CfgPointId),
    PrimitiveFact(Body, CfgPointId),
    ContradictoryPrimitiveFact(Body, CfgPointId),
    ContradictorySignedMagnitude(Body, CfgPointId),
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
                let mut expected =
                    vec![((m, producer), Dependency::Backing((m, digits, 0))); forward_count];
                expected.extend([
                    ((m, digits), Dependency::Backing((m, compact, 0))),
                    ((m, digits), Dependency::Backing((m, compact, 0))),
                    ((m, digits), Dependency::Backing((m, digits, 0))),
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
fn formatter_proof_real_simple_producers() {
    let mut strict_wide_failures = Vec::new();
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
        let module = &prepared.project().modules[0];
        let items = &module.items;
        let find = |name: &str| {
            items
                .iter()
                .position(|item| matches!(item, crate::ScalarItem::Function(f) if f.name == name))
                .unwrap()
        };
        let batch = ProofBatch::new(&build);
        let initial = batch.work.get();
        let field = &module
            .structs
            .iter()
            .find(|structure| structure.name == "utf8")
            .unwrap()
            .fields
            .iter()
            .find(|field| field.name == "data")
            .unwrap()
            .id;
        let call_site = |from: usize, to: usize| {
            let ScalarItem::Function(target) = &items[to] else {
                unreachable!()
            };
            let cfg = build.reference_cfg(0, from, None).unwrap();
            let mut matches = cfg.points.iter().filter(|point| {
                matches!(
                    &point.kind,
                    CfgPointKind::Call {
                        output: 0,
                        target: ResolvedCallTarget::ModuleCallable {
                            source,
                            declaration_span,
                            concrete: ConcreteCallSelection::Declaration,
                        },
                        ..
                    } if source == &module.source && declaration_span == &target.name_span
                )
            });
            let site = matches.next().unwrap().id;
            assert!(matches.next().is_none());
            site
        };
        let compact = find("_format_compact_bytes");
        let mut vertices = 0;
        let mut arcs = 0;
        for suffix in [
            "bool", "char", "i8", "i16", "i32", "i64", "u8", "u16", "u32", "u64",
        ] {
            let producer = find(&format!("_format_{suffix}"));
            let proof = batch
                .prove(0, producer)
                .unwrap_or_else(|error| panic!("{target_layout:?} {suffix}: {error:?}"));
            assert_eq!(proof.output, 0);
            assert_eq!(&proof.field, field);
            let cfg = build.reference_cfg(0, producer, None).unwrap();
            assert_eq!(cfg.allocations, BTreeSet::from([proof.owner]));
            vertices += cfg.points.len();
            arcs += cfg
                .points
                .iter()
                .map(|point| point.successors.len())
                .sum::<usize>();
            let expected = if matches!(suffix, "bool" | "char") {
                Vec::new()
            } else {
                let digits = find(&format!("_format_digits_{suffix}"));
                [
                    (producer, digits),
                    (digits, compact),
                    (digits, digits),
                    (compact, compact),
                ]
                .map(|(from, to)| {
                    (
                        (0, from),
                        call_site(from, to),
                        Dependency::Backing((0, to, 0)),
                    )
                })
                .to_vec()
            };
            assert_eq!(
                proof
                    .edges
                    .iter()
                    .map(|edge| (edge.from, edge.site, edge.to))
                    .collect::<Vec<_>>(),
                expected,
                "{target_layout:?} {suffix}"
            );
        }
        let work = batch.work.get();
        assert_eq!(work.index_builds, 1);
        assert_eq!(
            (work.modules, work.items, work.structs),
            (initial.modules, initial.items, initial.structs)
        );
        assert_eq!(work.definition_queries, 32);
        assert_eq!(work.struct_queries, 10);
        assert_eq!(work.helper_checks, 16);
        assert_eq!(work.dominance_walks, 20);
        assert_eq!(work.dominance_queries, 28);
        assert!(work.points <= 2 * vertices);
        assert!(work.arcs <= 2 * arcs);

        let char_producer = find("_format_char");
        let bitcast = build
            .reference_cfg(0, char_producer, None)
            .unwrap()
            .points
            .iter()
            .find(|point| {
                matches!(
                    &point.kind,
                    CfgPointKind::Call {
                        output: 0,
                        target: ResolvedCallTarget::Core(CoreOperationId::Bitcast),
                        ..
                    }
                )
            })
            .unwrap()
            .id;
        for (evidence, reason) in [
            (
                Withheld::ConversionOperand((0, char_producer), bitcast),
                "conversion operand type",
            ),
            (
                Withheld::ConversionResult((0, char_producer), bitcast),
                "conversion result type",
            ),
        ] {
            let result = prove_withheld(&build, char_producer, evidence);
            assert!(
                matches!(result, Err(Unproved::Missing(body, message)) if body == (0, char_producer) && message == reason),
                "{target_layout:?} {result:?}"
            );
        }

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
                .map(|edge| (edge.from.1, edge.to))
                .collect::<BTreeSet<_>>(),
            BTreeSet::from([
                (producer, Dependency::Backing((0, digits, 0))),
                (digits, Dependency::Backing((0, digits, 0))),
                (digits, Dependency::Backing((0, compact, 0))),
                (compact, Dependency::Backing((0, compact, 0)))
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
        let before = batch.work.get();
        let mut vertices = 0;
        let mut arcs = 0;
        for wide in ["i128", "u128"] {
            let producer = find(&format!("_format_{wide}"));
            let digits = find(&format!("_format_digits_{wide}"));
            let scale = find(&format!("_format_scale_{wide}"));
            let subtract = find(&format!("_format_subtract_{wide}"));
            let proof = batch
                .prove(0, producer)
                .unwrap_or_else(|error| panic!("{target_layout:?} {wide}: {error:?}"));
            assert_eq!(proof.output, 0);
            assert_eq!(&proof.field, field);
            let cfg = build.reference_cfg(0, producer, None).unwrap();
            assert_eq!(cfg.allocations, BTreeSet::from([proof.owner]));
            vertices += cfg.points.len();
            arcs += cfg
                .points
                .iter()
                .map(|point| point.successors.len())
                .sum::<usize>();
            let expected = [
                (producer, digits, Dependency::Backing((0, digits, 0))),
                (digits, scale, Dependency::ScalarOnly((0, scale))),
                (digits, subtract, Dependency::ScalarOnly((0, subtract))),
                (digits, digits, Dependency::Backing((0, digits, 0))),
                (scale, scale, Dependency::ScalarOnly((0, scale))),
                (subtract, subtract, Dependency::ScalarOnly((0, subtract))),
            ]
            .map(|(from, to, dependency)| ((0, from), call_site(from, to), dependency));
            assert_eq!(
                proof
                    .edges
                    .iter()
                    .map(|edge| (edge.from, edge.site, edge.to))
                    .collect::<Vec<_>>(),
                expected
            );
        }
        let after = batch.work.get();
        assert_eq!(after.definition_queries - before.definition_queries, 12);
        assert_eq!(after.struct_queries - before.struct_queries, 2);
        assert_eq!(after.helper_checks - before.helper_checks, 6);
        assert_eq!(after.dominance_walks - before.dominance_walks, 4);
        assert_eq!(after.dominance_queries - before.dominance_queries, 6);
        assert!(after.points - before.points <= 2 * vertices);
        assert!(after.arcs - before.arcs <= 2 * arcs);
        for wide in ["i128", "u128"] {
            let producer = find(&format!("_format_{wide}"));
            for helper in ["digits", "subtract"] {
                let body = (0, find(&format!("_format_{helper}_{wide}")));
                let cfg = build.reference_cfg(body.0, body.1, None).unwrap();
                let projections: Vec<_> = cfg
                    .points
                    .iter()
                    .filter_map(|p| {
                        matches!(p.kind, CfgPointKind::Call { output: 1, .. }).then_some(p.id)
                    })
                    .collect();
                assert_eq!(projections.len(), 1);
                let rejected = prove_withheld(
                    &build,
                    producer,
                    Withheld::TupleProjection(body, projections[0]),
                );
                assert!(
                    matches!(rejected, Err(Unproved::Missing(at, "tuple projection")) if at == body),
                    "{target_layout:?} {wide} {helper}: {rejected:?}"
                );
                let rejected = prove_withheld(
                    &build,
                    producer,
                    Withheld::ReorderedTupleProjection(body, projections[0]),
                );
                assert!(
                    matches!(rejected, Err(Unproved::Unsupported(at, "tuple projection identity")) if at == body),
                    "{target_layout:?} {wide} {helper}: {rejected:?}"
                );
            }
            let subtract = (0, find(&format!("_format_subtract_{wide}")));
            let cfg = build.reference_cfg(subtract.0, subtract.1, None).unwrap();
            let CfgPointKind::ReturnOutput {
                value_point: second_arm,
                ..
            } = cfg
                .points
                .iter()
                .find(|p| matches!(p.kind, CfgPointKind::ReturnOutput { output: 1, .. }))
                .unwrap()
                .kind
            else {
                unreachable!();
            };
            let rejected =
                prove_withheld(&build, producer, Withheld::TupleArm(subtract, second_arm));
            assert!(
                matches!(rejected, Err(Unproved::Missing(at, "tuple arm output")) if at == subtract),
                "{target_layout:?} {wide}: {rejected:?}"
            );
        }
        for wide in ["i128", "u128"] {
            let producer = find(&format!("_format_{wide}"));
            for (body, binding_name) in [
                (find(&format!("_format_digits_{wide}")), "zero_digit"),
                (find(&format!("_format_scale_{wide}")), "twice"),
                (producer, "minimum"),
            ] {
                if wide == "u128" && body == producer {
                    continue;
                }
                let body = (0, body);
                let view = View::new(&build, body).unwrap();
                let initializer = if body.1 == producer {
                    view.function.body.items.iter().find_map(|item| match item {
                        ScalarBlockItem::LocalBinding(binding) if binding.name == binding_name => {
                            Some(&binding.value)
                        }
                        _ => None,
                    })
                } else {
                    view.function.body.items.iter().find_map(|item| match item {
                        ScalarBlockItem::Expression(ScalarExpression::If {
                            then_branch, ..
                        }) => then_branch.items.iter().find_map(|item| match item {
                            ScalarBlockItem::LocalBinding(binding)
                                if binding.name == binding_name =>
                            {
                                Some(&binding.value)
                            }
                            _ => None,
                        }),
                        _ => None,
                    })
                }
                .unwrap();
                let point = view.eval(initializer).unwrap();
                assert!(matches!(
                    initializer,
                    ScalarExpression::Integer { .. }
                        | ScalarExpression::Binary { .. }
                        | ScalarExpression::Unary { .. }
                ));
                let rejected =
                    prove_withheld(&build, producer, Withheld::PrimitiveFact(body, point));
                assert!(
                    matches!(rejected, Err(Unproved::Missing(at, "wide primitive expression type")) if at == body),
                    "{target_layout:?} {wide} {binding_name}: {rejected:?}"
                );
                if binding_name == "zero_digit" {
                    let rejected = prove_withheld(
                        &build,
                        producer,
                        Withheld::ContradictoryPrimitiveFact(body, point),
                    );
                    assert!(
                        matches!(rejected, Err(Unproved::Unsupported(at, "wide primitive binding type")) if at == body),
                        "{target_layout:?} {wide} {binding_name}: {rejected:?}"
                    );
                }
            }
        }
        let producer = find("_format_i128");
        let body = (0, producer);
        let view = View::new(&build, body).unwrap();
        let mut indexed_writes = view.function.body.items.iter().filter_map(|item| {
            let ScalarBlockItem::Expression(ScalarExpression::UnitIf { then_branch, .. }) = item
            else {
                return None;
            };
            then_branch.items.iter().find_map(|item| {
                let ScalarBlockItem::Assignment(assignment) = item else {
                    return None;
                };
                let [rhs @ ScalarExpression::Integer { value, .. }] = assignment.values.as_slice()
                else {
                    return None;
                };
                let [target] = assignment.targets.as_slice() else {
                    return None;
                };
                (matches!(target.place, ScalarPlace::Index { .. }) && *value == 56.into())
                    .then_some((&target.place, rhs))
            })
        });
        let (place, rhs) = indexed_writes.next().unwrap();
        assert!(
            indexed_writes.next().is_none(),
            "unique signed minimum indexed write"
        );
        let frozen = view.nodes.frozen_install(view.facts, place).unwrap();
        assert_eq!(view.eval(rhs), Some(frozen.rhs));
        assert_eq!(
            view.nodes.checked_expression_type(rhs),
            Some(&ScalarType::U8)
        );
        assert_eq!(view.nodes.checked_place_type(place), Some(&ScalarType::U8));
        let rejected = prove_withheld(
            &build,
            producer,
            Withheld::ContradictoryPrimitiveFact(body, frozen.rhs),
        );
        if !matches!(rejected, Err(Unproved::Unsupported(at, "wide primitive assignment type")) if at == body)
        {
            strict_wide_failures.push(format!(
                "{target_layout:?} signed minimum indexed RHS: {rejected:?}"
            ));
        }

        let mut lexical_writes = view.function.body.items.iter().filter_map(|item| {
            let ScalarBlockItem::Expression(ScalarExpression::UnitIf { then_branch, .. }) = item
            else {
                return None;
            };
            then_branch.items.iter().find_map(|item| {
                let ScalarBlockItem::Assignment(assignment) = item else {
                    return None;
                };
                let [rhs @ ScalarExpression::Name { name, .. }] = assignment.values.as_slice()
                else {
                    return None;
                };
                let [target] = assignment.targets.as_slice() else {
                    return None;
                };
                (name == "one"
                    && matches!(&target.place, ScalarPlace::Name { name, .. } if name == "emit"))
                .then_some((&target.place, rhs))
            })
        });
        let (place, rhs) = lexical_writes.next().unwrap();
        assert!(
            lexical_writes.next().is_none(),
            "unique wide lexical emit write"
        );
        let frozen = view.nodes.frozen_install(view.facts, place).unwrap();
        assert_eq!(view.eval(rhs), Some(frozen.rhs));
        assert_eq!(
            view.nodes.checked_expression_type(rhs),
            Some(&ScalarType::U64)
        );
        assert_eq!(view.nodes.checked_place_type(place), Some(&ScalarType::U64));
        let rejected = prove_withheld(
            &build,
            producer,
            Withheld::ContradictoryPrimitiveFact(body, frozen.rhs),
        );
        if !matches!(rejected, Err(Unproved::Unsupported(at, "wide primitive assignment type")) if at == body)
        {
            strict_wide_failures.push(format!(
                "{target_layout:?} wide lexical emit RHS: {rejected:?}"
            ));
        }

        let mut minima = view
            .function
            .body
            .items
            .iter()
            .filter_map(|item| match item {
                ScalarBlockItem::LocalBinding(binding) if binding.name == "minimum" => {
                    Some(&binding.value)
                }
                _ => None,
            });
        let minimum = minima.next().unwrap();
        assert!(minima.next().is_none(), "unique signed minimum declaration");
        let ScalarExpression::Unary {
            operator: UnaryOperator::Negate,
            operand,
            ..
        } = minimum
        else {
            unreachable!("canonical signed minimum");
        };
        assert_eq!(
            view.nodes.checked_expression_type(minimum),
            Some(&ScalarType::I128)
        );
        assert_eq!(view.nodes.checked_expression_type(operand), None);
        let rejected = prove_withheld(
            &build,
            producer,
            Withheld::ContradictorySignedMagnitude(body, view.eval(operand).unwrap()),
        );
        if !matches!(rejected, Err(Unproved::Unsupported(at, "wide folded minimum identity")) if at == body)
        {
            strict_wide_failures.push(format!(
                "{target_layout:?} typed signed-minimum magnitude: {rejected:?}"
            ));
        }
        for unsupported in ["_format_f32", "_format_f64"] {
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
    assert!(strict_wide_failures.is_empty(), "{strict_wide_failures:#?}");
}

#[test]
fn formatter_proof_wide_checks_transitive_scalar_only_effects_and_result_types() {
    let cases = [
        ("eligible", "i128", "value"),
        (
            "effect",
            "i128",
            "u8[3] local; unsafe { *?u8 p = &?local[0]; core.free(p); }; value",
        ),
        ("wrong width", "u128", "value; 7"),
        ("address", "*?u8", "value; unsafe { &?outside[0] }"),
    ];
    for target_layout in [ScalarTargetLayout::NATIVE64, ScalarTargetLayout::WASM32] {
        for (case, output, body) in cases {
            let source = SourceIdentity::new("p".into(), "p".into(), "wide.w".into(), "r1".into());
            let text = format!(
                "%%start
struct Packet {{ *?u8 address; u64 length; }}
u8[3] outside = [1, 2, 3];
{output}(i128) leaf = fn(value) {{ {body} }};
i128(i128) bridge = fn(value) {{ leaf(value); value }};
Packet(i128) make = fn(value) {{
    bridge(value);
    u8[3] scratch;
    scratch[0] = 7;
    *?u8 data = null;
    unsafe {{ data = &?scratch[0]; }};
    Packet result = {{ .length = 1; .address = data; }};
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
            let find = |name| {
                prepared.project().modules[0]
                    .items
                    .iter()
                    .position(|item| matches!(item, ScalarItem::Function(f) if f.name == name))
                    .unwrap()
            };
            let make = find("make");
            let bridge = find("bridge");
            let leaf = find("leaf");
            let result = prove(&build, 0, make);
            match case {
                "eligible" => assert_eq!(
                    result
                        .unwrap()
                        .edges
                        .iter()
                        .map(|edge| (edge.from, edge.to))
                        .collect::<Vec<_>>(),
                    [
                        ((0, make), Dependency::ScalarOnly((0, bridge))),
                        ((0, bridge), Dependency::ScalarOnly((0, leaf)))
                    ]
                ),
                "effect" => assert!(
                    matches!(result, Err(Unproved::Unsupported(body, "release or indirect storage effect")) if body == (0, leaf)),
                    "{target_layout:?}: {result:?}"
                ),
                _ => assert!(
                    matches!(result, Err(Unproved::Unsupported(body, "wide helper output type")) if body == (0, bridge)),
                    "{target_layout:?} {case}: {result:?}"
                ),
            }
        }
    }
}

#[test]
fn formatter_proof_simple_recursive_effect_and_scalar_conversion_controls() {
    for target_layout in [ScalarTargetLayout::NATIVE64, ScalarTargetLayout::WASM32] {
        for (effect, conversion, expected) in [
            ("", "", "eligible"),
            (
                "unsafe { *?u8 p = &?buffer[0]; core.free(p); };",
                "",
                "helper effect",
            ),
            (
                "",
                "i32 signed = 7; u32 bits = core.bitcast<u32>(signed); bits;",
                "unsupported cast",
            ),
        ] {
            let source = SourceIdentity::new("p".into(), "p".into(), "case.w".into(), "r1".into());
            let text = format!(
                "%%start
struct Packet {{ *?u8 address; u64 length; }}
unit(u8[], bool) touch = fn(buffer, again) {{
    if (again) {{ touch(buffer, false); }} else {{
        buffer[0] = 7;
        {effect}
    }};
}};
Packet(bool) make = fn(again) {{
    u8[3] scratch;
    {conversion}
    touch(scratch, again);
    *?u8 data = null;
    unsafe {{ data = &?scratch[0]; }};
    Packet result = {{ .length = 1; .address = data; }};
    result
}};
%%end"
            );
            let parsed = parse_source(source.clone(), text, &[]);
            assert!(
                parsed.diagnostics.is_empty(),
                "{expected}: {:?}",
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
            .unwrap_or_else(|error| panic!("{expected}: {:?}", error.diagnostics));
            let build = analyze_scalar_build(&prepared);
            assert!(
                build.diagnostics().is_empty(),
                "{expected}: {:?}",
                build.diagnostics()
            );
            let find = |name| {
                prepared.project().modules[0]
                    .items
                    .iter()
                    .position(|item| matches!(item, ScalarItem::Function(f) if f.name == name))
                    .unwrap()
            };
            let make = find("make");
            let touch = find("touch");
            let result = prove(&build, 0, make);
            match expected {
                "eligible" => {
                    let proof = result.unwrap();
                    assert_eq!(
                        proof
                            .edges
                            .iter()
                            .map(|edge| (edge.from.1, edge.to))
                            .collect::<Vec<_>>(),
                        [
                            (make, Dependency::Backing((0, touch, 0))),
                            (touch, Dependency::Backing((0, touch, 0)))
                        ]
                    );
                }
                "helper effect" => assert!(
                    matches!(result, Err(Unproved::Unsupported(body, "release or indirect storage effect")) if body == (0, touch)),
                    "{target_layout:?} {result:?}"
                ),
                "unsupported cast" => assert!(
                    matches!(&result, Err(Unproved::Call(body, site, ResolvedCallTarget::Core(CoreOperationId::Bitcast), types))
                        if *body == (0, make) && types.as_slice() == [Some(ScalarType::I32)]
                            && matches!(&build.reference_cfg(0, make, None).unwrap().points[site.0].kind,
                                CfgPointKind::Call { target: ResolvedCallTarget::Core(CoreOperationId::Bitcast), .. })),
                    "{target_layout:?} {result:?}"
                ),
                _ => unreachable!(),
            }
        }
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

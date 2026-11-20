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

pub(super) fn is_withheld(evidence: Withheld) -> bool {
    WITHHELD.get() == Some(evidence)
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

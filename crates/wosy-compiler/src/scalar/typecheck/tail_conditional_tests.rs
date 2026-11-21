use super::*;
use crate::parse_source;

fn assert_conditional_types(
    text: &str,
    expected: &[ScalarType],
    expected_actual: &[ScalarType],
    expected_diagnostics: &[(&str, &str)],
) {
    let source = SourceIdentity::new(
        "project".into(),
        "package".into(),
        "pair.w".into(),
        "r1".into(),
    );
    let parsed = parse_source(source.clone(), text.into(), &[]);
    assert!(parsed.diagnostics.is_empty(), "{:?}", parsed.diagnostics);
    let block_node = parsed
        .result
        .canonical_cst()
        .root
        .descendants()
        .find(|node| node.kind() == SyntaxKind::Block)
        .expect("function body CST");
    let body = derive_block(&block_node);
    let signature = expected
        .iter()
        .map(|ty| ScalarOutput {
            ty: ty.clone(),
            span: ByteSpan::new(0, 0),
        })
        .collect::<Vec<_>>();
    let scope = BTreeMap::new();
    let program = ScalarProgramTyping {
        source: &source,
        structs: &[],
        enums: &[],
        target_layout: ScalarTargetLayout::NATIVE64,
    };
    let module = ScalarModuleTyping {
        source: &source,
        members: &scope,
        namespace_bindings: &[],
        structs: &[],
        enums: &[],
        target_layout: ScalarTargetLayout::NATIVE64,
    };
    for typing in [
        TailConditionalTyping::Program(&program),
        TailConditionalTyping::Module {
            module: &module,
            modules: std::slice::from_ref(&module),
        },
    ] {
        let tail = body
            .tail_conditional()
            .expect("single raw tail conditional");
        let then_branch = tail.then_branch;
        let owners = ScalarAssignmentOwners::new(&[]);
        let mut inference = ScalarInferenceBuilder::for_validation(&owners);
        let mut diagnostics = Vec::new();
        let actual = tail_conditional_types(
            tail,
            &signature,
            &scope,
            &BTreeSet::new(),
            &BTreeMap::new(),
            typing,
            &mut diagnostics,
            false,
            &ScalarOverloadEnvironment::single(&source, &[]).typing(0),
            &mut inference,
        );

        assert_eq!(actual, expected_actual);
        assert_eq!(
            diagnostics.len(),
            expected_diagnostics.len(),
            "{diagnostics:?}"
        );
        for (reported, &(code, fragment)) in diagnostics.iter().zip(expected_diagnostics) {
            assert_eq!(reported.code, code);
            let start = text.find(fragment).expect("diagnostic source fragment");
            assert_eq!(
                reported.labels[0].span,
                SourceSpan::new(
                    source.clone(),
                    ByteSpan::new(start as u32, (start + fragment.len()) as u32),
                ),
            );
        }
        if expected_diagnostics.is_empty() {
            let nested = then_branch.tail_conditional().expect("nested raw tail");
            let (_, mut occurrences) = inference.finish();
            assert_eq!(
                occurrences.take_expression_type(nested.condition),
                Some(ScalarType::Bool),
            );
            for arm in [nested.then_branch, nested.else_branch] {
                for ((expression, _), expected) in arm.final_expressions().zip(&signature) {
                    assert_eq!(
                        occurrences.take_expression_type(expression),
                        Some(expected.ty.clone()),
                    );
                }
            }
        }
    }
}

#[test]
fn checks_raw_nested_conditional_outputs_in_both_contexts() {
    use ScalarType::{Bool, Error, U16};

    let then_arm = "{ bool flag = true; 17, flag }";
    let else_arm = "{ bool flag = false; 42, flag }";
    for (condition, then_arm, else_arm, actual, diagnostic) in [
        ("false", then_arm, else_arm, [U16, Bool], None),
        ("1", then_arm, else_arm, [U16, Bool], Some(("B0005", None))),
        (
            "false",
            then_arm,
            "{ 42, 2 }",
            [U16, Bool],
            Some(("B0006", None)),
        ),
        (
            "false",
            "{ 17 }",
            else_arm,
            [U16, Error],
            Some(("B0004", Some("17"))),
        ),
        (
            "false",
            "{ 18; }",
            else_arm,
            [Error, Error],
            Some(("B0004", Some("{ 18; }"))),
        ),
    ] {
        let inner = format!("if ({condition}) {then_arm} else {else_arm}");
        let text = format!(
            "%%start\n(u16, bool)() pair = fn {{ if (true) {{ if (true) {{ 1; }} else {{ 2; }} {inner} }} else {{ bool flag = true; 9, flag }} }};\n%%end"
        );
        let diagnostic = diagnostic.map(|(code, fragment)| (code, fragment.unwrap_or(&inner)));
        assert_conditional_types(&text, &[U16, Bool], &actual, diagnostic.as_slice());
    }
}

#[test]
fn retains_then_types_for_mismatched_slots_and_propagates_errors_in_both_contexts() {
    use ScalarType::{Bool, Error, U16};

    let conditional = "if (true) { true, 3, missing, true, 5 } else { 2, false, 7, absent }";
    let text =
        format!("%%start\n(u16, u16, u16, u16, u16)() pair = fn {{ {conditional} }};\n%%end");
    assert_conditional_types(
        &text,
        &[U16, U16, U16, U16, U16],
        &[Bool, U16, Error, Error, Error],
        &[
            ("B0001", "missing"),
            ("B0004", "2"),
            ("B0001", "absent"),
            ("B0006", conditional),
            ("B0006", conditional),
        ],
    );
}

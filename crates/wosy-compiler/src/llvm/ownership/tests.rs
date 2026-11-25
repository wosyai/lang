use super::*;
use crate::scalar::ScalarOutputValue;
use crate::scalar::{
    ReferenceBindingId, ReferenceLoanId, ReferenceOriginId, ReferencePlaceId, ReferenceScopeId,
    ReturnedAllocationId, ScalarOutput, ScalarOutputSequence, ScalarPlace,
    ScalarReferenceMutability,
};
use wosy_syntax::{ByteSpan, SourceIdentity, SourceSpan};

fn projected_checked_return(independent_scalar_if: bool) -> ScalarFunction {
    let span = ByteSpan { start: 0, end: 80 };
    let value_span = ByteSpan { start: 25, end: 26 };
    let source = SourceIdentity::new(
        "project".into(),
        "package".into(),
        "src/main.w".into(),
        "r1".into(),
    );
    let mutability = ScalarReferenceMutability::Shared;
    let checked = ScalarType::CheckedReference {
        mutability,
        inner: Box::new(ScalarType::U8),
    };
    let block = |start, end, values: Vec<ScalarExpression>| ScalarBlock {
        items: values
            .iter()
            .cloned()
            .map(ScalarBlockItem::Expression)
            .collect(),
        terminated_items: vec![false; values.len()],
        final_output_values: values
            .iter()
            .cloned()
            .enumerate()
            .map(|(position, value)| ScalarOutputValue {
                position,
                ty: checked.clone(),
                span: value_span,
                value,
            })
            .collect(),
        expressions: values,
        span: ByteSpan { start, end },
        unsafe_context: false,
    };
    let name = |name: &str| ScalarExpression::Name {
        name: name.into(),
        span: value_span,
    };
    let conditional = |then_branch, else_branch| ScalarExpression::If {
        condition: Box::new(ScalarExpression::Boolean { value: true, span }),
        then_branch,
        else_branch,
        span,
    };
    let second = if independent_scalar_if {
        conditional(
            block(11, 14, vec![name("local")]),
            block(15, 18, vec![name("borrowed")]),
        )
    } else {
        name("borrowed")
    };
    let body = block(
        1,
        79,
        vec![conditional(
            block(8, 20, vec![name("local"), second]),
            block(
                21,
                75,
                vec![conditional(
                    block(30, 42, vec![name("borrowed"), name("local")]),
                    block(50, 72, vec![name("joined_first"), name("borrowed")]),
                )],
            ),
        )],
    );
    let binding = ReferenceBindingId {
        source: source.clone(),
        function_span: span,
        declaration_span: value_span,
        block_span: body.span,
        kind: ReferenceBindingKind::Declared,
    };
    let loan = ReferenceLoanId {
        mode: mutability,
        origin_place: ReferencePlaceId {
            source: source.clone(),
            function_span: span,
            declaration_span: value_span,
            block_span: body.span,
            binding: binding.clone(),
            projections: Vec::new(),
            place: ScalarPlace::Name {
                name: "value".into(),
                span: value_span,
            },
        },
        creation_span: value_span,
        parent: None,
    };
    let facts = vec![
        ReferenceOrigin::Fresh {
            allocation: ReturnedAllocationId {
                source: source.clone(),
                function_span: span,
                declaration_span: value_span,
                creation_span: value_span,
            },
            loan: loan.clone(),
        },
        ReferenceOrigin::BorrowedFrom { parameter: 0, loan },
    ];
    let mut points = Vec::new();
    let mut point = |kind, origins: &[usize]| {
        let id = CfgPointId(points.len());
        points.push(ReferenceFlowPoint {
            id,
            kind,
            source_span: SourceSpan {
                source: source.clone(),
                range: value_span,
            },
            scope: ReferenceScopeId(0),
            possible_origins: Some(origins.iter().copied().map(ReferenceOriginId).collect()),
            successors: Vec::new(),
        });
        id
    };
    let read = || CfgPointKind::Read {
        binding: binding.clone(),
    };
    let join = |then_point, else_point| CfgPointKind::ConditionalOutput {
        then_point,
        else_point,
    };
    let returned = |output, value_point| CfgPointKind::ReturnOutput {
        output,
        value_point,
        checked: true,
    };
    let a0 = point(read(), &[0]);
    let a1 = if independent_scalar_if {
        let then_point = point(read(), &[0]);
        let else_point = point(read(), &[1]);
        point(join(then_point, else_point), &[0, 1])
    } else {
        point(read(), &[1])
    };
    let [b0, b1] = [point(read(), &[1]), point(read(), &[0])];
    let [c0, c1] = [point(read(), &[0, 1]), point(read(), &[1])];
    let [inner0, inner1] = [point(join(b0, c0), &[0, 1]), point(join(b1, c1), &[0, 1])];
    let [root0, root1] = [
        point(join(a0, inner0), &[0, 1]),
        point(join(a1, inner1), &[0, 1]),
    ];
    point(returned(0, root0), &[0, 1]);
    point(returned(1, root1), &[0, 1]);

    ScalarFunction {
        name: "pair".into(),
        name_span: span,
        signature: ScalarType::Callable {
            outputs: ScalarOutputSequence {
                outputs: vec![
                    ScalarOutput {
                        ty: checked.clone(),
                        span
                    };
                    2
                ],
                span,
            },
            parameters: vec![checked],
        },
        parameters: vec!["borrowed".into()],
        parameter_spans: vec![span],
        body,
        span,
        runtime_enum_match: None,
        generic_parameters: Vec::new(),
        overload_arms: Vec::new(),
        reference_cfg: ScalarReferenceCfg {
            primary_call_points: Default::default(),
            facts,
            points,
            initial_points: Vec::new(),
            allocations: BTreeSet::new(),
            specialized_policies: Vec::new(),
        },
    }
}

#[test]
fn projected_checked_returns_classify_exact_arm_points_by_slot() {
    let function = projected_checked_return(false);
    let (fresh, branches) = fresh_return_outputs(&function, &function.reference_cfg).unwrap();
    assert!(fresh.is_empty());
    assert_eq!(
        branches,
        BTreeMap::from([
            (
                (8, 20),
                ReturnBranch { local: BTreeSet::from([0]), covered: BTreeSet::from([0, 1]), ..ReturnBranch::default() }
            ),
            (
                (30, 42),
                ReturnBranch { local: BTreeSet::from([1]), covered: BTreeSet::from([0, 1]), ..ReturnBranch::default() }
            ),
            (
                (50, 72),
                ReturnBranch {
                    covered: BTreeSet::from([0, 1]),
                    joined: BTreeMap::from([(0, "joined_first".into())]),
                    ..ReturnBranch::default()
                }
            ),
        ])
    );
}

#[test]
fn independent_scalar_if_retains_second_projected_owner_slot() {
    let function = projected_checked_return(true);
    let (fresh, branches) = fresh_return_outputs(&function, &function.reference_cfg).unwrap();
    assert!(fresh.is_empty());
    assert_eq!(branches[&(8, 20)].local, BTreeSet::from([0]));
    assert_eq!(branches[&(8, 20)].covered, BTreeSet::from([0]));
    assert_eq!(branches[&(11, 14)].local, BTreeSet::from([1]));
    assert_eq!(branches[&(11, 14)].covered, BTreeSet::from([1]));
    assert!(branches[&(15, 18)].local.is_empty());
    assert_eq!(branches[&(15, 18)].covered, BTreeSet::from([1]));
    assert_eq!(branches[&(30, 42)].local, BTreeSet::from([1]));
}

#[test]
fn projected_checked_returns_reject_malformed_arm_link() {
    let mut function = projected_checked_return(false);
    let cfg = &mut function.reference_cfg;
    let CfgPointKind::ReturnOutput { value_point, .. } = cfg.points[10].kind else {
        panic!("first return output");
    };
    let CfgPointKind::ConditionalOutput {
        ref mut else_point, ..
    } = cfg.points[value_point.0].kind
    else {
        panic!("root conditional point");
    };
    *else_point = CfgPointId(100);
    assert_eq!(
        fresh_return_outputs(&function, &function.reference_cfg).unwrap_err(),
        "invalid return output 0 value link CfgPointId(8) -> CfgPointId(100)"
    );
}

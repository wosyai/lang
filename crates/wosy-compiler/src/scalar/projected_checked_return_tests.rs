use super::*;

const HOLDER_FORWARD: &str = "%%start\nstruct Holder { *!u32 first; *!u32 second; }\nunit() run = fn {\nu32 item = 2;\nu32 other = 3;\nHolder box = { .first = &!item; .second = &!other; };\nHolder(Holder) forward = fn(value) { value };\nHolder moved = forward(box);\n*!u32 first = moved.first;\n*!u32 second = moved.second;\n*first = 4;\n*second = 5;\n};\nrun();\n%%end";

const MODULE_HOLDER_SETUP: &str = "%%start\nstruct Holder { *!u32 first; *!u32 second; }\nu32 item = 2;\nu32 other = 3;\nHolder box = { .first = &!item; .second = &!other; };\nHolder(Holder) forward = fn(value) { value };\nHolder moved = forward(box);\nmoved;\n%%end";

fn holder_forward(layout: ScalarTargetLayout, project: bool) {
    let parsed = parse_source(source(), HOLDER_FORWARD.to_owned(), &[]);
    assert!(parsed.diagnostics.is_empty(), "{:?}", parsed.diagnostics);
    let single = derive_scalar_program_with_layout(&parsed.result, layout);
    let diagnostics = if project {
        validate_scalar_project(ScalarProject::new(
            vec![ScalarModule::from_program(single.program, Vec::new())],
            vec![source()],
        ))
        .diagnostics
    } else {
        single.diagnostics
    };
    assert!(
        diagnostics.is_empty(),
        "{layout:?}, project={project}: {diagnostics:?}"
    );
}

fn module_holder_setup(layout: ScalarTargetLayout, project: bool) {
    let parsed = parse_source(source(), MODULE_HOLDER_SETUP.to_owned(), &[]);
    assert!(parsed.diagnostics.is_empty(), "{:?}", parsed.diagnostics);
    let single = derive_scalar_program_with_layout(&parsed.result, layout);
    let cfg = if project {
        let validated = validate_scalar_project(ScalarProject::new(
            vec![ScalarModule::from_program(single.program, Vec::new())],
            vec![source()],
        ));
        validated.project.modules[0].top_level_reference_cfg.clone()
    } else {
        single.program.top_level_reference_cfg
    };
    for (text, field) in [("&!item", 0), ("&!other", 1)] {
        let start = MODULE_HOLDER_SETUP.find(text).unwrap() as u32;
        let span = SourceSpan::new(source(), ByteSpan::new(start, start + text.len() as u32));
        let borrow = cfg.points.iter().find(|point| {
            point.source_span == span && matches!(point.kind, CfgPointKind::Borrow { .. })
        }).unwrap_or_else(|| panic!("{layout:?}, project={project}: missing module checked-field borrow {span:?}; points={:?}", cfg.points));
        assert!(cfg.points.iter().any(|point| {
            matches!(&point.kind, CfgPointKind::Assign { target, rhs_point, .. }
                if *rhs_point == borrow.id
                    && matches!(target.projections.as_slice(), [ReferencePlaceProjection::Field(ScalarFieldReference::Resolved(resolved))] if resolved.index == field))
        }), "{layout:?}, project={project}: missing evaluated field installation for {span:?}");
    }
}

#[test]
fn d3_projected_checked_return_holder_native_program() {
    holder_forward(ScalarTargetLayout::NATIVE64, false);
}

#[test]
fn d3_projected_checked_return_holder_native_project() {
    holder_forward(ScalarTargetLayout::NATIVE64, true);
}

#[test]
fn d3_projected_checked_return_holder_wasm_program() {
    holder_forward(ScalarTargetLayout::WASM32, false);
}

#[test]
fn d3_projected_checked_return_holder_wasm_project() {
    holder_forward(ScalarTargetLayout::WASM32, true);
}

#[test]
fn d3_projected_checked_return_module_setup_native_program() {
    module_holder_setup(ScalarTargetLayout::NATIVE64, false);
}

#[test]
fn d3_projected_checked_return_module_setup_native_project() {
    module_holder_setup(ScalarTargetLayout::NATIVE64, true);
}

#[test]
fn d3_projected_checked_return_module_setup_wasm_program() {
    module_holder_setup(ScalarTargetLayout::WASM32, false);
}

#[test]
fn d3_projected_checked_return_module_setup_wasm_project() {
    module_holder_setup(ScalarTargetLayout::WASM32, true);
}

const CORRELATED_EVALUATION: &str = "%%start\nbool() decide = fn { true };\n(u32, u32)() choose = fn { if (decide()) { 1, 2 } else { 2, 1 } };\n%%end";

const EFFECTFUL_CORRELATED_EVALUATION: &str = "%%start\nbool state = false;\nbool() decide = fn { state = !state; state };\n(u32, u32)() choose = fn { if (decide()) { 1, 2 } else { 2, 1 } };\n%%end";

fn correlated_evaluation(text: &str, layout: ScalarTargetLayout, project: bool) {
    let parsed = parse_source(source(), text.to_owned(), &[]);
    assert!(parsed.diagnostics.is_empty(), "{:?}", parsed.diagnostics);
    let single = derive_scalar_program_with_layout(&parsed.result, layout);
    let items = if project {
        let validated = validate_scalar_project(ScalarProject::new(
            vec![ScalarModule::from_program(single.program, Vec::new())],
            vec![source()],
        ));
        assert!(
            validated.diagnostics.is_empty(),
            "{:?}",
            validated.diagnostics
        );
        validated.project.modules[0].items.clone()
    } else {
        assert!(single.diagnostics.is_empty(), "{:?}", single.diagnostics);
        single.program.items
    };
    let function = items
        .iter()
        .find_map(|item| match item {
            ScalarItem::Function(function) if function.name == "choose" => Some(function),
            _ => None,
        })
        .expect("expected choose function");
    let start = text.find("decide())").unwrap() as u32;
    let span = SourceSpan::new(source(), ByteSpan::new(start, start + 8));
    let evaluations = function
        .reference_cfg
        .call_points
        .iter()
        .filter(|call| {
            call.occurrence.source_span == span
                && matches!(
                    function.reference_cfg.points[call.evaluated_point.0].kind,
                    CfgPointKind::Call { output: 0, .. }
                )
        })
        .collect::<Vec<_>>();
    assert_eq!(evaluations.len(), 1,
        "{layout:?}, project={project}: one complete return tuple must share its original condition evaluation; occurrences={evaluations:?}");
}

#[test]
fn d3_projected_checked_return_correlated_evaluation_native_program() {
    correlated_evaluation(CORRELATED_EVALUATION, ScalarTargetLayout::NATIVE64, false);
}

#[test]
fn d3_projected_checked_return_correlated_evaluation_native_project() {
    correlated_evaluation(CORRELATED_EVALUATION, ScalarTargetLayout::NATIVE64, true);
}

#[test]
fn d3_projected_checked_return_correlated_evaluation_wasm_program() {
    correlated_evaluation(CORRELATED_EVALUATION, ScalarTargetLayout::WASM32, false);
}

#[test]
fn d3_projected_checked_return_correlated_evaluation_wasm_project() {
    correlated_evaluation(CORRELATED_EVALUATION, ScalarTargetLayout::WASM32, true);
}

#[test]
fn d3_projected_checked_return_effectful_correlated_evaluation_native_program() {
    correlated_evaluation(
        EFFECTFUL_CORRELATED_EVALUATION,
        ScalarTargetLayout::NATIVE64,
        false,
    );
}

#[test]
fn d3_projected_checked_return_effectful_correlated_evaluation_native_project() {
    correlated_evaluation(
        EFFECTFUL_CORRELATED_EVALUATION,
        ScalarTargetLayout::NATIVE64,
        true,
    );
}

#[test]
fn d3_projected_checked_return_effectful_correlated_evaluation_wasm_program() {
    correlated_evaluation(
        EFFECTFUL_CORRELATED_EVALUATION,
        ScalarTargetLayout::WASM32,
        false,
    );
}

#[test]
fn d3_projected_checked_return_effectful_correlated_evaluation_wasm_project() {
    correlated_evaluation(
        EFFECTFUL_CORRELATED_EVALUATION,
        ScalarTargetLayout::WASM32,
        true,
    );
}

const LOAN_CREATION: &str = "%%start\nunit() run = fn { u32 item = 1; *!u32 parent = &!item; *!u32 child = &!(*parent); *child = 2; *parent = 3; };\nrun();\n%%end";

fn loan_creation(layout: ScalarTargetLayout, project: bool) {
    let parsed = parse_source(source(), LOAN_CREATION.to_owned(), &[]);
    assert!(parsed.diagnostics.is_empty(), "{:?}", parsed.diagnostics);
    let single = derive_scalar_program_with_layout(&parsed.result, layout);
    let items = if project {
        let validated = validate_scalar_project(ScalarProject::new(
            vec![ScalarModule::from_program(single.program, Vec::new())],
            vec![source()],
        ));
        assert!(
            validated.diagnostics.is_empty(),
            "{:?}",
            validated.diagnostics
        );
        validated.project.modules[0].items.clone()
    } else {
        assert!(single.diagnostics.is_empty(), "{:?}", single.diagnostics);
        single.program.items
    };
    let ScalarItem::Function(function) = &items[0] else {
        panic!("expected run function");
    };
    let cfg = &function.reference_cfg;
    let loans = ["&!item", "&!(*parent)"].map(|text| {
        let start = LOAN_CREATION.find(text).unwrap() as u32;
        let range = ByteSpan::new(start, start + text.len() as u32);
        cfg.points
            .iter()
            .find_map(|point| match &point.kind {
                CfgPointKind::Borrow { .. } if point.source_span.range == range => {
                    let loan = evaluated_borrow::accepted_loan(cfg, point.id);
                    assert_eq!(loan.creation_source, point.source_span.source);
                    assert_eq!(loan.creation_span, range);
                    Some(loan)
                }
                _ => None,
            })
            .expect("actual borrow point")
    });
    assert_eq!(loans[1].parent.as_deref(), Some(loans[0]));
    let restored: ScalarReferenceCfg =
        serde_json::from_str(&serde_json::to_string(cfg).unwrap()).unwrap();
    assert_eq!(*cfg, restored);
}

#[test]
fn d3_projected_checked_return_loan_creation_native_program() {
    loan_creation(ScalarTargetLayout::NATIVE64, false);
}

#[test]
fn d3_projected_checked_return_loan_creation_native_project() {
    loan_creation(ScalarTargetLayout::NATIVE64, true);
}

#[test]
fn d3_projected_checked_return_loan_creation_wasm_program() {
    loan_creation(ScalarTargetLayout::WASM32, false);
}

#[test]
fn d3_projected_checked_return_loan_creation_wasm_project() {
    loan_creation(ScalarTargetLayout::WASM32, true);
}

use super::*;

#[path = "evaluated_borrow_tests.rs"]
mod evaluated_borrow_tests;

const EXACT: &str = "%%start\nunit() run = fn { u32 item = 1; *!u32 parent = &!item; *!u32 child = &!(*parent); *child = 2; *parent = 3; };\nrun();\n%%end";

fn validate(text: &str, layout: ScalarTargetLayout, project: bool) {
    let parsed = parse_source(source(), text.to_owned(), &[]);
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

fn storage(layout: ScalarTargetLayout, project: bool) {
    let parsed = parse_source(source(), EXACT.to_owned(), &[]);
    let single = derive_scalar_program_with_layout(&parsed.result, layout);
    let items = if project {
        validate_scalar_project(ScalarProject::new(
            vec![ScalarModule::from_program(single.program, Vec::new())],
            vec![source()],
        ))
        .project
        .modules[0]
            .items
            .clone()
    } else {
        single.program.items
    };
    let ScalarItem::Function(function) = &items[0] else {
        panic!("run function")
    };
    let cfg = &function.reference_cfg;
    let child_write = cfg
        .points
        .iter()
        .find(|point| {
            point.source_span.range == ByteSpan::new(91, 96)
                && matches!(point.kind, CfgPointKind::AssignThrough { .. })
        })
        .expect("child write");
    let CfgPointKind::AssignThrough {
        previous_origins,
        previous_loans,
        candidates,
        ..
    } = &child_write.kind
    else {
        unreachable!()
    };
    assert!(
        previous_origins.is_empty(),
        "scalar destination contains no stored checked value: {previous_origins:?}"
    );
    assert!(previous_loans.is_empty());
    assert_eq!(candidates.len(), 1);
    assert_eq!(candidates[0].loan.creation_span, ByteSpan::new(77, 88));
    let parent_reads = cfg
        .points
        .iter()
        .filter(|point| {
            matches!(&point.kind,
        CfgPointKind::Read { binding } if binding.declaration_span == ByteSpan::new(46, 52))
        })
        .collect::<Vec<_>>();
    assert_eq!(parent_reads.len(), 2);
    assert_eq!(
        parent_reads[0].possible_origins,
        parent_reads[1].possible_origins
    );
    assert_eq!(parent_reads[0].possible_origins.as_ref().unwrap().len(), 1);
    let restored: ScalarReferenceCfg =
        serde_json::from_str(&serde_json::to_string(cfg).unwrap()).unwrap();
    assert_eq!(*cfg, restored);
}

#[test]
fn d3_reborrow_state_authority_native_program() {
    validate(EXACT, ScalarTargetLayout::NATIVE64, false);
}
#[test]
fn d3_reborrow_state_authority_native_project() {
    validate(EXACT, ScalarTargetLayout::NATIVE64, true);
}
#[test]
fn d3_reborrow_state_authority_wasm_program() {
    validate(EXACT, ScalarTargetLayout::WASM32, false);
}
#[test]
fn d3_reborrow_state_authority_wasm_project() {
    validate(EXACT, ScalarTargetLayout::WASM32, true);
}
#[test]
fn d3_reborrow_state_storage_native_program() {
    storage(ScalarTargetLayout::NATIVE64, false);
}
#[test]
fn d3_reborrow_state_storage_native_project() {
    storage(ScalarTargetLayout::NATIVE64, true);
}
#[test]
fn d3_reborrow_state_storage_wasm_program() {
    storage(ScalarTargetLayout::WASM32, false);
}
#[test]
fn d3_reborrow_state_storage_wasm_project() {
    storage(ScalarTargetLayout::WASM32, true);
}

fn matrix(layout: ScalarTargetLayout, project: bool) {
    let positives = [
        ("sequential", "u32 item = 1; *!u32 parent = &!item; *!u32 first = &!(*parent); *first = 2; *!u32 second = &!(*parent); *second = 3; *parent = 4;"),
        ("grandchild", "u32 item = 1; *!u32 parent = &!item; *!u32 child = &!(*parent); *!u32 grandchild = &!(*child); *grandchild = 2; *child = 3; *parent = 4;"),
        ("shared", "u32 item = 1; *u32 parent = &item; *u32 child = &(*parent); u32 a = *child; u32 b = *parent; a; b;"),
        ("child transfer", "u32 item = 1; *!u32 parent = &!item; *!u32 child = &!(*parent); *!u32 moved = child; *moved = 2; *parent = 3;"),
        ("parent transfer after last use", "u32 item = 1; *!u32 parent = &!item; *!u32 child = &!(*parent); *child = 2; *!u32 moved = parent; *moved = 3;"),
        ("shared alias slot replacement", "u32 first = 1; u32 second = 2; *u32 original = &first; *u32 alias = original; original = &second; u32 a = *alias; u32 b = *original; a; b;"),
        ("checked slot through pointer", "u32 first = 1; u32 second = 2; *u32 stored = &first; *u32 alias = stored; *!(*u32) slot = &!stored; *slot = &second; u32 a = *alias; u32 b = *stored; a; b;"),
        ("captured child", "u32 item = 1; *!u32 parent = &!item; *!u32 child = &!(*parent); unit() write = fn { *child = 2; }; write(); *parent = 3;"),
        ("direct reference call transfer", "u32 item = 1; *!u32 parent = &!item; *!u32 child = &!(*parent); *!u32(*!u32) forward = fn(value) { value }; *!u32 moved = forward(child); *moved = 2; *parent = 3;"),
    ];
    let negatives = [
        ("parent write during live child", "u32 item = 1; *!u32 parent = &!item; *!u32 child = &!(*parent); *parent = 2; *child = 3;", "parent = 2", "parent", "&!(*parent)"),
        ("parent read during live child", "u32 item = 1; *!u32 parent = &!item; *!u32 child = &!(*parent); u32 observed = *parent; *child = 3; observed;", "*parent;", "*parent", "&!(*parent)"),
        ("shared child blocks parent write", "u32 item = 1; *!u32 parent = &!item; *u32 child = &(*parent); *parent = 2; u32 observed = *child; observed;", "parent = 2", "parent", "&(*parent)"),
        ("parent move during live child", "u32 item = 1; *!u32 parent = &!item; *!u32 child = &!(*parent); *!u32 moved = parent; *child = 3; *moved = 4;", "parent;", "parent", "&!(*parent)"),
        ("transferred child remains live", "u32 item = 1; *!u32 parent = &!item; *!u32 child = &!(*parent); *!u32 moved = child; *parent = 3; *moved = 4;", "parent = 3", "parent", "&!(*parent)"),
        ("slot overwrite conflict", "u32 first = 1; u32 second = 2; *u32 stored = &first; *(*u32) slot = &stored; stored = &second; slot; stored;", "stored = &second", "stored", "&stored"),
        ("captured child conflict", "u32 item = 1; *!u32 parent = &!item; *!u32 child = &!(*parent); unit() write = fn { *child = 2; }; *parent = 3; write();", "parent = 3", "parent", "&!(*parent)"),
        ("stale parent slot address", "u32 item = 1; *!u32 parent = &!item; *!u32 moved = parent; *(*!u32) slot = &parent; slot; *moved = 2;", "&parent", "&parent", "parent;"),
    ];
    let mut failures = Vec::new();
    for (name, body) in positives {
        let text = format!("%%start\nunit() run = fn {{ {body} }};\nrun();\n%%end");
        let parsed = parse_source(source(), text.clone(), &[]);
        assert!(
            parsed.diagnostics.is_empty(),
            "{name}: {:?}",
            parsed.diagnostics
        );
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
        if !diagnostics.is_empty() {
            failures.push(format!("{name}: {diagnostics:?}"));
        }
    }
    for (name, body, use_text, target, loan_text) in negatives {
        let text = format!("%%start\nunit() run = fn {{ {body} }};\nrun();\n%%end");
        let parsed = parse_source(source(), text.clone(), &[]);
        assert!(
            parsed.diagnostics.is_empty(),
            "{name}: {:?}",
            parsed.diagnostics
        );
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
        let use_start = text.find(use_text).unwrap() as u32;
        let loan_start = text.find(loan_text).unwrap() as u32;
        let use_span = SourceSpan::new(
            source(),
            ByteSpan::new(use_start, use_start + target.len() as u32),
        );
        let loan_length = if name == "stale parent slot address" {
            "parent".len()
        } else {
            loan_text.len()
        };
        let loan_span = SourceSpan::new(
            source(),
            ByteSpan::new(loan_start, loan_start + loan_length as u32),
        );
        if !diagnostics.iter().any(|diagnostic| {
            diagnostic.code == "B0003"
                && diagnostic.labels.iter().any(|label| {
                    label.kind == DiagnosticLabelKind::Primary && label.span == use_span
                })
                && diagnostic.labels.iter().any(|label| {
                    label.kind == DiagnosticLabelKind::Secondary && label.span == loan_span
                })
        }) {
            failures.push(format!(
                "{name}: missing exact {use_span:?}/{loan_span:?}: {diagnostics:?}"
            ));
        }
    }
    assert!(
        failures.is_empty(),
        "{layout:?}, project={project}: {failures:#?}"
    );
}

#[test]
fn d3_reborrow_state_matrix_native_program() {
    matrix(ScalarTargetLayout::NATIVE64, false);
}
#[test]
fn d3_reborrow_state_matrix_native_project() {
    matrix(ScalarTargetLayout::NATIVE64, true);
}
#[test]
fn d3_reborrow_state_matrix_wasm_program() {
    matrix(ScalarTargetLayout::WASM32, false);
}
#[test]
fn d3_reborrow_state_matrix_wasm_project() {
    matrix(ScalarTargetLayout::WASM32, true);
}

fn fields(layout: ScalarTargetLayout, project: bool) {
    let texts = [
        "%%start\nstruct Pair { u32 first; u32 second; }\nunit() run = fn { Pair item = { .first = 1; .second = 2; }; *!Pair parent = &!item; *!u32 first = &!(*parent).first; *!u32 second = &!(*parent).second; *first = 3; *second = 4; (*parent).first = 5; };\nrun();\n%%end",
        "%%start\nstruct copy RefPair { *u32 first; *u32 second; }\nunit() run = fn { u32 a = 1; u32 b = 2; u32 c = 3; RefPair slots = { .first = &a; .second = &b; }; *u32 alias = slots.first; slots.first = &c; u32 x = *alias; u32 y = *slots.first; u32 z = *slots.second; x; y; z; };\nrun();\n%%end",
    ];
    for text in texts {
        validate(text, layout, project);
    }
}

#[test]
fn d3_reborrow_state_fields_native_program() {
    fields(ScalarTargetLayout::NATIVE64, false);
}
#[test]
fn d3_reborrow_state_fields_native_project() {
    fields(ScalarTargetLayout::NATIVE64, true);
}
#[test]
fn d3_reborrow_state_fields_wasm_program() {
    fields(ScalarTargetLayout::WASM32, false);
}
#[test]
fn d3_reborrow_state_fields_wasm_project() {
    fields(ScalarTargetLayout::WASM32, true);
}

fn identity(layout: ScalarTargetLayout, project: bool) {
    let text = "%%start\nunit() run = fn { u32 item = 1; *u32 parent = &item; *u32 child = &(*parent); *u32 sibling = &(*parent); u32 a = *child; u32 b = *sibling; a; b; };\nrun();\n%%end";
    let mut cfgs = Vec::new();
    for path in ["src/main.w", "src/other.w"] {
        let identity = SourceIdentity {
            path: path.to_owned(),
            ..source()
        };
        let parsed = parse_source(identity.clone(), text.to_owned(), &[]);
        assert!(parsed.diagnostics.is_empty(), "{:?}", parsed.diagnostics);
        let single = derive_scalar_program_with_layout(&parsed.result, layout);
        let items = if project {
            let validated = validate_scalar_project(ScalarProject::new(
                vec![ScalarModule::from_program(single.program, Vec::new())],
                vec![identity.clone()],
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
            panic!("run function")
        };
        let cfg = function.reference_cfg.clone();
        let origins = cfg
            .facts
            .iter()
            .enumerate()
            .filter_map(|(index, fact)| match fact {
                ReferenceOrigin::Fresh { allocation, loan } => {
                    Some((ReferenceOriginId(index), allocation, loan))
                }
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(origins.len(), 3);
        assert_eq!(origins[0].1, origins[1].1);
        assert_eq!(origins[1].1, origins[2].1);
        for (_, _, loan) in &origins {
            assert_eq!(loan.creation_source, identity);
        }
        assert_eq!(origins[1].2.parent.as_deref(), Some(origins[0].2));
        assert_eq!(origins[2].2.parent.as_deref(), Some(origins[0].2));
        assert!(super::super::reborrow::authorizes(
            &cfg.facts,
            super::super::reborrow::AccessAuthority::Through(origins[1].0),
            origins[0].2
        ));
        assert!(!super::super::reborrow::authorizes(
            &cfg.facts,
            super::super::reborrow::AccessAuthority::Through(origins[1].0),
            origins[2].2
        ));
        assert!(!super::super::reborrow::authorizes(
            &cfg.facts,
            super::super::reborrow::AccessAuthority::DirectStorage,
            origins[1].2
        ));
        let restored: ScalarReferenceCfg =
            serde_json::from_str(&serde_json::to_string(&cfg).unwrap()).unwrap();
        assert_eq!(restored, cfg);
        cfgs.push(cfg);
    }
    let ReferenceOrigin::Fresh { loan: other, .. } = &cfgs[1].facts[1] else {
        panic!("child origin")
    };
    assert!(!super::super::reborrow::authorizes(
        &cfgs[0].facts,
        super::super::reborrow::AccessAuthority::Through(ReferenceOriginId(1)),
        other
    ));
}

#[test]
fn d3_reborrow_state_identity_native_program() {
    identity(ScalarTargetLayout::NATIVE64, false);
}
#[test]
fn d3_reborrow_state_identity_native_project() {
    identity(ScalarTargetLayout::NATIVE64, true);
}
#[test]
fn d3_reborrow_state_identity_wasm_program() {
    identity(ScalarTargetLayout::WASM32, false);
}
#[test]
fn d3_reborrow_state_identity_wasm_project() {
    identity(ScalarTargetLayout::WASM32, true);
}

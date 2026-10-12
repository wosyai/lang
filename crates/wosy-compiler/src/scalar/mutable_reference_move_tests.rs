use super::*;

const HISTORICAL: &str = "%%start\n*!u32(*!u32) transport_mutable = fn(value) { value };\nunit() run = fn {\nu32 item = 2;\n*!u32 mutable = &!item;\n*!u32 forwarded = transport_mutable(mutable);\nu32 invalid_value = *mutable;\nu32 forwarded_value = *forwarded;\ninvalid_value;\nforwarded_value;\n};\nrun();\n%%end";

fn historical(layout: ScalarTargetLayout, project: bool) {
    let parsed = parse_source(source(), HISTORICAL.to_owned(), &[]);
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
    let use_start = HISTORICAL.find("mutable;\n").unwrap() as u32;
    let move_start = HISTORICAL.find("mutable);").unwrap() as u32;
    assert!(
        diagnostics.iter().any(|diagnostic| {
            diagnostic.code == "B0003"
                && diagnostic.labels.iter().any(|label| {
                    label.kind == super::super::super::DiagnosticLabelKind::Primary
                        && label.span.source == source()
                        && label.span.range == ByteSpan::new(use_start, use_start + 7)
                })
                && diagnostic.labels.iter().any(|label| {
                    label.kind == super::super::super::DiagnosticLabelKind::Secondary
                        && label.span.source == source()
                        && label.span.range == ByteSpan::new(move_start, move_start + 7)
                })
        }),
        "{layout:?}, project={project}: {diagnostics:?}"
    );
}

#[test]
fn d3_mutable_reference_move_historical_native_single() {
    historical(ScalarTargetLayout::NATIVE64, false);
}

#[test]
fn d3_mutable_reference_move_historical_native_project() {
    historical(ScalarTargetLayout::NATIVE64, true);
}

#[test]
fn d3_mutable_reference_move_historical_wasm_single() {
    historical(ScalarTargetLayout::WASM32, false);
}

#[test]
fn d3_mutable_reference_move_historical_wasm_project() {
    historical(ScalarTargetLayout::WASM32, true);
}

fn matrix(layout: ScalarTargetLayout, project: bool) {
    let mut failures = Vec::new();
    for (name, body, accepted) in [
        ("destination", "u32 item = 2; *!u32 parent = &!item; *!u32 moved = parent; *moved = 3; u32 seen = *moved; seen;", true),
        ("source read", "u32 item = 2; *!u32 parent = &!item; *!u32 moved = parent; u32 seen = *parent; seen; moved;", false),
        ("second move", "u32 item = 2; *!u32 parent = &!item; *!u32 moved = parent; *!u32 again = parent; moved; again;", false),
        ("shared slot borrow", "u32 item = 2; *!u32 parent = &!item; *!u32 moved = parent; *(*!u32) slot = &parent; slot; moved;", false),
        ("mutable slot borrow", "u32 item = 2; *!u32 parent = &!item; *!u32 moved = parent; *!(*!u32) slot = &!parent; slot; moved;", false),
        ("raw slot address", "u32 item = 2; *!u32 parent = &!item; *!u32 moved = parent; unsafe { &?parent; }; moved;", false),
        ("reborrow moved source", "u32 item = 2; *!u32 parent = &!item; *!u32 moved = parent; *u32 child = &(*parent); child; moved;", false),
        ("reinitialize", "u32 item = 2; u32 other = 3; *!u32 parent = &!item; *!u32 moved = parent; parent = &!other; *parent = 4; moved;", true),
        ("shared copy", "u32 item = 2; *u32 parent = &item; *u32 copied = parent; u32 first = *parent; u32 second = *copied; first; second;", true),
        ("temporary child", "u32 item = 2; *!u32 parent = &!item; u32 seen = *relay(&!(*parent)); *!u32 moved = relay(parent); *moved = seen; moved;", true),
        ("live child", "u32 item = 2; *!u32 parent = &!item; *u32 child = &(*parent); *!u32 moved = relay(parent); u32 seen = *child; *moved = seen;", false),
        ("field destination", "u32 item = 2; u32 other = 3; Holder box = { .first = &!item; .second = &!other; }; *!u32 moved = box.first; *moved = 4; *!u32 second = box.second; u32 sibling = *second; sibling;", true),
        ("field source", "u32 item = 2; u32 other = 3; Holder box = { .first = &!item; .second = &!other; }; *!u32 moved = box.first; *!u32 again = box.first; moved; again; box.second;", false),
        ("field raw address", "u32 item = 2; u32 other = 3; Holder box = { .first = &!item; .second = &!other; }; *!u32 moved = box.first; unsafe { &?box.first; }; moved; box.second;", false),
        ("field slot borrow", "u32 item = 2; u32 other = 3; Holder box = { .first = &!item; .second = &!other; }; *!u32 moved = box.first; *(*!u32) slot = &box.first; slot; moved; box.second;", false),
        ("field transfer", "u32 item = 2; u32 other = 3; Holder box = { .first = &!item; .second = &!other; }; Holder next = { .first = null; .second = null; }; next.first = box.first; *!u32 moved = next.first; *moved = 4; box.second; next.second;", true),
        ("field reinitialize", "u32 item = 2; u32 other = 3; u32 replacement = 4; Holder box = { .first = &!item; .second = &!other; }; *!u32 moved = box.first; box.first = &!replacement; *!u32 restored = box.first; moved; restored; box.second;", true),
        ("sibling write", "u32 item = 2; u32 other = 3; u32 replacement = 4; Holder box = { .first = &!item; .second = &!other; }; *!u32 moved = box.first; box.second = &!replacement; *!u32 again = box.first; moved; again; box.second;", false),
        ("aggregate destination", "u32 item = 2; u32 other = 3; Holder box = { .first = &!item; .second = &!other; }; Holder moved = box; *!u32 first = moved.first; *!u32 second = moved.second; *first = 4; *second = 5;", true),
        ("aggregate source", "u32 item = 2; u32 other = 3; Holder box = { .first = &!item; .second = &!other; }; Holder moved = box; *!u32 stale = box.first; stale; moved;", false),
        ("aggregate forwarding", "u32 item = 2; u32 other = 3; Holder box = { .first = &!item; .second = &!other; }; Holder(Holder) forward = fn(value) { value }; Holder moved = forward(box); *!u32 first = moved.first; *!u32 second = moved.second; *first = 4; *second = 5;", true),
        ("field swap", "u32 item = 2; u32 other = 3; Holder box = { .first = &!item; .second = &!other; }; box.first, box.second = box.second, box.first; *!u32 first = box.first; *!u32 second = box.second; first; second;", true),
        ("branch reinitialize", "u32 item = 2; u32 other = 3; *!u32 parent = &!item; if (flag) { *!u32 moved = parent; moved; parent = &!other; }; u32 seen = *parent; seen;", true),
        ("branch moved", "u32 item = 2; *!u32 parent = &!item; if (flag) { *!u32 moved = parent; moved; }; u32 seen = *parent; seen;", false),
        ("second consuming call", "u32 item = 2; *!u32 parent = &!item; *!u32 moved = relay(parent); *!u32 again = relay(parent); moved; again;", false),
        ("repeated child calls", "u32 item = 2; *!u32 parent = &!item; u32 first = *relay(&!(*parent)); u32 second = *relay(&!(*parent)); *!u32 moved = relay(parent); *moved = first + second;", true),
        ("callable forwarding", "u32 item = 2; *!u32 parent = &!item; *!u32(*!u32) operation = relay; *!u32 moved = operation(parent); *moved = 4;", true),
        ("deferred mutable", "u32 item = 2; *!u32 parent = &!item; *!u32 moved = identity<*!u32>(parent); *moved = 4;", true),
        ("deferred second use", "u32 item = 2; *!u32 parent = &!item; *!u32 moved = twice<*!u32>(parent); moved;", false),
        ("deferred shared", "u32 item = 2; *u32 parent = &item; *u32 copied = twice<*u32>(parent); parent; copied;", true),
    ] {
        let text = format!("%%start\nstruct Holder {{ *!u32 first; *!u32 second; }}\n*!u32(*!u32) relay = fn(value) {{ value }};\ngeneric T; T(T) identity = fn(value) {{ value }};\ngeneric T; T(T) twice = fn(value) {{ T first = value; first; value }};\nunit(bool) run = fn(flag) {{ flag; {body} }};\n%%end");
        let parsed = parse_source(source(), text.clone(), &[]);
        if !parsed.diagnostics.is_empty() {
            failures.push(format!("{name}: parse {:?}", parsed.diagnostics));
            continue;
        }
        let single = derive_scalar_program_with_layout(&parsed.result, layout);
        let diagnostics = if project {
            validate_scalar_project(ScalarProject::new(
                vec![ScalarModule::from_program(single.program, Vec::new())],
                vec![source()],
            )).diagnostics
        } else {
            single.diagnostics
        };
        if accepted {
            if !diagnostics.is_empty() {
                failures.push(format!("{name}: {diagnostics:?}"));
            }
        } else if !diagnostics.iter().any(|diagnostic| {
            diagnostic.code == "B0003"
                && diagnostic.labels.iter().any(|label| label.kind == super::super::super::DiagnosticLabelKind::Primary && label.span.source == source())
                && diagnostic.labels.iter().any(|label| label.kind == super::super::super::DiagnosticLabelKind::Secondary && label.span.source == source())
        }) {
            failures.push(format!("{name}: missing structural conflict: {diagnostics:?}"));
        }
    }
    assert!(
        failures.is_empty(),
        "{layout:?}, project={project}:\n{}",
        failures.join("\n")
    );
}

#[test]
fn d3_mutable_reference_move_matrix_native_single() {
    matrix(ScalarTargetLayout::NATIVE64, false);
}

#[test]
fn d3_mutable_reference_move_matrix_native_project() {
    matrix(ScalarTargetLayout::NATIVE64, true);
}

#[test]
fn d3_mutable_reference_move_matrix_wasm_single() {
    matrix(ScalarTargetLayout::WASM32, false);
}

#[test]
fn d3_mutable_reference_move_matrix_wasm_project() {
    matrix(ScalarTargetLayout::WASM32, true);
}

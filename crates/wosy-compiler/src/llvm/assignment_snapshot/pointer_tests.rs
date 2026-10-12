use super::{emitted, last_store};
use crate::ScalarTargetLayout;

fn pointer_rows() -> (crate::scalar::ScalarReferenceCfg, crate::ScalarAssignment) {
    let source = wosy_syntax::SourceIdentity::new(
        "project".into(),
        "package".into(),
        "src/main.w".into(),
        "r1".into(),
    );
    let parsed = crate::parse_source(source, "%%start\n*?u16() inspect = fn { u16 left = 11; u16 right = 22; unsafe { *?u16 target = &?left; *?u16 replacement = &?right; target = replacement } };\ninspect();\n%%end".to_owned(), &[]);
    assert!(parsed.diagnostics.is_empty(), "{:?}", parsed.diagnostics);
    let validation =
        crate::derive_scalar_program_with_layout(&parsed.result, ScalarTargetLayout::NATIVE64);
    assert!(
        validation.diagnostics.is_empty(),
        "{:?}",
        validation.diagnostics
    );
    let function = validation
        .program
        .items
        .iter()
        .find_map(|item| match item {
            crate::ScalarItem::Function(function) if function.name == "inspect" => Some(function),
            _ => None,
        })
        .expect("source function");
    let crate::ScalarExpression::Block(block) = &function.body.final_output_values[0].value else {
        panic!("actual unsafe block");
    };
    let crate::ScalarExpression::Assignment(assignment) = &block.final_output_values[0].value
    else {
        panic!("actual pointer assignment occurrence");
    };
    let assignment = assignment.as_ref().clone();
    (function.reference_cfg.clone(), assignment)
}

#[test]
fn assignment_saved_pointer_static_exact_source_shape() {
    let (cfg, assignment) = pointer_rows();
    let events =
        super::super::super::holder_events::HolderEvents::new(&cfg).expect("actual events");
    let (_, evaluation) = events
        .assignment_evaluation(&assignment)
        .expect("actual plan");
    let (_, shape) = events
        .assignment_value(&evaluation.results[0].value)
        .expect("actual shape");
    assert_eq!(events.assignment_result_shape(&assignment), Some(shape));
    assert!(
        matches!(&shape.ty, crate::ScalarType::RawPointer(inner) if **inner == crate::ScalarType::U16)
    );
}

#[test]
fn assignment_saved_pointer_static_duplicate_evaluation() {
    let (mut cfg, assignment) = pointer_rows();
    cfg.evaluated_values
        .assignment_evaluations
        .push(cfg.evaluated_values.assignment_evaluations[0].clone());
    let events =
        super::super::super::holder_events::HolderEvents::new(&cfg).expect("actual events");
    assert!(events.assignment_result_shape(&assignment).is_none());
    assert!(events.assignment_evaluation(&assignment).is_err());
}

#[test]
fn assignment_saved_pointer_static_duplicate_shape() {
    let (mut cfg, assignment) = pointer_rows();
    let result = &cfg.evaluated_values.assignment_evaluations[0].results[0];
    let fact = cfg
        .evaluated_values
        .values
        .iter()
        .find(|fact| fact.id == result.value)
        .expect("actual fact");
    let shape = cfg
        .evaluated_values
        .shapes
        .iter()
        .find(|shape| shape.id == fact.shape)
        .expect("actual shape")
        .clone();
    cfg.evaluated_values.shapes.push(shape);
    let events =
        super::super::super::holder_events::HolderEvents::new(&cfg).expect("actual events");
    assert!(events.assignment_result_shape(&assignment).is_none());
    assert!(events.assignment_evaluation(&assignment).is_err());
}

#[test]
fn assignment_saved_pointer_static_foreign_occurrence() {
    let (cfg, mut assignment) = pointer_rows();
    assignment.occurrence.source_span.source.path = "src/foreign.w".into();
    let events =
        super::super::super::holder_events::HolderEvents::new(&cfg).expect("actual events");
    assert!(events.assignment_result_shape(&assignment).is_none());
    assert!(events.assignment_evaluation(&assignment).is_err());
}

#[test]
fn assignment_saved_pointer_emitted_requires_exact_table_key() {
    let (cfg, assignment) = pointer_rows();
    let events =
        super::super::super::holder_events::HolderEvents::new(&cfg).expect("actual events");
    let error = events
        .emitted_assignment_result_shape(&assignment)
        .expect_err("actual emitted table required");
    assert!(
        error.contains("missing emitted saved assignment result"),
        "{error}"
    );
    assert!(error.contains("src/main.w"), "{error}");
}

#[test]
fn assignment_saved_pointer_emitted_rejects_changed_shape() {
    let (cfg, assignment) = pointer_rows();
    let mut events =
        super::super::super::holder_events::HolderEvents::new(&cfg).expect("actual events");
    let (_, evaluation) = events
        .assignment_evaluation(&assignment)
        .expect("actual plan");
    let result = evaluation.results[0].clone();
    let (fact, shape) = events
        .assignment_value(&result.value)
        .expect("actual result");
    let fact = fact.clone();
    let mut shape = shape.clone();
    shape.ty = crate::ScalarType::U16;
    let error = events
        .record_saved_assignment_output(super::super::SavedAssignmentOutput {
            snapshot: evaluation.previous[0].clone(),
            result,
            shape,
            fact,
            payload: super::super::super::EmitValue::Unit,
        })
        .expect_err("shape mismatch rejected before pointer consumption");
    assert!(
        error.contains("emitted saved result differs from actual facts"),
        "{error}"
    );
}

#[test]
fn assignment_saved_pointer_emitted_rejects_snapshot_id_as_result() {
    let (cfg, assignment) = pointer_rows();
    let mut events =
        super::super::super::holder_events::HolderEvents::new(&cfg).expect("actual events");
    let (_, evaluation) = events
        .assignment_evaluation(&assignment)
        .expect("actual plan");
    let snapshot = evaluation.previous[0].clone();
    let crate::scalar::SnapshotState::Saved(id) = &snapshot.state else {
        panic!("actual saved pointer");
    };
    let (fact, shape) = events.assignment_value(id).expect("actual snapshot fact");
    let mut result = evaluation.results[0].clone();
    let fact = fact.clone();
    let shape = shape.clone();
    let result_fact = events
        .assignment_value(&result.value)
        .expect("actual result fact")
        .0
        .clone();
    result.value = id.clone();
    let error = events
        .record_saved_assignment_output(super::super::SavedAssignmentOutput {
            snapshot,
            result,
            shape,
            fact: result_fact,
            payload: super::super::super::EmitValue::Unit,
        })
        .expect_err("distinct actual result identity required");
    assert_ne!(fact.id, evaluation.results[0].value);
    assert!(
        error.contains("emitted saved result differs from actual facts"),
        "{error}"
    );
}

fn raw_destination(layout: ScalarTargetLayout, project: bool) {
    let body = emitted("%%start\nu16() inspect = fn { u16 left = 11; u16 right = 22; unsafe { *?u16 target = &?left; *?u16 replacement = &?right; *(target = replacement) = 33; target; } left };\ninspect();\n%%end", layout, project);
    assert_saved_pointer_use(&body, "i16", false, layout);
}

fn shared_read(layout: ScalarTargetLayout, project: bool) {
    let body = emitted("%%start\ni64() inspect = fn { i64 left = 11; i64 right = 22; *i64 target = &left; *i64 replacement = &right; i64 old = *(target = replacement); target; old };\ninspect();\n%%end", layout, project);
    assert_saved_pointer_use(&body, "i64", true, layout);
    assert!(body.contains("load i64, ptr %deref,"), "{body}");
    assert!(body.contains("ptr %old,"), "{body}");
}

fn mutable_destination(layout: ScalarTargetLayout, project: bool) {
    let body = emitted("%%start\nu32() inspect = fn { u32 left = 11; u32 right = 22; *!u32 target = &!left; *!u32 replacement = &!right; *(target = replacement) = 33; target; left };\ninspect();\n%%end", layout, project);
    assert_saved_pointer_use(&body, "i32", true, layout);
}

fn assert_saved_pointer_use(body: &str, pointee: &str, checked: bool, layout: ScalarTargetLayout) {
    let snapshot = body
        .lines()
        .find(|line| {
            line.contains("assignment_previous")
                && line.contains("load ")
                && line.contains("ptr %target,")
        })
        .expect("old saved pointer bits");
    let pointer_id = snapshot
        .split_once(" = ")
        .expect("old pointer SSA")
        .0
        .trim_start();
    let old = body.find(snapshot).expect("snapshot position");
    let replaced = body
        .find(last_store(body, "%target"))
        .expect("new pointer installation");
    let address = body
        .lines()
        .find(|line| line.contains("%deref = inttoptr") && line.contains(pointer_id))
        .expect("direct use of saved old pointer SSA");
    let dereference = body.find(address).expect("outer address position");
    assert!(old < replaced && replaced < dereference, "{body}");
    assert_eq!(
        body.matches("assignment_previous = load").count(),
        1,
        "{body}"
    );
    assert_eq!(body.matches("inttoptr").count(), 1, "{body}");
    if checked {
        let pointer_integer = if layout == ScalarTargetLayout::WASM32 {
            "i32"
        } else {
            "i64"
        };
        assert!(
            body.contains(&format!(
                "deref_is_null = icmp eq {pointer_integer} {pointer_id}, 0"
            )),
            "{body}"
        );
        assert!(body.contains("deref_null_continue"), "{body}");
    } else {
        assert!(!body.contains("deref_is_null"), "{body}");
    }
    if body.contains("ptr %old,") {
        assert!(
            body.contains(&format!("load {pointee}, ptr %deref,")),
            "{body}"
        );
    } else {
        assert!(
            body.lines()
                .any(|line| line.contains(&format!("store {pointee} "))
                    && line.contains("ptr %deref,")),
            "{body}"
        );
        assert!(
            dereference < body.find(last_store(body, "%deref")).expect("outer store"),
            "{body}"
        );
    }
}

fn field_destination(layout: ScalarTargetLayout, project: bool) {
    let body = emitted("%%start\nstruct copy Cell { u16 value; }\nu16() inspect = fn { Cell left = { .value = 11; }; Cell right = { .value = 22; }; unsafe { *?Cell target = &?left; *?Cell replacement = &?right; (*(target = replacement)).value = 33; target; } left.value };\ninspect();\n%%end", layout, project);
    let snapshot = body
        .lines()
        .find(|line| line.contains("assignment_previous") && line.contains("ptr %target,"))
        .expect("old pointer to resolved struct");
    let id = snapshot
        .split_once(" = ")
        .expect("saved struct-pointer SSA")
        .0
        .trim_start();
    let dereference = body
        .lines()
        .find(|line| line.contains("%deref = inttoptr") && line.contains(id))
        .expect("saved struct pointee address");
    let field = body
        .lines()
        .find(|line| line.contains("getelementptr inbounds i8, ptr %deref,"))
        .expect("typed field projection of saved referent");
    let field_id = field.split_once(" = ").expect("field SSA").0.trim_start();
    assert!(
        body.find(snapshot).expect("old")
            < body
                .find(last_store(&body, "%target"))
                .expect("replacement"),
        "{body}"
    );
    assert!(
        body.find(dereference).expect("old address") < body.find(field).expect("field"),
        "{body}"
    );
    assert!(
        body.lines()
            .any(|line| line.contains("store i16 ") && line.contains(&format!("ptr {field_id},"))),
        "{body}"
    );
    assert_eq!(body.matches("inttoptr").count(), 1, "{body}");
    assert!(!body.contains("deref_is_null"), "{body}");
}

fn array_destination(layout: ScalarTargetLayout, project: bool) {
    let body = emitted("%%start\nu8() inspect = fn { u8[2] left = [11, 12]; u8[2] right = [22, 23]; unsafe { *?(u8[2]) target = &?left; *?(u8[2]) replacement = &?right; (*(target = replacement))[1] = 33; target; } left[1] };\ninspect();\n%%end", layout, project);
    let snapshot = body
        .lines()
        .find(|line| line.contains("assignment_previous") && line.contains("ptr %target,"))
        .expect("old pointer to resolved fixed array");
    let id = snapshot
        .split_once(" = ")
        .expect("saved array-pointer SSA")
        .0
        .trim_start();
    assert!(
        body.lines()
            .any(|line| line.contains("%deref = inttoptr") && line.contains(id)),
        "{body}"
    );
    let element = body
        .lines()
        .find(|line| line.contains("getelementptr inbounds [2 x i8], ptr %deref,"))
        .expect("typed fixed-array projection of saved referent");
    let element_id = element
        .split_once(" = ")
        .expect("element SSA")
        .0
        .trim_start();
    assert!(
        body.find(last_store(&body, "%target"))
            .expect("replacement pointer")
            < body.find(element).expect("old element"),
        "{body}"
    );
    assert!(
        body.lines()
            .any(|line| line.contains("store i8 ") && line.contains(&format!("ptr {element_id},"))),
        "{body}"
    );
    assert_eq!(body.matches("inttoptr").count(), 1, "{body}");
}

fn first_of_multiple(layout: ScalarTargetLayout, project: bool) {
    let body = emitted("%%start\nu16() inspect = fn { u16 left = 11; u16 right = 22; i32 other = 1; unsafe { *?u16 target = &?left; *?u16 replacement = &?right; *(target, other = replacement, 2) = 33; target; } other; left };\ninspect();\n%%end", layout, project);
    let snapshot = body
        .lines()
        .find(|line| line.contains("assignment_previous") && line.contains("ptr %target,"))
        .expect("first saved pointer output");
    let id = snapshot
        .split_once(" = ")
        .expect("saved pointer SSA")
        .0
        .trim_start();
    assert!(
        body.lines()
            .any(|line| line.contains("insertvalue") && line.contains(id) && line.ends_with(", 0")),
        "{body}"
    );
    let extracted = body
        .lines()
        .find(|line| line.contains("extractvalue") && line.ends_with(", 0"))
        .expect("existing receiving rule selects output zero");
    let extracted_id = extracted
        .split_once(" = ")
        .expect("received pointer SSA")
        .0
        .trim_start();
    assert!(
        body.lines()
            .any(|line| line.contains("%deref = inttoptr") && line.contains(extracted_id)),
        "{body}"
    );
    assert!(
        body.lines()
            .any(|line| line.contains("store i16 ") && line.contains("ptr %deref,")),
        "{body}"
    );
    assert_eq!(body.matches("inttoptr").count(), 1, "{body}");
}

#[test]
fn assignment_saved_pointer_raw_program_native64() {
    raw_destination(ScalarTargetLayout::NATIVE64, false);
}
#[test]
fn assignment_saved_pointer_raw_program_wasm32() {
    raw_destination(ScalarTargetLayout::WASM32, false);
}
#[test]
fn assignment_saved_pointer_raw_project_native64() {
    raw_destination(ScalarTargetLayout::NATIVE64, true);
}
#[test]
fn assignment_saved_pointer_raw_project_wasm32() {
    raw_destination(ScalarTargetLayout::WASM32, true);
}
#[test]
fn assignment_saved_pointer_shared_program_native64() {
    shared_read(ScalarTargetLayout::NATIVE64, false);
}
#[test]
fn assignment_saved_pointer_shared_program_wasm32() {
    shared_read(ScalarTargetLayout::WASM32, false);
}
#[test]
fn assignment_saved_pointer_shared_project_native64() {
    shared_read(ScalarTargetLayout::NATIVE64, true);
}
#[test]
fn assignment_saved_pointer_shared_project_wasm32() {
    shared_read(ScalarTargetLayout::WASM32, true);
}
#[test]
fn assignment_saved_pointer_mutable_program_native64() {
    mutable_destination(ScalarTargetLayout::NATIVE64, false);
}
#[test]
fn assignment_saved_pointer_mutable_program_wasm32() {
    mutable_destination(ScalarTargetLayout::WASM32, false);
}
#[test]
fn assignment_saved_pointer_mutable_project_native64() {
    mutable_destination(ScalarTargetLayout::NATIVE64, true);
}
#[test]
fn assignment_saved_pointer_mutable_project_wasm32() {
    mutable_destination(ScalarTargetLayout::WASM32, true);
}
#[test]
fn assignment_saved_pointer_field_program_native64() {
    field_destination(ScalarTargetLayout::NATIVE64, false);
}
#[test]
fn assignment_saved_pointer_field_program_wasm32() {
    field_destination(ScalarTargetLayout::WASM32, false);
}
#[test]
fn assignment_saved_pointer_field_project_native64() {
    field_destination(ScalarTargetLayout::NATIVE64, true);
}
#[test]
fn assignment_saved_pointer_field_project_wasm32() {
    field_destination(ScalarTargetLayout::WASM32, true);
}
#[test]
fn assignment_saved_pointer_array_program_native64() {
    array_destination(ScalarTargetLayout::NATIVE64, false);
}
#[test]
fn assignment_saved_pointer_array_program_wasm32() {
    array_destination(ScalarTargetLayout::WASM32, false);
}
#[test]
fn assignment_saved_pointer_array_project_native64() {
    array_destination(ScalarTargetLayout::NATIVE64, true);
}
#[test]
fn assignment_saved_pointer_array_project_wasm32() {
    array_destination(ScalarTargetLayout::WASM32, true);
}
#[test]
fn assignment_saved_pointer_multiple_program_native64() {
    first_of_multiple(ScalarTargetLayout::NATIVE64, false);
}
#[test]
fn assignment_saved_pointer_multiple_program_wasm32() {
    first_of_multiple(ScalarTargetLayout::WASM32, false);
}
#[test]
fn assignment_saved_pointer_multiple_project_native64() {
    first_of_multiple(ScalarTargetLayout::NATIVE64, true);
}
#[test]
fn assignment_saved_pointer_multiple_project_wasm32() {
    first_of_multiple(ScalarTargetLayout::WASM32, true);
}

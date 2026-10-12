use super::super::{emit_scalar_llvm, emit_scalar_project_llvm, project_function_name};
use crate::{
    derive_scalar_program_with_layout, parse_source, validate_scalar_project, ScalarModule,
    ScalarProject, ScalarTargetLayout,
};
use wosy_syntax::SourceIdentity;

fn emitted(text: &str, layout: ScalarTargetLayout, project: bool) -> String {
    let source = SourceIdentity::new(
        "project".into(),
        "package".into(),
        "src/main.w".into(),
        "r1".into(),
    );
    let parsed = parse_source(source.clone(), text.to_owned(), &[]);
    assert!(parsed.diagnostics.is_empty(), "{:?}", parsed.diagnostics);
    let single = derive_scalar_program_with_layout(&parsed.result, layout);
    assert!(single.diagnostics.is_empty(), "{:?}", single.diagnostics);
    let (llvm, name) = if project {
        let validation = validate_scalar_project(ScalarProject::new(
            vec![ScalarModule::from_program(
                single.program.clone(),
                Vec::new(),
            )],
            vec![source.clone()],
        ));
        assert!(
            validation.diagnostics.is_empty(),
            "{:?}",
            validation.diagnostics
        );
        (
            emit_scalar_project_llvm(&validation).expect("actual project LLVM"),
            project_function_name(&source, "inspect"),
        )
    } else {
        (
            emit_scalar_llvm(&single).expect("actual program LLVM"),
            "inspect".to_owned(),
        )
    };
    let text = llvm.to_text();
    let body = text
        .lines()
        .skip_while(|line| !(line.starts_with("define ") && line.contains(&format!("@{name}("))))
        .take_while(|line| *line != "}")
        .collect::<Vec<_>>()
        .join("\n");
    assert!(!body.is_empty(), "{text}");
    body
}

fn last_store<'a>(body: &'a str, pointer: &str) -> &'a str {
    body.lines()
        .filter(|line| line.contains("store ") && line.contains(&format!("ptr {pointer},")))
        .last()
        .expect("actual target store")
}

fn numeric(layout: ScalarTargetLayout, project: bool) {
    let body = emitted("%%start\ni32() inspect = fn { i32 value = 1; i32 old = (value = 2); value; old };\ninspect();\n%%end", layout, project);
    let snapshot = body
        .find("%assignment_previous = load i32, ptr %value")
        .expect("old value load");
    let commit = body
        .find(last_store(&body, "%value"))
        .expect("new value store");
    let old_store = body
        .find("store i32 %assignment_previous, ptr %old")
        .expect("saved result binding");
    assert!(snapshot < commit && commit < old_store, "{body}");
    assert_eq!(
        body.matches("assignment_previous = load").count(),
        1,
        "{body}"
    );
    assert!(body.contains("store i32 1, ptr %value"), "{body}");
    assert!(
        body.contains("store i32 2, ptr %assignment_value"),
        "{body}"
    );
}

fn nested(layout: ScalarTargetLayout, project: bool) {
    let body = emitted("%%start\ni32() inspect = fn { i32 x = 1; i32 old = (x = (x = 2)); x; old };\ninspect();\n%%end", layout, project);
    let snapshots = body
        .lines()
        .filter(|line| line.contains("assignment_previous") && line.contains("load i32, ptr %x,"))
        .collect::<Vec<_>>();
    assert_eq!(snapshots.len(), 2, "{body}");
    let inner = body.find(snapshots[0]).expect("inner old1");
    let outer = body.find(snapshots[1]).expect("outer old2");
    let stores = body
        .lines()
        .filter(|line| line.contains("store i32 %assignment_value") && line.contains("ptr %x,"))
        .collect::<Vec<_>>();
    assert_eq!(stores.len(), 2, "{body}");
    let inner_store = body.find(stores[0]).expect("inner write2");
    let outer_store = body.find(stores[1]).expect("outer write1");
    let outer_id = snapshots[1]
        .split_once(" = ")
        .expect("outer SSA")
        .0
        .trim_start();
    assert!(
        inner < inner_store && inner_store < outer && outer < outer_store,
        "{body}"
    );
    assert!(
        body.contains(&format!("store i32 {outer_id}, ptr %old")),
        "{body}"
    );
    assert!(
        body[inner_store..outer].contains("store i32 %assignment_previous, ptr %assignment_value"),
        "{body}"
    );
}

fn multiple(layout: ScalarTargetLayout, project: bool) {
    let body = emitted("%%start\n(i32, i32)() inspect = fn { i32 left = 1; i32 right = 2; left, right = 3, 4 };\ninspect();\n%%end", layout, project);
    let left = body
        .lines()
        .find(|line| line.contains("assignment_previous") && line.contains("load i32, ptr %left,"))
        .expect("saved first output");
    let right = body
        .lines()
        .find(|line| line.contains("assignment_previous") && line.contains("load i32, ptr %right,"))
        .expect("saved second output");
    let first_commit = body.find(last_store(&body, "%left")).expect("first commit");
    let second_commit = body
        .find(last_store(&body, "%right"))
        .expect("second commit");
    assert!(
        body.find(left).expect("first snapshot") < body.find(right).expect("second snapshot"),
        "{body}"
    );
    assert!(
        body.find(right).expect("all snapshots") < first_commit && first_commit < second_commit,
        "{body}"
    );
    let left_id = left.split_once(" = ").expect("first SSA").0.trim_start();
    let right_id = right.split_once(" = ").expect("second SSA").0.trim_start();
    assert!(body.contains(&format!("i32 {left_id}, 0")), "{body}");
    assert!(body.contains(&format!("i32 {right_id}, 1")), "{body}");
    assert!(
        second_commit < body.find("insertvalue").expect("saved output assembly"),
        "{body}"
    );
}

fn selector_effect(layout: ScalarTargetLayout, project: bool) {
    let body = emitted("%%start\n(u64, u8)() inspect = fn { u64 index = 0; u8[3] items = [11, 22, 44]; index, items[{ index = 2; index }] = 1, 33 };\ninspect();\n%%end", layout, project);
    let gep = body
        .lines()
        .find(|line| line.contains("getelementptr") && line.contains("ptr %items,"))
        .expect("actual retained item pointer");
    let pointer = gep.split_once(" = ").expect("frozen SSA").0.trim_start();
    let frozen = body.find(gep).expect("freeze position");
    let nested_store = body[..frozen]
        .rfind(last_store(&body[..frozen], "%index"))
        .expect("selector effect");
    let index_snapshot = body
        .find("%assignment_previous = load i64, ptr %index")
        .expect("post-effect previous index");
    let item_snapshot = body
        .lines()
        .find(|line| {
            line.contains("assignment_previous")
                && line.contains(&format!("load i8, ptr {pointer},"))
        })
        .expect("snapshot through same frozen SSA");
    let first_commit = body
        .rfind(last_store(&body, "%index"))
        .expect("first outer write");
    let second_commit = body
        .find(last_store(&body, pointer))
        .expect("same pointer outer write");
    assert!(nested_store < frozen && frozen < index_snapshot, "{body}");
    assert!(
        index_snapshot < body.find(item_snapshot).expect("second old")
            && body.find(item_snapshot).expect("all old") < first_commit,
        "{body}"
    );
    assert!(first_commit < second_commit, "{body}");
    assert_eq!(
        body.matches("getelementptr inbounds [3 x i8], ptr %items,")
            .count(),
        1,
        "{body}"
    );
}

fn fixed(layout: ScalarTargetLayout, project: bool) {
    let body = emitted("%%start\nu8() inspect = fn { u8[2] items = [11, 22]; u8[2] replacement = [33, 44]; u8[2] old = (items = replacement); items[0] = 55; old[0] };\ninspect();\n%%end", layout, project);
    let snapshot = body
        .find("%assignment_previous = load [2 x i8], ptr %items")
        .expect("complete old bytes");
    let snapshot_store = body
        .find("store [2 x i8] %assignment_previous, ptr %assignment_previous_storage")
        .expect("independent saved bytes");
    let commit = body
        .find(last_store(&body, "%items"))
        .expect("replacement bytes installation");
    assert!(
        body.contains("%assignment_previous_storage = alloca [2 x i8]"),
        "{body}"
    );
    assert!(
        snapshot < snapshot_store && snapshot_store < commit,
        "{body}"
    );
    assert!(
        body[commit..].contains("load [2 x i8], ptr %assignment_previous_storage"),
        "{body}"
    );
    assert_eq!(
        body.matches("assignment_previous = load [2 x i8], ptr %items")
            .count(),
        1,
        "{body}"
    );
}

fn argument_block_final(layout: ScalarTargetLayout, project: bool) {
    let source = SourceIdentity::new(
        "project".into(),
        "package".into(),
        "src/main.w".into(),
        "r1".into(),
    );
    let identity = if project {
        project_function_name(&source, "identity")
    } else {
        "identity".to_owned()
    };
    let body = emitted("%%start\ni32(i32) identity = fn(value) { value };\ni32() inspect = fn { i32 x = 1; identity({ x = 2 }) };\ninspect();\n%%end", layout, project);
    let snapshot = body
        .find("%assignment_previous = load i32, ptr %x")
        .expect("actual block result snapshot");
    let commit = body.find(last_store(&body, "%x")).expect("block commit");
    let call = body
        .find(&format!("call i32 @{identity}(i32 %assignment_previous)"))
        .expect("saved argument without replay");
    assert!(snapshot < commit && commit < call, "{body}");
    assert_eq!(
        body.matches(&format!("call i32 @{identity}(")).count(),
        1,
        "{body}"
    );
    assert_eq!(
        body.matches("assignment_previous = load").count(),
        1,
        "{body}"
    );
    assert!(
        call < body
            .find("ret i32 %call")
            .expect("actual final returned argument result"),
        "{body}"
    );
}

fn inferred_discard(layout: ScalarTargetLayout, project: bool) {
    let body = emitted("%%start\ni32(i32) inspect = fn(value) { inferred = value; inferred = 2; inferred };\ninspect(1);\n%%end", layout, project);
    assert!(body.contains("%inferred = alloca i32"), "{body}");
    assert!(!body.contains("assignment_previous"), "{body}");
    assert_eq!(
        body.lines()
            .filter(|line| line.contains("store i32") && line.contains("ptr %inferred,"))
            .count(),
        2,
        "{body}"
    );
}

fn actual_lookup_rows() -> (crate::scalar::ScalarReferenceCfg, crate::ScalarAssignment) {
    let source = SourceIdentity::new(
        "project".into(),
        "package".into(),
        "src/main.w".into(),
        "r1".into(),
    );
    let parsed = parse_source(
        source,
        "%%start\ni32() inspect = fn { i32 value = 1; value = 2 };\ninspect();\n%%end".to_owned(),
        &[],
    );
    assert!(parsed.diagnostics.is_empty(), "{:?}", parsed.diagnostics);
    let validation =
        derive_scalar_program_with_layout(&parsed.result, ScalarTargetLayout::NATIVE64);
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
    let assignment = function
        .body
        .final_output_values
        .iter()
        .find_map(|output| match &output.value {
            crate::ScalarExpression::Assignment(assignment) => Some(assignment.as_ref().clone()),
            _ => None,
        })
        .expect("source final assignment");
    (function.reference_cfg.clone(), assignment)
}

#[test]
fn assignment_saved_snapshot_lookup_retains_distinct_actual_fact_identities() {
    let (cfg, assignment) = actual_lookup_rows();
    let events =
        super::super::holder_events::HolderEvents::new(&cfg).expect("actual holder events");
    let (plan, evaluation) = events
        .assignment_evaluation(&assignment)
        .expect("exact source-qualified pair");
    assert_eq!(plan.operation.producer, cfg.producer_owner);
    let crate::scalar::SnapshotState::Saved(snapshot) = &evaluation.previous[0].state else {
        panic!("actual observed saved snapshot");
    };
    let result = &evaluation.results[0];
    let (snapshot_fact, snapshot_shape) = events
        .assignment_value(snapshot)
        .expect("actual snapshot fact");
    let (result_fact, result_shape) = events
        .assignment_value(&result.value)
        .expect("actual result fact");
    assert_ne!(snapshot_fact.id, result_fact.id);
    assert_eq!(snapshot_fact.id.resolved, result_fact.id.resolved);
    assert_eq!(snapshot_shape, result_shape);
    assert_eq!(result.snapshot, snapshot.point);
}

#[test]
fn assignment_saved_snapshot_lookup_rejects_duplicate_actual_plan() {
    let (mut cfg, assignment) = actual_lookup_rows();
    cfg.evaluated_values
        .assignment_plans
        .push(cfg.evaluated_values.assignment_plans[0].clone());
    let events = super::super::holder_events::HolderEvents::new(&cfg).expect("holder events");
    let error = events
        .assignment_evaluation(&assignment)
        .expect_err("duplicate actual plan rejected");
    assert!(
        error.contains("expected one assignment plan/evaluation"),
        "{error}"
    );
    assert!(error.contains("src/main.w"), "{error}");
}

#[test]
fn assignment_saved_snapshot_lookup_rejects_replaced_snapshot_point() {
    let (mut cfg, assignment) = actual_lookup_rows();
    let evaluation = &mut cfg.evaluated_values.assignment_evaluations[0];
    evaluation.writes[0].previous = evaluation.destinations[0].point;
    let events = super::super::holder_events::HolderEvents::new(&cfg).expect("holder events");
    let error = events
        .assignment_evaluation(&assignment)
        .expect_err("different commit previous rejected");
    assert!(
        error.contains("invalid assignment destination/previous/commit row"),
        "{error}"
    );
}

#[test]
fn assignment_saved_snapshot_lookup_rejects_result_reusing_snapshot_identity() {
    let (mut cfg, assignment) = actual_lookup_rows();
    let evaluation = &mut cfg.evaluated_values.assignment_evaluations[0];
    let crate::scalar::SnapshotState::Saved(snapshot) = &evaluation.previous[0].state else {
        panic!("actual observed saved snapshot");
    };
    evaluation.results[0].value = snapshot.clone();
    let events = super::super::holder_events::HolderEvents::new(&cfg).expect("holder events");
    let error = events
        .assignment_evaluation(&assignment)
        .expect_err("snapshot identity is distinct from result");
    assert!(
        error.contains("invalid assignment snapshot/result correspondence"),
        "{error}"
    );
}

#[test]
fn assignment_saved_snapshot_numeric_program_native64() {
    numeric(ScalarTargetLayout::NATIVE64, false);
}
#[test]
fn assignment_saved_snapshot_numeric_program_wasm32() {
    numeric(ScalarTargetLayout::WASM32, false);
}
#[test]
fn assignment_saved_snapshot_numeric_project_native64() {
    numeric(ScalarTargetLayout::NATIVE64, true);
}
#[test]
fn assignment_saved_snapshot_numeric_project_wasm32() {
    numeric(ScalarTargetLayout::WASM32, true);
}
#[test]
fn assignment_saved_snapshot_nested_program_native64() {
    nested(ScalarTargetLayout::NATIVE64, false);
}
#[test]
fn assignment_saved_snapshot_nested_program_wasm32() {
    nested(ScalarTargetLayout::WASM32, false);
}
#[test]
fn assignment_saved_snapshot_nested_project_native64() {
    nested(ScalarTargetLayout::NATIVE64, true);
}
#[test]
fn assignment_saved_snapshot_nested_project_wasm32() {
    nested(ScalarTargetLayout::WASM32, true);
}
#[test]
fn assignment_saved_snapshot_multiple_program_native64() {
    multiple(ScalarTargetLayout::NATIVE64, false);
}
#[test]
fn assignment_saved_snapshot_multiple_program_wasm32() {
    multiple(ScalarTargetLayout::WASM32, false);
}
#[test]
fn assignment_saved_snapshot_multiple_project_native64() {
    multiple(ScalarTargetLayout::NATIVE64, true);
}
#[test]
fn assignment_saved_snapshot_multiple_project_wasm32() {
    multiple(ScalarTargetLayout::WASM32, true);
}
#[test]
fn assignment_saved_snapshot_selector_effect_program_native64() {
    selector_effect(ScalarTargetLayout::NATIVE64, false);
}
#[test]
fn assignment_saved_snapshot_selector_effect_program_wasm32() {
    selector_effect(ScalarTargetLayout::WASM32, false);
}
#[test]
fn assignment_saved_snapshot_selector_effect_project_native64() {
    selector_effect(ScalarTargetLayout::NATIVE64, true);
}
#[test]
fn assignment_saved_snapshot_selector_effect_project_wasm32() {
    selector_effect(ScalarTargetLayout::WASM32, true);
}
#[test]
fn assignment_saved_snapshot_fixed_program_native64() {
    fixed(ScalarTargetLayout::NATIVE64, false);
}
#[test]
fn assignment_saved_snapshot_fixed_program_wasm32() {
    fixed(ScalarTargetLayout::WASM32, false);
}
#[test]
fn assignment_saved_snapshot_fixed_project_native64() {
    fixed(ScalarTargetLayout::NATIVE64, true);
}
#[test]
fn assignment_saved_snapshot_fixed_project_wasm32() {
    fixed(ScalarTargetLayout::WASM32, true);
}
#[test]
fn assignment_saved_snapshot_argument_block_final_program_native64() {
    argument_block_final(ScalarTargetLayout::NATIVE64, false);
}
#[test]
fn assignment_saved_snapshot_argument_block_final_program_wasm32() {
    argument_block_final(ScalarTargetLayout::WASM32, false);
}
#[test]
fn assignment_saved_snapshot_argument_block_final_project_native64() {
    argument_block_final(ScalarTargetLayout::NATIVE64, true);
}
#[test]
fn assignment_saved_snapshot_argument_block_final_project_wasm32() {
    argument_block_final(ScalarTargetLayout::WASM32, true);
}
#[test]
fn assignment_saved_snapshot_inferred_discard_program_native64() {
    inferred_discard(ScalarTargetLayout::NATIVE64, false);
}
#[test]
fn assignment_saved_snapshot_inferred_discard_program_wasm32() {
    inferred_discard(ScalarTargetLayout::WASM32, false);
}
#[test]
fn assignment_saved_snapshot_inferred_discard_project_native64() {
    inferred_discard(ScalarTargetLayout::NATIVE64, true);
}
#[test]
fn assignment_saved_snapshot_inferred_discard_project_wasm32() {
    inferred_discard(ScalarTargetLayout::WASM32, true);
}

#[path = "pointer_tests.rs"]
mod pointer_tests;

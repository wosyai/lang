use super::super::{emit_scalar_llvm, emit_scalar_project_llvm, project_function_name};
use crate::{
    derive_scalar_program_with_layout, parse_source, validate_scalar_project, ScalarModule,
    ScalarProject, ScalarTargetLayout,
};
use wosy_syntax::SourceIdentity;

fn emitted_inspect(text: &str, layout: ScalarTargetLayout, project_pipeline: bool) -> String {
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
    let (llvm, name) = if project_pipeline {
        let project = validate_scalar_project(ScalarProject::new(
            vec![ScalarModule::from_program(
                single.program.clone(),
                Vec::new(),
            )],
            vec![source.clone()],
        ));
        assert!(project.diagnostics.is_empty(), "{:?}", project.diagnostics);
        (
            emit_scalar_project_llvm(&project).expect("project emission"),
            project_function_name(&source, "inspect"),
        )
    } else {
        (
            emit_scalar_llvm(&single).expect("program emission"),
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
    assert!(!body.is_empty(), "missing actual inspect function: {text}");
    body
}

fn last_store<'a>(body: &'a str, pointer: &str) -> &'a str {
    body.lines()
        .filter(|line| line.contains("store ") && line.contains(&format!("ptr {pointer},")))
        .last()
        .expect("actual target store")
}

fn frozen_index(layout: ScalarTargetLayout, project: bool) {
    let body = emitted_inspect(
        "%%start\nu64() inspect = fn { u64 index = 0; u8[2] items = [11, 22]; u64 next_index = 1; u8 replacement = 33; index, items[index] = next_index, replacement; items; index };\ninspect();\n%%end",
        layout,
        project,
    );
    let gep = body
        .lines()
        .find(|line| line.contains("getelementptr") && line.contains("ptr %items,"))
        .expect("actual frozen item destination");
    let pointer = gep.split_once(" = ").expect("GEP SSA").0.trim_start();
    let selected = body.find(gep).expect("frozen GEP");
    let committed = body
        .rfind(last_store(&body, "%index"))
        .expect("ordered index store");
    let item_store = body
        .lines()
        .find(|line| line.contains("store i8") && line.contains(&format!("ptr {pointer},")))
        .expect("store to exact frozen SSA");
    let index_load = body[..selected]
        .rfind("load i64, ptr %index")
        .expect("pre-write index load");
    assert!(index_load < selected && selected < committed, "{body}");
    assert!(
        committed < body.find(item_store).expect("item commit"),
        "{body}"
    );
    assert_eq!(
        body.matches("getelementptr inbounds [2 x i8], ptr %items,")
            .count(),
        1,
        "{body}"
    );
}

fn frozen_pointer(layout: ScalarTargetLayout, project: bool) {
    let body = emitted_inspect(
        "%%start\nu8() inspect = fn { u8 left = 11; u8 right = 22; unsafe { *?u8 pointer = &?left; *?u8 next_pointer = &?right; pointer, *pointer = next_pointer, 33; pointer; } left };\ninspect();\n%%end",
        layout,
        project,
    );
    let gep = body
        .lines()
        .find(|line| line.contains("%deref = inttoptr"))
        .expect("frozen dereference");
    let frozen = body.find(gep).expect("dereference position");
    let pointer_store = body
        .rfind(last_store(&body, "%pointer"))
        .expect("pointer replacement");
    let pointee_store = body.find("ptr %deref,").expect("same frozen pointer store");
    assert!(
        frozen < pointer_store && pointer_store < pointee_store,
        "{body}"
    );
    assert_eq!(body.matches("inttoptr").count(), 1, "{body}");
}

fn selector_effect(layout: ScalarTargetLayout, project: bool) {
    let body = emitted_inspect(
        "%%start\nu64() inspect = fn { u64 index = 0; u8[3] items = [11, 22, 44]; u64 next_index = 1; u8 replacement = 33; index, items[{ index = 2; index }] = next_index, replacement; items; index };\ninspect();\n%%end",
        layout,
        project,
    );
    let selected = body
        .lines()
        .find(|line| line.contains("getelementptr") && line.contains("ptr %items,"))
        .expect("destination after nested selector effect");
    let frozen = body.find(selected).expect("frozen destination");
    let selector_write = body[..frozen]
        .rfind(last_store(&body[..frozen], "%index"))
        .expect("nested assignment write");
    let outer_write = body
        .rfind(last_store(&body, "%index"))
        .expect("outer first write");
    let pointer = selected
        .split_once(" = ")
        .expect("frozen SSA")
        .0
        .trim_start();
    let item_store = body
        .lines()
        .find(|line| line.contains("store i8") && line.contains(&format!("ptr {pointer},")))
        .expect("exact retained item destination");
    assert!(selector_write < frozen && frozen < outer_write, "{body}");
    assert!(
        outer_write < body.find(item_store).expect("item write"),
        "{body}"
    );
    assert_eq!(
        body.matches("getelementptr inbounds [3 x i8], ptr %items,")
            .count(),
        1,
        "{body}"
    );
}

fn callable_read_transaction(layout: ScalarTargetLayout, project: bool) {
    let source = SourceIdentity::new(
        "project".into(),
        "package".into(),
        "src/main.w".into(),
        "r1".into(),
    );
    let next = if project {
        project_function_name(&source, "next")
    } else {
        "next".to_owned()
    };
    let body = emitted_inspect(
        "%%start\nu64 counter = 0;\nu64() next = fn { counter = counter + 1; counter };\nunit(u64) inspect = fn(count) { (u8())[count] readers; u64 index = 0; index, readers[next()] = 1, fn { 7 }; readers; index; };\ninspect(2);\n%%end",
        layout,
        project,
    );
    let call = format!("call i64 @{next}()");
    assert_eq!(body.matches(&call).count(), 1, "{body}");
    let selector = body.find(&call).expect("once-only user selector");
    let length = body
        .find("selected_current_length = load")
        .expect("shared length read");
    let acquire = "call void @__wosy_begin_graph_mutation()";
    let release = "call void @__wosy_end_graph_mutation()";
    let read_begin = body[..length].rfind(acquire).expect("read acquire");
    let gep = body
        .find("%array_element = getelementptr")
        .expect("frozen callable destination");
    let read_end = body[gep..].find(release).expect("successful read release") + gep;
    let index_commit = body
        .rfind(last_store(&body, "%index"))
        .expect("first outer commit");
    let publication = body
        .find("store ptr %array_element, ptr %selected_address_slot")
        .expect("same frozen address publication");
    let commit_begin = body[..publication]
        .rfind(acquire)
        .expect("separate commit acquire");
    let commit_end = body[publication..].find(release).expect("commit release") + publication;
    assert!(
        selector < read_begin && read_begin < length && length < gep,
        "{body}"
    );
    assert!(
        gep < read_end && read_end < index_commit && index_commit < commit_begin,
        "{body}"
    );
    assert!(
        commit_begin < publication && publication < commit_end,
        "{body}"
    );
    assert_eq!(
        body[..selector].matches(acquire).count(),
        body[..selector].matches(release).count(),
        "{body}"
    );
    assert_eq!(
        body.matches(acquire).count(),
        body.matches(release).count(),
        "{body}"
    );
    assert_eq!(
        body.matches("selected_current_length = load").count(),
        1,
        "{body}"
    );
    assert_eq!(
        body.matches("%array_element = getelementptr").count(),
        1,
        "{body}"
    );
    assert_eq!(
        body.matches("store ptr %array_element, ptr %selected_address_slot")
            .count(),
        1,
        "{body}"
    );
    assert!(
        body[commit_begin..commit_end].lines().any(|line| {
            line.contains("store { ptr, ptr }") && line.contains("ptr %array_element,")
        }),
        "{body}"
    );
    assert!(
        body.contains("store i64 %call, ptr %selected_index_slot"),
        "{body}"
    );
    assert!(!body[read_begin..read_end].contains(&call), "{body}");
    assert!(
        body[read_begin..read_end]
            .lines()
            .filter(|line| line.contains("call "))
            .all(|line| {
                line.contains(acquire) || line.contains("call void @__wosy_core_system_panic()")
            }),
        "{body}"
    );
    assert!(
        body[read_begin..read_end].contains("call void @__wosy_core_system_panic()"),
        "{body}"
    );
    assert!(body[read_begin..read_end].contains("unreachable"), "{body}");
}

fn namespace_selector(layout: ScalarTargetLayout) {
    let root_source = SourceIdentity::new(
        "project".into(),
        "package".into(),
        "src/main.w".into(),
        "r1".into(),
    );
    let child_source = SourceIdentity::new(
        "project".into(),
        "package".into(),
        "src/child.w".into(),
        "r1".into(),
    );
    let child = parse_source(
        child_source.clone(),
        "%%start\nu64 index = 0;\nu64() next = fn { index };\n%%end".to_owned(),
        &[],
    );
    let root = parse_source(
        root_source.clone(),
        "%%start\nchild = namespace package \"src/child.w\";\nunit() inspect = fn { u8[2] items = [11, 22]; child.index, items[child.next()] = 1, 33; items; };\ninspect();\n%%end".to_owned(),
        &[],
    );
    assert!(child.diagnostics.is_empty(), "{:?}", child.diagnostics);
    assert!(root.diagnostics.is_empty(), "{:?}", root.diagnostics);
    let child = derive_scalar_program_with_layout(&child.result, layout).program;
    let root = derive_scalar_program_with_layout(&root.result, layout).program;
    let namespaces = root
        .items
        .iter()
        .filter_map(|item| match item {
            crate::ScalarItem::Namespace(namespace) => Some(crate::ScalarNamespaceBinding {
                binding: namespace.binding.clone(),
                target: child_source.clone(),
                span: namespace.span,
            }),
            _ => None,
        })
        .collect();
    let project = validate_scalar_project(ScalarProject::new(
        vec![
            ScalarModule::new(root_source.clone(), root.items, namespaces),
            ScalarModule::new(child_source.clone(), child.items, Vec::new()),
        ],
        vec![root_source.clone(), child_source.clone()],
    ));
    assert!(project.diagnostics.is_empty(), "{:?}", project.diagnostics);
    let text = emit_scalar_project_llvm(&project)
        .expect("namespace selector emission")
        .to_text();
    let name = project_function_name(&root_source, "inspect");
    let body = text
        .lines()
        .skip_while(|line| !(line.starts_with("define ") && line.contains(&format!("@{name}("))))
        .take_while(|line| *line != "}")
        .collect::<Vec<_>>()
        .join("\n");
    let call = format!(
        "call i64 @{}()",
        project_function_name(&child_source, "next")
    );
    assert_eq!(body.matches(&call).count(), 1, "{body}");
    let destination = body
        .lines()
        .find(|line| line.contains("getelementptr") && line.contains("ptr %items,"))
        .expect("namespaced selector's frozen destination");
    let gep = body.find(destination).expect("frozen GEP");
    let global = super::super::project_global_name(&child_source, "index");
    let global_store = body
        .find(last_store(&body, &format!("@{global}")))
        .expect("declaring module store");
    assert!(
        body.find(&call).expect("namespace call") < gep && gep < global_store,
        "{body}"
    );
    let pointer = destination
        .split_once(" = ")
        .expect("destination SSA")
        .0
        .trim_start();
    assert!(
        global_store
            < body
                .find(last_store(&body, pointer))
                .expect("frozen item commit"),
        "{body}"
    );
}

#[test]
fn assignment_frozen_destination_index_program_native64() {
    frozen_index(ScalarTargetLayout::NATIVE64, false);
}
#[test]
fn assignment_frozen_destination_index_program_wasm32() {
    frozen_index(ScalarTargetLayout::WASM32, false);
}
#[test]
fn assignment_frozen_destination_index_project_native64() {
    frozen_index(ScalarTargetLayout::NATIVE64, true);
}
#[test]
fn assignment_frozen_destination_index_project_wasm32() {
    frozen_index(ScalarTargetLayout::WASM32, true);
}
#[test]
fn assignment_frozen_destination_pointer_program_native64() {
    frozen_pointer(ScalarTargetLayout::NATIVE64, false);
}
#[test]
fn assignment_frozen_destination_pointer_program_wasm32() {
    frozen_pointer(ScalarTargetLayout::WASM32, false);
}
#[test]
fn assignment_frozen_destination_pointer_project_native64() {
    frozen_pointer(ScalarTargetLayout::NATIVE64, true);
}
#[test]
fn assignment_frozen_destination_pointer_project_wasm32() {
    frozen_pointer(ScalarTargetLayout::WASM32, true);
}
#[test]
fn assignment_frozen_destination_selector_effect_program_native64() {
    selector_effect(ScalarTargetLayout::NATIVE64, false);
}
#[test]
fn assignment_frozen_destination_selector_effect_program_wasm32() {
    selector_effect(ScalarTargetLayout::WASM32, false);
}
#[test]
fn assignment_frozen_destination_selector_effect_project_native64() {
    selector_effect(ScalarTargetLayout::NATIVE64, true);
}
#[test]
fn assignment_frozen_destination_selector_effect_project_wasm32() {
    selector_effect(ScalarTargetLayout::WASM32, true);
}
#[test]
fn assignment_frozen_destination_callable_read_program_native64() {
    callable_read_transaction(ScalarTargetLayout::NATIVE64, false);
}
#[test]
fn assignment_frozen_destination_callable_read_program_wasm32() {
    callable_read_transaction(ScalarTargetLayout::WASM32, false);
}
#[test]
fn assignment_frozen_destination_callable_read_project_native64() {
    callable_read_transaction(ScalarTargetLayout::NATIVE64, true);
}
#[test]
fn assignment_frozen_destination_callable_read_project_wasm32() {
    callable_read_transaction(ScalarTargetLayout::WASM32, true);
}
#[test]
fn assignment_frozen_destination_namespace_selector_project_native64() {
    namespace_selector(ScalarTargetLayout::NATIVE64);
}
#[test]
fn assignment_frozen_destination_namespace_selector_project_wasm32() {
    namespace_selector(ScalarTargetLayout::WASM32);
}

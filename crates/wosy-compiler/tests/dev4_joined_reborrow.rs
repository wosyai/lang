use wosy_compiler::{
    derive_scalar_program_with_layout, emit_scalar_llvm, emit_scalar_project_llvm, parse_source,
    validate_scalar_project, ScalarModule, ScalarProject, ScalarTargetLayout,
};
use wosy_syntax::SourceIdentity;

#[derive(Clone, Copy)]
enum Expected {
    Valid,
    NullParent,
    LiveConflict,
    Declined,
}

fn check(text: &str, expected: Expected) {
    check_case(text, expected, None);
}

fn check_case(text: &str, expected: Expected, witness_dir: Option<&str>) {
    let source = SourceIdentity::new(
        "project".into(),
        "package".into(),
        "src/main.w".into(),
        "r1".into(),
    );
    for (target, layout) in [
        ("native64", ScalarTargetLayout::NATIVE64),
        ("wasm32", ScalarTargetLayout::WASM32),
    ] {
        let parsed = parse_source(source.clone(), text.to_owned(), &[]);
        assert!(parsed.diagnostics.is_empty(), "{:?}", parsed.diagnostics);
        let single = derive_scalar_program_with_layout(&parsed.result, layout);
        let project = validate_scalar_project(ScalarProject::new(
            vec![ScalarModule::from_program(single.program.clone(), Vec::new())],
            vec![source.clone()],
        ));
        for (route, diagnostics) in [
            ("standalone", &single.diagnostics), ("project", &project.diagnostics)
        ] {
            match expected {
                Expected::Valid => assert!(diagnostics.is_empty(), "{target}/{route}: {diagnostics:?}"),
                Expected::NullParent => {
                    let start = text.find("&(*parent)").expect("child reborrow") as u32;
                    assert!(diagnostics.iter().any(|diagnostic| {
                        diagnostic.code == "B0003"
                            && diagnostic.message == "missing required dereference origin"
                            && diagnostic.labels.first().is_some_and(|label| label.span.range.start == start)
                    }), "{target}/{route}: {diagnostics:?}");
                }
                Expected::Declined => assert!(diagnostics.iter().any(|item| item.code == "B0003"), "{target}/{route}: {diagnostics:?}"),
                Expected::LiveConflict => {
                    let write = text.find("first = 3").expect("owner write") as u32;
                    let child = text.find("&(*parent)").expect("child reborrow") as u32;
                    assert!(diagnostics.iter().any(|diagnostic| {
                        diagnostic.code == "B0003"
                            && diagnostic.labels.first().is_some_and(|label| label.span.range.start == write)
                            && diagnostic.labels.iter().skip(1).any(|label| label.span.range.start == child)
                    }), "{target}/{route}: {diagnostics:?}");
                }
            }
        }
        if matches!(expected, Expected::Valid) && text.contains("child") {
            let standalone = emit_scalar_llvm(&single).expect("standalone LLVM").to_text();
            let project = emit_scalar_project_llvm(&project).expect("project LLVM").to_text();
            if let Some(out) = witness_dir.and_then(std::env::var_os) {
                for (route, ir) in [("standalone", standalone), ("project", project)] {
                    let path = std::path::PathBuf::from(&out).join(format!("{target}-{route}.ll"));
                    std::fs::write(path, &ir).unwrap();
                }
            }
        }
    }
}

fn joined(second: &str, child: bool, before_read: &str, after_read: &str) -> String {
    let observe = if child { "*u8 child = &(*parent);" } else { "" };
    let read = if child { "*child" } else { "*parent" };
    format!(r#"%%start
u8(bool) inspect = fn(choose) {{
    u8 first = 7;
    u8 second = 9;
    *!u8 parent = null;
    if (choose) {{ parent = &!first; }} else {{ {second} }};
    {observe}
    {before_read}
    u8 seen = {read};
    {after_read}
    seen
}};
if (inspect(true) != 7 || inspect(false) != 9) {{ core.system_panic(); }};
%%end"#)
}

#[test]
fn distinct_joined_shared_child_is_resolved_per_path() {
    check(&joined("parent = &!second;", false, "", ""), Expected::Valid);
    check(&joined("parent = &!second;", true, "", "first = 3;"), Expected::Valid);
}

#[test]
fn conditional_null_parent_does_not_issue_a_child_loan() {
    check(&joined("second;", true, "", ""), Expected::NullParent);
}

#[test]
fn child_blocks_selected_owner_write_until_its_last_use() {
    check(&joined("parent = &!second;", true, "first = 3;", ""), Expected::LiveConflict);
}


#[test]
fn joined_child_never_outlives_a_branch_local_owner() {
    check(&joined("second; u8 branch_local = 9; parent = &!branch_local;", true, "", ""), Expected::Declined);
}

#[test]
fn joined_shared_child_does_not_widen_same_root_rebound_parents() {
    check(&joined("second; parent = &!first;", true, "", ""), Expected::NullParent);
}

#[test]
fn joined_child_survives_consecutive_unrelated_scalar_stores() {
    let program = joined("parent = &!second;", true, "unrelated = 4; unrelated = 5;", "first = 3; unrelated;");
    let program = program.replace("u8 first = 7;", "u8 first = 7; u8 unrelated = 0;");
    check(&program, Expected::Valid);
}

#[test]
fn joined_child_still_blocks_selected_owner_after_an_unrelated_store() {
    let program = joined("parent = &!second;", true, "unrelated = 4; first = 3;", "unrelated;");
    let program = program.replace("u8 first = 7;", "u8 first = 7; u8 unrelated = 0;");
    check(&program, Expected::LiveConflict);
}

#[test]
fn joined_child_reads_both_use_the_selected_owner() {
    let program = joined("parent = &!second;", true, "", "u8 again = *child; first = 3;");
    let program = program.replace("    seen\n", "    if (again == seen) { seen } else { 0 }\n");
    check(&program, Expected::Valid);
}

#[test]
fn joined_child_stays_live_between_typed_reads() {
    let program = joined("parent = &!second;", true, "", "first = 3; u8 again = *child; again;");
    check(&program, Expected::LiveConflict);
}

#[test]
fn joined_child_still_declines_reference_aliases() {
    let program = joined("parent = &!second;", true, "", "*u8 alias = child; u8 again = *alias; again;");
    check(&program, Expected::Declined);
}

fn joined_with_scalar_call(after_read: &str) -> String {
    let program = joined("parent = &!second;", true, "", after_read);
    program.replace("%%start\n", "%%start\nu8(u8) scalar = fn(value) { value };\n")
}

#[test]
fn joined_child_survives_a_direct_scalar_call() {
    let program = joined_with_scalar_call("u8 through_call = scalar(seen); u8 again = *child; first = 3;");
    let program = program.replace("    seen\n", "    if (again == through_call) { seen } else { 0 }\n");
    check_case(&program, Expected::Valid, Some("WOSY_REBORROW_TEST_IR_DIR"));
}

#[test]
fn joined_child_remains_live_after_a_direct_scalar_call() {
    let program = joined_with_scalar_call("u8 through_call = scalar(seen); first = 3; u8 again = *child; through_call; again;");
    check(&program, Expected::LiveConflict);
}

fn joined_with_pointer_read(after_read: &str, behavior: &str) -> String {
    let program = joined("parent = &!second;", true, "", after_read);
    program.replace("%%start\n", &format!("%%start\nu8(*u8) pointer = fn(value) {{ {behavior} }};\n"))
}

#[test]
fn joined_child_is_read_by_checked_callee_before_selected_owner_is_reusable() {
    let program = joined_with_pointer_read(
        "u8 through_call = pointer(child); first = 3;", "*value",
    );
    let program = program.replace("    seen\n", "    if (through_call == seen) { through_call } else { 0 }\n");
    check_case(&program, Expected::Valid, Some("WOSY_REBORROW_CALL_TEST_IR_DIR"));
}

#[test]
fn joined_child_stays_live_until_checked_callee_reads_it() {
    let program = joined_with_pointer_read(
        "first = 3; u8 through_call = pointer(child); through_call;", "*value",
    );
    check(&program, Expected::LiveConflict);
}

#[test]
fn joined_child_is_read_by_checked_scalar_arithmetic_before_owner_reuse() {
    for behavior in ["*value + 1", "1 + *value"] {
        let program = joined_with_pointer_read(
            "u8 through_call = pointer(child); first = 3;", behavior,
        );
        let program = program.replace(
            "    seen\n",
            "    if (through_call == seen + 1) { seen } else { 0 }\n",
        );
        check(&program, Expected::Valid);
    }
}

#[test]
fn joined_child_stays_live_until_checked_scalar_arithmetic_reads_it() {
    let program = joined_with_pointer_read(
        "first = 3; u8 through_call = pointer(child); through_call;", "*value + 1",
    );
    check(&program, Expected::LiveConflict);
}

#[test]
fn joined_child_declines_a_callee_without_a_prepared_read_effect() {
    let program = joined_with_pointer_read(
        "u8 through_call = pointer(child); through_call;", "*value + *value",
    );
    check(&program, Expected::NullParent);
}

#[test]
fn joined_child_cannot_escape_in_a_callee_return() {
    let program = joined("parent = &!second;", true, "", "*u8 escaped = pointer(child); *escaped;");
    let program = program.replace("%%start\n", "%%start\n*u8(*u8) pointer = fn(value) { value };\n");
    check(&program, Expected::NullParent);
}

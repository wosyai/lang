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
    check_case(text, expected, false);
}

fn check_case(text: &str, expected: Expected, write_witness: bool) {
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
            if let Some(out) = std::env::var_os("WOSY_REBORROW_TEST_IR_DIR").filter(|_| write_witness) {
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
    check_case(&program, Expected::Valid, true);
}

#[test]
fn joined_child_still_blocks_selected_owner_after_an_unrelated_store() {
    let program = joined("parent = &!second;", true, "unrelated = 4; first = 3;", "unrelated;");
    let program = program.replace("u8 first = 7;", "u8 first = 7; u8 unrelated = 0;");
    check(&program, Expected::LiveConflict);
}

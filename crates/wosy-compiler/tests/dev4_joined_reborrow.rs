use wosy_compiler::{
    analyze_scalar_build, derive_scalar_declarations_from_cst, derive_scalar_program_with_layout,
    emit_scalar_build_project_llvm, emit_scalar_llvm, emit_scalar_project_llvm, parse_source,
    prepare_scalar_project, validate_scalar_project, ScalarModule, ScalarNamespaceBinding,
    ScalarPreparationInputs, ScalarProject, ScalarTargetLayout,
};
use wosy_syntax::SourceIdentity;

#[derive(Clone, Copy)]
enum Expected {
    Valid,
    NullParent,
    LiveConflict,
    LiveSelectedFieldConflict,
    LiveDescendantConflict,
    LiveFamilyConflict,
    LiveMutableConflict,
    FrozenMutableConflict,
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
                Expected::LiveConflict | Expected::LiveSelectedFieldConflict | Expected::LiveDescendantConflict | Expected::LiveFamilyConflict | Expected::LiveMutableConflict | Expected::FrozenMutableConflict => {
                    let write = match expected {
                        Expected::FrozenMutableConflict => "*child = seen + 4",
                        Expected::LiveSelectedFieldConflict => "owner.left = 3",
                        _ => "first = 3",
                    };
                    let write = text.find(write).expect("conflicting write") as u32;
                    let exact_operand = match expected {
                        // Indirect stores first read their checked holder; a direct
                        // projected store is diagnosed at its resolved field token.
                        Expected::FrozenMutableConflict => Some((1, "child".len())),
                        Expected::LiveSelectedFieldConflict => Some(("owner.".len(), "left".len())),
                        _ => None,
                    };
                    let write = write + exact_operand.map_or(0, |(shift, _)| shift as u32);
                    let loan = match expected {
                        Expected::LiveDescendantConflict => "&(*child)",
                        Expected::LiveFamilyConflict => "&(*left)",
                        Expected::LiveMutableConflict => "&!(*parent)",
                        Expected::FrozenMutableConflict => "&(*child)",
                        _ => "&(*parent)",
                    };
                    let child = text.find(loan).expect("child reborrow") as u32;
                    assert!(diagnostics.iter().any(|diagnostic| {
                        diagnostic.code == "B0003"
                            && (exact_operand.is_none() || diagnostic.message == "read or move of unavailable place")
                            && diagnostic.labels.first().is_some_and(|label| {
                                label.span.range.start == write
                                    && exact_operand.is_none_or(|(_, len)| label.span.range.end == write + len as u32)
                            })
                            && diagnostic.labels.iter().skip(1).any(|label| {
                                label.span.range.start == child
                                    && (exact_operand.is_none() || label.span.range.end == child + loan.len() as u32)
                            })
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
fn joined_sibling_scalar_fields_keep_disjoint_place_authority() {
    let program = joined("parent = &!owner.right;", true, "", "owner.other = 4; u8 observed = *child;")
        .replace("%%start\n", "%%start\nstruct copy Trio { u8 left; u8 right; u8 other; }\n")
        .replace("u8 first = 7;\n    u8 second = 9;",
            "Trio owner = { .left = 7; .right = 9; .other = 0; };")
        .replace("&!first", "&!owner.left")
        .replace("    seen\n", "    if (observed == seen && owner.other == 4) { observed } else { 0 }\n");
    check_case(&program, Expected::Valid, Some("WOSY_JOINED_SIBLING_FIELDS_IR_DIR"));
    let live = program.replace("owner.other = 4;", "owner.left = 3; owner.other = 4;");
    assert_ne!(live, program);
    check(&live, Expected::LiveSelectedFieldConflict);
    let overlapping = program.replace("&!owner.right", "&!owner.left");
    assert_ne!(overlapping, program);
    check(&overlapping, Expected::NullParent);
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
fn joined_child_observations_in_nested_arms_end_before_owner_reuse() {
    let program = joined_with_pointer_read(
        "u8 observed = if (choose) { pointer(child) } else { *child }; first = 3;",
        "*value",
    ).replace("    seen\n", "    if (observed == seen && first == 3) { observed } else { 0 }\n");
    check_case(&program, Expected::Valid, Some("WOSY_JOINED_BRANCH_READ_IR_DIR"));

    let live = program.replace(
        "if (choose) { pointer(child) } else { *child }; first = 3;",
        "if (choose) { first = 3; pointer(child) } else { *child };",
    );
    assert_ne!(live, program);
    check(&live, Expected::LiveConflict);
}

#[test]
fn joined_child_stays_live_between_typed_reads() {
    let program = joined("parent = &!second;", true, "", "first = 3; u8 again = *child; again;");
    check(&program, Expected::LiveConflict);
}

#[test]
fn joined_child_alias_keeps_its_selected_predecessor_until_last_use() {
    let program = joined("parent = &!second;", true, "",
        "*u8 alias = child; u8 again = *alias; first = 3;");
    let program = program.replace("    seen\n", "    if (again == seen) { again } else { 0 }\n");
    check_case(&program, Expected::Valid, Some("WOSY_REBORROW_ALIAS_IR_DIR"));

    let live = program.replace("u8 again = *alias; first = 3;", "first = 3; u8 again = *alias;");
    assert_ne!(live, program);
    check(&live, Expected::LiveConflict);
}

#[test]
fn joined_child_alias_retains_its_origin_through_a_certified_reader() {
    let program = joined_with_pointer_read(
        "*u8 alias = child; *u8 copied = alias; u8 again = pointer(copied); first = 3;", "*value",
    );
    let program = program.replace("    seen\n", "    if (again == seen) { again } else { 0 }\n");
    check_case(&program, Expected::Valid, Some("WOSY_REBORROW_ALIAS_CALL_IR_DIR"));
    let live = program.replace("u8 again = pointer(copied); first = 3;", "first = 3; u8 again = pointer(copied);");
    assert_ne!(live, program);
    check(&live, Expected::LiveConflict);
}

#[test]
fn joined_child_alias_chain_is_bounded_with_no_fallback_authority() {
    for (size, expected) in [(8, Expected::Valid), (9, Expected::Declined)] {
        let mut body = String::new();
        let mut from = "child".to_owned();
        for i in 0..size {
            let name = format!("alias_{i}");
            body.push_str(&format!("*u8 {name} = {from}; "));
            from = name;
        }
        body.push_str(&format!("u8 again = *{from}; first = 3;"));
        let program = joined("parent = &!second;", true, "", &body)
            .replace("    seen\n", "    if (again == seen) { again } else { 0 }\n");
        check(&program, expected);
    }
}

#[test]
fn joined_child_alias_does_not_authorize_rebinding() {
    let program = joined("parent = &!second;", true, "",
        "*u8 alias = child; alias = null; u8 again = *alias; again;");
    check(&program, Expected::Declined);
}

#[test]
fn joined_shared_descendant_uses_the_selected_parent_until_its_last_read() {
    let program = joined("parent = &!second;", true, "",
        "*u8 nested = &(*child); u8 again = *nested; first = 3;")
        .replace("    seen\n", "    if (again == seen) { again } else { 0 }\n");
    check_case(&program, Expected::Valid, Some("WOSY_REBORROW_DESCENDANT_IR_DIR"));
    let live = program.replace("u8 again = *nested; first = 3;", "first = 3; u8 again = *nested;");
    assert_ne!(live, program);
    check(&live, Expected::LiveDescendantConflict);
    let alias = program.replace("*u8 nested = &(*child);", "*u8 alias = child; *u8 nested = &(*alias);");
    assert_ne!(alias, program);
    check(&alias, Expected::Valid);
    let third = program.replace("u8 again = *nested;", "*u8 third = &(*nested); u8 again = *third;");
    assert_ne!(third, program);
    check(&third, Expected::Valid);
}

#[test]
fn joined_loan_family_preserves_sibling_and_nested_alias_authority() {
    let program = joined_with_pointer_read(
        "*u8 left = &(*child); *u8 right = &(*child); *u8 deep = &(*left); *u8 alias = deep; \
         u8 sibling = *right; u8 again = pointer(alias); first = 3;", "*value",
    ).replace("    seen\n", "    if (again == seen && sibling == seen) { again } else { 0 }\n");
    check_case(&program, Expected::Valid, Some("WOSY_REBORROW_FAMILY_IR_DIR"));
    let live = program.replace("u8 again = pointer(alias); first = 3;", "first = 3; u8 again = pointer(alias);");
    assert_ne!(live, program);
    check(&live, Expected::LiveFamilyConflict);
}

#[test]
fn joined_descendant_family_has_a_fixed_issuance_bound() {
    for (size, expected) in [(4, Expected::Valid), (5, Expected::Declined)] {
        let mut body = String::new();
        let mut from = "child".to_owned();
        for i in 0..size {
            let name = format!("borrow_{i}");
            body.push_str(&format!("*u8 {name} = &(*{from}); "));
            from = name;
        }
        body.push_str(&format!("u8 again = *{from}; first = 3;"));
        let program = joined("parent = &!second;", true, "", &body)
            .replace("    seen\n", "    if (again == seen) { again } else { 0 }\n");
        check(&program, expected);
    }
}

#[test]
fn joined_mutable_child_updates_only_its_selected_lexical_owner() {
    let program = joined("parent = &!second;", true, "",
        "*child = seen + 2; u8 changed = *child; u8 left = first; u8 right = second; first = 3;")
        .replace("*u8 child = &(*parent);", "*!u8 child = &!(*parent);")
        .replace("    seen\n", "    if (choose) { if (changed == 9 && left == 9 && right == 9) { seen } else { 0 } } else { if (changed == 11 && left == 7 && right == 11) { seen } else { 0 } }\n");
    assert!(program.contains("*!u8 child = &!(*parent);"));
    check_case(&program, Expected::Valid, Some("WOSY_JOINED_MUTABLE_IR_DIR"));
    let live = program.replace("u8 changed = *child; u8 left = first; u8 right = second; first = 3;",
        "first = 3; u8 changed = *child; u8 left = first; u8 right = second;");
    assert_ne!(live, program);
    check(&live, Expected::LiveMutableConflict);

    let descendant = program.replace("u8 changed = *child;", "*!u8 nested = &!(*child); u8 changed = *nested;");
    assert_ne!(descendant, program);
    check(&descendant, Expected::Declined);
}

#[test]
fn joined_mutable_child_writes_in_separate_arms_after_edge_local_freeze() {
    let program = joined_with_pointer_read(
        "u8 observed = if (choose) { *u8 frozen = &(*child); u8 read = pointer(frozen); *child = seen + 4; read } \
         else { *child = seen + 2; seen }; u8 changed = *child; u8 left = first; u8 right = second;",
        "*value",
    ).replace("*u8 child = &(*parent);", "*!u8 child = &!(*parent);")
        .replace("    seen\n", "    if (choose) { if (observed == 7 && changed == 11 && left == 11 && right == 9) { seen } else { 0 } } else { if (observed == 9 && changed == 11 && left == 7 && right == 11) { seen } else { 0 } }\n");
    check_case(&program, Expected::Valid, Some("WOSY_JOINED_BRANCH_WRITE_IR_DIR"));

    let live = program.replace(
        "u8 read = pointer(frozen); *child = seen + 4;",
        "*child = seen + 4; u8 read = pointer(frozen);",
    );
    assert_ne!(live, program);
    check(&live, Expected::FrozenMutableConflict);
}

#[test]
fn joined_mutable_child_releases_branch_local_freeze_before_merge() {
    let program = joined_with_pointer_read(
        "u8 observed = if (choose) { *u8 frozen = &(*child); pointer(frozen) } else { *child }; \
         *child = seen + 4; u8 changed = *child; u8 left = first; u8 right = second;",
        "*value",
    ).replace("*u8 child = &(*parent);", "*!u8 child = &!(*parent);")
        .replace("    seen\n", "    if (choose) { if (observed == 7 && changed == 11 && left == 11 && right == 9) { seen } else { 0 } } else { if (observed == 9 && changed == 13 && left == 7 && right == 13) { seen } else { 0 } }\n");
    check_case(&program, Expected::Valid, Some("WOSY_JOINED_BRANCH_FREEZE_IR_DIR"));

    let live = program.replace(
        "*u8 frozen = &(*child); pointer(frozen)",
        "*u8 frozen = &(*child); first = 3; pointer(frozen)",
    );
    assert_ne!(live, program);
    check(&live, Expected::LiveDescendantConflict);
}

#[test]
fn joined_mutable_child_resumes_writes_after_shared_descendant_last_use() {
    let program = joined_with_pointer_read(
        "*child = seen + 2; *u8 frozen = &(*child); *u8 alias = frozen; u8 observed = pointer(alias); \
         *child = seen + 4; u8 changed = *child; u8 left = first; u8 right = second;", "*value",
    ).replace("*u8 child = &(*parent);", "*!u8 child = &!(*parent);")
        .replace("    seen\n", "    if (choose) { if (observed == 9 && changed == 11 && left == 11 && right == 9) { seen } else { 0 } } else { if (observed == 11 && changed == 13 && left == 7 && right == 13) { seen } else { 0 } }\n");
    check_case(&program, Expected::Valid, Some("WOSY_JOINED_FREEZE_IR_DIR"));
    let live = program.replace("u8 observed = pointer(alias); *child = seen + 4;",
        "*child = seen + 4; u8 observed = pointer(alias);");
    assert_ne!(live, program);
    check(&live, Expected::FrozenMutableConflict);
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

#[test]
fn joined_child_uses_only_the_certified_imported_reader() {
    let app = SourceIdentity::new("project".into(), "package".into(), "src/main.w".into(), "r1".into());
    let imported = SourceIdentity::new("project".into(), "package".into(), "src/reader.w".into(), "r1".into());
    let decoy = SourceIdentity::new("project".into(), "package".into(), "src/decoy.w".into(), "r1".into());
    let certified = "%%start\nu8(*u8) read = fn(view) { *view };\n%%end";
    let uncertified = certified.replace("{ *view }", "{ u8 observed = *view; observed }");
    let program = joined("parent = &!second;", true, "",
        "u8 observed = reader.read(child); first = 3;")
        .replace("%%start", "%%start\nreader = namespace package \"src/reader.w\";")
        .replace("    seen\n", "    if (observed == seen && first == 3) { observed } else { 0 }\n");
    let live = program.replace("u8 observed = reader.read(child); first = 3;",
        "first = 3; u8 observed = reader.read(child);");
    assert_ne!(live, program);
    let declarations = |source: &SourceIdentity, text: &str| {
        let parsed = parse_source(source.clone(), text.to_owned(), &[]);
        assert!(parsed.diagnostics.is_empty(), "{:?}", parsed.diagnostics);
        derive_scalar_declarations_from_cst(parsed.result.canonical_cst())
    };
    for (target, target_layout) in [("native64", ScalarTargetLayout::NATIVE64), ("wasm32", ScalarTargetLayout::WASM32)] {
        let prepare = |main: &str, reader: &str| {
            let root = declarations(&app, main);
            let namespace = root.namespaces().next().unwrap();
            let bindings = vec![ScalarNamespaceBinding { binding: namespace.binding.clone(),
                target: imported.clone(), span: namespace.span }];
            prepare_scalar_project(vec![(root, bindings),
                (declarations(&imported, reader), vec![]), (declarations(&decoy, certified), vec![])],
                vec![app.clone(), imported.clone(), decoy.clone()], ScalarPreparationInputs { target_layout }).unwrap()
        };
        let prepared = prepare(&program, certified);
        let build = analyze_scalar_build(&prepared);
        assert!(build.diagnostics().is_empty(), "{target}: {:?}", build.diagnostics());
        let ir = emit_scalar_build_project_llvm(&build).expect("imported joined read LLVM").to_text();
        if let Some(out) = std::env::var_os("WOSY_JOINED_IMPORTED_READ_IR_DIR") {
            let path = std::path::PathBuf::from(out).join(format!("{target}-project.ll"));
            std::fs::write(path, ir).unwrap();
        }
        let prepared = prepare(&program, &uncertified);
        let build = analyze_scalar_build(&prepared);
        let reborrow = program.find("&(*parent)").unwrap() as u32;
        assert!(build.diagnostics().iter().any(|d| d.code == "B0003" && d.labels.first()
            .is_some_and(|label| label.span.range.start == reborrow)), "{target}: {:?}", build.diagnostics());

        let prepared = prepare(&live, certified);
        let build = analyze_scalar_build(&prepared);
        let write = live.find("first = 3").unwrap() as u32;
        assert!(build.diagnostics().iter().any(|d| d.code == "B0003"
            && d.labels.first().is_some_and(|label| label.span.range.start == write)
            && d.labels.iter().skip(1).any(|label| label.span.range.start == reborrow)),
            "{target}: {:?}", build.diagnostics());
    }
}

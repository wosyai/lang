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
    DynamicProjection,
    OutOfRangeProjection,
    KnownAbsentProjection,
    LiveConflict,
    LiveSelectedFieldConflict,
    ExactProjectionConflict { target: &'static str, operand: &'static str, loan: &'static str },
    LiveNestedFieldConflict,
    LiveJoinedProjectionConflict,
    LiveParentProjectionConflict,
    LiveLexicalSiblingConflict,
    LiveMutableProjectionConflict,
    LiveIndexedProjectionConflict,
    LiveSelectedIndexConflict,
    LiveDescendantConflict,
    LiveFamilyConflict,
    LiveMutableConflict,
    FrozenMutableConflict,
    LiveExclusiveConflict,
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
                Expected::NullParent | Expected::DynamicProjection | Expected::OutOfRangeProjection | Expected::KnownAbsentProjection => {
                    let (reborrow, message) = match expected {
                        Expected::DynamicProjection => ("&!(*parent).values[index]",
                            "checked address requires a storage name or direct storage field"),
                        Expected::OutOfRangeProjection => ("&!(*parent).values[2]",
                            "checked projected borrow requires an in-range literal index into a fixed ordinary scalar array"),
                        Expected::KnownAbsentProjection => ("&!(*parent).values[1]", "missing required dereference origin"),
                        _ => ("&(*parent)", "missing required dereference origin"),
                    };
                    let start = text.find(reborrow).expect("child reborrow") as u32;
                    assert!(diagnostics.iter().any(|diagnostic| {
                        diagnostic.code == "B0003"
                            && diagnostic.message == message
                            && diagnostic.labels.first().is_some_and(|label| label.span.range.start == start)
                    }), "{target}/{route}: {diagnostics:?}");
                }
                Expected::Declined => assert!(diagnostics.iter().any(|item| item.code == "B0003"), "{target}/{route}: {diagnostics:?}"),
                Expected::ExactProjectionConflict { target: write, operand, loan } => {
                    let start = (text.find(write).expect("conflicting projection") + write.find(operand).unwrap()) as u32;
                    let end = start + operand.len() as u32;
                    let issued = text.find(loan).expect("exact loan") as u32;
                    assert!(diagnostics.iter().any(|d| d.code == "B0003"
                        && d.message == "read or move of unavailable place"
                        && d.labels.first().is_some_and(|label| label.span.range.start == start && label.span.range.end == end)
                        && d.labels.iter().skip(1).any(|label| label.span.range.start == issued
                            && label.span.range.end == issued + loan.len() as u32)),
                        "{target}/{route}: {diagnostics:?}");
                }
                Expected::LiveConflict | Expected::LiveSelectedFieldConflict | Expected::LiveNestedFieldConflict | Expected::LiveJoinedProjectionConflict | Expected::LiveParentProjectionConflict | Expected::LiveLexicalSiblingConflict | Expected::LiveMutableProjectionConflict | Expected::LiveIndexedProjectionConflict | Expected::LiveSelectedIndexConflict | Expected::LiveDescendantConflict | Expected::LiveFamilyConflict | Expected::LiveMutableConflict | Expected::FrozenMutableConflict | Expected::LiveExclusiveConflict => {
                    let write = match expected {
                        Expected::FrozenMutableConflict | Expected::LiveExclusiveConflict => "*child = seen + 4",
                        Expected::LiveSelectedFieldConflict => "owner.left = 3",
                        Expected::LiveNestedFieldConflict => "owner.values[0] = 3",
                        Expected::LiveParentProjectionConflict => "(*parent).value = 4",
                        Expected::LiveLexicalSiblingConflict => "first.other = 4",
                        Expected::LiveJoinedProjectionConflict | Expected::LiveMutableProjectionConflict => "first.value = 3",
                        Expected::LiveSelectedIndexConflict => "bytes[0] = 3",
                        Expected::LiveIndexedProjectionConflict => "first.values[1] = 3",
                        _ => "first = 3",
                    };
                    let write = text.find(write).expect("conflicting write") as u32;
                    let exact_operand = match expected {
                        // Indirect stores first read their checked holder; a direct
                        // projected store is diagnosed at its resolved field token.
                        Expected::FrozenMutableConflict | Expected::LiveExclusiveConflict => Some((1, "child".len())),
                        Expected::LiveSelectedFieldConflict => Some(("owner.".len(), "left".len())),
                        Expected::LiveNestedFieldConflict => Some(("owner.".len(), "values".len())),
                        Expected::LiveParentProjectionConflict => Some(("(*parent).".len(), "value".len())),
                        Expected::LiveLexicalSiblingConflict => Some(("first.".len(), "other".len())),
                        Expected::LiveJoinedProjectionConflict | Expected::LiveMutableProjectionConflict => Some(("first.".len(), "value".len())),
                        Expected::LiveSelectedIndexConflict => Some((0, "bytes".len())),
                        Expected::LiveIndexedProjectionConflict => Some(("first.".len(), "values".len())),
                        _ => None,
                    };
                    let write = write + exact_operand.map_or(0, |(shift, _)| shift as u32);
                    let loan = match expected {
                        Expected::LiveDescendantConflict => "&(*child)",
                        Expected::LiveFamilyConflict => "&(*left)",
                        Expected::LiveMutableConflict => "&!(*parent)",
                        Expected::FrozenMutableConflict => "&(*child)",
                        Expected::LiveExclusiveConflict => "&!(*child)",
                        Expected::LiveJoinedProjectionConflict | Expected::LiveParentProjectionConflict
                            | Expected::LiveLexicalSiblingConflict => "&(*parent).value",
                        Expected::LiveMutableProjectionConflict => "&!(*parent).value",
                        Expected::LiveIndexedProjectionConflict => "&!(*parent).values[1]",
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
fn nested_holder_liveness_respects_exact_overwrite_and_last_use() {
    let program = r#"%%start
u8(bool, bool) select_value = fn(outer, inner) {
    u8 first = 7;
    u8 second = 9;
    *u8 child = &first;
    if (outer) {
        if (inner) { child = &second; first = 3; } else { second = 11; };
    } else {
        child = &second;
        first = 5;
    };
    u8 value = *child;
    first = 1;
    if (first == 1) { value } else { 0 }
};
u8(bool) inspect = fn(choose) {
    if (choose) {
        if (select_value(true, true) == 9 && select_value(true, false) == 7) { 7 } else { 0 }
    } else {
        if (select_value(false, true) == 9 && select_value(false, false) == 9) { 9 } else { 0 }
    }
};
if (inspect(true) != 7 || inspect(false) != 9) { core.system_panic(); };
%%end"#;
    check_case(program, Expected::Valid, Some("WOSY_NESTED_HOLDER_LIVENESS_IR_DIR"));

    let live = program.replace("second = 11;", "first = 4; second = 11;");
    assert_ne!(live, program);
    check(&live, Expected::ExactProjectionConflict {
        target: "first = 4", operand: "first", loan: "&first",
    });
}

#[test]
fn selected_allocation_loan_ends_before_lexical_cleanup_after_reader_call() {
    let program = r#"%%start
u8(*u8) read_selected = fn(view) { *view };
u8(u64, bool) observe = fn(count, choose) {
    u8[count] first;
    u8[count] second;
    first[0] = 7;
    second[0] = 9;
    *!u8 parent = null;
    if (choose) { parent = &!first[0]; } else { parent = &!second[0]; };
    *u8 child = &(*parent);
    read_selected(child)
};
u8(bool) inspect = fn(choose) { observe(2, choose) };
if (inspect(true) != 7 || inspect(false) != 9) { core.system_panic(); };
%%end"#;
    check_case(program, Expected::Valid, Some("WOSY_ALLOCATED_JOINED_READER_IR_DIR"));

    let duplicate_allocation = program.replace("&!second[0]", "&!first[1]")
        .replace("    u8[count] second;\n", "").replace("second[0] = 9;", "first[1] = 9;");
    assert_ne!(duplicate_allocation, program);
    check(&duplicate_allocation, Expected::NullParent);
}

#[test]
fn checked_release_tracks_selected_allocation_and_its_loan_cohort() {
    let program = r#"%%start
u8(u64, bool) read_other = fn(count, choose) {
    u8[count] first;
    u8[count] second;
    first[0] = 7;
    second[0] = 9;
    *u8 selected = &first[1];
    *u8 child = &second[0];
    if (choose) {
        selected = &second[1];
        child = &first[0];
    };
    unsafe { core.free(selected); };
    *child
};
u8(bool) inspect = fn(choose) { read_other(2, choose) };
if (inspect(true) != 7 || inspect(false) != 9) { core.system_panic(); };
%%end"#;
    check_case(program, Expected::Valid, Some("WOSY_CHECKED_RELEASE_COHORT_IR_DIR"));

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
fn joined_constant_array_elements_keep_disjoint_place_authority() {
    let program = joined("parent = &!bytes[1];", true, "", "bytes[2] = 4; u8 observed = *child;")
        .replace("u8 first = 7;\n    u8 second = 9;", "u8[3] bytes = [7, 9, 0];")
        .replace("&!first", "&!bytes[0]")
        .replace("    seen\n", "    if (observed == seen && bytes[2] == 4) { observed } else { 0 }\n");
    check_case(&program, Expected::Valid, Some("WOSY_JOINED_ARRAY_ELEMENTS_IR_DIR"));
    let live = program.replace("bytes[2] = 4;", "bytes[0] = 3; bytes[2] = 4;");
    assert_ne!(live, program);
    check(&live, Expected::LiveSelectedIndexConflict);
    let overlapping = program.replace("&!bytes[1]", "&!bytes[0]");
    assert_ne!(overlapping, program);
    check(&overlapping, Expected::NullParent);
    let dynamic = program.replace("parent = &!bytes[1];", "u64 slot = 1; parent = &!bytes[slot];");
    assert_ne!(dynamic, program);
    check(&dynamic, Expected::NullParent);
}

#[test]
fn joined_composite_places_preserve_index_and_field_disjointness() {
    let program = joined("parent = &!owner.values[1];", true, "", "owner.other[0] = 4; u8 observed = *child;")
        .replace("%%start\n", "%%start\nstruct Record { u8[2] values; u8[2] other; }\n")
        .replace("u8 first = 7;\n    u8 second = 9;",
            "Record owner = { .values = [7, 9]; .other = [0, 0]; };")
        .replace("&!first", "&!owner.values[0]")
        .replace("    seen\n", "    if (observed == seen && owner.other[0] == 4) { observed } else { 0 }\n");
    check_case(&program, Expected::Valid, Some("WOSY_JOINED_COMPOSITE_PLACES_IR_DIR"));
    let live = program.replace("owner.other[0] = 4;", "owner.values[0] = 3; owner.other[0] = 4;");
    assert_ne!(live, program);
    check(&live, Expected::LiveNestedFieldConflict);
    let overlapping = program.replace("&!owner.values[1]", "&!owner.values[0]");
    assert_ne!(overlapping, program);
    check(&overlapping, Expected::NullParent);
    let dynamic = program.replace("parent = &!owner.values[1];",
        "u64 index = 1; parent = &!owner.values[index];");
    assert_ne!(dynamic, program);
    check(&dynamic, Expected::NullParent);
}

#[test]
fn joined_nested_selection_resolves_four_exact_lexical_predecessors() {
    let program = r#"%%start
u8(bool, bool) select_value = fn(outer, inner) {
    u8 first = 7;
    u8 second = 8;
    u8 third = 9;
    u8 fourth = 10;
    *!u8 parent = null;
    if (outer) {
        if (inner) { parent = &!first; } else { parent = &!second; };
    } else {
        if (inner) { parent = &!third; } else { parent = &!fourth; };
    };
    *u8 child = &(*parent);
    u8 seen = *child;
    first = 3;
    if (first == 3) { seen } else { 0 }
};
u8(bool) inspect = fn(choose) {
    if (choose) {
        if (select_value(true, true) == 7 && select_value(true, false) == 8) { 7 } else { 0 }
    } else {
        if (select_value(false, true) == 9 && select_value(false, false) == 10) { 9 } else { 0 }
    }
};
if (inspect(true) != 7 || inspect(false) != 9) { core.system_panic(); };
%%end"#;
    check_case(program, Expected::Valid, Some("WOSY_JOINED_FOUR_PARENTS_IR_DIR"));
    let overlap = program.replace("parent = &!fourth;", "fourth; parent = &!first;");
    assert_ne!(overlap, program);
    check(&overlap, Expected::NullParent);
    let absent = program.replace("parent = &!fourth;", "fourth;");
    assert_ne!(absent, program);
    check(&absent, Expected::NullParent);
    let above_cap = program
        .replace("u8(bool, bool) select_value = fn(outer, inner)",
            "u8(bool, bool, bool) select_value = fn(outer, inner, extra)")
        .replace("u8 fourth = 10;", "u8 fourth = 10; u8 fifth = 11;")
        .replace("parent = &!fourth;", "if (extra) { parent = &!fourth; } else { parent = &!fifth; };")
        .replace("select_value(true, true)", "select_value(true, true, false)")
        .replace("select_value(true, false)", "select_value(true, false, false)")
        .replace("select_value(false, true)", "select_value(false, true, false)")
        .replace("select_value(false, false) == 10",
            "select_value(false, false, true) == 10 && select_value(false, false, false) == 11");
    assert_ne!(above_cap, program);
    check(&above_cap, Expected::NullParent);
}

#[test]
fn joined_predecessors_reuse_same_loan_across_independent_edges() {
    let program = r#"%%start
u8(bool, bool) select_value = fn(outer, inner) {
    u8 first = 7;
    u8 second = 9;
    u8 third = 13;
    u8 fourth = 15;
    *!u8 parent = null;
    *u8 side = null;
    if (outer) { parent = &!first; } else { parent = &!second; };
    if (inner) { side = &third; } else { side = &fourth; };
    *u8 child = &(*parent);
    u8 seen = *child;
    u8 side_value = *side;
    first = 3;
    bool correct_side = if (inner) { side_value == 13 } else { side_value == 15 };
    if (first == 3 && correct_side) { seen } else { 0 }
};
u8(bool) inspect = fn(choose) {
    u8 left = select_value(choose, true);
    u8 right = select_value(choose, false);
    if (left == right) { left } else { 0 }
};
if (inspect(true) != 7 || inspect(false) != 9) { core.system_panic(); };
%%end"#;
    check_case(program, Expected::Valid, Some("WOSY_JOINED_DUPLICATE_EDGES_IR_DIR"));
    let live = program.replace("u8 seen = *child;\n    u8 side_value = *side;\n    first = 3;",
        "first = 3;\n    u8 seen = *child;\n    u8 side_value = *side;");
    assert_ne!(live, program);
    check(&live, Expected::LiveConflict);
}

#[test]
fn joined_struct_parent_reborrows_one_exact_scalar_field() {
    let program = joined("parent = &!second;", true, "", "first.value = 3;")
        .replace("%%start\n", "%%start\nstruct copy Pair { u8 value; u8 other; }\n")
        .replace("u8 first = 7;\n    u8 second = 9;",
            "Pair first = { .value = 7; .other = 1; }; Pair second = { .value = 9; .other = 2; };")
        .replace("*!u8 parent = null;", "*!Pair parent = null;")
        .replace("&(*parent)", "&(*parent).value")
        .replace("    seen\n", "    if (first.value == 3 && second.value == 9) { seen } else { 0 }\n");
    check_case(&program, Expected::Valid, Some("WOSY_JOINED_STRUCT_PROJECTION_IR_DIR"));

    let live = program.replace("u8 seen = *child;\n    first.value = 3;",
        "first.value = 3;\n    u8 seen = *child;");
    assert_ne!(live, program);
    check(&live, Expected::LiveJoinedProjectionConflict);
}

#[test]
fn joined_struct_parent_grants_one_exclusive_scalar_field() {
    let program = joined_with_pointer_read(
        "*child = seen + 2; *u8 frozen = &(*child); u8 read = pointer(frozen); *child = seen + 4; \
         u8 changed = *child; u8 first_value = first.value; u8 second_value = second.value;",
        "*value",
    ).replace("%%start\n", "%%start\nstruct copy Pair { u8 value; u8 other; }\n")
        .replace("u8 first = 7;\n    u8 second = 9;",
            "Pair first = { .value = 7; .other = 1; }; Pair second = { .value = 9; .other = 2; };")
        .replace("*!u8 parent = null;", "*!Pair parent = null;")
        .replace("*u8 child = &(*parent);", "*!u8 child = &!(*parent).value;")
        .replace("    seen\n",
            "    if (first.other == 1 && second.other == 2 && read == seen + 2 && changed == seen + 4) { if (choose) { if (first_value == 11 && second_value == 9) { seen } else { 0 } } else { if (first_value == 7 && second_value == 13) { seen } else { 0 } } } else { 0 }\n");
    check_case(&program, Expected::Valid, Some("WOSY_JOINED_MUTABLE_PROJECTION_IR_DIR"));

    let live = program.replace("u8 seen = *child;", "first.value = 3; u8 seen = *child;");
    assert_ne!(live, program);
    check(&live, Expected::LiveMutableProjectionConflict);
    let frozen = program.replace("u8 read = pointer(frozen); *child = seen + 4;",
        "*child = seen + 4; u8 read = pointer(frozen);");
    assert_ne!(frozen, program);
    check(&frozen, Expected::FrozenMutableConflict);
}

#[test]
fn joined_struct_parent_grants_in_range_literal_array_element() {
    let program = joined("parent = &!second;", true, "",
        "*child = seen + 2; u8 changed = *child; u8 left = first.values[1]; u8 right = second.values[1];")
        .replace("%%start\n", "%%start\nstruct Record { u8[2] values; }\n")
        .replace("u8 first = 7;\n    u8 second = 9;",
            "Record first = { .values = [1, 7]; }; Record second = { .values = [2, 9]; };")
        .replace("*!u8 parent = null;", "*!Record parent = null;")
        .replace("*u8 child = &(*parent);", "*!u8 child = &!(*parent).values[1];")
        .replace("    seen\n",
            "    if (first.values[0] == 1 && second.values[0] == 2 && changed == seen + 2) { if (choose) { if (left == 9 && right == 9) { seen } else { 0 } } else { if (left == 7 && right == 11) { seen } else { 0 } } } else { 0 }\n");
    check_case(&program, Expected::Valid, Some("WOSY_JOINED_INDEXED_PROJECTION_IR_DIR"));

    let live = program.replace("u8 seen = *child;", "first.values[1] = 3; u8 seen = *child;");
    assert_ne!(live, program);
    check(&live, Expected::LiveIndexedProjectionConflict);
    let dynamic = program.replace("*!u8 child = &!(*parent).values[1];",
        "u64 index = 1; *!u8 child = &!(*parent).values[index];");
    assert_ne!(dynamic, program);
    check(&dynamic, Expected::DynamicProjection);
    let outside = program.replace("&!(*parent).values[1]", "&!(*parent).values[2]");
    assert_ne!(outside, program);
    check(&outside, Expected::OutOfRangeProjection);

}

#[test]
fn joined_shared_field_permits_sibling_write_only_through_selected_parent() {
    let program = joined("parent = &!second;", true, "",
        "(*parent).other = 4; u8 observed = *child; u8 left = first.other; u8 right = second.other;")
        .replace("%%start\n", "%%start\nstruct copy Pair { u8 value; u8 other; }\n")
        .replace("u8 first = 7;\n    u8 second = 9;",
            "Pair first = { .value = 7; .other = 1; }; Pair second = { .value = 9; .other = 2; };")
        .replace("*!u8 parent = null;", "*!Pair parent = null;")
        .replace("&(*parent)", "&(*parent).value")
        .replace("    seen\n",
            "    if (observed == seen) { if (choose) { if (left == 4 && right == 2) { seen } else { 0 } } else { if (left == 1 && right == 4) { seen } else { 0 } } } else { 0 }\n");
    check_case(&program, Expected::Valid, Some("WOSY_JOINED_PARENT_SIBLING_IR_DIR"));

    let overlap = program.replace("(*parent).other = 4;", "(*parent).value = 4;");
    assert_ne!(overlap, program);
    check(&overlap, Expected::LiveParentProjectionConflict);
    let lexical = program.replace("(*parent).other = 4;", "first.other = 4;");
    assert_ne!(lexical, program);
    check(&lexical, Expected::LiveLexicalSiblingConflict);
}

#[test]
fn joined_exclusive_field_and_selected_parent_write_disjoint_fields() {
    let program = joined("parent = &!second;", true, "",
        "*child = seen + 2; (*parent).other = 4; u8 observed = *child; \
         u8 left = first.other; u8 right = second.other; u8 left_value = first.value; u8 right_value = second.value;")
        .replace("%%start\n", "%%start\nstruct copy Pair { u8 value; u8 other; }\n")
        .replace("u8 first = 7;\n    u8 second = 9;",
            "Pair first = { .value = 7; .other = 1; }; Pair second = { .value = 9; .other = 2; };")
        .replace("*!u8 parent = null;", "*!Pair parent = null;")
        .replace("*u8 child = &(*parent);", "*!u8 child = &!(*parent).value;")
        .replace("    seen\n",
            "    if (observed == seen + 2) { if (choose) { if (left == 4 && right == 2 && left_value == 9 && right_value == 9) { seen } else { 0 } } else { if (left == 1 && right == 4 && left_value == 7 && right_value == 11) { seen } else { 0 } } } else { 0 }\n");
    check_case(&program, Expected::Valid, Some("WOSY_JOINED_PARENT_EXCLUSIVE_SIBLING_IR_DIR"));

    for (target, operand) in [("(*parent).value = 4", "value"), ("first.other = 4", "other")] {
        let live = program.replace("(*parent).other = 4", target);
        assert_ne!(live, program);
        check(&live, Expected::ExactProjectionConflict { target, operand, loan: "&!(*parent).value" });
    }
}

#[test]
fn known_mutable_parent_grants_in_range_literal_array_element() {
    let program = r#"%%start
struct Record { u8[2] values; }
u8(bool) inspect = fn(choose) {
    Record first = { .values = [1, 7]; };
    *!Record parent = &!first;
    *!u8 child = &!(*parent).values[1];
    u8 seen = *child;
    *child = seen + 2;
    u8 observed = *child;
    if (observed == 9 && first.values[0] == 1 && first.values[1] == 9) {
        if (choose) { 7 } else { 9 }
    } else { 0 }
};
if (inspect(true) != 7 || inspect(false) != 9) { core.system_panic(); };
%%end"#;
    check_case(program, Expected::Valid, Some("WOSY_KNOWN_INDEXED_PROJECTION_IR_DIR"));

    let live = program.replace("u8 observed = *child;", "first.values[1] = 3; u8 observed = *child;");
    assert_ne!(live, program);
    check(&live, Expected::LiveIndexedProjectionConflict);

    let absent = program.replace("*!Record parent = &!first;", "*!Record parent = null;");
    assert_ne!(absent, program);
    check(&absent, Expected::KnownAbsentProjection);
}

#[test]
fn selected_parent_writes_disjoint_literal_array_element_while_child_lives() {
    let program = joined("parent = &!second;", true, "",
        "*child = seen + 2; (*parent).values[0] = 4; u8 observed = *child; \
         u8 left = first.values[0]; u8 right = second.values[0];")
        .replace("%%start\n", "%%start\nstruct Record { u8[2] values; }\n")
        .replace("u8 first = 7;\n    u8 second = 9;",
            "Record first = { .values = [1, 7]; }; Record second = { .values = [2, 9]; };")
        .replace("*!u8 parent = null;", "*!Record parent = null;")
        .replace("*u8 child = &(*parent);", "*!u8 child = &!(*parent).values[1];")
        .replace("    seen\n",
            "    if (observed == seen + 2) { if (choose) { if (left == 4 && right == 2) { seen } else { 0 } } else { if (left == 1 && right == 4) { seen } else { 0 } } } else { 0 }\n");
    check_case(&program, Expected::Valid, Some("WOSY_JOINED_PARENT_DISJOINT_INDEX_IR_DIR"));

    for (replacement, target, operand) in [
        ("(*parent).values[1] = 4", "(*parent).values[1] = 4", "values"),
        ("first.values[0] = 4", "first.values[0] = 4", "values"),
        ("u64 index = 0; (*parent).values[index] = 4", "(*parent).values[index] = 4", "index"),
    ] {
        let live = program.replace("(*parent).values[0] = 4", replacement);
        assert_ne!(live, program);
        check(&live, Expected::ExactProjectionConflict { target, operand, loan: "&!(*parent).values[1]" });
    }
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
fn joined_child_first_observed_in_nested_arms_stays_live_until_used() {
    let program = joined_with_pointer_read("first = 3;", "*value")
        .replace("u8 seen = *child;",
            "u8 seen = if (choose) { pointer(child) } else { *child };")
        .replace("    seen\n", "    if (first == 3) { seen } else { 0 }\n");
    check_case(&program, Expected::Valid, Some("WOSY_JOINED_FIRST_BRANCH_READ_IR_DIR"));

    let live = program.replace("u8 seen = if (choose) { pointer(child) } else { *child };\n    first = 3;",
        "first = 3;\n    u8 seen = if (choose) { pointer(child) } else { *child };");
    assert_ne!(live, program);
    check(&live, Expected::LiveConflict);
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

}

#[test]
fn joined_exclusive_descendant_transfers_scalar_write_until_last_use() {
    let program = joined("parent = &!second;", true, "",
        "*!u8 nested = &!(*child); *nested = seen + 2; u8 during = *nested; *child = seen + 4; \
         u8 changed = *child; u8 left = first; u8 right = second;")
        .replace("*u8 child = &(*parent);", "*!u8 child = &!(*parent);")
        .replace("    seen\n", "    if (choose) { if (during == 9 && changed == 11 && left == 11 && right == 9) { seen } else { 0 } } else { if (during == 11 && changed == 13 && left == 7 && right == 13) { seen } else { 0 } }\n");
    check_case(&program, Expected::Valid, Some("WOSY_JOINED_EXCLUSIVE_IR_DIR"));

    let live = program.replace("u8 during = *nested; *child = seen + 4;",
        "*child = seen + 4; u8 during = *nested;");
    assert_ne!(live, program);
    check(&live, Expected::LiveExclusiveConflict);

    let second = program.replace("*child = seen + 4; u8 changed = *child;",
        "*!u8 again = &!(*child); *again = seen + 4; u8 changed = *again;");
    assert_ne!(second, program);
    check(&second, Expected::Declined);
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
fn joined_child_is_read_by_exact_callee_with_scalar_prefix_argument() {
    let program = joined_with_pointer_read(
        "u8 bias = 2; u8 observed = pointer(bias, child); first = 3;", "*value",
    ).replace("u8(*u8) pointer = fn(value) { *value };",
        "u8(u8, *u8) pointer = fn(bias, value) { bias + *value };")
        .replace("    seen\n", "    if (observed == seen + bias && first == 3) { seen } else { 0 }\n");
    check_case(&program, Expected::Valid, Some("WOSY_JOINED_SCALAR_PREFIX_IR_DIR"));

    let live = program.replace("u8 observed = pointer(bias, child); first = 3;",
        "first = 3; u8 observed = pointer(bias, child);");
    assert_ne!(live, program);
    check(&live, Expected::LiveConflict);

    let uncertified = program.replace("bias + *value", "bias + *value + *value");
    assert_ne!(uncertified, program);
    check(&uncertified, Expected::NullParent);
}

#[test]
fn joined_child_returned_by_exact_identity_keeps_loan_live() {
    let program = joined("parent = &!second;", true, "",
        "*u8 echoed = identity(child); u8 observed = *echoed; first = 3;")
        .replace("%%start\n", "%%start\n*u8(*u8) identity = fn(value) { value };\n")
        .replace("    seen\n", "    if (observed == seen && first == 3) { observed } else { 0 }\n");
    check_case(&program, Expected::Valid, Some("WOSY_JOINED_RETURNED_ALIAS_IR_DIR"));

    let live = program.replace("u8 observed = *echoed; first = 3;",
        "first = 3; u8 observed = *echoed;");
    assert_ne!(live, program);
    check(&live, Expected::LiveConflict);

    let rebound = program.replace("u8 observed = *echoed;", "echoed = &second; u8 observed = *echoed;");
    assert_ne!(rebound, program);
    check(&rebound, Expected::NullParent);
}

#[test]
fn joined_child_returned_through_grounded_alias_chain_remains_exact() {
    let program = joined("parent = &!second;", true, "",
        "*u8 echoed = identity(child); u8 observed = *echoed; first = 3;")
        .replace("%%start\n", "%%start\n*u8(*u8) identity = fn(value) { middle(value) };\n\
            *u8(*u8) middle = fn(value) { terminal(value) };\n\
            *u8(*u8) terminal = fn(value) { value };\n")
        .replace("    seen\n", "    if (observed == seen && first == 3) { observed } else { 0 }\n");
    check_case(&program, Expected::Valid, Some("WOSY_JOINED_RETURNED_CHAIN_IR_DIR"));

    let live = program.replace("u8 observed = *echoed; first = 3;",
        "first = 3; u8 observed = *echoed;");
    assert_ne!(live, program);
    check(&live, Expected::LiveConflict);

    let cycle = program.replace("{ value };", "{ identity(value) };");
    assert_ne!(cycle, program);
    check(&cycle, Expected::NullParent);
}

#[test]
fn joined_child_crosses_grounded_forward_reader_chain() {
    let program = joined_with_pointer_read("u8 observed = pointer(child); first = 3;", "*value")
        .replace("u8(*u8) pointer = fn(value) { *value };",
            "u8(*u8) pointer = fn(value) { middle(value) };\n\
             u8(*u8) middle = fn(value) { terminal(value) };\n\
             u8(*u8) terminal = fn(value) { *value };")
        .replace("    seen\n", "    if (observed == seen && first == 3) { observed } else { 0 }\n");
    check_case(&program, Expected::Valid, Some("WOSY_JOINED_FORWARD_READER_IR_DIR"));

    let live = program.replace("u8 observed = pointer(child); first = 3;",
        "first = 3; u8 observed = pointer(child);");
    assert_ne!(live, program);
    check(&live, Expected::LiveConflict);

    let ungrounded = program.replace("{ *value };", "{ *value + *value };");
    assert_ne!(ungrounded, program);
    check(&ungrounded, Expected::NullParent);

    let cycle = program.replace("{ *value };", "{ pointer(value) };");
    assert_ne!(cycle, program);
    check(&cycle, Expected::NullParent);
}

#[test]
fn joined_child_is_read_by_bounded_pure_conditional_callee() {
    let body = "if (*value == 7) { *value + 1 } else { *value + 2 }";
    let program = joined_with_pointer_read("u8 observed = pointer(child); first = 3;", body)
        .replace("    seen\n", "    if (first == 3) { if (choose) { if (observed == 8) { seen } else { 0 } } else { if (observed == 11) { seen } else { 0 } } } else { 0 }\n");
    check_case(&program, Expected::Valid, Some("WOSY_JOINED_CONDITIONAL_READER_IR_DIR"));

    let live = program.replace("u8 observed = pointer(child); first = 3;",
        "first = 3; u8 observed = pointer(child);");
    assert_ne!(live, program);
    check(&live, Expected::LiveConflict);
    let opaque = program.replace("{ *value + 2 }", "{ u8 local = *value; local + 2 }");
    assert_ne!(opaque, program);
    check(&opaque, Expected::NullParent);
    // 31 shallow arithmetic nodes plus the condition and other arm exceed
    // the reader budget without producing a long linear expression.
    let pair = "(1 + 1)";
    let four = format!("({pair} + {pair})");
    let eight = format!("({four} + {four})");
    let above_cap = program.replace("*value + 2", &format!("({eight} + {eight})"));
    assert_ne!(above_cap, program);
    check(&above_cap, Expected::NullParent);
}

#[test]
fn selected_loan_reader_uses_exact_independent_boolean_formal() {
    let program = joined_with_pointer_read(
        "u8 low = pointer(true, child); u8 high = pointer(false, child); first = 3;", "*value",
    ).replace("u8(*u8) pointer = fn(value) { *value };",
        "u8(bool, *u8) pointer = fn(use_low, value) { if (use_low) { *value + 1 } else { *value + 2 } };")
        .replace("    seen\n",
            "    if (first == 3 && low == seen + 1 && high == seen + 2) { seen } else { 0 }\n");
    check_case(&program, Expected::Valid, Some("WOSY_JOINED_CONDITIONAL_BOOL_IR_DIR"));

    let live = program.replace("u8 high = pointer(false, child); first = 3;",
        "first = 3; u8 high = pointer(false, child);");
    assert_ne!(live, program);
    check(&live, Expected::LiveConflict);
    let uncertified = program.replace("{ *value + 2 }", "{ u8 local = *value; local + 2 }");
    assert_ne!(uncertified, program);
    check(&uncertified, Expected::NullParent);
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
fn returned_joined_alias_cannot_escape_its_lexical_owners() {
    let program = r#"%%start
*u8(*u8) identity = fn(value) { value };
*u8(bool) escape = fn(choose) {
    u8 first = 7;
    u8 second = 9;
    *!u8 parent = null;
    if (choose) { parent = &!first; } else { parent = &!second; };
    *u8 child = &(*parent);
    *u8 echoed = identity(child);
    echoed
};
%%end"#;
    check(program, Expected::NullParent);
    let wrapper = program.replace("*u8(*u8) identity = fn(value) { value };",
        "*u8(*u8) identity = fn(value) { terminal(value) };\n*u8(*u8) terminal = fn(value) { value };");
    assert_ne!(wrapper, program);
    check(&wrapper, Expected::NullParent);
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

#[test]
fn joined_reader_wrapper_is_grounded_across_exact_module_declarations() {
    let source = |path: &str| SourceIdentity::new("project".into(), "package".into(), path.into(), "r1".into());
    let app = source("src/main.w");
    let relay = source("src/relay.w");
    let terminal = source("src/terminal.w");
    let decoy = source("src/decoy.w");
    let certified = "%%start\nu8(*u8) read = fn(view) { *view };\n%%end";
    let uncertified = certified.replace("{ *view }", "{ *view + *view }");
    let wrapper = "%%start\nend_reader = namespace package \"src/terminal.w\";\n\
        u8(*u8) read = fn(view) { end_reader.read(view) };\n%%end";
    let program = joined("parent = &!second;", true, "",
        "u8 observed = relay.read(child); first = 3;")
        .replace("%%start", "%%start\nrelay = namespace package \"src/relay.w\";")
        .replace("    seen\n", "    if (observed == seen && first == 3) { observed } else { 0 }\n");
    let declarations = |source: &SourceIdentity, text: &str| {
        let parsed = parse_source(source.clone(), text.to_owned(), &[]);
        assert!(parsed.diagnostics.is_empty(), "{:?}", parsed.diagnostics);
        derive_scalar_declarations_from_cst(parsed.result.canonical_cst())
    };
    for (target, target_layout) in [("native64", ScalarTargetLayout::NATIVE64), ("wasm32", ScalarTargetLayout::WASM32)] {
        let prepare = |terminal_body: &str| {
            let root = declarations(&app, &program);
            let relay_module = declarations(&relay, wrapper);
            let ns = root.namespaces().next().unwrap();
            let root_bindings = vec![ScalarNamespaceBinding {
                binding: ns.binding.clone(), target: relay.clone(), span: ns.span,
            }];
            let ns = relay_module.namespaces().next().unwrap();
            let relay_bindings = vec![ScalarNamespaceBinding {
                binding: ns.binding.clone(), target: terminal.clone(), span: ns.span,
            }];
            prepare_scalar_project(vec![(root, root_bindings), (relay_module, relay_bindings),
                (declarations(&terminal, terminal_body), vec![]), (declarations(&decoy, certified), vec![])],
                vec![app.clone(), relay.clone(), terminal.clone(), decoy.clone()],
                ScalarPreparationInputs { target_layout }).unwrap()
        };
        let prepared = prepare(certified);
        let build = analyze_scalar_build(&prepared);
        assert!(build.diagnostics().is_empty(), "{target}: {:?}", build.diagnostics());
        let ir = emit_scalar_build_project_llvm(&build).expect("cross-module joined reader LLVM").to_text();
        if let Some(out) = std::env::var_os("WOSY_JOINED_MODULE_WRAPPER_IR_DIR") {
            std::fs::write(std::path::PathBuf::from(out).join(format!("{target}-project.ll")), ir).unwrap();
        }
        let prepared = prepare(&uncertified);
        let build = analyze_scalar_build(&prepared);
        let reborrow = program.find("&(*parent)").unwrap() as u32;
        assert!(build.diagnostics().iter().any(|d| d.code == "B0003"
            && d.message == "missing required dereference origin" && d.labels.first()
                .is_some_and(|label| label.span.range.start == reborrow)), "{target}: {:?}", build.diagnostics());
    }
}

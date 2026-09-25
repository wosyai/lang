use std::collections::BTreeMap;

use inkwell::context::Context;

use super::emit_scalar_llvm;
use super::emit_scalar_project_llvm;
use super::function_type;
use super::project_function_name;
use super::project_global_name;
use super::storage_type;
use super::{LlvmFunction, LlvmFunctionAttributes, LlvmPartition, LlvmValueType};
use crate::scalar::{CfgPointKind, CoreOperationId, ReleaseTarget, ScalarOverloadSelection};
use crate::{
    derive_scalar_program, derive_scalar_program_with_layout, parse_source,
    validate_scalar_project, ScalarModule, ScalarOutput, ScalarOutputSequence, ScalarProject,
    ScalarTargetLayout, ScalarType,
};
use wosy_syntax::{ByteSpan, SourceIdentity};

#[test]
fn checked_aggregate_reads_materialize_sized_values_on_both_targets_and_pipelines() {
    let source = SourceIdentity::new(
        "project".into(),
        "package".into(),
        "src/main.w".into(),
        "r1".into(),
    );
    let text = "%%start\nstruct copy Point { u8 first; u32 second; }\n*Point(*Point) forward = fn(value) { value };\nu32(*Point) read = fn(value) { u32 field = (*value).second; Point copied_point = *value; copied_point.second + field };\nu8(*(u8[2])) read_array = fn(value) { u8[2] copied_bytes = *value; u8 item = copied_bytes[0]; copied_bytes[1] + item };\nPoint sample = { .first = 3; .second = 8; };\n*Point checked = &sample;\nu32 result = read(forward(checked));\nu8[2] bytes = [4, 5];\n*(u8[2]) array = &bytes;\nu8 array_result = read_array(array);\n%%end";
    for (layout, pointer_width) in [
        (ScalarTargetLayout::WASM32, "i32"),
        (ScalarTargetLayout::NATIVE64, "i64"),
    ] {
        let parsed = parse_source(source.clone(), text.to_owned(), &[]);
        let single = derive_scalar_program_with_layout(&parsed.result, layout);
        assert!(single.diagnostics.is_empty(), "{:?}", single.diagnostics);
        let project = validate_scalar_project(ScalarProject::new(
            vec![ScalarModule::from_program(
                single.program.clone(),
                Vec::new(),
            )],
            vec![source.clone()],
        ));
        assert!(project.diagnostics.is_empty(), "{:?}", project.diagnostics);
        for llvm in [
            emit_scalar_llvm(&single).expect("single LLVM").to_text(),
            emit_scalar_project_llvm(&project)
                .expect("project LLVM")
                .to_text(),
        ] {
            assert!(llvm.contains("load [8 x i8], ptr %deref"), "{llvm}");
            assert!(llvm.contains("load [2 x i8], ptr %deref"), "{llvm}");
            assert!(
                llvm.contains("getelementptr inbounds i8, ptr %deref, i8 4"),
                "{llvm}"
            );
            assert!(
                llvm.contains(&format!("inttoptr {pointer_width}")),
                "{llvm}"
            );
            assert!(
                llvm.contains("call void @__wosy_core_system_panic"),
                "{llvm}"
            );
            assert!(!llvm.contains("alloca ptr, align 1"), "{llvm}");
        }
    }
}

#[test]
fn checked_aggregate_return_reads_in_caller_on_both_targets_and_pipelines() {
    let source = SourceIdentity::new(
        "project".into(),
        "package".into(),
        "src/main.w".into(),
        "r1".into(),
    );
    let text = "%%start\nstruct copy Point { u8 first; u32 second; }\n*Point() make = fn { Point local = { .first = 3; .second = 8; }; &local };\n*(u8[2])() make_array = fn { u8[2] local = [4, 5]; &local };\nu32() read = fn { *Point result = make(); u32 field = (*result).second; Point whole = *result; whole.second + field };\nu8() read_array = fn { *(u8[2]) result = make_array(); u8[2] whole = *result; whole[0] + whole[1] };\n%%end";
    for layout in [ScalarTargetLayout::WASM32, ScalarTargetLayout::NATIVE64] {
        let parsed = parse_source(source.clone(), text.to_owned(), &[]);
        let single = derive_scalar_program_with_layout(&parsed.result, layout);
        assert!(single.diagnostics.is_empty(), "{:?}", single.diagnostics);
        let project = validate_scalar_project(ScalarProject::new(
            vec![ScalarModule::from_program(
                single.program.clone(),
                Vec::new(),
            )],
            vec![source.clone()],
        ));
        assert!(project.diagnostics.is_empty(), "{:?}", project.diagnostics);
        for llvm in [
            emit_scalar_llvm(&single).expect("single LLVM").to_text(),
            emit_scalar_project_llvm(&project)
                .expect("project LLVM")
                .to_text(),
        ] {
            assert!(llvm.contains("load [8 x i8], ptr %deref"), "{llvm}");
            assert!(llvm.contains("load [2 x i8], ptr %deref"), "{llvm}");
            assert!(
                llvm.contains("%return_result_value = load [2 x i8]"),
                "{llvm}"
            );
        }
    }
}

#[test]
fn validated_return_origins_materialize_local_pointees_and_forward_existing_owners() {
    let source = SourceIdentity::new(
        "project".into(),
        "package".into(),
        "src/main.w".into(),
        "r1".into(),
    );
    let text = "%%start\nstruct copy Point { u8 value; }\nstruct Payload { u8 value; }\n*u8(u8) scalar = fn(v) { unsafe { &v } };\n*Point() make_point = fn { Point local = { .value = 3; }; &local };\n*(u8[2])() make_array = fn { u8[2] local = [4, 5]; &local };\n*!Payload() make_payload = fn { Payload local = { .value = 6; }; &!local };\n*!(Payload[2])() make_payloads = fn { Payload[2] local = [{ .value = 7; }, { .value = 8; }]; &!local };\n*Point() forward = fn { make_point() };\n(*u8, *Point)() pair = fn { u8 first = 9; Point second = { .value = 10; }; &first, &second };\n*u8(*u8) borrow = fn(v) { v };\n*u8() empty = fn { null };\nunit() caller = fn { *Point held = forward(); Point read = *held; read.value; };\n%%end";
    for layout in [ScalarTargetLayout::WASM32, ScalarTargetLayout::NATIVE64] {
        let parsed = parse_source(source.clone(), text.to_owned(), &[]);
        let single = derive_scalar_program_with_layout(&parsed.result, layout);
        assert!(single.diagnostics.is_empty(), "{:?}", single.diagnostics);
        let project = validate_scalar_project(ScalarProject::new(
            vec![ScalarModule::from_program(
                single.program.clone(),
                Vec::new(),
            )],
            vec![source.clone()],
        ));
        assert!(project.diagnostics.is_empty(), "{:?}", project.diagnostics);
        for llvm in [
            emit_scalar_llvm(&single).expect("single LLVM").to_text(),
            emit_scalar_project_llvm(&project)
                .expect("project LLVM")
                .to_text(),
        ] {
            assert!(llvm.contains("return_result_value = load i8"), "{llvm}");
            assert!(
                llvm.contains("return_result_value = load [1 x i8]"),
                "{llvm}"
            );
            assert!(
                llvm.contains("return_result_value = load [2 x i8]"),
                "{llvm}"
            );
            assert!(
                llvm.contains("return_result_value = load [2 x [1 x i8]]"),
                "{llvm}"
            );
            assert_eq!(
                llvm.matches("load [1 x i8], ptr %return_result_source")
                    .count(),
                3,
                "{llvm}"
            );
            assert_eq!(
                llvm.matches("call void @__wosy_core_free(ptr %return_result_release)")
                    .count(),
                1,
                "{llvm}"
            );
        }
    }
}

#[test]
fn invalid_checked_return_escape_is_diagnosed_before_llvm_on_both_layouts() {
    let source = SourceIdentity::new(
        "project".into(),
        "package".into(),
        "src/main.w".into(),
        "r1".into(),
    );
    let text = "%%start\nstruct Payload { u8 value; }\n*Payload() escape = fn { Payload local = { .value = 1; }; *Payload held = &local; Payload moved = local; moved; held };\n%%end";
    for layout in [ScalarTargetLayout::WASM32, ScalarTargetLayout::NATIVE64] {
        let parsed = parse_source(source.clone(), text.to_owned(), &[]);
        let single = derive_scalar_program_with_layout(&parsed.result, layout);
        let project = validate_scalar_project(ScalarProject::new(
            vec![ScalarModule::from_program(
                single.program.clone(),
                Vec::new(),
            )],
            vec![source.clone()],
        ));
        for diagnostics in [&single.diagnostics, &project.diagnostics] {
            assert!(
                diagnostics.iter().any(|diagnostic| {
                    diagnostic.code == "B0003"
                        && diagnostic.labels[0].span.range.start
                            == text.rfind("held };").unwrap() as u32
                        && diagnostic.labels.iter().any(|label| {
                            label.span.range.start == text.find("&local").unwrap() as u32
                        })
                }),
                "{diagnostics:?}"
            );
        }
    }
}

#[test]
fn fresh_or_null_checked_return_releases_only_live_caller_allocation() {
    let source = SourceIdentity::new(
        "project".into(),
        "package".into(),
        "src/main.w".into(),
        "r1".into(),
    );
    let text = "%%start\nstruct copy Point { u8 value; }\n*Point(bool) choose = fn(flag) { Point sample = { .value = 1; }; if (flag) { &sample } else { null } };\nunit() caller = fn { *Point result = choose(true); if (result != null) { Point value = *result; value.value; }; };\n%%end";
    for layout in [ScalarTargetLayout::WASM32, ScalarTargetLayout::NATIVE64] {
        let parsed = parse_source(source.clone(), text.to_owned(), &[]);
        let single = derive_scalar_program_with_layout(&parsed.result, layout);
        assert!(single.diagnostics.is_empty(), "{:?}", single.diagnostics);
        let project = validate_scalar_project(ScalarProject::new(
            vec![ScalarModule::from_program(
                single.program.clone(),
                Vec::new(),
            )],
            vec![source.clone()],
        ));
        assert!(project.diagnostics.is_empty(), "{:?}", project.diagnostics);
        for llvm in [
            emit_scalar_llvm(&single).expect("single LLVM").to_text(),
            emit_scalar_project_llvm(&project)
                .expect("project LLVM")
                .to_text(),
        ] {
            assert!(
                llvm.contains("return_result_release_live = icmp ne"),
                "{llvm}"
            );
            assert!(llvm.contains("return_result_release.next"), "{llvm}");
        }
    }
}

#[test]
fn mixed_fresh_and_borrowed_return_materializes_only_fresh_branch() {
    let source = SourceIdentity::new(
        "project".into(),
        "package".into(),
        "src/main.w".into(),
        "r1".into(),
    );
    let text = "%%start\nstruct copy Point { u8 value; }\n*Point(*Point, bool) choose = fn(input, flag) { Point local = { .value = 1; }; if (flag) { &local } else { input } };\n*Point(*Point, bool) forward = fn(input, flag) { choose(input, flag) };\nunit() caller = fn { Point outside = { .value = 2; }; *Point borrowed = &outside; *Point fresh = forward(borrowed, true); Point first = *fresh; *Point same = forward(borrowed, false); Point second = *same; first.value; second.value; };\n%%end";
    for layout in [ScalarTargetLayout::WASM32, ScalarTargetLayout::NATIVE64] {
        let parsed = parse_source(source.clone(), text.to_owned(), &[]);
        let single = derive_scalar_program_with_layout(&parsed.result, layout);
        assert!(single.diagnostics.is_empty(), "{:?}", single.diagnostics);
        let project = validate_scalar_project(ScalarProject::new(
            vec![ScalarModule::from_program(
                single.program.clone(),
                Vec::new(),
            )],
            vec![source.clone()],
        ));
        assert!(project.diagnostics.is_empty(), "{:?}", project.diagnostics);
        for llvm in [
            emit_scalar_llvm(&single).expect("single LLVM").to_text(),
            emit_scalar_project_llvm(&project)
                .expect("project LLVM")
                .to_text(),
        ] {
            assert_eq!(
                llvm.matches("return_result_value = load [1 x i8]").count(),
                1,
                "{llvm}"
            );
            assert!(llvm.contains("return_owner_0 = phi i1"), "{llvm}");
            assert!(llvm.contains("checked_return_owned_0 = load i1"), "{llvm}");
            assert!(llvm.contains("return_result_owned = and i1"), "{llvm}");
        }
    }
}

#[test]
fn mixed_multi_output_checked_returns_transfer_each_owner_independently() {
    let source = SourceIdentity::new(
        "project".into(),
        "package".into(),
        "src/main.w".into(),
        "r1".into(),
    );
    let text = "%%start\nstruct copy Point { u8 value; }\n(*Point, *Point)(*Point, *Point, bool, bool) choose = fn(left, right, flag, nullable) { Point local_left = { .value = 11; }; Point local_right = { .value = 22; }; if (flag) { &local_left, right } else { if (nullable) { left, null } else { left, &local_right } } };\n(*Point, *Point)(*Point, *Point, bool, bool) forward = fn(left, right, flag, nullable) { *Point from_left, *Point from_right = choose(left, right, flag, nullable); from_left, from_right };\nunit() caller = fn { Point base_left = { .value = 3; }; Point base_right = { .value = 4; }; *Point first_left, *Point first_right = forward(&base_left, &base_right, true, false); Point sample_one = *first_left; Point sample_two = *first_right; *Point second_left, *Point second_right = forward(&base_left, &base_right, false, false); Point sample_three = *second_left; Point sample_four = *second_right; *Point third_left, *Point third_right = forward(&base_left, &base_right, false, true); Point sample_five = *third_left; if (sample_one.value != 11 || sample_two.value != 4 || sample_three.value != 3 || sample_four.value != 22 || sample_five.value != 3 || third_right != null || base_left.value != 3 || base_right.value != 4) { core.system_panic(); }; };\ncaller();\n%%end";
    for layout in [ScalarTargetLayout::WASM32, ScalarTargetLayout::NATIVE64] {
        let parsed = parse_source(source.clone(), text.to_owned(), &[]);
        let single = derive_scalar_program_with_layout(&parsed.result, layout);
        assert!(single.diagnostics.is_empty(), "{:?}", single.diagnostics);
        let project = validate_scalar_project(ScalarProject::new(
            vec![ScalarModule::from_program(
                single.program.clone(),
                Vec::new(),
            )],
            vec![source.clone()],
        ));
        assert!(project.diagnostics.is_empty(), "{:?}", project.diagnostics);
        for llvm in [
            emit_scalar_llvm(&single).expect("single LLVM").to_text(),
            emit_scalar_project_llvm(&project)
                .expect("project LLVM")
                .to_text(),
        ] {
            for position in 0..2 {
                assert!(
                    llvm.contains(&format!("return_owner_{position} = phi i1")),
                    "{llvm}"
                );
                assert!(
                    llvm.contains(&format!("checked_return_owner_{position}")),
                    "{llvm}"
                );
                assert!(
                    llvm.contains(&format!("checked_return_owned_{position} = load i1")),
                    "{llvm}"
                );
            }
            assert_eq!(
                llvm.matches(" = and i1 %return_result_release_live")
                    .count(),
                6,
                "{llvm}"
            );
            assert_eq!(
                llvm.matches("call void @__wosy_core_free(ptr %return_result_release")
                    .count(),
                6,
                "{llvm}"
            );
        }
    }
}

#[test]
fn joined_local_and_forwarded_checked_assignment_returns_by_runtime_owner() {
    let source = SourceIdentity::new(
        "project".into(),
        "package".into(),
        "src/main.w".into(),
        "r1".into(),
    );
    let text = "%%start\nstruct copy Point { u8 value; }\n*Point(bool) allocate = fn(flag) { Point local = { .value = 22; }; if (flag) { &local } else { null } };\n*Point(bool, bool) choose = fn(local, external) { *Point result = null; if (local) { Point owned = { .value = 11; }; result = &owned; }; if (external) { result = allocate(true); }; result };\n*Point(*Point, bool, bool) choose_borrow = fn(input, local, borrowed) { *Point result = null; if (local) { Point owned = { .value = 44; }; result = &owned; }; if (borrowed) { result = input; }; result };\nunit() caller = fn { *Point first = choose(true, false); Point one = *first; *Point second = choose(false, true); Point two = *second; *Point absent = choose(false, false); Point outside = { .value = 33; }; *Point local = choose_borrow(&outside, true, false); Point three = *local; *Point borrowed = choose_borrow(&outside, false, true); Point four = *borrowed; *Point replaced = choose_borrow(&outside, true, true); Point five = *replaced; *Point empty = choose_borrow(&outside, false, false); if (one.value != 11 || two.value != 22 || absent != null || three.value != 44 || four.value != 33 || five.value != 33 || empty != null || outside.value != 33) { core.system_panic(); }; };\ncaller();\n%%end";
    for layout in [ScalarTargetLayout::WASM32, ScalarTargetLayout::NATIVE64] {
        let parsed = parse_source(source.clone(), text.to_owned(), &[]);
        let single = derive_scalar_program_with_layout(&parsed.result, layout);
        assert!(single.diagnostics.is_empty(), "{:?}", single.diagnostics);
        let project = validate_scalar_project(ScalarProject::new(
            vec![ScalarModule::from_program(
                single.program.clone(),
                Vec::new(),
            )],
            vec![source.clone()],
        ));
        assert!(project.diagnostics.is_empty(), "{:?}", project.diagnostics);
        for llvm in [
            emit_scalar_llvm(&single).expect("single LLVM").to_text(),
            emit_scalar_project_llvm(&project)
                .expect("project LLVM")
                .to_text(),
        ] {
            assert!(llvm.contains("return_result_owned"), "{llvm}");
            assert!(llvm.contains("checked_return_owned_0"), "{llvm}");
        }
    }
}

#[test]
fn move_only_checked_aggregate_is_consumed_as_one_sized_value() {
    let source = SourceIdentity::new(
        "project".into(),
        "package".into(),
        "src/main.w".into(),
        "r1".into(),
    );
    let text = "%%start\nstruct Payload { u8 value; }\nunit(Payload) consume = fn(item) { *!Payload writer = &!item; u8 prior = (*writer).value; Payload taken = *writer; prior; taken; };\nunit(Payload[2]) consume_array = fn(items) { *!(Payload[2]) writer = &!items; Payload[2] taken = *writer; taken; };\n%%end";
    for layout in [ScalarTargetLayout::WASM32, ScalarTargetLayout::NATIVE64] {
        let parsed = parse_source(source.clone(), text.to_owned(), &[]);
        let single = derive_scalar_program_with_layout(&parsed.result, layout);
        assert!(single.diagnostics.is_empty(), "{:?}", single.diagnostics);
        let project = validate_scalar_project(ScalarProject::new(
            vec![ScalarModule::from_program(
                single.program.clone(),
                Vec::new(),
            )],
            vec![source.clone()],
        ));
        assert!(project.diagnostics.is_empty(), "{:?}", project.diagnostics);
        for llvm in [
            emit_scalar_llvm(&single).expect("single LLVM").to_text(),
            emit_scalar_project_llvm(&project)
                .expect("project LLVM")
                .to_text(),
        ] {
            assert!(llvm.contains("load [1 x i8], ptr %deref"), "{llvm}");
            assert!(llvm.contains("load [2 x [1 x i8]], ptr %deref"), "{llvm}");
            assert!(
                !llvm.contains("call void @__wosy_core_free(ptr %deref"),
                "{llvm}"
            );
        }
    }
}

#[test]
fn checked_core_free_releases_backing_once_and_preserves_raw_abi_on_both_targets_and_pipelines() {
    let source = SourceIdentity::new(
        "project".into(),
        "package".into(),
        "src/main.w".into(),
        "r1".into(),
    );
    let text = "%%start\nunit(u64) checked = fn(count) { u8[count] bytes; *u8 reader = &bytes[0]; reader; unsafe { core.free(&bytes); } };\nunit() raw = fn { unsafe { *?u8 pointer = core.alloc(1, 1); core.free(pointer); core.free(core.alloc(1, 1)); } };\n%%end";
    for (layout, width) in [
        (ScalarTargetLayout::WASM32, "i32"),
        (ScalarTargetLayout::NATIVE64, "i64"),
    ] {
        let parsed = parse_source(source.clone(), text.to_owned(), &[]);
        let single = derive_scalar_program_with_layout(&parsed.result, layout);
        assert!(single.diagnostics.is_empty(), "{:?}", single.diagnostics);
        let project = validate_scalar_project(ScalarProject::new(
            vec![ScalarModule::from_program(
                single.program.clone(),
                Vec::new(),
            )],
            vec![source.clone()],
        ));
        assert!(project.diagnostics.is_empty(), "{:?}", project.diagnostics);
        for (llvm, checked_name, raw_name) in [
            (
                emit_scalar_llvm(&single).expect("single LLVM").to_text(),
                "checked".to_owned(),
                "raw".to_owned(),
            ),
            (
                emit_scalar_project_llvm(&project)
                    .expect("project LLVM")
                    .to_text(),
                project_function_name(&source, "checked"),
                project_function_name(&source, "raw"),
            ),
        ] {
            let checked = &llvm[llvm.find(&format!("@{checked_name}(")).unwrap()..];
            let checked = &checked[..checked.find("\n}").unwrap()];
            assert!(
                checked.contains("%checked_release_backing = load ptr, ptr %bytes_backing"),
                "{llvm}"
            );
            assert!(
                checked.contains("call void @__wosy_core_free(ptr %checked_release_backing)"),
                "{llvm}"
            );
            assert!(
                checked.contains("store i1 false, ptr %bytes_live"),
                "{llvm}"
            );
            assert!(
                checked
                    .contains("br i1 %allocation_live, label %release.live, label %release.next"),
                "{llvm}"
            );
            assert!(!checked.contains("@core.free"), "{llvm}");
            let raw = &llvm[llvm.find(&format!("@{raw_name}(")).unwrap()..];
            let raw = &raw[..raw.find("\n}").unwrap()];
            assert!(raw.contains(&format!("inttoptr {width}")), "{llvm}");
            assert_eq!(
                raw.matches("call void @__wosy_core_free(ptr %free_pointer")
                    .count(),
                2,
                "{llvm}"
            );
        }
    }
}

#[test]
fn checked_core_free_selects_joined_owner_without_freeing_other_allocation() {
    let source = SourceIdentity::new(
        "project".into(),
        "package".into(),
        "src/main.w".into(),
        "r1".into(),
    );
    let text = "%%start\nunit(u64, bool) release = fn(count, flag) { u8[count] first; u8[count] second; *u8 alias = &first[0]; if (flag) { alias = &second[0]; }; unsafe { core.free(alias); } };\n%%end";
    for layout in [ScalarTargetLayout::WASM32, ScalarTargetLayout::NATIVE64] {
        let parsed = parse_source(source.clone(), text.to_owned(), &[]);
        let single = derive_scalar_program_with_layout(&parsed.result, layout);
        assert!(single.diagnostics.is_empty(), "{:?}", single.diagnostics);
        let project = validate_scalar_project(ScalarProject::new(
            vec![ScalarModule::from_program(
                single.program.clone(),
                Vec::new(),
            )],
            vec![source.clone()],
        ));
        assert!(project.diagnostics.is_empty(), "{:?}", project.diagnostics);
        for llvm in [
            emit_scalar_llvm(&single).expect("single LLVM").to_text(),
            emit_scalar_project_llvm(&project)
                .expect("project LLVM")
                .to_text(),
        ] {
            assert_eq!(
                llvm.lines()
                    .filter(|line| line.contains("core_effect_target") && line.contains("icmp eq"))
                    .count(),
                2,
                "{llvm}"
            );
            assert!(llvm.contains("load ptr, ptr %first_backing"), "{llvm}");
            assert!(llvm.contains("load ptr, ptr %second_backing"), "{llvm}");
            assert_eq!(
                llvm.matches("call void @__wosy_core_free(ptr %checked_release_backing")
                    .count(),
                2,
                "{llvm}"
            );
            assert_eq!(
                llvm.matches("br i1 %checked_release_live").count(),
                2,
                "{llvm}"
            );
            assert_eq!(
                llvm.matches("call void @__wosy_core_free(ptr %runtime_array_owner")
                    .count(),
                2,
                "{llvm}"
            );
        }
    }
}

#[test]
fn joined_projections_of_one_owner_apply_each_core_effect_once() {
    let source = SourceIdentity::new(
        "project".into(),
        "package".into(),
        "src/main.w".into(),
        "r1".into(),
    );
    let text = "%%start\nunit(u64, bool) release = fn(count, flag) { u8[count] bytes; *u8 alias = &bytes[0]; if (flag) { alias = &bytes[1]; }; unsafe { core.free(alias); } };\nunit(u64, bool, *?u8) relocate = fn(count, flag, raw) { u8[count] bytes; *u8 alias = &bytes[0]; if (flag) { alias = &bytes[1]; }; unsafe { core.rebind(alias, raw, count); } };\nunit(u64, bool) discard = fn(count, flag) { u8[count] bytes; *u8 alias = &bytes[0]; if (flag) { alias = &bytes[1]; }; unsafe { core.invalidate(alias); } };\n%%end";
    for layout in [ScalarTargetLayout::WASM32, ScalarTargetLayout::NATIVE64] {
        let parsed = parse_source(source.clone(), text.to_owned(), &[]);
        let single = derive_scalar_program_with_layout(&parsed.result, layout);
        assert!(single.diagnostics.is_empty(), "{:?}", single.diagnostics);
        for (item, operation) in single.program.items.iter().zip([
            CoreOperationId::Free,
            CoreOperationId::Rebind,
            CoreOperationId::Invalidate,
        ]) {
            let crate::ScalarItem::Function(function) = item else {
                panic!("core effect function")
            };
            let candidates = function
                .reference_cfg
                .points
                .iter()
                .find_map(|point| match &point.kind {
                    CfgPointKind::Release {
                        target: ReleaseTarget::Checked { candidates, .. },
                    } if operation == CoreOperationId::Free => Some(candidates),
                    CfgPointKind::Invalidate {
                        candidates,
                        operation: actual,
                        ..
                    } if *actual == operation => Some(candidates),
                    _ => None,
                })
                .expect("typed core effect candidates");
            assert_eq!(candidates.len(), 2);
            assert_eq!(candidates[0].place.binding, candidates[1].place.binding);
            assert_ne!(
                candidates[0].place.projections,
                candidates[1].place.projections
            );
        }
        let project = validate_scalar_project(ScalarProject::new(
            vec![ScalarModule::from_program(
                single.program.clone(),
                Vec::new(),
            )],
            vec![source.clone()],
        ));
        assert!(project.diagnostics.is_empty(), "{:?}", project.diagnostics);
        for (llvm, names) in [
            (
                emit_scalar_llvm(&single).expect("single LLVM").to_text(),
                [
                    "release".to_owned(),
                    "relocate".to_owned(),
                    "discard".to_owned(),
                ],
            ),
            (
                emit_scalar_project_llvm(&project)
                    .expect("project LLVM")
                    .to_text(),
                [
                    project_function_name(&source, "release"),
                    project_function_name(&source, "relocate"),
                    project_function_name(&source, "discard"),
                ],
            ),
        ] {
            let bodies = names.map(|name| {
                let body = &llvm[llvm.find(&format!("@{name}(")).unwrap()..];
                &body[..body.find("\n}").unwrap()]
            });
            assert_eq!(
                bodies[0]
                    .matches("call void @__wosy_core_free(ptr %checked_release_backing")
                    .count(),
                1,
                "{llvm}"
            );
            assert_eq!(
                bodies[0].matches("store i1 false, ptr %bytes_live").count(),
                1,
                "{llvm}"
            );
            assert_eq!(
                bodies[1]
                    .matches("store ptr %rebound_address, ptr %bytes_backing")
                    .count(),
                1,
                "{llvm}"
            );
            assert_eq!(
                bodies[1]
                    .lines()
                    .filter(|line| line.contains("store i64") && line.contains("ptr %bytes_length"))
                    .count(),
                2,
                "{llvm}"
            );
            assert_eq!(
                bodies[2].matches("store i1 false, ptr %bytes_live").count(),
                1,
                "{llvm}"
            );
            for body in bodies {
                assert!(!body.contains("%core_effect_target"), "{llvm}");
            }
        }
    }
}

#[test]
fn emits_typed_invalidation_and_rebind_effects_in_both_pipelines_and_targets() {
    let source = SourceIdentity::new(
        "project".into(),
        "package".into(),
        "src/main.w".into(),
        "r1".into(),
    );
    let text = "%%start\nunit(u64) discard = fn(count) { u8[count] bytes; bytes[0] = 3; unsafe { core.invalidate(&bytes); } };\nu8(u64, *?u8, u64) replace = fn(count, raw, next) { u8[count] bytes; unsafe { core.rebind(&bytes, raw, next); }; bytes[0] = 9; bytes[0] };\n%%end";
    for (layout, width) in [
        (ScalarTargetLayout::WASM32, "i32"),
        (ScalarTargetLayout::NATIVE64, "i64"),
    ] {
        let parsed = parse_source(source.clone(), text.to_owned(), &[]);
        let single = derive_scalar_program_with_layout(&parsed.result, layout);
        assert!(single.diagnostics.is_empty(), "{:?}", single.diagnostics);
        let project = validate_scalar_project(ScalarProject::new(
            vec![ScalarModule::from_program(
                single.program.clone(),
                Vec::new(),
            )],
            vec![source.clone()],
        ));
        assert!(project.diagnostics.is_empty(), "{:?}", project.diagnostics);
        for (llvm, discard_name, replace_name) in [
            (
                emit_scalar_llvm(&single).expect("single LLVM").to_text(),
                "discard".to_owned(),
                "replace".to_owned(),
            ),
            (
                emit_scalar_project_llvm(&project)
                    .expect("project LLVM")
                    .to_text(),
                project_function_name(&source, "discard"),
                project_function_name(&source, "replace"),
            ),
        ] {
            let discard = &llvm[llvm.find(&format!("@{discard_name}(")).unwrap()..];
            let discard = &discard[..discard.find("\n}").unwrap()];
            assert!(
                discard.contains("store i1 false, ptr %bytes_live"),
                "{llvm}"
            );
            assert!(
                discard
                    .contains("br i1 %allocation_live, label %release.live, label %release.next"),
                "{llvm}"
            );
            assert!(
                discard.find("store i1 false, ptr %bytes_live").unwrap()
                    < discard.find("br i1 %allocation_live").unwrap(),
                "{llvm}"
            );
            assert!(!discard.contains("@core.invalidate"), "{llvm}");
            let replace = &llvm[llvm.find(&format!("@{replace_name}(")).unwrap()..];
            let replace = &replace[..replace.find("\n}").unwrap()];
            assert!(replace.contains("%bytes_backing = alloca ptr"), "{llvm}");
            assert!(replace.contains("%bytes_length = alloca i64"), "{llvm}");
            assert!(replace.contains(&format!("inttoptr {width}")), "{llvm}");
            assert!(
                replace.contains("store ptr %rebound_address, ptr %bytes_backing"),
                "{llvm}"
            );
            assert!(
                replace.contains("store i64 %next6, ptr %bytes_length"),
                "{llvm}"
            );
            assert!(replace.contains("load ptr, ptr %bytes_backing"), "{llvm}");
            assert!(!replace.contains("@core.rebind"), "{llvm}");
        }
    }
}

#[test]
fn invalidation_of_joined_alias_selects_exact_runtime_owner_on_both_targets() {
    let source = SourceIdentity::new(
        "project".into(),
        "package".into(),
        "src/main.w".into(),
        "r1".into(),
    );
    let text = "%%start\nunit(u64, bool) discard_one = fn(count, flag) { u8[count] first; u8[count] second; *u8 alias = &first[0]; if (flag) { alias = &second[0]; }; unsafe { core.invalidate(alias); } };\n%%end";
    for layout in [ScalarTargetLayout::WASM32, ScalarTargetLayout::NATIVE64] {
        let parsed = parse_source(source.clone(), text.to_owned(), &[]);
        let single = derive_scalar_program_with_layout(&parsed.result, layout);
        assert!(single.diagnostics.is_empty(), "{:?}", single.diagnostics);
        let project = validate_scalar_project(ScalarProject::new(
            vec![ScalarModule::from_program(
                single.program.clone(),
                Vec::new(),
            )],
            vec![source.clone()],
        ));
        assert!(project.diagnostics.is_empty(), "{:?}", project.diagnostics);
        for llvm in [
            emit_scalar_llvm(&single).expect("single LLVM").to_text(),
            emit_scalar_project_llvm(&project)
                .expect("project LLVM")
                .to_text(),
        ] {
            assert_eq!(
                llvm.lines()
                    .filter(|line| line.contains("core_effect_target") && line.contains("icmp eq"))
                    .count(),
                2,
                "{llvm}"
            );
            assert!(llvm.contains("%first_live = alloca i1"), "{llvm}");
            assert!(llvm.contains("%second_live = alloca i1"), "{llvm}");
            assert_eq!(
                llvm.matches("store i1 false, ptr %first_live").count(),
                1,
                "{llvm}"
            );
            assert_eq!(
                llvm.matches("store i1 false, ptr %second_live").count(),
                1,
                "{llvm}"
            );
            assert_eq!(
                llvm.lines()
                    .filter(|line| line.contains("br i1 %allocation_live"))
                    .count(),
                2,
                "{llvm}"
            );
        }
    }
}

#[test]
fn rejects_fixed_array_lengths_that_exceed_llvm_array_size() {
    let context = Context::create();
    let array = ScalarType::Array {
        element: Box::new(ScalarType::U8),
        length: u64::from(u32::MAX) + 1,
        length_span: ByteSpan::new(0, 1),
        span: ByteSpan::new(0, 1),
    };
    assert_eq!(
        storage_type(&context, &array, &[], ScalarTargetLayout::NATIVE64).unwrap_err(),
        "fixed array length exceeds LLVM array size"
    );
}

#[test]
fn accepts_fixed_array_lengths_at_llvm_array_size_limit() {
    let context = Context::create();
    let array = ScalarType::Array {
        element: Box::new(ScalarType::U8),
        length: u64::from(u32::MAX),
        length_span: ByteSpan::new(0, 1),
        span: ByteSpan::new(0, 1),
    };
    assert!(storage_type(&context, &array, &[], ScalarTargetLayout::NATIVE64).is_ok());
}

#[test]
fn lowers_nested_project_while_loops_with_runtime_array_operations_and_conditionals() {
    let source = SourceIdentity::new(
        "project".into(),
        "package".into(),
        "src/main.w".into(),
        "r1".into(),
    );
    let scalar = derive_scalar_program(
            &parse_source(
                source.clone(),
                "%%start\ni32(u64) count = fn(length) { u8[length] bytes; i32 outer = 0; while (outer < 1) { i32 index = 0; while (index < 1) { if (index == 0) { bytes[0] = 1; }; index = index + 1; } outer = outer + 1; } outer };\n%%end".into(),
                &[],
            )
            .result,
        );
    assert!(scalar.diagnostics.is_empty(), "{:?}", scalar.diagnostics);
    let program = scalar.program;
    let validation = validate_scalar_project(ScalarProject::new(
        vec![ScalarModule::new(source.clone(), program.items, Vec::new())],
        vec![source],
    ));
    assert!(
        validation.diagnostics.is_empty(),
        "{:?}",
        validation.diagnostics
    );

    let llvm = emit_scalar_project_llvm(&validation)
        .expect("project loop LLVM")
        .to_text();
    assert!(llvm.contains("while.cond.0:"), "{llvm}");
    assert!(llvm.contains("while.cond.3:"), "{llvm}");
    assert!(llvm.contains("getelementptr inbounds i8"), "{llvm}");
}

#[test]
fn lowers_integer_literals_in_binary_expressions_to_the_left_operand_width() {
    let validation = derive_scalar_program(
        &parse_source(
            SourceIdentity::new(
                "project".into(),
                "package".into(),
                "src/main.w".into(),
                "r1".into(),
            ),
            "%%start\nbool() equal = fn { u8 value = 10; value == 10 };\n%%end".into(),
            &[],
        )
        .result,
    );
    assert!(
        validation.diagnostics.is_empty(),
        "{:?}",
        validation.diagnostics
    );

    let llvm = emit_scalar_llvm(&validation)
        .expect("integer comparison LLVM")
        .to_text();
    assert!(llvm.contains("icmp eq i8"), "{llvm}");
}

#[test]
fn emits_multiple_unit_conditionals_in_a_while_body() {
    let source = SourceIdentity::new(
        "project".into(),
        "package".into(),
        "src/main.w".into(),
        "r1".into(),
    );
    let validation = derive_scalar_program(
            &parse_source(
                source,
                "%%start\n(i32, bool)() run = fn { bool running = true; if (true) { while (running) { if (running) { running = false; } else { running = false; }; if (running) { running = running; } else { running = running; }; } running = false; 1, true } else { 2, false } };\n%%end".into(),
                &[],
            )
            .result,
        );
    assert!(
        validation.diagnostics.is_empty(),
        "{:?}",
        validation.diagnostics
    );

    let llvm = emit_scalar_llvm(&validation)
        .expect("multiple loop unit conditional LLVM")
        .to_text();
    assert_eq!(
        llvm.lines()
            .filter(|line| line.starts_with("while.cond.") && line.contains(':'))
            .count(),
        1,
        "{llvm}"
    );
}

#[test]
fn emits_project_multiple_unit_conditionals_in_a_while_body_once() {
    let source = SourceIdentity::new(
        "project".into(),
        "package".into(),
        "src/main.w".into(),
        "r1".into(),
    );
    let scalar = derive_scalar_program(
            &parse_source(
                source.clone(),
                "%%start\n(i32, bool)() run = fn { bool running = true; if (true) { while (running) { if (running) { running = false; } else { running = false; }; if (running) { running = running; } else { running = running; }; } running = false; 1, true } else { 2, false } };\n%%end".into(),
                &[],
            )
            .result,
        );
    assert!(scalar.diagnostics.is_empty(), "{:?}", scalar.diagnostics);
    let program = scalar.program;
    let validation = validate_scalar_project(ScalarProject::new(
        vec![ScalarModule::new(source.clone(), program.items, Vec::new())],
        vec![source],
    ));
    assert!(
        validation.diagnostics.is_empty(),
        "{:?}",
        validation.diagnostics
    );

    let llvm = emit_scalar_project_llvm(&validation)
        .expect("project multiple loop unit conditional LLVM")
        .to_text();
    assert_eq!(
        llvm.lines()
            .filter(|line| line.starts_with("while.cond.") && line.contains(':'))
            .count(),
        1,
        "{llvm}"
    );
}

#[test]
fn emits_recursive_fixed_array_index_reads_and_addresses_for_both_targets() {
    let source = SourceIdentity::new(
        "project".into(),
        "package".into(),
        "src/main.w".into(),
        "r1".into(),
    );
    let text = "%%start\nu8() read = fn { u8[2][2] matrix = [[1, 2], [3, 4]]; u64 row = 1; u64 column = 0; u8 value = matrix[row][column]; *u8 shared = &matrix[row][column]; shared; unsafe { *?u8 raw = &?matrix[row][column]; raw; }; value };\n%%end";
    for layout in [ScalarTargetLayout::WASM32, ScalarTargetLayout::NATIVE64] {
        let parsed = parse_source(source.clone(), text.to_owned(), &[]);
        let validation = crate::derive_scalar_program_with_layout(&parsed.result, layout);
        assert!(
            validation.diagnostics.is_empty(),
            "{:?}",
            validation.diagnostics
        );
        let llvm = emit_scalar_llvm(&validation)
            .expect("indexed LLVM")
            .to_text();
        assert!(
            llvm.contains("getelementptr inbounds [2 x [2 x i8]]"),
            "{llvm}"
        );
        assert!(llvm.contains("getelementptr inbounds [2 x i8]"), "{llvm}");
        assert!(llvm.contains("ptrtoint"), "{llvm}");
        assert!(!llvm.contains("icmp ult"), "{llvm}");
    }
}

#[test]
fn emits_runtime_array_allocation_and_indexing_for_both_targets() {
    let source = SourceIdentity::new(
        "project".into(),
        "package".into(),
        "src/main.w".into(),
        "r1".into(),
    );
    let text =
        "%%start\nu8(u64) read = fn(length) { u8[length] bytes; bytes[0] = 7; bytes[0] };\n%%end";
    for layout in [ScalarTargetLayout::WASM32, ScalarTargetLayout::NATIVE64] {
        let parsed = parse_source(source.clone(), text.to_owned(), &[]);
        let validation = crate::derive_scalar_program_with_layout(&parsed.result, layout);
        assert!(
            validation.diagnostics.is_empty(),
            "{:?}",
            validation.diagnostics
        );
        let project = validate_scalar_project(ScalarProject::new(
            vec![ScalarModule::from_program(
                validation.program.clone(),
                Vec::new(),
            )],
            vec![source.clone()],
        ));
        assert!(project.diagnostics.is_empty(), "{:?}", project.diagnostics);
        for llvm in [
            emit_scalar_llvm(&validation)
                .expect("runtime array LLVM")
                .to_text(),
            emit_scalar_project_llvm(&project)
                .expect("project runtime array LLVM")
                .to_text(),
        ] {
            assert!(
                llvm.contains("declare ptr @__wosy_core_alloc(i64, i64)"),
                "{llvm}"
            );
            assert!(
                llvm.contains("call ptr @__wosy_core_alloc(i64 %allocation_size, i64 1)"),
                "{llvm}"
            );
            assert!(llvm.contains("icmp eq ptr %allocation, null"), "{llvm}");
            assert!(
                llvm.contains("call void @__wosy_core_system_panic()\n  unreachable"),
                "{llvm}"
            );
            assert!(llvm.contains("getelementptr inbounds i8"), "{llvm}");
            assert!(
                llvm.contains("getelementptr inbounds i8, ptr %allocation, i64 0"),
                "{llvm}"
            );
            assert!(!llvm.contains("%bytes_backing"), "{llvm}");
            assert!(!llvm.contains("icmp ult"), "{llvm}");
        }
    }
}

#[test]
fn emits_one_release_for_a_moved_top_level_runtime_array_for_both_targets() {
    let source = SourceIdentity::new(
        "project".into(),
        "package".into(),
        "src/main.w".into(),
        "r1".into(),
    );
    let text = "%%start\nu64 length = 1;\nu8[length] first;\nu8[] second = first;\nsecond[0] = 7;\nu8 value = second[0];\n%%end";
    for layout in [ScalarTargetLayout::WASM32, ScalarTargetLayout::NATIVE64] {
        let parsed = parse_source(source.clone(), text.to_owned(), &[]);
        let validation = crate::derive_scalar_program_with_layout(&parsed.result, layout);
        assert!(
            validation.diagnostics.is_empty(),
            "{:?}",
            validation.diagnostics
        );
        let llvm = emit_scalar_llvm(&validation)
            .expect("moved runtime array LLVM")
            .to_text();
        assert!(llvm.contains("call ptr @__wosy_core_alloc"), "{llvm}");
        assert_eq!(
            llvm.matches("call void @__wosy_core_free").count(),
            1,
            "{llvm}"
        );
    }
}

#[test]
fn transfers_runtime_array_ownership_through_a_returned_aggregate_pointer() {
    let source = SourceIdentity::new(
        "project".into(),
        "package".into(),
        "src/main.w".into(),
        "r1".into(),
    );
    let text = "%%start\nstruct Packet { *?u8 data; u64 length; }\n*Packet() make_packet = fn { u64 length = 1; u8[length] bytes; bytes[0] = 4; *?u8 data = null; unsafe { data = &?bytes[0]; }; Packet result = { .data = data; .length = length; }; &result };\nunit() release = fn { u64 length = 1; u8[length] temporary; temporary[0] = 9; };\n%%end";
    for layout in [ScalarTargetLayout::WASM32, ScalarTargetLayout::NATIVE64] {
        let parsed = parse_source(source.clone(), text.to_owned(), &[]);
        let validation = crate::derive_scalar_program_with_layout(&parsed.result, layout);
        assert!(
            validation.diagnostics.is_empty(),
            "{:?}",
            validation.diagnostics
        );
        let llvm = emit_scalar_llvm(&validation)
            .expect("returned aggregate pointer LLVM")
            .to_text();
        let packet = &llvm[llvm.find("@make_packet(").expect("packet function")..];
        let packet = &packet[..packet.find("\n}").expect("packet function end")];
        assert!(!packet.contains("call void @__wosy_core_free"), "{llvm}");
        assert_eq!(
            llvm.matches("call void @__wosy_core_free").count(),
            1,
            "{llvm}"
        );
    }
}

#[test]
fn emits_shared_core_allocation_release_and_panic_abi_for_both_targets() {
    let source = SourceIdentity::new(
        "project".into(),
        "package".into(),
        "src/main.w".into(),
        "r1".into(),
    );
    let text = "%%start\nunit() release = fn { unsafe { *?u8 storage = core.alloc(16, 8); core.free(storage); }; };\nunit() panic = fn { core.system_panic(); };\ni64(u64) allocate = fn(length) { i64[length] values; values[0] = 7; values[0] };\n%%end";
    for (layout, pointer_width) in [
        (ScalarTargetLayout::WASM32, 32),
        (ScalarTargetLayout::NATIVE64, 64),
    ] {
        let parsed = parse_source(source.clone(), text.to_owned(), &[]);
        let validation = crate::derive_scalar_program_with_layout(&parsed.result, layout);
        assert!(
            validation.diagnostics.is_empty(),
            "{:?}",
            validation.diagnostics
        );
        let llvm = emit_scalar_llvm(&validation)
            .expect("core allocation LLVM")
            .to_text();
        assert!(
            llvm.contains("declare ptr @__wosy_core_alloc(i64, i64)"),
            "{llvm}"
        );
        assert!(
            llvm.contains("declare void @__wosy_core_free(ptr)"),
            "{llvm}"
        );
        assert!(
            llvm.contains("declare void @__wosy_core_system_panic()"),
            "{llvm}"
        );
        assert!(
            llvm.contains("call ptr @__wosy_core_alloc(i64 16, i64 8)"),
            "{llvm}"
        );
        assert!(
            llvm.contains(&format!("ptrtoint ptr %allocation to i{pointer_width}")),
            "{llvm}"
        );
        assert!(
            llvm.contains(&format!("inttoptr i{pointer_width}")),
            "{llvm}"
        );
        assert!(
            llvm.contains("call void @__wosy_core_free(ptr %free_pointer)"),
            "{llvm}"
        );
        assert!(
            llvm.contains("call ptr @__wosy_core_alloc(i64 %allocation_size, i64 8)"),
            "{llvm}"
        );
        assert!(llvm.contains("allocation_panic:"), "{llvm}");
        assert!(llvm.contains("unreachable"), "{llvm}");
        assert!(llvm.contains("define void @panic()"), "{llvm}");
    }
}

#[test]
fn retains_core_runtime_declarations_when_serializing_wasm_externs() {
    let source = SourceIdentity::new(
        "project".into(),
        "package".into(),
        "src/main.w".into(),
        "r1".into(),
    );
    let parsed = parse_source(
            source,
            "%%start\nenv = extern wasm \"host\" { unit() log; };\nunit() run = fn { u64 length = 1; u8[length] bytes; bytes[0] = 1; env.log(); };\nrun();\n%%end".into(),
            &[],
        );
    let validation =
        crate::derive_scalar_program_with_layout(&parsed.result, ScalarTargetLayout::WASM32);
    assert!(
        validation.diagnostics.is_empty(),
        "{:?}",
        validation.diagnostics
    );

    let llvm = emit_scalar_llvm(&validation)
        .expect("core runtime LLVM with wasm extern")
        .to_text();
    assert!(
        llvm.contains("declare ptr @__wosy_core_alloc(i64, i64)"),
        "{llvm}"
    );
    assert!(
        llvm.contains("declare void @__wosy_core_system_panic()"),
        "{llvm}"
    );
    assert!(llvm.contains("wasm-import-module\"=\"host\""), "{llvm}");
}

#[test]
fn emits_mutable_typed_place_writes_for_wasm32_and_native64() {
    let source = SourceIdentity::new(
        "project".into(),
        "package".into(),
        "src/main.w".into(),
        "r1".into(),
    );
    let text = "%%start\nstruct Buffer { u8 value; u8[3] bytes; }\nu8() write = fn { u8[3] values = [1, 2, 3]; Buffer holder = { .value = 4; .bytes = [5, 6, 7]; }; *!u8 writer = &!values[1]; values[0] = 8; holder.value = 9; *writer = 10; holder.bytes[2] = 11; values[0], values[2] = 12, 13; values[0] };\n%%end";
    for layout in [ScalarTargetLayout::WASM32, ScalarTargetLayout::NATIVE64] {
        let parsed = parse_source(source.clone(), text.to_owned(), &[]);
        let validation = crate::derive_scalar_program_with_layout(&parsed.result, layout);
        assert!(
            validation.diagnostics.is_empty(),
            "{:?}",
            validation.diagnostics
        );
        let llvm = emit_scalar_llvm(&validation)
            .expect("mutable place LLVM")
            .to_text();
        assert!(llvm.contains("getelementptr inbounds [3 x i8]"), "{llvm}");
        assert!(llvm.contains("inttoptr"), "{llvm}");
        assert!(!llvm.contains("icmp ult"), "{llvm}");
    }
}

#[test]
fn serializes_each_extern_with_its_wasm_import_attribute_group() {
    let partition = LlvmPartition {
            module_name: "test".into(),
            declarations: vec![
                LlvmFunction {
                    name: "first".into(),
                    result: LlvmValueType::I32,
                    parameters: Vec::new(),
                    attributes: vec![LlvmFunctionAttributes {
                        group: 3,
                        wasm_import_module: "one".into(),
                        wasm_import_name: "first_import".into(),
                    }],
                    body: String::new(),
                },
                LlvmFunction {
                    name: "second".into(),
                    result: LlvmValueType::Void,
                    parameters: Vec::new(),
                    attributes: vec![LlvmFunctionAttributes {
                        group: 7,
                        wasm_import_module: "two".into(),
                        wasm_import_name: "second_import".into(),
                    }],
                    body: String::new(),
                },
            ],
            functions: Vec::new(),
            text: "; ModuleID = 'test'\nsource_filename = \"test\"\n\ndeclare i32 @first()\ndeclare void @second()\n".into(),
        };

    let text = partition.to_text();

    assert!(text.contains("declare i32 @first() #3"), "{text}");
    assert!(text.contains("declare void @second() #7"), "{text}");
    assert!(
            text.contains(
                "attributes #3 = { \"wasm-import-module\"=\"one\" \"wasm-import-name\"=\"first_import\" }"
            ),
            "{text}"
        );
    assert!(
            text.contains(
                "attributes #7 = { \"wasm-import-module\"=\"two\" \"wasm-import-name\"=\"second_import\" }"
            ),
            "{text}"
        );
}

#[test]
fn preserves_callable_return_metadata_for_every_output_form() {
    let context = Context::create();
    let sequence = |types: Vec<ScalarType>| ScalarOutputSequence {
        outputs: types
            .into_iter()
            .map(|ty| ScalarOutput {
                ty,
                span: wosy_syntax::ByteSpan::new(0, 0),
            })
            .collect(),
        span: wosy_syntax::ByteSpan::new(0, 0),
    };
    let callable = |outputs| ScalarType::Callable {
        outputs,
        parameters: Vec::new(),
    };

    assert_eq!(
        function_type(
            &context,
            &callable(sequence(Vec::new())),
            ScalarTargetLayout::WASM32,
        )
        .expect("zero-output function type")
        .1,
        LlvmValueType::Void
    );
    assert_eq!(
        function_type(
            &context,
            &callable(sequence(vec![ScalarType::I32])),
            ScalarTargetLayout::WASM32,
        )
        .expect("direct function type")
        .1,
        LlvmValueType::I32
    );
    assert_eq!(
        function_type(
            &context,
            &callable(sequence(vec![ScalarType::I32, ScalarType::Bool])),
            ScalarTargetLayout::WASM32,
        )
        .expect("aggregate function type")
        .1,
        LlvmValueType::Aggregate(vec![LlvmValueType::I32, LlvmValueType::I1])
    );
    assert_eq!(
        function_type(
            &context,
            &callable(sequence(vec![ScalarType::Unit, ScalarType::I32])),
            ScalarTargetLayout::WASM32,
        )
        .expect("unit aggregate function type")
        .1,
        LlvmValueType::Aggregate(vec![LlvmValueType::I8, LlvmValueType::I32])
    );
}

#[test]
fn collects_function_and_declaration_aggregate_metadata() {
    let source = SourceIdentity::new(
        "project".into(),
        "package".into(),
        "src/main.w".into(),
        "r1".into(),
    );
    let validation = derive_scalar_program(
            &parse_source(
                source,
                "%%start\nenv = extern wasm \"metadata\" { unit() void_result; i32() direct_result; (i32, bool)() aggregate_result; (unit, i32)() unit_aggregate_result; };\nunit() local_void = fn { env.void_result() };\ni32() local_direct = fn { env.direct_result() };\n(i32, bool)() local_aggregate = fn { env.aggregate_result() };\n(unit, i32)() local_unit_aggregate = fn { env.unit_aggregate_result() };\n%%end".into(),
                &[],
            )
            .result,
        );
    assert!(
        validation.diagnostics.is_empty(),
        "{:?}",
        validation.diagnostics
    );

    let partition = emit_scalar_llvm(&validation).expect("LLVM metadata");
    let function_results = partition
        .functions
        .iter()
        .filter(|function| function.name.starts_with("local_"))
        .map(|function| function.result.clone())
        .collect::<Vec<_>>();
    let declaration_results = partition
        .declarations
        .iter()
        .map(|declaration| declaration.result.clone())
        .collect::<Vec<_>>();
    assert_eq!(
        function_results,
        vec![
            LlvmValueType::Aggregate(vec![LlvmValueType::I32, LlvmValueType::I1]),
            LlvmValueType::I32,
            LlvmValueType::Aggregate(vec![LlvmValueType::I8, LlvmValueType::I32]),
            LlvmValueType::Void,
        ]
    );
    assert_eq!(
        declaration_results,
        vec![
            LlvmValueType::Void,
            LlvmValueType::I32,
            LlvmValueType::Aggregate(vec![LlvmValueType::I32, LlvmValueType::I1]),
            LlvmValueType::Aggregate(vec![LlvmValueType::I8, LlvmValueType::I32]),
        ]
    );
}

#[test]
fn emits_typed_scalar_calls_and_verified_module() {
    let source = SourceIdentity::new(
        "project".into(),
        "package".into(),
        "src/main.w".into(),
        "r1".into(),
    );
    let parsed = parse_source(
            source,
            "%%start\nbool() flag = fn { true };\nunit() touch = fn { i32 marker = 0; marker = marker; };\ni32() count = fn { 42 };\nbool selected = flag();\nunit done = touch();\ni32 result = count();\n%%end"
                .into(),
            &[],
        );
    let partition = emit_scalar_llvm(&derive_scalar_program(&parsed.result)).expect("LLVM");
    let text = partition.to_text();
    assert!(text.contains("call i1 @flag"));
    assert!(text.contains("call void @touch"));
    assert!(text.contains("call i32 @count"));
}

#[test]
fn emits_concrete_ordinary_generic_direct_call() {
    let source = SourceIdentity::new(
        "project".into(),
        "package".into(),
        "src/main.w".into(),
        "r1".into(),
    );
    let validation = derive_scalar_program(
            &parse_source(
                source.clone(),
                "%%start\ngeneric T;\nT(T) identity = fn(value) { value };\ni64 result = identity<i64>(1);\n%%end"
                    .into(),
                &[],
            )
            .result,
        );
    assert!(
        validation.diagnostics.is_empty(),
        "{:?}",
        validation.diagnostics
    );

    let text = emit_scalar_llvm(&validation)
        .expect("ordinary generic LLVM")
        .to_text();

    assert!(text.contains("define i64 @wosy_generic__"), "{text}");
    assert!(text.contains("call i64 @wosy_generic__"), "{text}");

    let project = validate_scalar_project(ScalarProject::new(
        vec![ScalarModule::from_program(
            validation.program.clone(),
            Vec::new(),
        )],
        vec![source],
    ));
    let project_text = emit_scalar_project_llvm(&project)
        .expect("project ordinary generic LLVM")
        .to_text();
    assert!(
        project_text.contains("call i64 @wosy_generic__"),
        "{project_text}"
    );
}

#[test]
fn emits_concrete_ordinary_generic_multi_output_call() {
    let source = SourceIdentity::new(
        "project".into(),
        "package".into(),
        "src/main.w".into(),
        "r1".into(),
    );
    let validation = derive_scalar_program(
            &parse_source(
                source,
                "%%start\ngeneric T;\n(T, T)(T) pair = fn(value) { value };\ni64 first, i64 second = pair<i64>(1);\n%%end"
                    .into(),
                &[],
            )
            .result,
        );
    assert!(
        validation.diagnostics.is_empty(),
        "{:?}",
        validation.diagnostics
    );

    let text = emit_scalar_llvm(&validation)
        .expect("ordinary generic multi-output LLVM")
        .to_text();

    assert!(
        text.contains("define { i64, i64 } @wosy_generic__"),
        "{text}"
    );
    assert!(text.contains("call { i64, i64 } @wosy_generic__"), "{text}");
    assert!(text.contains("extractvalue { i64, i64 }"), "{text}");
}

#[test]
fn lowers_ordinary_and_generic_overload_arms_with_ordered_outputs() {
    let source = SourceIdentity::new(
        "project".into(),
        "package".into(),
        "src/main.w".into(),
        "r1".into(),
    );
    let validation = derive_scalar_program(
            &parse_source(
                source.clone(),
                "%%start\ncast = overload {\n    i32(i64) => fn(value) { 1 };\n    generic T;\n    T(T) => fn(value) { value };\n};\npair = overload {\n    generic T;\n    (T, T)(T) => fn(value) { value };\n};\ni64 wide = 1;\ni32 narrow = cast(wide);\nbool flag = true;\nbool copied = cast(flag);\ni64 first, i64 second = pair(wide);\n%%end"
                    .into(),
                &[],
            )
            .result,
        );
    assert!(
        validation.diagnostics.is_empty(),
        "{:?}",
        validation.diagnostics
    );

    let text = emit_scalar_llvm(&validation)
        .expect("overload LLVM")
        .to_text();

    assert!(text.contains("define i32 @wosy_generic__"), "{text}");
    assert!(text.contains("define i1 @wosy_generic__"), "{text}");
    assert!(
        text.contains("define { i64, i64 } @wosy_generic__"),
        "{text}"
    );
    assert!(text.contains("call i32 @wosy_generic__"), "{text}");
    assert!(text.contains("call i1 @wosy_generic__"), "{text}");
    assert!(text.contains("call { i64, i64 } @wosy_generic__"), "{text}");
    assert!(
        text.contains("extractvalue { i64, i64 } %call3, 0"),
        "{text}"
    );
    assert!(
        text.contains("extractvalue { i64, i64 } %call3, 1"),
        "{text}"
    );

    let project = validate_scalar_project(ScalarProject::new(
        vec![ScalarModule::from_program(
            validation.program.clone(),
            Vec::new(),
        )],
        vec![source],
    ));
    let project_text = emit_scalar_project_llvm(&project)
        .expect("project overload LLVM")
        .to_text();
    assert!(
        project_text.contains("call { i64, i64 } @wosy_generic__"),
        "{project_text}"
    );
}

#[test]
fn emits_distinct_overload_arms_for_shared_input_types() {
    let source = SourceIdentity::new(
        "project".into(),
        "package".into(),
        "src/main.w".into(),
        "r1".into(),
    );
    let validation = derive_scalar_program(
            &parse_source(
                source,
                "%%start\nselect = overload {\n    (i32, bool)(i64) => fn(value) { 1, true };\n    (bool, i32)(i64) => fn(value) { true, 1 };\n};\ni64 input = 1;\ni32 integer, bool flag = select(input);\nbool boolean, i32 number = select(input);\n%%end".into(),
                &[],
            )
            .result,
        );
    assert!(
        validation.diagnostics.is_empty(),
        "{:?}",
        validation.diagnostics
    );

    let text = emit_scalar_llvm(&validation)
        .expect("selected overload LLVM")
        .to_text();
    assert!(
        text.contains("define { i32, i1 } @wosy_generic__"),
        "{text}"
    );
    assert!(
        text.contains("define { i1, i32 } @wosy_generic__"),
        "{text}"
    );
    assert!(text.contains("call { i32, i1 } @wosy_generic__"), "{text}");
    assert!(text.contains("call { i1, i32 } @wosy_generic__"), "{text}");
}

#[test]
fn lowers_overload_arms_through_project_namespaces() {
    let child_source = SourceIdentity::new(
        "project".into(),
        "package".into(),
        "src/child.w".into(),
        "r1".into(),
    );
    let child = derive_scalar_program(
        &parse_source(
            child_source.clone(),
            "%%start\nidentity = overload {\n    i64(i64) => fn(value) { value };\n};\n%%end"
                .into(),
            &[],
        )
        .result,
    )
    .program;
    let root_source = SourceIdentity::new(
        "project".into(),
        "package".into(),
        "src/main.w".into(),
        "r1".into(),
    );
    let root = derive_scalar_program(
            &parse_source(
                root_source.clone(),
                "%%start\nchild = namespace package \"src/child.w\";\ni64 value = 1;\ni64 copied = child.identity(value);\n%%end".into(),
                &[],
            )
            .result,
        )
        .program;
    let namespace_bindings = root
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
    let validation = validate_scalar_project(ScalarProject::new(
        vec![
            ScalarModule::new(root_source.clone(), root.items, namespace_bindings),
            ScalarModule::new(child_source.clone(), child.items, Vec::new()),
        ],
        vec![root_source, child_source],
    ));
    assert!(
        validation.diagnostics.is_empty(),
        "{:?}",
        validation.diagnostics
    );

    let text = emit_scalar_project_llvm(&validation)
        .expect("namespaced overload LLVM")
        .to_text();
    assert!(text.contains("define i64 @wosy_generic__"), "{text}");
    assert!(text.contains("call i64 @wosy_generic__"), "{text}");
}

#[test]
fn lowers_generic_overload_arms_through_project_namespaces() {
    let child_source = SourceIdentity::new(
        "project".into(),
        "package".into(),
        "src/child.w".into(),
        "r1".into(),
    );
    let child = derive_scalar_program(
            &parse_source(
                child_source.clone(),
                "%%start\nidentity = overload {\n    i32(i64) => fn(value) { 1 };\n    generic T;\n    T(T) => fn(value) { value };\n};\nbool initial = true;\nbool copied = identity(initial);\n%%end"
                    .into(),
                &[],
            )
            .result,
        )
        .program;
    let root_source = SourceIdentity::new(
        "project".into(),
        "package".into(),
        "src/main.w".into(),
        "r1".into(),
    );
    let mut root = derive_scalar_program(
            &parse_source(
                root_source.clone(),
                "%%start\nchild = namespace package \"src/child.w\";\nchar value = 'a';\nchar copied = child.identity(value);\n%%end".into(),
                &[],
            )
            .result,
        )
        .program;
    let crate::ScalarItem::Binding(binding) = root
        .items
        .last_mut()
        .expect("qualified generic overload binding")
    else {
        panic!("qualified generic overload binding");
    };
    let crate::ScalarExpression::Call {
        overload_selection, ..
    } = &mut binding.value
    else {
        panic!("qualified generic overload call");
    };
    *overload_selection = Some(ScalarOverloadSelection {
        arm_index: 1,
        substitutions: BTreeMap::from([("T".into(), crate::ScalarType::Char)]),
    });
    let namespace_bindings = root
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
    let validation = crate::ScalarProjectValidation {
        project: ScalarProject::new(
            vec![
                ScalarModule::new(root_source.clone(), root.items, namespace_bindings),
                ScalarModule::new(child_source.clone(), child.items, Vec::new()),
            ],
            vec![root_source, child_source],
        ),
        diagnostics: Vec::new(),
    };

    let text = emit_scalar_project_llvm(&validation)
        .expect("namespaced generic overload LLVM")
        .to_text();
    assert!(text.contains("define i32 @wosy_generic__"), "{text}");
    assert!(text.contains("call i32 @wosy_generic__"), "{text}");
}

#[test]
fn emits_empty_utf8_call_arguments_with_resolved_parameter_types() {
    let source = SourceIdentity::new(
        "project".into(),
        "package".into(),
        "src/main.w".into(),
        "r1".into(),
    );
    let mut program = derive_scalar_program(
        &parse_source(
            source.clone(),
            "%%start
struct utf8 {
    *?u8 data;
    u64 length;
}
u64(utf8) print = fn(text) { text.length };
utf8 value = { .data = null; .length = 0; };
u64 reported = print(value);
%%end"
                .into(),
            &[],
        )
        .result,
    );
    assert!(program.diagnostics.is_empty(), "{:?}", program.diagnostics);
    let crate::ScalarItem::Binding(binding) = &mut program.program.items[2] else {
        panic!("call binding");
    };
    let crate::ScalarExpression::Call { arguments, .. } = &mut binding.value else {
        panic!("call expression");
    };
    arguments[0] = crate::ScalarExpression::Utf8 {
        value: Vec::new(),
        span: wosy_syntax::ByteSpan::new(0, 0),
    };
    let text = emit_scalar_llvm(&program)
        .expect("empty UTF-8 call LLVM")
        .to_text();

    assert!(
        text.contains("@wosy_utf8_literal_0 = private constant [0 x i8]"),
        "{text}"
    );
    assert!(text.contains("store i64 0"), "{text}");
    assert!(
        text.contains("call i64 @print(ptr %utf8_literal)"),
        "{text}"
    );
}

#[test]
fn emits_project_public_empty_utf8_output_call() {
    let std_source = SourceIdentity::new(
        "project".into(),
        "stdlib".into(),
        "src/bootstrap.w".into(),
        "r1".into(),
    );
    let std = derive_scalar_program(
        &parse_source(
            std_source.clone(),
            "%%start
struct utf8 {
    *?u8 data;
    u64 length;
}
(u64, bool)(utf8) print = fn(text) { text.length, true };
%%end"
                .into(),
            &[],
        )
        .result,
    )
    .program;
    let source = SourceIdentity::new(
        "project".into(),
        "package".into(),
        "src/main.w".into(),
        "r1".into(),
    );
    let root = derive_scalar_program(
        &parse_source(
            source.clone(),
            "%%start
text = namespace stdlib \"src/bootstrap.w\";
u64 reported, bool complete = text.print(\"\");
%%end"
                .into(),
            &[],
        )
        .result,
    )
    .program;
    let validation = validate_scalar_project(ScalarProject::new(
        vec![
            ScalarModule::from_program(
                root,
                vec![crate::ScalarNamespaceBinding {
                    binding: "text".into(),
                    target: std_source.clone(),
                    span: wosy_syntax::ByteSpan::new(0, 0),
                }],
            ),
            ScalarModule::from_program(std, Vec::new()),
        ],
        vec![source, std_source.clone()],
    ));
    assert!(
        validation.diagnostics.is_empty(),
        "{:?}",
        validation.diagnostics
    );
    let text = emit_scalar_project_llvm(&validation)
        .expect("project public empty UTF-8 output LLVM")
        .to_text();

    assert!(
        text.contains("@wosy_utf8_literal_0 = private constant [0 x i8]"),
        "{text}"
    );
    assert!(text.contains("store i64 0"), "{text}");
    assert!(
        text.contains(&format!(
            "call {{ i64, i1 }} @{}(ptr %utf8_literal)",
            project_function_name(&std_source, "print")
        )),
        "{text}"
    );
}

#[test]
fn emits_wasi_read_into_with_one_iovec_for_wasm32_and_native64() {
    let source = SourceIdentity::new(
        "project".into(),
        "package".into(),
        "src/read_into.w".into(),
        "r1".into(),
    );
    let text = "%%start
struct _Iovec { *?u8 data; u32 length; }
struct _Nread { u32 value; }
_wasi = extern wasm \"wasi_snapshot_preview1\" { unsafe i32(i32, *?_Iovec, i32, *?_Nread) _fd_read; };
i32(i32, *?_Iovec, *?_Nread) _fd_read_once = fn(descriptor, iovec_address, count_address) { unsafe { _wasi._fd_read(descriptor, iovec_address, 1, count_address) } };
(u64, bool)(*?u8, u64) read_into = fn(destination, capacity) {
	_Iovec entry = { .data = destination; .length = core.int_trunc<u32>(capacity); };
	_Nread count_cell = { .value = 0; };
	i32 result = 0;
	unsafe { result = _fd_read_once(0, &?entry, &?count_cell); };
	core.int_extend<u64>(count_cell.value), result == 0
};
*?u8 buffer = null;
u64 requested_capacity = 4;
u64 count = 0;
bool complete = false;
count, complete = read_into(buffer, requested_capacity);
%%end";
    for (layout, iovec_size) in [
        (ScalarTargetLayout::WASM32, 8),
        (ScalarTargetLayout::NATIVE64, 16),
    ] {
        let validation = derive_scalar_program_with_layout(
            &parse_source(source.clone(), text.into(), &[]).result,
            layout,
        );
        assert!(
            validation.diagnostics.is_empty(),
            "{:?}",
            validation.diagnostics
        );
        let llvm = emit_scalar_llvm(&validation)
            .expect("read_into LLVM")
            .to_text();
        let pointer_width = layout.pointer_size * 8;
        assert!(llvm.contains("wasm-import-name\"=\"fd_read"), "{llvm}");
        assert!(
            llvm.contains(&format!("i32 0, i{pointer_width} %address")),
            "{llvm}"
        );
        assert!(
            llvm.contains(&format!("i32 1, i{pointer_width} %count_address")),
            "{llvm}"
        );
        assert!(
            llvm.contains(&format!("alloca [{iovec_size} x i8]")),
            "{llvm}"
        );
        assert!(llvm.contains("trunc i64"), "{llvm}");
        assert!(llvm.contains("zext i32"), "{llvm}");
        assert!(!llvm.contains("icmp eq i32 %destination"), "{llvm}");
    }
}

#[test]
fn emits_generic_integer_conversions_for_single_file_and_project() {
    let source = SourceIdentity::new(
        "project".into(),
        "package".into(),
        "src/main.w".into(),
        "r1".into(),
    );
    let program = derive_scalar_program(
            &parse_source(
                source.clone(),
                "%%start\ni8() signed_result = fn { 1 };\ni16 signed_from_call = core.int_extend<i16>(signed_result());\ni64 signed_source = 42;\ni128 signed_wide = core.int_extend<i128>(signed_source);\nu64 unsigned_source = 42;\nu128 unsigned_wide = core.int_extend<u128>(unsigned_source);\ni8 signed_narrow = core.int_trunc<i8>(signed_wide);\nu8 unsigned_narrow = core.int_trunc<u8>(unsigned_wide);\n%%end".into(),
                &[],
            )
            .result,
        );
    let single = emit_scalar_llvm(&program)
        .expect("single-file conversion LLVM")
        .to_text();
    assert!(single.contains("sext i64"), "{single}");
    assert!(single.contains("sext i8"), "{single}");
    assert!(single.contains("zext i64"), "{single}");
    assert!(single.contains("trunc i128"), "{single}");

    let validation = validate_scalar_project(ScalarProject::new(
        vec![ScalarModule::from_program(program.program, Vec::new())],
        vec![source],
    ));
    assert!(
        validation.diagnostics.is_empty(),
        "{:?}",
        validation.diagnostics
    );
    let project = emit_scalar_project_llvm(&validation)
        .expect("project conversion LLVM")
        .to_text();
    assert!(project.contains("sext i64"), "{project}");
    assert!(project.contains("zext i64"), "{project}");
    assert!(project.contains("trunc i128"), "{project}");
}

#[test]
fn emits_float_conversions_for_single_file_and_project() {
    let source = SourceIdentity::new(
        "project".into(),
        "package".into(),
        "src/main.w".into(),
        "r1".into(),
    );
    let program = derive_scalar_program(
            &parse_source(
                source.clone(),
                "%%start\nu64 uint_source = 42;\ni64 sint_source = 42;\nf64 wide_source = 1.5;\nf32 narrow_source = 1.5;\nf64 from_uint = core.uint_to_float<f64>(uint_source);\nf32 from_sint = core.sint_to_float<f32>(sint_source);\nf64 from_uint_literal = core.uint_to_float<f64>(7);\ni32 to_sint = core.float_to_sint_trunc<i32>(wide_source);\nu64 to_uint = core.float_to_uint_trunc<u64>(narrow_source);\nf32 narrowed = core.float_trunc<f32>(wide_source);\nf64 widened = core.float_extend<f64>(narrow_source);\n%%end".into(),
                &[],
            )
            .result,
        );
    assert!(program.diagnostics.is_empty(), "{:?}", program.diagnostics);
    let single = emit_scalar_llvm(&program)
        .expect("single-file float conversion LLVM")
        .to_text();
    assert!(single.contains("uitofp"), "{single}");
    assert!(single.contains("sitofp"), "{single}");
    assert!(single.contains("fptosi"), "{single}");
    assert!(single.contains("fptoui"), "{single}");
    assert!(single.contains("fptrunc"), "{single}");
    assert!(single.contains("fpext"), "{single}");

    let validation = validate_scalar_project(ScalarProject::new(
        vec![ScalarModule::from_program(program.program, Vec::new())],
        vec![source],
    ));
    assert!(
        validation.diagnostics.is_empty(),
        "{:?}",
        validation.diagnostics
    );
    let project = emit_scalar_project_llvm(&validation)
        .expect("project float conversion LLVM")
        .to_text();
    assert!(project.contains("uitofp"), "{project}");
    assert!(project.contains("sitofp"), "{project}");
    assert!(project.contains("fptosi"), "{project}");
    assert!(project.contains("fptoui"), "{project}");
    assert!(project.contains("fptrunc"), "{project}");
    assert!(project.contains("fpext"), "{project}");
}

#[test]
fn emits_bitcast_for_single_file_and_project() {
    let source = SourceIdentity::new(
        "project".into(),
        "package".into(),
        "src/main.w".into(),
        "r1".into(),
    );
    let program = derive_scalar_program(
            &parse_source(
                source.clone(),
                "%%start\nchar letter = 'A';\nu32 code = core.bitcast<u32>(letter);\nf32 bits = core.bitcast<f32>(code);\nu32 roundtrip = core.bitcast<u32>(bits);\ni64 wider = 42;\nf64 dwbits = core.bitcast<f64>(wider);\ni64 back = core.bitcast<i64>(dwbits);\n%%end".into(),
                &[],
            )
            .result,
        );
    assert!(program.diagnostics.is_empty(), "{:?}", program.diagnostics);
    let single = emit_scalar_llvm(&program)
        .expect("single-file bitcast LLVM")
        .to_text();
    assert!(single.contains("bitcast"), "{single}");

    let validation = validate_scalar_project(ScalarProject::new(
        vec![ScalarModule::from_program(program.program, Vec::new())],
        vec![source],
    ));
    assert!(
        validation.diagnostics.is_empty(),
        "{:?}",
        validation.diagnostics
    );
    let project = emit_scalar_project_llvm(&validation)
        .expect("project bitcast LLVM")
        .to_text();
    assert!(project.contains("bitcast"), "{project}");
}

#[test]
fn emits_pointer_cast_for_single_file_and_project() {
    let source = SourceIdentity::new(
        "project".into(),
        "package".into(),
        "src/main.w".into(),
        "r1".into(),
    );
    let program = derive_scalar_program(
            &parse_source(
                source.clone(),
                "%%start\nstruct Block {\n\tu8 tag;\n\tu32 value;\n}\n*?u8 base = null;\n*?u8 roundtrip = unsafe { core.pointer_cast<*?u8>(core.pointer_cast<*?Block>(base)) };\n*?u8(*?u8) ident = fn(pointer) { unsafe { core.pointer_cast<*?u8>(pointer) } };\nu8[4] bytes = [1, 2, 3, 4];\n*?u8 addr = unsafe { core.pointer_cast<*?u8>(&?bytes[2]) };\n*u8 shared = unsafe { core.pointer_cast<*u8>(base) };\n*!u8 exclusive = unsafe { core.pointer_cast<*!u8>(base) };\n%%end".into(),
                &[],
            )
            .result,
        );
    assert!(program.diagnostics.is_empty(), "{:?}", program.diagnostics);
    let single = emit_scalar_llvm(&program)
        .expect("single-file pointer cast LLVM")
        .to_text();
    assert!(
        single.contains("define i32 @ident(i32 %pointer)"),
        "{single}"
    );
    assert!(single.contains("ret i32 %pointer"), "{single}");
    assert!(
        single.contains("store i32 %base, ptr @roundtrip"),
        "{single}"
    );
    assert!(single.contains("store i32 %base4, ptr @shared"), "{single}");
    assert!(
        single.contains("store i32 %base5, ptr @exclusive"),
        "{single}"
    );
    assert!(!single.contains("inttoptr"), "{single}");
    assert!(!single.contains("deref_null"), "{single}");

    let validation = validate_scalar_project(ScalarProject::new(
        vec![ScalarModule::from_program(program.program, Vec::new())],
        vec![source],
    ));
    assert!(
        validation.diagnostics.is_empty(),
        "{:?}",
        validation.diagnostics
    );
    let project = emit_scalar_project_llvm(&validation)
        .expect("project pointer cast LLVM")
        .to_text();
    assert!(project.contains("(i32 %pointer)"), "{project}");
    assert!(project.contains("ret i32 %pointer"), "{project}");
    assert!(project.contains("store i32 %base, ptr @"), "{project}");
    assert!(project.contains("store i32 %base4, ptr @"), "{project}");
    assert!(project.contains("store i32 %base5, ptr @"), "{project}");
    assert!(!project.contains("inttoptr"), "{project}");
    assert!(!project.contains("deref_null"), "{project}");
}

#[test]
fn emits_guard_for_direct_pointer_cast_dereference_in_single_file_and_project() {
    for (destination, field) in [
        ("*u8", false),
        ("*!u8", false),
        ("*Record", true),
        ("*!Record", true),
    ] {
        let source = SourceIdentity::new(
            "project".into(),
            "package".into(),
            "src/main.w".into(),
            "r1".into(),
        );
        let program = derive_scalar_program(
            &parse_source(
                source.clone(),
                format!(
                    "%%start\n{}u8(*?u8) read = fn(pointer) {{ unsafe {{ {} }} }};\n%%end",
                    if field {
                        "struct Record { u8 tag; }\n"
                    } else {
                        ""
                    },
                    if field {
                        format!("(*core.pointer_cast<{destination}>(pointer)).tag")
                    } else {
                        format!("*core.pointer_cast<{destination}>(pointer)")
                    },
                ),
                &[],
            )
            .result,
        );
        assert!(program.diagnostics.is_empty(), "{:?}", program.diagnostics);
        let single = emit_scalar_llvm(&program)
            .expect("single-file direct pointer cast dereference LLVM")
            .to_text();
        assert!(single.contains("deref_is_null"), "{destination}: {single}");
        assert!(
            single.contains("deref_null_panic"),
            "{destination}: {single}"
        );
        assert!(
            single.contains("deref_null_continue"),
            "{destination}: {single}"
        );
        assert!(
            single.contains("call void @__wosy_core_system_panic()"),
            "{destination}: {single}"
        );
        assert!(single.contains("inttoptr"), "{destination}: {single}");
        assert!(single.contains("load i8"), "{destination}: {single}");
        if field {
            assert!(
                single.contains("getelementptr inbounds"),
                "{destination}: {single}"
            );
        }

        let validation = validate_scalar_project(ScalarProject::new(
            vec![ScalarModule::from_program(program.program, Vec::new())],
            vec![source],
        ));
        assert!(
            validation.diagnostics.is_empty(),
            "{:?}",
            validation.diagnostics
        );
        let project = emit_scalar_project_llvm(&validation)
            .expect("project direct pointer cast dereference LLVM")
            .to_text();
        assert!(
            project.contains("deref_is_null"),
            "{destination}: {project}"
        );
        assert!(
            project.contains("deref_null_panic"),
            "{destination}: {project}"
        );
        assert!(
            project.contains("deref_null_continue"),
            "{destination}: {project}"
        );
        assert!(
            project.contains("call void @__wosy_core_system_panic()"),
            "{destination}: {project}"
        );
        assert!(project.contains("inttoptr"), "{destination}: {project}");
        assert!(project.contains("load i8"), "{destination}: {project}");
        if field {
            assert!(
                project.contains("getelementptr inbounds"),
                "{destination}: {project}"
            );
        }
    }
}

#[test]
fn emits_raw_offset_and_load_for_single_file_and_project() {
    let source = SourceIdentity::new(
        "project".into(),
        "package".into(),
        "src/main.w".into(),
        "r1".into(),
    );
    let program = derive_scalar_program(
            &parse_source(
                source.clone(),
                "%%start\nu8(*?u8, i64) read = fn(pointer, index) { unsafe { core.load<u8>(core.offset<u8>(pointer, index)) } };\n%%end".into(),
                &[],
            )
            .result,
        );
    assert!(program.diagnostics.is_empty(), "{:?}", program.diagnostics);
    let single = emit_scalar_llvm(&program)
        .expect("single-file offset and load LLVM")
        .to_text();
    assert!(single.contains("inttoptr"), "{single}");
    assert!(single.contains("getelementptr inbounds i8"), "{single}");
    assert!(single.contains("ptrtoint"), "{single}");
    assert!(single.contains("load i8"), "{single}");

    let validation = validate_scalar_project(ScalarProject::new(
        vec![ScalarModule::from_program(program.program, Vec::new())],
        vec![source],
    ));
    assert!(
        validation.diagnostics.is_empty(),
        "{:?}",
        validation.diagnostics
    );
    let project = emit_scalar_project_llvm(&validation)
        .expect("project offset and load LLVM")
        .to_text();
    assert!(project.contains("inttoptr"), "{project}");
    assert!(project.contains("getelementptr inbounds i8"), "{project}");
    assert!(project.contains("ptrtoint"), "{project}");
    assert!(project.contains("load i8"), "{project}");
}

#[test]
fn emits_generic_integer_conversion_for_struct_member_source() {
    let source = SourceIdentity::new(
        "project".into(),
        "package".into(),
        "src/main.w".into(),
        "r1".into(),
    );
    let program = derive_scalar_program(
            &parse_source(
                source.clone(),
                "%%start\nstruct utf8 {\n\tu64 length;\n}\nu32(utf8) truncate = fn(text) { core.int_trunc<u32>(text.length) };\n%%end".into(),
                &[],
            )
            .result,
        );
    assert!(program.diagnostics.is_empty(), "{:?}", program.diagnostics);
    let single = emit_scalar_llvm(&program)
        .expect("single-file member conversion LLVM")
        .to_text();
    assert!(single.contains("trunc i64"), "{single}");

    let validation = validate_scalar_project(ScalarProject::new(
        vec![ScalarModule::from_program(program.program, Vec::new())],
        vec![source],
    ));
    assert!(
        validation.diagnostics.is_empty(),
        "{:?}",
        validation.diagnostics
    );
    let project = emit_scalar_project_llvm(&validation)
        .expect("project member conversion LLVM")
        .to_text();
    assert!(project.contains("trunc i64"), "{project}");
}

#[test]
fn emits_unary_bitwise_and_shift_instructions() {
    let source = SourceIdentity::new(
        "project".into(),
        "package".into(),
        "src/operators.w".into(),
        "r1".into(),
    );
    let validation = derive_scalar_program(
            &parse_source(
                source,
                "%%start\nbool(bool) logical_not = fn(value) { !value };\ni32(i32) bitwise_not = fn(value) { ~value };\ni32(i32, i32) bitwise = fn(left, right) { (left & right) | (left ^ right) };\ni32(i32, i32) signed_shift = fn(value, count) { (value << count) >> count };\nu32(u32, u32) unsigned_shift = fn(value, count) { value >> count };\nbool inverted = logical_not(true);\ni32 complemented = bitwise_not(1);\ni32 combined = bitwise(7, 3);\ni32 shifted = signed_shift(8, 1);\nu32 logical_shifted = unsigned_shift(8, 1);\n%%end".into(),
                &[],
            )
            .result,
        );
    assert!(
        validation.diagnostics.is_empty(),
        "{:?}",
        validation.diagnostics
    );
    let text = emit_scalar_llvm(&validation)
        .expect("operator LLVM")
        .to_text();
    assert!(text.contains("xor i1"), "{text}");
    assert!(text.contains("xor i32"), "{text}");
    assert!(text.contains("and i32"), "{text}");
    assert!(text.contains("or i32"), "{text}");
    assert!(text.contains("shl i32"), "{text}");
    assert!(text.contains("ashr i32"), "{text}");
    assert!(text.contains("lshr i32"), "{text}");
}

#[test]
fn emits_typed_integer_negation_and_signed_division_remainder() {
    let source = SourceIdentity::new(
        "project".into(),
        "package".into(),
        "src/negate.w".into(),
        "r1".into(),
    );
    let validation = derive_scalar_program(
            &parse_source(
                source,
                "%%start\ni32(i32) negate = fn(value) { -value };\ni8(i8) negate_narrow = fn(value) { -value };\ni32 quotient = -7 / 3;\ni32 remainder = -7 % 3;\ni32 computed_quotient = negate(7) / 3;\ni32 computed_remainder = negate(7) % 3;\ni8 seed = 7;\ni8 signed = negate_narrow(seed);\n%%end".into(),
                &[],
            )
            .result,
        );
    assert!(
        validation.diagnostics.is_empty(),
        "{:?}",
        validation.diagnostics
    );
    let text = emit_scalar_llvm(&validation)
        .expect("negation LLVM")
        .to_text();
    assert!(text.contains("store i32 -2"), "{text}");
    assert!(text.contains("store i32 -1"), "{text}");
    assert!(text.contains("sub i32 0, %value"), "{text}");
    assert!(text.contains("sdiv i32 %call, 3"), "{text}");
    assert!(text.contains("srem i32 %call1, 3"), "{text}");
    assert!(text.contains("sub i8 0, %value"), "{text}");
    assert!(text.contains("store i8 %call2"), "{text}");
}

#[test]
fn emits_unit_if_with_then_and_merge_blocks_without_phi() {
    let source = SourceIdentity::new(
        "project".into(),
        "package".into(),
        "src/unit_if.w".into(),
        "r1".into(),
    );
    let validation = derive_scalar_program(
            &parse_source(
                source,
                "%%start\nunit() touch = fn { 1; };\nunit() run = fn { if (true) { touch(); }; };\n%%end"
                    .into(),
                &[],
            )
            .result,
        );
    assert!(
        validation.diagnostics.is_empty(),
        "{:?}",
        validation.diagnostics
    );
    let text = emit_scalar_llvm(&validation)
        .expect("unit conditional LLVM")
        .to_text();
    let run = text.split("define void @run").nth(1).expect("run function");
    assert!(
        run.contains("br i1 true, label %if.then, label %if.merge"),
        "{run}"
    );
    assert!(run.contains("if.then:"), "{run}");
    assert!(run.contains("if.merge:"), "{run}");
    assert!(!run.contains(" phi "), "{run}");
}

#[test]
fn emits_float_types_literals_operations_and_comparisons() {
    let source = SourceIdentity::new(
        "project".into(),
        "package".into(),
        "src/floats.w".into(),
        "r1".into(),
    );
    let validation = derive_scalar_program(
            &parse_source(
                source,
                "%%start\nf32(f32) identity = fn(value) { value };\nf32 a = 1.25;\nf64 b = 2e1;\nf32 sum = a + 2.5;\nf32 echoed = identity(4e-1);\nb < 3e1;\n%%end".into(),
                &[],
            )
            .result,
        );
    assert!(
        validation.diagnostics.is_empty(),
        "{:?}",
        validation.diagnostics
    );
    let text = emit_scalar_llvm(&validation).expect("float LLVM").to_text();
    assert!(text.contains("global float"), "{text}");
    assert!(text.contains("global double"), "{text}");
    assert!(text.contains("fadd float"), "{text}");
    assert!(text.contains("fcmp olt double"), "{text}");
    assert!(text.contains("call float @identity(float"), "{text}");
}

#[test]
fn emits_distinct_char_and_artifact_id_primitive_signatures() {
    let source = SourceIdentity::new(
        "project".into(),
        "package".into(),
        "src/main.w".into(),
        "r1".into(),
    );
    let validation = derive_scalar_program(
            &parse_source(
                source,
                "%%start\nenv = extern wasm \"primitive\" { char() read_char; artifact_id() read_artifact; };\n%%end".into(),
                &[],
            )
            .result,
        );
    assert!(
        validation.diagnostics.is_empty(),
        "{:?}",
        validation.diagnostics
    );
    let partition = emit_scalar_llvm(&validation).expect("primitive LLVM");
    let text = partition.to_text();
    assert!(text.contains("declare i32 @wosy_extern__"));
    assert!(text.contains("declare ptr @wosy_extern__"));
    assert!(text.contains("wasm-import-name\"=\"read_char"));
    assert!(text.contains("wasm-import-name\"=\"read_artifact"));
}

#[test]
fn emits_generic_extern_module_symbol_and_signature() {
    let source = SourceIdentity::new(
        "project".into(),
        "package".into(),
        "src/main.w".into(),
        "r1".into(),
    );
    let validation = derive_scalar_program(
            &parse_source(
                source,
                "%%start\nenv = extern wasm \"helper\" { i32(i32) read; unit() flush; };\ni32 value = env.read(7);\nenv.flush();\n%%end".into(),
                &[],
            )
            .result,
        );
    let partition = emit_scalar_llvm(&validation).expect("generic extern LLVM");
    assert_eq!(partition.declarations.len(), 2);
    assert!(partition.declarations[0].name.starts_with("wosy_extern__"));
    assert_eq!(
        partition.declarations[0].attributes[0].wasm_import_module,
        "helper"
    );
    assert!(partition.declarations[1].name.starts_with("wosy_extern__"));
    assert!(partition.to_text().contains("declare i32 @wosy_extern__"));
    assert!(partition.to_text().contains("declare void @wosy_extern__"));
    assert!(partition.to_text().contains("call i32 @wosy_extern__"));
}

#[test]
fn emits_injective_extern_identity_for_same_symbols() {
    let source = SourceIdentity::new(
        "project".into(),
        "package".into(),
        "src/main.w".into(),
        "r1".into(),
    );
    let validation = derive_scalar_program(
            &parse_source(
                source,
                "%%start\nleft = extern wasm \"shared\" { i32() same; };\nright = extern wasm \"shared\" { i32() same; };\nother = extern wasm \"other\" { i32() same; };\ni32 first = left.same();\ni32 second = right.same();\ni32 third = other.same();\n%%end".into(),
                &[],
            )
            .result,
        );
    assert!(
        validation.diagnostics.is_empty(),
        "{:?}",
        validation.diagnostics
    );
    let partition = emit_scalar_llvm(&validation).expect("colliding extern LLVM");
    assert_eq!(partition.declarations.len(), 3, "{}", partition.to_text());
    assert_eq!(
        partition
            .declarations
            .iter()
            .map(|declaration| declaration.name.as_str())
            .collect::<std::collections::BTreeSet<_>>()
            .len(),
        3
    );
    assert_eq!(
        partition
            .declarations
            .iter()
            .map(|declaration| declaration.attributes[0].wasm_import_name.as_str())
            .collect::<Vec<_>>(),
        vec!["same", "same", "same"]
    );
    assert_eq!(
        partition
            .declarations
            .iter()
            .map(|declaration| declaration.attributes[0].wasm_import_module.as_str())
            .collect::<Vec<_>>(),
        vec!["shared", "shared", "other"]
    );
    let text = partition.to_text();
    for declaration in &partition.declarations {
        assert!(text.contains(&format!("call i32 @{}()", declaration.name)));
        let attributes = &declaration.attributes[0];
        assert!(
            text.contains(&format!(
                "declare i32 @{}() #{}",
                declaration.name, attributes.group
            )),
            "{text}"
        );
        assert!(
            text.contains(&format!(
                "attributes #{} = {{ \"wasm-import-module\"=\"{}\" \"wasm-import-name\"=\"{}\" }}",
                attributes.group, attributes.wasm_import_module, attributes.wasm_import_name
            )),
            "{text}"
        );
    }
}

#[test]
fn emits_ordered_multi_output_aggregate_returns_calls_and_receivers() {
    let source = SourceIdentity::new(
        "project".into(),
        "package".into(),
        "src/main.w".into(),
        "r1".into(),
    );
    let validation = derive_scalar_program(
            &parse_source(
                source,
                "%%start\nenv = extern wasm \"helper\" { (i32, u64)() read; };\n(i32, u64)() pair = fn { env.read() };\ni32 first, u64 second = pair();\n%%end".into(),
                &[],
            )
            .result,
        );
    assert!(
        validation.diagnostics.is_empty(),
        "{:?}",
        validation.diagnostics
    );
    let partition = emit_scalar_llvm(&validation).expect("multi-output LLVM");
    let text = partition.to_text();
    assert!(text.contains("define { i32, i64 } @pair"), "{text}");
    assert!(
        text.contains("declare { i32, i64 } @wosy_extern__"),
        "{text}"
    );
    assert!(text.contains("call { i32, i64 } @pair"), "{text}");
    assert!(text.contains("extractvalue { i32, i64 }"), "{text}");
    assert!(
        text.contains("extractvalue { i32, i64 } %call, 0"),
        "{text}"
    );
    assert!(
        text.contains("extractvalue { i32, i64 } %call, 1"),
        "{text}"
    );
}

#[test]
fn emits_nested_conditional_multi_outputs_in_aggregate_order() {
    let source = SourceIdentity::new(
        "project".into(),
        "package".into(),
        "src/main.w".into(),
        "r1".into(),
    );
    let validation = derive_scalar_program(
            &parse_source(
                source,
                "%%start\n(i32, bool)() pair = fn { if (true) { if (true) { 1, true } else { 2, false } } else { 3, true } };\n%%end".into(),
                &[],
            )
            .result,
        );
    assert!(
        validation.diagnostics.is_empty(),
        "{:?}",
        validation.diagnostics
    );

    let text = emit_scalar_llvm(&validation)
        .expect("nested conditional LLVM")
        .to_text();
    let pair = text
        .split("define { i32, i1 } @pair")
        .nth(1)
        .expect("pair function");
    assert_eq!(pair.matches("phi { i32, i1 }").count(), 2, "{pair}");
    assert!(pair.contains("{ i32 1, i1 true }"), "{pair}");
    assert!(pair.contains("{ i32 2, i1 false }"), "{pair}");
    assert!(pair.contains("{ i32 3, i1 true }"), "{pair}");
    assert!(pair.contains("ret { i32, i1 }"), "{pair}");
}

#[test]
fn materializes_multi_output_assignments_before_ordered_writes() {
    let source = SourceIdentity::new(
        "project".into(),
        "package".into(),
        "src/main.w".into(),
        "r1".into(),
    );
    let validation = derive_scalar_program(
            &parse_source(
                source,
                "%%start\n(i32, i32)(i32, i32) pair = fn(first, second) { first, second };\ni32() swap = fn { i32 left = 1; i32 right = 2; right, left = pair(left, right); left };\ni32() inferred = fn { first, second = pair(3, 4); first + second };\n%%end".into(),
                &[],
            )
            .result,
        );
    assert!(
        validation.diagnostics.is_empty(),
        "{:?}",
        validation.diagnostics
    );
    let text = emit_scalar_llvm(&validation)
        .expect("ordered assignment LLVM")
        .to_text();
    let swap = text.split("define i32 @swap").nth(1).expect("swap");
    let call = swap.find("call { i32, i32 } @pair").expect("pair call");
    let first_output = swap
        .find("extractvalue { i32, i32 } %call, 0")
        .expect("first output");
    let second_output = swap
        .find("extractvalue { i32, i32 } %call, 1")
        .expect("second output");
    let right_store = swap.rfind("ptr %right").expect("right store");
    let left_store = swap.rfind("ptr %left").expect("left store");
    assert!(call < first_output, "{swap}");
    assert!(first_output < second_output, "{swap}");
    assert!(second_output < right_store, "{swap}");
    assert!(right_store < left_store, "{swap}");

    let inferred = text.split("define i32 @inferred").nth(1).expect("inferred");
    assert!(inferred.contains("%first = alloca i32"), "{inferred}");
    assert!(inferred.contains("%second = alloca i32"), "{inferred}");
}

#[test]
fn emits_distinct_independent_output_values_in_source_order() {
    let source = SourceIdentity::new(
        "project".into(),
        "package".into(),
        "src/main.w".into(),
        "r1".into(),
    );
    let validation = derive_scalar_program(
        &parse_source(
            source,
            "%%start\ni32 first, u64 second = 1, 2;\n%%end".into(),
            &[],
        )
        .result,
    );
    assert!(
        validation.diagnostics.is_empty(),
        "{:?}",
        validation.diagnostics
    );
    let text = emit_scalar_llvm(&validation)
        .expect("independent output LLVM")
        .to_text();
    assert!(text.contains("store i32 1"), "{text}");
    assert!(text.contains("store i64 2"), "{text}");
}

#[test]
fn emits_function_final_outputs_from_locals_in_order() {
    let source = SourceIdentity::new(
        "project".into(),
        "package".into(),
        "src/main.w".into(),
        "r1".into(),
    );
    let validation = derive_scalar_program(
        &parse_source(
            source,
            "%%start\n(i32, u64)() pair = fn { i32 local = 1; local, 2 };\n%%end".into(),
            &[],
        )
        .result,
    );
    assert!(
        validation.diagnostics.is_empty(),
        "{:?}",
        validation.diagnostics
    );
    let text = emit_scalar_llvm(&validation)
        .expect("final output LLVM")
        .to_text();
    let pair = text
        .split("define { i32, i64 } @pair")
        .nth(1)
        .expect("pair");
    assert!(pair.contains("load i32"), "{pair}");
    assert!(pair.contains("insertvalue { i32, i64 }"), "{pair}");
    assert!(
        pair.contains("insertvalue { i32, i64 } %output, i64 2, 1"),
        "{pair}"
    );
}

#[test]
fn emits_project_final_outputs_with_single_file_parity() {
    let source = SourceIdentity::new(
        "project".into(),
        "package".into(),
        "src/main.w".into(),
        "r1".into(),
    );
    let program = derive_scalar_program(
        &parse_source(
            source.clone(),
            "%%start\n(i32, u64)() pair = fn { i32 local = 1; local, 2 };\n%%end".into(),
            &[],
        )
        .result,
    )
    .program;
    let validation = validate_scalar_project(ScalarProject::new(
        vec![ScalarModule::new(source.clone(), program.items, Vec::new())],
        vec![source.clone()],
    ));
    assert!(
        validation.diagnostics.is_empty(),
        "{:?}",
        validation.diagnostics
    );
    let text = emit_scalar_project_llvm(&validation)
        .expect("project final output LLVM")
        .to_text();
    let pair = text
        .split(&format!(
            "define {{ i32, i64 }} @{}",
            project_function_name(&source, "pair")
        ))
        .nth(1)
        .expect("project pair");
    assert!(pair.contains("load i32"), "{pair}");
    assert!(
        pair.contains("insertvalue { i32, i64 } %output, i64 2, 1"),
        "{pair}"
    );
}

#[test]
fn preserves_direct_one_output_and_void_zero_output_function_abis() {
    let source = SourceIdentity::new(
        "project".into(),
        "package".into(),
        "src/main.w".into(),
        "r1".into(),
    );
    let validation = derive_scalar_program(
            &parse_source(
                source,
                "%%start\ni32() one = fn { i32 local = 1; local };\nunit() zero = fn { i32 local = 1; local; };\n%%end".into(),
                &[],
            )
            .result,
        );
    assert!(
        validation.diagnostics.is_empty(),
        "{:?}",
        validation.diagnostics
    );
    let text = emit_scalar_llvm(&validation).expect("arity LLVM").to_text();
    assert!(text.contains("define i32 @one"), "{text}");
    assert!(text.contains("define void @zero"), "{text}");
}

#[test]
fn emits_equal_independent_output_values_with_typed_receivers() {
    let source = SourceIdentity::new(
        "project".into(),
        "package".into(),
        "src/main.w".into(),
        "r1".into(),
    );
    let mut validation = derive_scalar_program(
        &parse_source(
            source,
            "%%start\ni32 first, u64 second = 1, 1;\n%%end".into(),
            &[],
        )
        .result,
    );
    assert!(
        validation.diagnostics.is_empty(),
        "{:?}",
        validation.diagnostics
    );
    let crate::ScalarItem::Binding(binding) = &mut validation.program.items[0] else {
        panic!("output binding");
    };
    binding.output_values[1].value = binding.output_values[0].value.clone();
    let text = emit_scalar_llvm(&validation)
        .expect("equal independent output LLVM")
        .to_text();
    assert!(text.contains("store i32 1"), "{text}");
    assert!(text.contains("store i64 1"), "{text}");
}

#[test]
fn emits_zero_initialized_unit_output_fields_without_truncation() {
    let source = SourceIdentity::new(
        "project".into(),
        "package".into(),
        "src/main.w".into(),
        "r1".into(),
    );
    let validation = derive_scalar_program(
            &parse_source(
                source,
                "%%start\nenv = extern wasm \"helper\" { (i32, unit, u64)() read; };\ni32 first, unit done, u64 last = env.read();\n%%end".into(),
                &[],
            )
            .result,
        );
    assert!(
        validation.diagnostics.is_empty(),
        "{:?}",
        validation.diagnostics
    );
    let text = emit_scalar_llvm(&validation)
        .expect("unit aggregate LLVM")
        .to_text();
    assert!(
        text.contains("declare { i32, i8, i64 } @wosy_extern__"),
        "{text}"
    );
    assert!(
        text.contains("extractvalue { i32, i8, i64 } %call, 0"),
        "{text}"
    );
    assert!(
        text.contains("extractvalue { i32, i8, i64 } %call, 2"),
        "{text}"
    );
    assert!(
        !text.contains("extractvalue { i32, i8, i64 } %call, 3"),
        "{text}"
    );
}

#[test]
fn emits_ordered_multi_output_project_namespace_calls() {
    let child_source = SourceIdentity::new(
        "project".into(),
        "package".into(),
        "src/child.w".into(),
        "r1".into(),
    );
    let child = derive_scalar_program(
            &parse_source(
                child_source.clone(),
                "%%start\nenv = extern wasm \"helper\" { (i32, u64)() read; };\n(i32, u64)() pair = fn { env.read() };\n%%end".into(),
                &[],
            )
            .result,
        )
        .program;
    let root_source = SourceIdentity::new(
        "project".into(),
        "package".into(),
        "src/main.w".into(),
        "r1".into(),
    );
    let root = derive_scalar_program(
            &parse_source(
                root_source.clone(),
                "%%start\nchild = namespace package \"src/child.w\";\ni32 first, u64 second = child.pair();\n%%end".into(),
                &[],
            )
            .result,
        )
        .program;
    let namespace_bindings = root
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
    let validation = validate_scalar_project(ScalarProject::new(
        vec![
            ScalarModule::new(root_source.clone(), root.items, namespace_bindings),
            ScalarModule::new(child_source.clone(), child.items, Vec::new()),
        ],
        vec![root_source, child_source],
    ));
    assert!(
        validation.diagnostics.is_empty(),
        "{:?}",
        validation.diagnostics
    );
    let text = emit_scalar_project_llvm(&validation)
        .expect("project LLVM")
        .to_text();
    assert!(text.contains("define { i32, i64 } @wosy_fn__"), "{text}");
    assert!(text.contains("call { i32, i64 } @wosy_fn__"), "{text}");
}

#[test]
fn emits_project_binding_store_and_function_load() {
    let source = SourceIdentity::new(
        "project".into(),
        "package".into(),
        "src/main.w".into(),
        "r1".into(),
    );
    let parsed = parse_source(
        source.clone(),
        "%%start\ni32 count = 41;\ni32() read = fn { count + 1 };\n%%end".into(),
        &[],
    );
    let program = derive_scalar_program(&parsed.result).program;
    let project = ScalarProject::new(
        vec![ScalarModule::new(source.clone(), program.items, Vec::new())],
        vec![source],
    );
    let validation = validate_scalar_project(project);
    let text = emit_scalar_project_llvm(&validation)
        .expect("LLVM")
        .to_text();
    let main = text.split("define i32 @main").nth(1).expect("main");
    let read = text
        .split("define i32 @wosy_fn__7061636b616765__7372632f6d61696e2e77__7231__72656164")
        .nth(1)
        .expect("read");
    assert!(main.contains("store i32 41"));
    assert!(read.contains("load i32, ptr"));
}

#[test]
fn initializes_namespaces_recursively_in_source_order_once() {
    let child_source = SourceIdentity::new(
        "project".into(),
        "package".into(),
        "src/child.w".into(),
        "r1".into(),
    );
    let child = derive_scalar_program(
        &parse_source(
            child_source.clone(),
            "%%start\ni32 value = 7;\ni32() read = fn { value };\n%%end".into(),
            &[],
        )
        .result,
    )
    .program;
    let root_source = SourceIdentity::new(
        "project".into(),
        "package".into(),
        "src/main.w".into(),
        "r1".into(),
    );
    let root = derive_scalar_program(
            &parse_source(
                root_source.clone(),
                "%%start\nfirst = namespace package \"src/child.w\";\ni32 observed = first.read();\nagain = namespace package \"src/child.w\";\ni32 after = 9;\n%%end".into(),
                &[],
            )
            .result,
        )
        .program;
    let namespace_bindings = root
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
    let project = ScalarProject::new(
        vec![
            ScalarModule::new(root_source.clone(), root.items, namespace_bindings),
            ScalarModule::new(child_source.clone(), child.items, Vec::new()),
        ],
        vec![root_source, child_source.clone()],
    );
    let text = emit_scalar_project_llvm(&validate_scalar_project(project))
        .expect("verified project LLVM")
        .to_text();
    let main = text.split("define i32 @main").nth(1).expect("main");
    let child_global = project_global_name(&child_source, "value");
    let child_store = main.find(&format!("store i32 7, ptr @{child_global}"));
    let read_call = main.find(&format!(
        "call i32 @{}",
        project_function_name(&child_source, "read")
    ));
    let observed_store = main.find("store i32 %call, ptr");
    assert_eq!(
        main.matches(&format!("store i32 7, ptr @{child_global}"))
            .count(),
        1
    );
    assert!(child_store.expect("child initialization") < read_call.expect("read call"));
    assert!(read_call.expect("read call") < observed_store.expect("observed store"));
}

#[test]
fn releases_same_named_top_level_runtime_arrays_per_project_module() {
    let first_source = SourceIdentity::new(
        "project".into(),
        "package".into(),
        "src/first.w".into(),
        "r1".into(),
    );
    let second_source = SourceIdentity::new(
        "project".into(),
        "package".into(),
        "src/second.w".into(),
        "r1".into(),
    );
    let first_module_text = "%%start\nu64 length = 1;\nu8[length] bytes;\nbytes[0] = 7;\nu8() read = fn { bytes[0] };\n%%end";
    let second_module_text = "%%start\nu64 length = 1;\nu8[length] bytes;\nbytes[0] = 11;\nu8() read = fn { bytes[0] };\n%%end";
    let first = derive_scalar_program(
        &parse_source(first_source.clone(), first_module_text.into(), &[]).result,
    )
    .program;
    let second = derive_scalar_program(
        &parse_source(second_source.clone(), second_module_text.into(), &[]).result,
    )
    .program;
    let root_source = SourceIdentity::new(
        "project".into(),
        "package".into(),
        "src/main.w".into(),
        "r1".into(),
    );
    let root = derive_scalar_program(
            &parse_source(
                root_source.clone(),
                "%%start\nfirst = namespace package \"src/first.w\";\nsecond = namespace package \"src/second.w\";\nu8 first_value = first.read();\nu8 second_value = second.read();\nu8 first_expected = 7;\nu8 second_expected = 11;\nif (first_value != first_expected || second_value != second_expected) { core.system_panic(); };\n%%end".into(),
                &[],
            )
            .result,
        )
        .program;
    let namespace_bindings = root
        .items
        .iter()
        .filter_map(|item| match item {
            crate::ScalarItem::Namespace(namespace) => Some(crate::ScalarNamespaceBinding {
                binding: namespace.binding.clone(),
                target: match namespace.binding.as_str() {
                    "first" => first_source.clone(),
                    "second" => second_source.clone(),
                    binding => panic!("unexpected namespace binding {binding}"),
                },
                span: namespace.span,
            }),
            _ => None,
        })
        .collect();
    let project = ScalarProject::new(
        vec![
            ScalarModule::new(root_source.clone(), root.items, namespace_bindings),
            ScalarModule::new(first_source.clone(), first.items, Vec::new()),
            ScalarModule::new(second_source.clone(), second.items, Vec::new()),
        ],
        vec![root_source, first_source.clone(), second_source.clone()],
    );
    for layout in [ScalarTargetLayout::WASM32, ScalarTargetLayout::NATIVE64] {
        let mut project = project.clone();
        for module in &mut project.modules {
            module.target_layout = layout;
        }
        let validation = validate_scalar_project(project);
        assert!(
            validation.diagnostics.is_empty(),
            "{:?}",
            validation.diagnostics
        );
        let text = emit_scalar_project_llvm(&validation)
            .expect("project runtime-array LLVM")
            .to_text();
        assert_eq!(
            text.matches("call ptr @__wosy_core_alloc").count(),
            2,
            "{text}"
        );
        assert_eq!(
            text.matches("call void @__wosy_core_free").count(),
            2,
            "{text}"
        );
        for (source, value) in [(&first_source, 7), (&second_source, 11)] {
            let global = project_global_name(source, "bytes");
            let read = project_function_name(source, "read");
            assert!(
                text.lines().any(|line| {
                    line.contains("store ptr %") && line.contains(&format!("ptr @{global}"))
                }),
                "{text}"
            );
            let body = text
                .split(&format!("define i8 @{read}"))
                .nth(1)
                .expect("read function")
                .split("}")
                .next()
                .expect("read function body");
            let load = body
                .find(&format!("load ptr, ptr @{global}"))
                .expect("global pointer load");
            let gep = body.find("getelementptr inbounds i8").expect("indexed GEP");
            assert!(load < gep, "{body}");
            assert!(text.contains(&format!("store i8 {value}")), "{text}");
        }
    }
}

#[test]
fn emits_ordered_top_level_assignment_and_global_function_load() {
    let source = SourceIdentity::new(
        "project".into(),
        "package".into(),
        "src/main.w".into(),
        "r1".into(),
    );
    let parsed = parse_source(
        source.clone(),
        "%%start\ni32 value = 0;\nvalue = 1;\ni32() read = fn { value };\nvalue;\n%%end".into(),
        &[],
    );
    let validation = derive_scalar_program(&parsed.result);
    let text = emit_scalar_llvm(&validation)
        .expect("verified LLVM")
        .to_text();
    let main = text.split("define i32 @main").nth(1).expect("main");
    assert!(
        main.find("store i32 0").expect("initial store")
            < main.find("store i32 1").expect("assignment store")
    );
    assert!(text.contains("@value = internal global i32 0"));
    assert!(text.contains("define i32 @read"));
    assert!(text.contains("load i32, ptr @value"));

    let program = validation.program;
    let project = ScalarProject::new(
        vec![ScalarModule::new(source.clone(), program.items, Vec::new())],
        vec![source],
    );
    let project_text = emit_scalar_project_llvm(&validate_scalar_project(project))
        .expect("verified project LLVM")
        .to_text();
    let project_main = project_text
        .split("define i32 @main")
        .nth(1)
        .expect("project main");
    assert!(
        project_main
            .find("store i32 0")
            .expect("project initial store")
            < project_main
                .find("store i32 1")
                .expect("project assignment store")
    );
    assert!(project_text.contains("load i32, ptr @wosy_fn__7061636b616765__7372632f6d61696e2e77__7231__676c6f62616c5f76616c7565"));
}

#[test]
fn encodes_colliding_source_paths_as_distinct_verified_symbols() {
    let first_source = SourceIdentity::new(
        "project".into(),
        "package".into(),
        "src/a-b.w".into(),
        "r1".into(),
    );
    let second_source = SourceIdentity::new(
        "project".into(),
        "package".into(),
        "src/a/b.w".into(),
        "r1".into(),
    );
    let first_program = derive_scalar_program(
        &parse_source(
            first_source.clone(),
            "%%start\ni32() same = fn { 1 };\n%%end".into(),
            &[],
        )
        .result,
    )
    .program;
    let second_program = derive_scalar_program(
        &parse_source(
            second_source.clone(),
            "%%start\ni32() same = fn { 2 };\n%%end".into(),
            &[],
        )
        .result,
    )
    .program;
    let project = ScalarProject::new(
        vec![
            ScalarModule::new(first_source.clone(), first_program.items, Vec::new()),
            ScalarModule::new(second_source.clone(), second_program.items, Vec::new()),
        ],
        vec![first_source.clone(), second_source.clone()],
    );
    let text = emit_scalar_project_llvm(&validate_scalar_project(project))
        .expect("verified LLVM")
        .to_text();
    let first_name = project_function_name(&first_source, "same");
    let second_name = project_function_name(&second_source, "same");

    assert_ne!(first_name, second_name);
    assert!(text.contains(&format!("define i32 @{first_name}")));
    assert!(text.contains(&format!("define i32 @{second_name}")));
}

#[test]
fn emits_qualified_member_load_and_store() {
    let child_source = SourceIdentity::new(
        "project".into(),
        "package".into(),
        "src/child.w".into(),
        "r1".into(),
    );
    let child = derive_scalar_program(
        &parse_source(
            child_source.clone(),
            "%%start\ni32 value = 7;\n%%end".into(),
            &[],
        )
        .result,
    )
    .program;
    let root_source = SourceIdentity::new(
        "project".into(),
        "package".into(),
        "src/main.w".into(),
        "r1".into(),
    );
    let root = derive_scalar_program(
            &parse_source(
                root_source.clone(),
                "%%start\nmath = namespace package \"src/child.w\";\ni32() read = fn { math.value };\ni32 result = read();\nmath.value = result + 1;\n%%end".into(),
                &[],
            )
            .result,
        )
        .program;
    let namespace_bindings = root
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
    let project = ScalarProject::new(
        vec![
            ScalarModule::new(root_source, root.items, namespace_bindings),
            ScalarModule::new(child_source.clone(), child.items, Vec::new()),
        ],
        vec![
            SourceIdentity::new(
                "project".into(),
                "package".into(),
                "src/main.w".into(),
                "r1".into(),
            ),
            child_source.clone(),
        ],
    );
    let text = emit_scalar_project_llvm(&validate_scalar_project(project))
        .expect("qualified member LLVM")
        .to_text();
    let global = project_global_name(&child_source, "value");
    let main = text.split("define i32 @main").nth(1).expect("main");
    let temporary_store = main.find("store i32 %add, ptr %assignment_value");
    let member_store = main.rfind(&format!("ptr @{global}"));
    assert!(temporary_store.is_some(), "{main}");
    assert!(member_store.is_some(), "{main}");
    assert!(temporary_store < member_store, "{main}");
    let read = text
        .split(&format!(
            "define i32 @{}",
            project_function_name(
                &SourceIdentity::new(
                    "project".into(),
                    "package".into(),
                    "src/main.w".into(),
                    "r1".into(),
                ),
                "read"
            )
        ))
        .nth(1)
        .expect("read");
    assert!(read.contains(&format!("load i32, ptr @{global}")));
}

#[test]
fn verifies_qualified_member_project_module() {
    let child_source = SourceIdentity::new(
        "project".into(),
        "package".into(),
        "src/child.w".into(),
        "r1".into(),
    );
    let child = derive_scalar_program(
        &parse_source(
            child_source.clone(),
            "%%start\ni32 value = 7;\n%%end".into(),
            &[],
        )
        .result,
    )
    .program;
    let root_source = SourceIdentity::new(
        "project".into(),
        "package".into(),
        "src/main.w".into(),
        "r1".into(),
    );
    let root = derive_scalar_program(
        &parse_source(
            root_source.clone(),
            "%%start\nmath = namespace package \"src/child.w\";\ni32 result = math.value;\n%%end"
                .into(),
            &[],
        )
        .result,
    )
    .program;
    let namespace_bindings = root
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
    let validation = validate_scalar_project(ScalarProject::new(
        vec![
            ScalarModule::new(root_source.clone(), root.items, namespace_bindings),
            ScalarModule::new(child_source.clone(), child.items, Vec::new()),
        ],
        vec![root_source, child_source],
    ));
    assert!(emit_scalar_project_llvm(&validation).is_ok());
}

#[test]
fn emits_single_file_unit_bindings_as_effect_only() {
    let source = SourceIdentity::new(
        "project".into(),
        "package".into(),
        "src/main.w".into(),
        "r1".into(),
    );
    let validation = derive_scalar_program(
        &parse_source(
            source,
            "%%start
unit() touch = fn { touch(); };
unit done = touch();
unit() use = fn {
    unit local = touch();
    local;
    local = touch();
    done;
    done = touch();
};
%%end"
                .into(),
            &[],
        )
        .result,
    );
    let text = emit_scalar_llvm(&validation).expect("unit LLVM").to_text();

    assert_eq!(text.matches("call void @touch()").count(), 5);
    assert!(!text.contains("declare i32 @fd_write("));
    assert!(
        text.contains("source_filename = \"src/main.w\"\n\ndefine void @touch()"),
        "{text}"
    );
    assert!(!text.contains("global"), "{text}");
    assert!(!text.contains("alloca"));
    assert!(!text.contains("load"));
    assert!(!text.contains("store"));
}

#[test]
fn emits_all_integer_widths_in_textual_llvm() {
    let source = SourceIdentity::new(
        "project".into(),
        "package".into(),
        "src/integers.w".into(),
        "r1".into(),
    );
    let validation = crate::derive_scalar_program(
            &crate::parse_source(
                source,
                "%%start\ni8 a = 1;\ni16 b = 2;\ni32 c = 3;\ni64 d = 4;\ni128 e = 5;\nu8 f = 6;\nu16 g = 7;\nu32 h = 8;\nu64 i = 9;\nu128 j = 10;\n%%end".into(),
                &[],
            )
            .result,
        );
    let text = emit_scalar_llvm(&validation)
        .expect("integer LLVM")
        .to_text();
    for ty in ["i8", "i16", "i32", "i64", "i128"] {
        assert!(text.contains(&format!("global {ty}")), "{ty}: {text}");
    }
}

#[test]
fn emits_single_file_top_level_unit_reads_and_assignments_as_effect_only() {
    let source = SourceIdentity::new(
        "project".into(),
        "package".into(),
        "src/main.w".into(),
        "r1".into(),
    );
    let validation = derive_scalar_program(
        &parse_source(
            source,
            "%%start
unit() touch = fn { touch(); };
unit marker = touch();
marker;
marker = touch();
%%end"
                .into(),
            &[],
        )
        .result,
    );
    let text = emit_scalar_llvm(&validation)
        .expect("unit top-level LLVM")
        .to_text();

    assert_eq!(text.matches("call void @touch()").count(), 3);
    assert!(!text.contains("@marker"));
    assert!(!text.contains("alloca"));
    assert!(!text.contains("load"));
    assert!(!text.contains("store"));
}

#[test]
fn emits_project_unit_export_reads_and_assignments_as_effect_only() {
    let child_source = SourceIdentity::new(
        "project".into(),
        "package".into(),
        "src/child.w".into(),
        "r1".into(),
    );
    let child = derive_scalar_program(
        &parse_source(
            child_source.clone(),
            "%%start
unit() touch = fn { touch(); };
unit marker = touch();
%%end"
                .into(),
            &[],
        )
        .result,
    )
    .program;
    let root_source = SourceIdentity::new(
        "project".into(),
        "package".into(),
        "src/main.w".into(),
        "r1".into(),
    );
    let root = derive_scalar_program(
        &parse_source(
            root_source.clone(),
            "%%start
child = namespace package \"src/child.w\";
unit() use = fn {
    unit local = child.marker;
    local;
    child.marker = child.touch();
};
unit observed = child.marker;
child.marker = child.touch();
%%end"
                .into(),
            &[],
        )
        .result,
    )
    .program;
    let namespace_bindings = root
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
    let validation = validate_scalar_project(ScalarProject::new(
        vec![
            ScalarModule::new(root_source.clone(), root.items, namespace_bindings),
            ScalarModule::new(child_source.clone(), child.items, Vec::new()),
        ],
        vec![root_source, child_source.clone()],
    ));
    assert!(
        validation.diagnostics.is_empty(),
        "{:?}",
        validation.diagnostics
    );
    let text = emit_scalar_project_llvm(&validation)
        .expect("unit project LLVM")
        .to_text();

    let child_global = project_global_name(&child_source, "marker");
    assert_eq!(text.matches("call void @wosy_fn").count(), 4);
    assert!(!text.contains(&child_global));
    assert!(!text.contains("alloca"));
    assert!(!text.contains("load"));
    assert!(!text.contains("store"));
}

#[test]
fn emits_short_circuit_cfg_for_all_boolean_paths() {
    let cases = [
        ("false_and", "false && right()", "false"),
        ("true_and", "true && right()", "true"),
        ("true_or", "true || right()", "true"),
        ("false_or", "false || right()", "false"),
    ];

    for (name, expression, left) in cases {
        let source = SourceIdentity::new(
            "project".into(),
            "package".into(),
            format!("src/{name}.w"),
            "r1".into(),
        );
        let validation = derive_scalar_program(
            &parse_source(
                source,
                format!(
                    "%%start\nbool() right = fn {{ true }};\nbool result = {expression};\n%%end"
                ),
                &[],
            )
            .result,
        );
        let text = emit_scalar_llvm(&validation)
            .expect("short-circuit LLVM")
            .to_text();
        let main = text.split("define i32 @main").nth(1).expect("main");
        assert_eq!(main.matches("call i1 @right()").count(), 1);
        assert!(main.contains("phi i1"));
        assert!(main.contains(&format!("br i1 {left}")));
        assert!(main.contains("short_circuit.rhs"));
        assert!(main.contains("short_circuit.merge"));
        assert!(
            main.find("br i1").expect("conditional branch")
                < main.find("call i1 @right()").expect("RHS call")
        );
    }
}

#[test]
fn emits_project_short_circuit_cfg_with_qualified_rhs() {
    let child_source = SourceIdentity::new(
        "project".into(),
        "package".into(),
        "src/child.w".into(),
        "r1".into(),
    );
    let child = derive_scalar_program(
        &parse_source(
            child_source.clone(),
            "%%start\nbool() right = fn { true };\n%%end".into(),
            &[],
        )
        .result,
    )
    .program;
    let root_source = SourceIdentity::new(
        "project".into(),
        "package".into(),
        "src/main.w".into(),
        "r1".into(),
    );
    let root = derive_scalar_program(
            &parse_source(
                root_source.clone(),
                "%%start\nchild = namespace package \"src/child.w\";\nbool result = false && child.right();\n%%end".into(),
                &[],
            )
            .result,
        )
        .program;
    let namespace_bindings = root
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
    let validation = validate_scalar_project(ScalarProject::new(
        vec![
            ScalarModule::new(root_source.clone(), root.items, namespace_bindings),
            ScalarModule::new(child_source.clone(), child.items, Vec::new()),
        ],
        vec![root_source, child_source.clone()],
    ));
    let text = emit_scalar_project_llvm(&validation)
        .expect("project short-circuit LLVM")
        .to_text();
    assert!(text.contains("phi i1"));
    assert!(text.contains(&format!(
        "call i1 @{}",
        project_function_name(&child_source, "right")
    )));
}

#[test]
fn emits_wasm32_struct_layout_literals_and_field_addresses() {
    let source = SourceIdentity::new(
        "project".into(),
        "package".into(),
        "src/main.w".into(),
        "r1".into(),
    );
    let validation = derive_scalar_program(
            &parse_source(
                source,
                "%%start\nstruct Pair {\n\tu8 first;\n\tu32 second;\n}\nPair item = { .second = 2; .first = 1; };\nunsafe { *?u8 address = &?item.first; };\n%%end".into(),
                &[],
            )
            .result,
        );
    assert!(
        validation.diagnostics.is_empty(),
        "{:?}",
        validation.diagnostics
    );
    let text = emit_scalar_llvm(&validation)
        .expect("struct LLVM")
        .to_text();
    assert!(
        text.contains("@item = internal global [8 x i8] zeroinitializer"),
        "{text}"
    );
    assert!(text.contains("store i8 1"), "{text}");
    assert!(text.contains("store i32 2"), "{text}");
    assert!(
        text.contains("getelementptr inbounds i8, ptr %struct_literal, i8 4"),
        "{text}"
    );
    assert!(text.contains("ptrtoint"), "{text}");
}

#[test]
fn emits_wasm32_pointer_struct_layout_for_wasi_iovec() {
    let source = SourceIdentity::new(
        "project".into(),
        "package".into(),
        "src/wasi.w".into(),
        "r1".into(),
    );
    let validation = derive_scalar_program(
            &parse_source(
                source,
                "%%start\nstruct WasiIovec {\n\t*?u8 buf;\n\tu32 len;\n}\nWasiIovec iovec = { .buf = null; .len = 2; };\nunsafe { *?u32 len_address = &?iovec.len; };\n%%end".into(),
                &[],
            )
            .result,
        );

    assert!(
        validation.diagnostics.is_empty(),
        "{:?}",
        validation.diagnostics
    );
    let text = emit_scalar_llvm(&validation)
        .expect("WASI iovec LLVM")
        .to_text();
    assert!(
        text.contains("@iovec = internal global [8 x i8] zeroinitializer"),
        "{text}"
    );
    assert!(text.contains("store i32 0"), "{text}");
    assert!(
        text.contains("getelementptr inbounds i8, ptr %struct_literal, i8 4"),
        "{text}"
    );
}

#[test]
fn emits_native64_pointer_struct_layout_literals_and_field_addresses() {
    let source = SourceIdentity::new(
        "project".into(),
        "package".into(),
        "src/main.w".into(),
        "r1".into(),
    );
    let parsed = parse_source(
            source,
            "%%start\nstruct Pair {\n\t*?u8 address;\n\tu32 count;\n}\nPair item = { .address = null; .count = 2; };\nunsafe { *?u32 count_address = &?item.count; };\n%%end".into(),
            &[],
        );
    let validation =
        crate::derive_scalar_program_with_layout(&parsed.result, ScalarTargetLayout::NATIVE64);

    assert!(
        validation.diagnostics.is_empty(),
        "{:?}",
        validation.diagnostics
    );
    let text = emit_scalar_llvm(&validation)
        .expect("native struct LLVM")
        .to_text();
    assert!(
        text.contains("@item = internal global [16 x i8] zeroinitializer"),
        "{text}"
    );
    assert!(text.contains("store i64 0"), "{text}");
    assert!(
        text.contains("getelementptr inbounds i8, ptr %struct_literal, i8 8"),
        "{text}"
    );
    assert!(
        text.contains("ptrtoint (ptr getelementptr inbounds (i8, ptr @item, i8 8) to i64)"),
        "{text}"
    );
}

#[test]
fn lowers_checked_reference_field_addresses_and_callable_transport_at_target_width() {
    for (layout, pointer_type, field_offset) in [
        (ScalarTargetLayout::WASM32, "i32", 4),
        (ScalarTargetLayout::NATIVE64, "i64", 8),
    ] {
        let source = SourceIdentity::new(
            "project".into(),
            "package".into(),
            "src/checked_references.w".into(),
            "r1".into(),
        );
        let parsed = parse_source(
            source,
            "%%start
struct Record {
    *?u8 bytes;
    u32 count;
}
*u32(*u32) transport = fn(value) { value };
*!u32(*!u32) transport_mutable = fn(value) { value };
Record item = { .bytes = null; .count = 2; };
*u32 shared = &item.count;
*!u32 mutable = &!item.count;
*Record record_shared = &item;
u32 shared_value = *shared;
u32 mutable_value = *mutable;
u32 field_value = (*record_shared).count;
*u32 forwarded = transport(shared);
*!u32 mutable_forwarded = transport_mutable(mutable);
u32 forwarded_value = *transport(shared);
u32 mutable_forwarded_value = *transport_mutable(mutable);
forwarded;
mutable_forwarded;
%%end"
                .into(),
            &[],
        );
        let validation = crate::derive_scalar_program_with_layout(&parsed.result, layout);
        assert!(
            validation.diagnostics.is_empty(),
            "{:?}",
            validation.diagnostics
        );

        let text = emit_scalar_llvm(&validation)
            .expect("checked-reference LLVM")
            .to_text();

        assert!(
            text.contains(&format!(
                "define {pointer_type} @transport({pointer_type} %value)"
            )),
            "{text}"
        );
        assert!(
            text.contains(&format!(
                "define {pointer_type} @transport_mutable({pointer_type} %value)"
            )),
            "{text}"
        );
        assert!(
            text.contains(&format!("call {pointer_type} @transport({pointer_type}")),
            "{text}"
        );
        assert!(
            text.contains(&format!(
                "call {pointer_type} @transport_mutable({pointer_type}"
            )),
            "{text}"
        );
        assert!(
            text.contains(&format!(
                "getelementptr inbounds i8, ptr %struct_literal, i8 {field_offset}"
            )),
            "{text}"
        );
        assert!(
                text.contains(&format!(
                    "ptrtoint (ptr getelementptr inbounds (i8, ptr @item, i8 {field_offset}) to {pointer_type})"
                )),
                "{text}"
            );
        assert!(text.contains(&format!("inttoptr {pointer_type}")), "{text}");
        assert!(text.contains("load i32"), "{text}");
    }
}

#[test]
fn emits_imported_callable_result_field_read_for_target_layouts() {
    for (layout, pointer_type, field_offset) in [
        (ScalarTargetLayout::WASM32, "i32", 4),
        (ScalarTargetLayout::NATIVE64, "i64", 4),
    ] {
        let child_source = SourceIdentity::new(
            "project".into(),
            "package".into(),
            "src/child.w".into(),
            "r1".into(),
        );
        let child = crate::derive_scalar_program_with_layout(
                &parse_source(
                    child_source.clone(),
                    "%%start\nstruct Pair {\n\tu8 first;\n\tu32 second;\n}\n*Pair(*Pair) forward = fn(value) { value };\n%%end".into(),
                    &[],
                )
                .result,
                layout,
            )
            .program;
        let root_source = SourceIdentity::new(
            "project".into(),
            "package".into(),
            "src/main.w".into(),
            "r1".into(),
        );
        let root = crate::derive_scalar_program_with_layout(
                &parse_source(
                    root_source.clone(),
                    "%%start\nchild = namespace package \"src/child.w\";\nchild.Pair item = { .first = 1; .second = 2; };\n*child.Pair pair = &item;\nu32 forwarded_read = (*child.forward(pair)).second;\n%%end".into(),
                    &[],
                )
                .result,
                layout,
            )
            .program;
        let namespace = match &root.items[0] {
            crate::ScalarItem::Namespace(namespace) => (namespace.binding.clone(), namespace.span),
            _ => panic!("namespace item"),
        };
        let forward = project_function_name(&child_source, "forward");
        let validation = validate_scalar_project(ScalarProject::new(
            vec![
                ScalarModule::from_program(
                    root,
                    vec![crate::ScalarNamespaceBinding {
                        binding: namespace.0,
                        target: child_source.clone(),
                        span: namespace.1,
                    }],
                ),
                ScalarModule::from_program(child, Vec::new()),
            ],
            vec![root_source, child_source],
        ));
        let text = emit_scalar_project_llvm(&validation)
            .expect("imported struct LLVM")
            .to_text();
        assert!(text.contains("[8 x i8]"), "{text}");
        assert!(
            text.contains(&format!(
                "getelementptr inbounds i8, ptr %deref, i8 {field_offset}"
            )),
            "{text}"
        );
        assert!(
            text.contains(&format!("call {pointer_type} @{forward}({pointer_type}")),
            "{text}"
        );
        assert!(text.contains(&format!("inttoptr {pointer_type}")), "{text}");
    }
}

#[test]
fn emits_direct_local_struct_addresses_and_loaded_pointer_bindings_for_externs() {
    let source = SourceIdentity::new(
        "project".into(),
        "package".into(),
        "src/main.w".into(),
        "r1".into(),
    );
    let program = derive_scalar_program(
            &parse_source(
                source.clone(),
                "%%start\nstruct Outer {\n\tPair pair;\n}\nstruct Pair {\n\tu32 value;\n\tu8 tag;\n}\nraw = extern wasm \"env\" { unit(*?Pair) touch; unit(*?u8) inspect; };\nunit(Outer) forward = fn(value) {\n\tOuter local = { .pair = { .value = 2; .tag = 1; }; };\n\tlocal;\n\tunsafe {\n\t\t*?Outer outer = &?value;\n\t\t*?Pair pointer = &?(*outer).pair;\n\t\traw.touch(pointer);\n\t\t*?u8 field = &?(*pointer).tag;\n\t\traw.inspect(field);\n\t};\n};\n%%end"
                    .into(),
                &[],
            )
            .result,
        )
        .program;
    let validation = validate_scalar_project(ScalarProject::new(
        vec![ScalarModule::from_program(program, Vec::new())],
        vec![source.clone()],
    ));
    assert!(
        validation.diagnostics.is_empty(),
        "{:?}",
        validation.diagnostics
    );
    let text = emit_scalar_project_llvm(&validation)
        .expect("raw struct pointer LLVM")
        .to_text();
    let forward = text
        .split(&format!(
            "define void @{}",
            project_function_name(&source, "forward")
        ))
        .nth(1)
        .expect("forward");

    assert!(text.contains("declare void @wosy_extern__"), "{text}");
    assert!(text.contains("wasm-import-name\"=\"touch"), "{text}");
    assert!(text.contains("wasm-import-name\"=\"inspect"), "{text}");
    assert!(forward.contains("alloca [8 x i8]"), "{forward}");
    assert!(forward.contains("store [8 x i8]"), "{forward}");
    assert!(forward.contains("%local = alloca [8 x i8]"), "{forward}");
    assert!(
        forward.contains("store [8 x i8] %struct_value") && forward.contains("ptr %local"),
        "{forward}"
    );
    assert!(forward.contains("ptrtoint ptr %pair"), "{forward}");
    assert!(
        forward.contains("getelementptr inbounds i8, ptr %deref") && forward.contains("i8 4"),
        "{forward}"
    );
    assert!(forward.contains("load i32, ptr %pointer"), "{forward}");
    assert!(forward.contains("call void @wosy_extern__"), "{forward}");
    assert!(forward.contains("load i32, ptr %field"), "{forward}");
    assert!(forward.contains("call void @wosy_extern__"), "{forward}");
}

#[test]
fn emits_project_struct_storage_address_for_generic_extern() {
    let source = SourceIdentity::new(
        "project".into(),
        "package".into(),
        "src/main.w".into(),
        "r1".into(),
    );
    let program = derive_scalar_program(
            &parse_source(
                source.clone(),
                "%%start\nstruct Outer {\n\tPair pair;\n}\nstruct Pair {\n\tu8 tag;\n\tu32 value;\n}\nraw = extern wasm \"env\" { unit(*?Pair) touch; };\nOuter value = { .pair = { .tag = 1; .value = 2; }; };\nunsafe { *?Pair pointer = &?value.pair; raw.touch(pointer); };\n%%end"
                    .into(),
                &[],
            )
            .result,
        )
        .program;
    let validation = validate_scalar_project(ScalarProject::new(
        vec![ScalarModule::from_program(program, Vec::new())],
        vec![source.clone()],
    ));
    assert!(
        validation.diagnostics.is_empty(),
        "{:?}",
        validation.diagnostics
    );
    let text = emit_scalar_project_llvm(&validation)
        .expect("project raw struct pointer LLVM")
        .to_text();
    let main = text.split("define i32 @main").nth(1).expect("main");
    let value = project_global_name(&source, "value");

    assert!(text.contains("declare void @wosy_extern__"), "{text}");
    assert!(text.contains("wasm-import-name\"=\"touch"), "{text}");
    assert!(
        text.contains(&format!("@{value} = internal global [8 x i8]")),
        "{text}"
    );
    assert!(
        main.contains(&format!("ptrtoint (ptr @{value} to i32)")),
        "{main}"
    );
    assert!(main.contains("load i32, ptr %pointer"), "{main}");
    assert!(main.contains("call void @wosy_extern__"), "{main}");
}

#[test]
fn emits_utf8_literal_as_private_struct_storage() {
    let std_source = SourceIdentity::new(
        "project".into(),
        "stdlib".into(),
        "src/bootstrap.w".into(),
        "r1".into(),
    );
    let std = derive_scalar_program(
        &parse_source(
            std_source.clone(),
            "%%start
struct utf8 {
    *?u8 data;
    u64 length;
}
%%end"
                .into(),
            &[],
        )
        .result,
    )
    .program;
    let source = SourceIdentity::new(
        "project".into(),
        "package".into(),
        "src/main.w".into(),
        "r1".into(),
    );
    let root = derive_scalar_program(
        &parse_source(
            source.clone(),
            "%%start
text = namespace stdlib \"src/bootstrap.w\";
text.utf8 value = \"hé\";
%%end"
                .into(),
            &[],
        )
        .result,
    )
    .program;
    let validation = validate_scalar_project(ScalarProject::new(
        vec![
            ScalarModule::from_program(
                root,
                vec![crate::ScalarNamespaceBinding {
                    binding: "text".into(),
                    target: std_source.clone(),
                    span: wosy_syntax::ByteSpan::new(0, 0),
                }],
            ),
            ScalarModule::from_program(std, Vec::new()),
        ],
        vec![source, std_source],
    ));
    assert!(
        validation.diagnostics.is_empty(),
        "{:?}",
        validation.diagnostics
    );
    let text = emit_scalar_project_llvm(&validation)
        .expect("UTF-8 LLVM")
        .to_text();
    assert!(
        text.contains("@wosy_utf8_literal_0 = private constant [3 x i8]"),
        "{text}"
    );
    assert!(text.contains("store i64 3"), "{text}");
    assert!(text.contains("i8 0"), "{text}");
}

#[test]
fn emits_fixed_array_callable_transport_with_typed_address_storage() {
    let source = SourceIdentity::new(
        "project".into(),
        "package".into(),
        "src/main.w".into(),
        "r1".into(),
    );
    let validation = derive_scalar_program(
            &parse_source(
                source,
                "%%start\nu8[2](u8[2]) transport = fn(bytes) { bytes };\nunit(u8) observe = fn(value) { value; };\nunit() run = fn { u8[2] bytes = [7, 9]; transport(bytes); observe(1); };\n%%end".into(),
                &[],
            )
            .result,
        );
    assert!(
        validation.diagnostics.is_empty(),
        "{:?}",
        validation.diagnostics
    );
    let text = emit_scalar_llvm(&validation)
        .expect("fixed array callable LLVM")
        .to_text();
    let transport = text
        .split("define ptr @transport(ptr %bytes)")
        .nth(1)
        .expect("transport function");
    let run = text
        .split("define void @run()")
        .nth(1)
        .expect("run function");

    assert!(text.contains("[2 x i8]"), "{text}");
    assert!(run.contains("%array_literal = alloca [2 x i8]"), "{run}");
    let first = run
        .find("store i8 7, ptr %array_element")
        .expect("first element");
    let second = run
        .find("store i8 9, ptr %array_element")
        .expect("second element");
    assert!(first < second, "{run}");
    assert!(run.contains("%bytes = alloca [2 x i8]"), "{run}");
    assert!(run.contains("call ptr @transport(ptr %bytes)"), "{run}");
    assert!(transport.contains("ret ptr %bytes"), "{transport}");
}

#[test]
fn emits_valid_nested_struct_array_storage_with_final_layout_offsets() {
    let source = SourceIdentity::new(
        "project".into(),
        "package".into(),
        "src/main.w".into(),
        "r1".into(),
    );
    let validation = derive_scalar_program(
        &parse_source(
            source,
            "%%start
struct Leaf {
    u16 value;
}
struct Grid {
    Leaf[2][2] leaves;
    u8 marker;
}
raw = extern wasm \"env\" { unit(*?u8) inspect; };
unit(Grid) store_grid = fn(value) {
    value;
};
unit(*?Grid) marker_address = fn(pointer) {
    unsafe {
        *?u8 marker = &?(*pointer).marker;
        raw.inspect(marker);
    };
};
%%end"
                .into(),
            &[],
        )
        .result,
    );
    assert!(
        validation.diagnostics.is_empty(),
        "{:?}",
        validation.diagnostics
    );
    let leaf = validation
        .program
        .structs
        .iter()
        .find(|structure| structure.name == "Leaf")
        .expect("Leaf layout");
    let grid = validation
        .program
        .structs
        .iter()
        .find(|structure| structure.name == "Grid")
        .expect("Grid layout");

    assert_eq!(
        leaf.layout
            .as_ref()
            .map(|layout| (layout.size, layout.alignment)),
        Some((2, 2))
    );
    assert_eq!(leaf.fields[0].offset, Some(0));
    assert_eq!(
        grid.layout
            .as_ref()
            .map(|layout| (layout.size, layout.alignment)),
        Some((10, 2))
    );
    assert_eq!(grid.fields[0].offset, Some(0));
    assert_eq!(grid.fields[1].offset, Some(8));

    let text = emit_scalar_llvm(&validation)
        .expect("nested aggregate LLVM")
        .to_text();

    assert!(text.contains("alloca [10 x i8]"), "{text}");
    assert!(
        text.contains("getelementptr inbounds i8, ptr %deref, i8 8"),
        "{text}"
    );
}

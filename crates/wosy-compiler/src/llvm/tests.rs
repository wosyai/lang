use std::collections::BTreeMap;

use inkwell::context::Context;
use inkwell::module::Linkage;

use super::emit_scalar_llvm;
use super::emit_scalar_project_llvm;
use super::function_type;
use super::generic_specialization_name;
use super::ownership;
use super::project_function_name;
use super::project_global_name;
use super::storage_type;
use super::{LlvmFunction, LlvmFunctionAttributes, LlvmPartition, LlvmValueType};
use crate::scalar::{CfgPointKind, CoreOperationId, ReleaseTarget, ScalarOverloadSelection};
use crate::{
    derive_scalar_program, derive_scalar_program_with_layout, parse_source,
    validate_scalar_project, ScalarBlockItem, ScalarExpression, ScalarItem, ScalarModule,
    ScalarOutput, ScalarOutputSequence, ScalarProject, ScalarTargetLayout, ScalarType,
};
use wosy_syntax::{ByteSpan, SourceIdentity};

#[test]
fn struct_backed_capture_without_counted_route_reports_connected_error_on_both_targets() {
    let source = SourceIdentity::new(
        "project".into(),
        "package".into(),
        "src/main.w".into(),
        "r1".into(),
    );
    let text = "%%start\nstruct Holder { u8[] items; }\nunit() run = fn { u8[2] fixed = [4, 5]; u8[] backing = fixed; Holder held = { .items = backing; }; u8() getter = fn { held.items[0] }; getter(); };\nrun();\n%%end";
    for layout in [ScalarTargetLayout::NATIVE64, ScalarTargetLayout::WASM32] {
        let parsed = parse_source(source.clone(), text.to_owned(), &[]);
        assert!(parsed.diagnostics.is_empty(), "{:?}", parsed.diagnostics);
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
        for result in [
            emit_scalar_llvm(&single).map(|llvm| llvm.to_text()),
            emit_scalar_project_llvm(&project).map(|llvm| llvm.to_text()),
        ] {
            let error = result.expect_err("struct backing without a counted route must fail");
            assert!(
                error.contains("lacks a connected counted cell route"),
                "unexpected error: {error}"
            );
        }
    }
}

#[test]
fn global_callable_field_calls_from_function_verify_on_both_layouts() {
    let source = SourceIdentity::new(
        "project".into(),
        "package".into(),
        "src/main.w".into(),
        "r1".into(),
    );
    let text = "%%start\nstruct Dispatch { u8() action; }\nu8 seed = 7;\nDispatch global_field = { .action = fn { seed }; };\nu8() invoke = fn { u8 first = global_field.action(); Dispatch copy = global_field; u8 second = copy.action(); first + second };\nu8 result = invoke();\n%%end";
    for layout in [ScalarTargetLayout::NATIVE64, ScalarTargetLayout::WASM32] {
        let parsed = parse_source(source.clone(), text.to_owned(), &[]);
        assert!(parsed.diagnostics.is_empty(), "{:?}", parsed.diagnostics);
        let single = derive_scalar_program_with_layout(&parsed.result, layout);
        assert!(single.diagnostics.is_empty(), "{:?}", single.diagnostics);
        assert!(
            !single.program.terminal_owners.is_empty(),
            "terminal owners: {:?}; top-level exits: {:?}",
            single.program.terminal_owners,
            single.program.top_level_reference_cfg.holder_exits
        );
        let project = validate_scalar_project(ScalarProject::new(
            vec![ScalarModule::from_program(
                single.program.clone(),
                Vec::new(),
            )],
            vec![source.clone()],
        ));
        assert!(project.diagnostics.is_empty(), "{:?}", project.diagnostics);
        for result in [
            emit_scalar_llvm(&single),
            emit_scalar_project_llvm(&project),
        ] {
            let llvm = result.expect("global callable field IR").to_text();
            assert!(llvm.contains("@__wosy_begin_graph_mutation"), "{llvm}");
            assert!(llvm.contains("@__wosy_end_graph_mutation"), "{llvm}");
            assert!(
                llvm.contains("terminal_holder_environment"),
                "missing program-terminal owner release: {llvm}"
            );
        }
    }
}

#[test]
fn independent_project_global_terminal_owners_use_their_declaring_sources() {
    let main_source = SourceIdentity::new(
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
    let main_text = "%%start\nchild = namespace app \"src/child.w\";\nstruct Dispatch { u8() action; }\nu8 seed = 7;\nDispatch own = { .action = fn { seed }; };\nu8 result = child.read() + own.action();\n%%end";
    let child_text = "%%start\nstruct Dispatch { u8() action; }\nu8 seed = 8;\nDispatch own = { .action = fn { seed }; };\nu8() read = fn { own.action() };\n%%end";
    for layout in [ScalarTargetLayout::NATIVE64, ScalarTargetLayout::WASM32] {
        let main = derive_scalar_program_with_layout(
            &parse_source(main_source.clone(), main_text.to_owned(), &[]).result,
            layout,
        );
        let child = derive_scalar_program_with_layout(
            &parse_source(child_source.clone(), child_text.to_owned(), &[]).result,
            layout,
        );
        let span = match &main.program.items[0] {
            ScalarItem::Namespace(namespace) => namespace.span,
            _ => panic!("namespace"),
        };
        let project = validate_scalar_project(ScalarProject::new(
            vec![
                ScalarModule::from_program(
                    main.program,
                    vec![crate::scalar::ScalarNamespaceBinding {
                        binding: "child".into(),
                        target: child_source.clone(),
                        span,
                    }],
                ),
                ScalarModule::from_program(child.program, Vec::new()),
            ],
            vec![main_source.clone(), child_source.clone()],
        ));
        assert!(project.diagnostics.is_empty(), "{:?}", project.diagnostics);
        let child_module = project
            .project
            .modules
            .iter()
            .find(|module| module.source == child_source)
            .unwrap();
        let reader = child_module
            .items
            .iter()
            .find_map(|item| match item {
                ScalarItem::Function(function) if function.name == "read" => Some(function),
                _ => None,
            })
            .unwrap();
        assert_eq!(reader.reference_cfg.global_callable_input_paths.len(), 1);
        let incoming = &reader.reference_cfg.global_callable_input_paths[0];
        assert_eq!(incoming.call.source, main_source);
        assert!(incoming.holders.iter().all(|holder| match &holder.holder {
            crate::scalar::CallableHolderPlace::Binding(binding) => binding.source == main_source,
            crate::scalar::CallableHolderPlace::Projection(place) =>
                place.binding.source == main_source,
            _ => false,
        }));
        assert_eq!(
            project.project.terminal_owners.len(),
            2,
            "{:?}; exits: {:?}",
            project.project.terminal_owners,
            project
                .project
                .modules
                .iter()
                .map(|module| &module.top_level_reference_cfg.holder_exits)
                .collect::<Vec<_>>()
        );
        let llvm = emit_scalar_project_llvm(&project).unwrap().to_text();
        assert_eq!(
            llvm.matches("extractvalue { ptr, ptr } %terminal_holder")
                .count(),
            2,
            "{llvm}"
        );
    }
}

#[test]
fn global_fixed_callable_elements_have_distinct_terminal_slots_on_both_layouts() {
    let source = SourceIdentity::new(
        "project".into(),
        "package".into(),
        "src/main.w".into(),
        "r1".into(),
    );
    let text = "%%start\nu8 seed = 7;\nu8()[2] global_actions = [fn { seed }, fn { seed + 1 }];\nu8 first = global_actions[0]();\n%%end";
    for layout in [ScalarTargetLayout::NATIVE64, ScalarTargetLayout::WASM32] {
        let parsed = parse_source(source.clone(), text.to_owned(), &[]);
        let single = derive_scalar_program_with_layout(&parsed.result, layout);
        assert!(single.diagnostics.is_empty(), "{:?}", single.diagnostics);
        assert_eq!(
            single.program.terminal_owners.len(),
            2,
            "{:?}",
            single.program.terminal_owners
        );
        assert_ne!(
            single.program.terminal_owners[0].holder,
            single.program.terminal_owners[1].holder
        );
        let llvm = emit_scalar_llvm(&single).unwrap().to_text();
        assert_eq!(
            llvm.matches("extractvalue { ptr, ptr } %terminal_holder")
                .count(),
            2,
            "{llvm}"
        );
    }
}

#[test]
fn graph_mutation_primitive_verifies_and_pairs_on_each_target() {
    for layout in [ScalarTargetLayout::NATIVE64, ScalarTargetLayout::WASM32] {
        let context = Context::create();
        let module = context.create_module("transaction");
        let builder = context.create_builder();
        ownership::emit_graph_mutation_primitive(&context, &module, layout, false).unwrap();
        let caller = module.add_function(
            "transaction_test_caller",
            context.void_type().fn_type(&[], false),
            Some(Linkage::Internal),
        );
        builder.position_at_end(context.append_basic_block(caller, "entry"));
        ownership::begin_graph_mutation(&builder, &module).unwrap();
        ownership::end_graph_mutation(&builder, &module).unwrap();
        builder.build_return(None).unwrap();
        module.verify().unwrap();
        let ir = module.print_to_string().to_string();
        assert_eq!(
            ir.matches("define internal void @__wosy_begin_graph_mutation(")
                .count(),
            1,
            "{ir}"
        );
        assert_eq!(
            ir.matches("define internal void @__wosy_end_graph_mutation(")
                .count(),
            1,
            "{ir}"
        );
        assert_eq!(
            ir.matches("call void @__wosy_begin_graph_mutation()")
                .count(),
            1,
            "{ir}"
        );
        assert_eq!(
            ir.matches("call void @__wosy_end_graph_mutation()").count(),
            1,
            "{ir}"
        );
        assert!(
            ir.find("call void @__wosy_begin_graph_mutation()")
                < ir.find("call void @__wosy_end_graph_mutation()"),
            "{ir}"
        );
        if layout == ScalarTargetLayout::NATIVE64 {
            assert_eq!(
                ir.matches("@__wosy_graph_mutation_lock = private global i32 0")
                    .count(),
                1,
                "{ir}"
            );
            assert!(
                ir.contains(
                    "cmpxchg ptr @__wosy_graph_mutation_lock, i32 0, i32 1 acquire monotonic"
                ),
                "{ir}"
            );
            assert!(ir.contains("br i1 %graph_mutation_locked, label %graph_mutation.acquired, label %graph_mutation.attempt"), "{ir}");
            assert!(
                ir.contains("store atomic i32 0, ptr @__wosy_graph_mutation_lock release, align 4"),
                "{ir}"
            );
        } else {
            assert!(!ir.contains("@__wosy_graph_mutation_lock"), "{ir}");
            assert!(
                !ir.contains("cmpxchg") && !ir.contains("store atomic"),
                "{ir}"
            );
        }
    }
}

#[test]
fn captured_environment_cells_and_lengths_initialize_in_one_transaction() {
    let source = SourceIdentity::new(
        "project".into(),
        "package".into(),
        "src/main.w".into(),
        "r1".into(),
    );
    let text = "%%start\nu8(u64) read = fn(count) { u8[count] bytes; bytes[0] = 7; u8() getter = fn { bytes[0] }; getter() };\n%%end";
    for layout in [ScalarTargetLayout::NATIVE64, ScalarTargetLayout::WASM32] {
        let parsed = parse_source(source.clone(), text.into(), &[]);
        assert!(parsed.diagnostics.is_empty(), "{:?}", parsed.diagnostics);
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
        for ir in [
            emit_scalar_llvm(&single).unwrap().to_text(),
            emit_scalar_project_llvm(&project).unwrap().to_text(),
        ] {
            let cell = ir
                .find("environment_cell = getelementptr")
                .expect("cell address");
            let start = ir[..cell]
                .rfind("call void @__wosy_begin_graph_mutation()")
                .expect("capture transaction");
            let length = ir[cell..]
                .find("environment_length = getelementptr")
                .expect("length address")
                + cell;
            let end = ir[length..]
                .find("call void @__wosy_end_graph_mutation()")
                .expect("capture release")
                + length;
            assert!(start < cell && cell < length && length < end, "{ir}");
            assert_eq!(
                ir[start..end]
                    .matches("call void @__wosy_begin_graph_mutation()")
                    .count(),
                1,
                "{ir}"
            );
        }
    }
}

#[test]
fn fixed_callable_widening_retains_both_elements_before_binding_publication() {
    let source = SourceIdentity::new(
        "project".into(),
        "package".into(),
        "src/main.w".into(),
        "r1".into(),
    );
    let text = "%%start\nunit() inspect = fn { u8 first = 1; u8 second = 2; (u8())[2] fixed = [fn { first }, fn { second }]; (u8())[] copied = fixed; fixed[0]; copied[1]; };\n%%end";
    for layout in [ScalarTargetLayout::NATIVE64, ScalarTargetLayout::WASM32] {
        let parsed = parse_source(source.clone(), text.into(), &[]);
        assert!(parsed.diagnostics.is_empty(), "{:?}", parsed.diagnostics);
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
        for ir in [
            emit_scalar_llvm(&single).unwrap().to_text(),
            emit_scalar_project_llvm(&project).unwrap().to_text(),
        ] {
            let begin = ir
                .find("fixed_copy_retained_environment")
                .expect("both copied holders retained");
            let transaction = ir[..begin]
                .rfind("call void @__wosy_begin_graph_mutation()")
                .expect("copy lock");
            let release = ir[begin..]
                .find("call void @__wosy_end_graph_mutation()")
                .expect("copy release")
                + begin;
            assert!(transaction < begin && begin < release, "{ir}");
            assert!(
                ir[transaction..release].contains("fixed_copy_walk_index"),
                "{ir}"
            );
        }
    }
}

#[test]
fn fixed_callable_widening_replacement_retains_before_old_backing_exit() {
    let source = SourceIdentity::new(
        "project".into(),
        "package".into(),
        "src/main.w".into(),
        "r1".into(),
    );
    let text = "%%start\nunit() inspect = fn { u8 first = 1; u8 second = 2; (u8())[2] fixed = [fn { first }, fn { second }]; (u8())[] copied = fixed; copied = fixed; fixed[0]; copied[1]; };\n%%end";
    for layout in [ScalarTargetLayout::NATIVE64, ScalarTargetLayout::WASM32] {
        let parsed = parse_source(source.clone(), text.into(), &[]);
        assert!(parsed.diagnostics.is_empty(), "{:?}", parsed.diagnostics);
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
        for ir in [
            emit_scalar_llvm(&single).unwrap().to_text(),
            emit_scalar_project_llvm(&project).unwrap().to_text(),
        ] {
            let retains = ir
                .lines()
                .filter(|line| {
                    line.contains("fixed_copy_retained_environment")
                        && line.contains("= extractvalue")
                })
                .count();
            assert_eq!(retains, 2, "{ir}");
            let old = ir
                .find("call void @__wosy_core_free(ptr %callable_array_allocation_base)")
                .expect("old backing released");
            let start = ir[..old]
                .rfind("call void @__wosy_begin_graph_mutation()")
                .expect("replacement transaction");
            let new = ir[old..]
                .find("fixed_copy_retained_environment")
                .expect("new copy retains")
                + old;
            let end = ir[new..]
                .find("call void @__wosy_end_graph_mutation()")
                .expect("replacement unlock")
                + new;
            assert!(start < old && old < new && new < end, "{ir}");
        }
    }
}

#[test]
fn fixed_callable_widening_branch_and_loop_retain_before_scope_exit() {
    let source = SourceIdentity::new(
        "project".into(),
        "package".into(),
        "src/main.w".into(),
        "r1".into(),
    );
    let text = "%%start\nunit() inspect = fn { u8 first = 1; u8 second = 2; (u8())[2] fixed = [fn { first }, fn { second }]; if (true) { (u8())[] branch = fixed; branch[0]; }; u64 index = 0; while (index < 2) { (u8())[] loop_copy = fixed; loop_copy[1]; index = index + 1; } fixed[0]; };\n%%end";
    for layout in [ScalarTargetLayout::NATIVE64, ScalarTargetLayout::WASM32] {
        let parsed = parse_source(source.clone(), text.into(), &[]);
        assert!(parsed.diagnostics.is_empty(), "{:?}", parsed.diagnostics);
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
        for ir in [
            emit_scalar_llvm(&single).unwrap().to_text(),
            emit_scalar_project_llvm(&project).unwrap().to_text(),
        ] {
            let retain_walks = ir
                .lines()
                .filter(|line| {
                    line.contains("= extractvalue")
                        && line.contains("fixed_copy_retained_environment")
                })
                .count();
            assert_eq!(retain_walks, 2, "{ir}");
            assert!(
                ir.matches("call void @__wosy_core_free(ptr %callable_array_allocation_base")
                    .count()
                    >= 2,
                "{ir}"
            );
        }
    }
}

#[test]
fn copied_callable_struct_field_and_overwrite_share_holder_transactions() {
    let source = SourceIdentity::new(
        "project".into(),
        "package".into(),
        "src/main.w".into(),
        "r1".into(),
    );
    let text = "%%start\nstruct copy Box { u8() field; }\nunit() inspect = fn { u8 value = 3; Box original = { .field = fn { value }; }; Box copied = original; copied.field = fn { value }; original.field(); copied.field(); };\n%%end";
    for layout in [ScalarTargetLayout::NATIVE64, ScalarTargetLayout::WASM32] {
        let parsed = parse_source(source.clone(), text.into(), &[]);
        assert!(parsed.diagnostics.is_empty(), "{:?}", parsed.diagnostics);
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
        for ir in [
            emit_scalar_llvm(&single).unwrap().to_text(),
            emit_scalar_project_llvm(&project).unwrap().to_text(),
        ] {
            let copy = ir
                .find("ptr %copied, align")
                .expect("copied struct publication");
            let begin = ir[..copy]
                .rfind("call void @__wosy_begin_graph_mutation()")
                .expect("copy lock");
            let end = ir[copy..]
                .find("call void @__wosy_end_graph_mutation()")
                .expect("copy unlock")
                + copy;
            assert!(
                ir[copy..end]
                    .contains("br i1 %environment_present, label %environment.retain.present"),
                "{}",
                &ir[begin..end]
            );
            assert!(
                ir[end..].contains("%environment_holder_retained = add i64"),
                "{ir}"
            );
            assert!(ir[begin..copy].contains("ptr %original, align 1"), "{ir}");
            assert!(
                ir[end..].contains("%environment_holder_dropped = sub i64"),
                "{ir}"
            );
        }
    }
}

#[test]
fn graph_mutation_primitive_is_one_per_single_or_combined_project_module() {
    let root = SourceIdentity::new(
        "project".into(),
        "package".into(),
        "src/main.w".into(),
        "r1".into(),
    );
    let child = SourceIdentity::new(
        "project".into(),
        "package".into(),
        "src/child.w".into(),
        "r1".into(),
    );
    for layout in [ScalarTargetLayout::NATIVE64, ScalarTargetLayout::WASM32] {
        let parsed = parse_source(
            root.clone(),
            "%%start\ni32() main_value = fn { 42 };\n%%end".into(),
            &[],
        );
        let single = derive_scalar_program_with_layout(&parsed.result, layout);
        assert!(single.diagnostics.is_empty(), "{:?}", single.diagnostics);
        let child_parsed = parse_source(
            child.clone(),
            "%%start\ni32() child_value = fn { 7 };\n%%end".into(),
            &[],
        );
        let child_program = derive_scalar_program_with_layout(&child_parsed.result, layout);
        assert!(
            child_program.diagnostics.is_empty(),
            "{:?}",
            child_program.diagnostics
        );
        let project = validate_scalar_project(ScalarProject::new(
            vec![
                ScalarModule::from_program(single.program.clone(), Vec::new()),
                ScalarModule::from_program(child_program.program, Vec::new()),
            ],
            vec![child.clone(), root.clone()],
        ));
        assert!(project.diagnostics.is_empty(), "{:?}", project.diagnostics);
        for ir in [
            emit_scalar_llvm(&single).unwrap().to_text(),
            emit_scalar_project_llvm(&project).unwrap().to_text(),
        ] {
            assert_eq!(
                ir.matches("define internal void @__wosy_begin_graph_mutation(")
                    .count(),
                1,
                "{ir}"
            );
            assert_eq!(
                ir.matches("define internal void @__wosy_end_graph_mutation(")
                    .count(),
                1,
                "{ir}"
            );
            assert_eq!(
                ir.matches("@__wosy_graph_mutation_lock = private global")
                    .count(),
                usize::from(layout == ScalarTargetLayout::NATIVE64),
                "{ir}"
            );
            assert_eq!(
                ir.matches("call void @__wosy_begin_graph_mutation()")
                    .count(),
                0,
                "{ir}"
            );
            assert_eq!(
                ir.matches("call void @__wosy_end_graph_mutation()").count(),
                0,
                "{ir}"
            );
        }
    }
}

#[test]
fn selected_read_and_local_replacement_publish_under_one_graph_boundary() {
    let source = SourceIdentity::new(
        "project".into(),
        "package".into(),
        "src/main.w".into(),
        "r1".into(),
    );
    let text = "%%start\nstruct copy Twin { u8(u8) write; u8(u64) read; }\nTwin(u64) make = fn(count) { (u8())[count] slots; u8 first = 1; slots[0] = fn { first }; u8() kept = slots[0]; (u8())[count] next; u8 second = 2; next[0] = fn { second }; slots = next; Twin result = { .write = fn(value) { first = value; first }; .read = fn(index) { if (index == 0) { kept() } else { kept() } }; }; result };\nmake(2);\n%%end";
    for layout in [ScalarTargetLayout::NATIVE64, ScalarTargetLayout::WASM32] {
        let parsed = parse_source(source.clone(), text.into(), &[]);
        assert!(parsed.diagnostics.is_empty(), "{:?}", parsed.diagnostics);
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
        for ir in [
            emit_scalar_llvm(&single).unwrap().to_text(),
            emit_scalar_project_llvm(&project).unwrap().to_text(),
        ] {
            assert_eq!(
                ir.matches("@__wosy_graph_mutation_lock = private global")
                    .count(),
                usize::from(layout == ScalarTargetLayout::NATIVE64)
            );
            let read = ir.find("%read_callable = load").expect("selected read");
            let begin = ir[..read]
                .rfind("call void @__wosy_begin_graph_mutation()")
                .expect("read acquire");
            let current_length = ir[begin..read]
                .find("selected_current_length")
                .expect("shared length loaded under read transaction")
                + begin;
            let current_backing = ir[begin..read]
                .find("selected_current_backing")
                .expect("replaceable backing loaded under read transaction")
                + begin;
            assert!(
                begin < current_length
                    && current_length < current_backing
                    && current_backing < read,
                "{ir}"
            );
            let done = ir[read..].find("indexed_read.done:").expect("read join") + read;
            let end = ir[done..]
                .find("call void @__wosy_end_graph_mutation()")
                .expect("read release")
                + done;
            let retained = ir[end..]
                .find("environment_holder_retained")
                .expect("selected retain")
                + end;
            assert!(
                begin < read && read < done && done < end && end < retained,
                "{ir}"
            );
            assert!(
                ir[end..retained].contains("br label %indexed_read.done"),
                "{ir}"
            );
            assert!(
                !ir[begin + 1..end].contains("call void @__wosy_begin_graph_mutation()"),
                "{ir}"
            );
            assert_eq!(
                ir.matches("call void @__wosy_begin_graph_mutation()")
                    .count(),
                ir.matches("call void @__wosy_end_graph_mutation()").count(),
                "{ir}"
            );
        }
    }
}

#[test]
fn acyclic_captured_callable_value_remains_verifiable_on_both_targets() {
    let source = SourceIdentity::new(
        "project".into(),
        "package".into(),
        "src/main.w".into(),
        "r1".into(),
    );
    let text = "%%start\n(unit())() make = fn { unit() inner = fn { 1; }; unit() reader = fn { inner(); }; reader };\n%%end";
    for layout in [ScalarTargetLayout::WASM32, ScalarTargetLayout::NATIVE64] {
        let parsed = parse_source(source.clone(), text.to_owned(), &[]);
        assert!(parsed.diagnostics.is_empty(), "{:?}", parsed.diagnostics);
        let single = derive_scalar_program_with_layout(&parsed.result, layout);
        let project = validate_scalar_project(ScalarProject::new(
            vec![ScalarModule::from_program(
                single.program.clone(),
                Vec::new(),
            )],
            vec![source.clone()],
        ));
        assert!(single.diagnostics.is_empty(), "{:?}", single.diagnostics);
        assert!(project.diagnostics.is_empty(), "{:?}", project.diagnostics);
        for result in [
            emit_scalar_llvm(&single),
            emit_scalar_project_llvm(&project),
        ] {
            assert!(result.is_ok(), "{layout:?}: {result:?}");
        }
    }
}

#[test]
fn selected_overload_nested_callable_emits_on_both_targets_and_pipelines() {
    let source = SourceIdentity::new(
        "project".into(),
        "package".into(),
        "src/main.w".into(),
        "r1".into(),
    );
    let text = "%%start\nmake = overload { (u8())(u8) => fn(value) { u8() inner = fn { value }; inner }; (u16())(u16) => fn(value) { u16() inner = fn { value }; inner }; };\nu8() first = make(1);\nu16() second = make(2);\nu8 one = first();\nu16 two = second();\n%%end";
    for layout in [ScalarTargetLayout::NATIVE64, ScalarTargetLayout::WASM32] {
        let parsed = parse_source(source.clone(), text.to_owned(), &[]);
        assert!(parsed.diagnostics.is_empty(), "{:?}", parsed.diagnostics);
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
        for emitted in [
            emit_scalar_llvm(&single),
            emit_scalar_project_llvm(&project),
        ] {
            let llvm = emitted.expect("selected nested overload LLVM").to_text();
            assert!(
                llvm.contains("define internal i8 @__wosy_closure__"),
                "{llvm}"
            );
            assert!(
                llvm.contains("define internal i16 @__wosy_closure__"),
                "{llvm}"
            );
            assert!(llvm.contains("call i8 %field_code"), "{llvm}");
            assert!(llvm.contains("call i16 %field_code"), "{llvm}");
        }
    }
}

#[test]
fn closure_runtime_array_parameter_and_capture_length_on_both_targets_and_pipelines() {
    let source = SourceIdentity::new(
        "project".into(),
        "package".into(),
        "src/main.w".into(),
        "r1".into(),
    );
    let text = "%%start\nu8[](u64) make = fn(count) { u8[count] captured; u8[](u8[], u64) forward = fn(input, unrelated) { unrelated; captured[0] = 3; input }; u8[3] fixed = [1, 2, 3]; forward(fixed, 91) };\n%%end";
    for layout in [ScalarTargetLayout::WASM32, ScalarTargetLayout::NATIVE64] {
        let parsed = parse_source(source.clone(), text.to_owned(), &[]);
        assert!(parsed.diagnostics.is_empty(), "{:?}", parsed.diagnostics);
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
        for result in [
            emit_scalar_llvm(&single),
            emit_scalar_project_llvm(&project),
        ] {
            let llvm = result.expect("literal length ABI").to_text();
            assert!(
                llvm.contains("(ptr %0, ptr %1, i64 %2, i64 %3, ptr %4)"),
                "{llvm}"
            );
            assert!(llvm.contains("i64 3, i64 91"), "{llvm}");
            assert!(llvm.contains("capture_length_cell"), "{llvm}");
        }
    }
}

#[test]
fn returned_literal_forwards_captured_array_length_on_both_targets_and_pipelines() {
    let source = SourceIdentity::new(
        "project".into(),
        "package".into(),
        "src/main.w".into(),
        "r1".into(),
    );
    let text = "%%start\nstruct Holder { u8[](u8[], u64) read; }\nHolder(u64) make = fn(count) { u8[count] bytes; Holder made = { .read = fn(input, unrelated) { unrelated; bytes[0] = 3; input }; }; made };\nu8[](u64) run = fn(count) { Holder later = make(count); u8[3] fixed = [1, 2, 3]; later.read(fixed, 91) };\n%%end";
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
        for result in [
            emit_scalar_llvm(&single),
            emit_scalar_project_llvm(&project),
        ] {
            let llvm = result.expect("returned closure length ABI").to_text();
            assert!(llvm.contains("capture_length_cell"), "{llvm}");
            assert!(llvm.contains("closure_return_length_0"), "{llvm}");
            assert!(llvm.contains("i64 3, i64 91, ptr"), "{llvm}");
        }
    }
}

#[test]
fn literal_multi_output_array_slot_follows_checked_owner_slot_on_both_targets() {
    let source = SourceIdentity::new(
        "project".into(),
        "package".into(),
        "src/main.w".into(),
        "r1".into(),
    );
    let text = "%%start\nstruct copy Point { u8 value; }\nstruct Dispatch { (*Point, u8[])(*Point, u8[], bool) choose; }\nunit() build = fn { Dispatch runner = { .choose = fn(external, bytes, local) { *Point selected = external; if (local) { Point sample = { .value = 7; }; selected = &sample; }; selected, bytes }; }; runner; };\n%%end";
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
        for result in [
            emit_scalar_llvm(&single),
            emit_scalar_project_llvm(&project),
        ] {
            let llvm = result.expect("literal checked-owner length ABI").to_text();
            assert!(
                llvm.contains("ptr %0, i32 %1, ptr %2, i64 %3, i1 %4, ptr %5, ptr %6)")
                    || llvm.contains("ptr %0, i64 %1, ptr %2, i64 %3, i1 %4, ptr %5, ptr %6)"),
                "{llvm}"
            );
            assert!(llvm.contains("store i64 %array_length, ptr %6"), "{llvm}");
        }
    }
}

#[test]
fn literal_array_call_without_evaluated_source_length_reports_transport_error() {
    let source = SourceIdentity::new(
        "project".into(),
        "package".into(),
        "src/main.w".into(),
        "r1".into(),
    );
    let text = "%%start\nu8[](bool) run = fn(flag) { u8[](u8[]) forward = fn(input) { input }; u8[2] fixed_left = [1, 2]; u8[3] fixed_right = [3, 4, 5]; u8[] left = fixed_left; u8[] right = fixed_right; forward(if (flag) { left } else { right }) };\n%%end";
    for layout in [ScalarTargetLayout::WASM32, ScalarTargetLayout::NATIVE64] {
        let parsed = parse_source(source.clone(), text.to_owned(), &[]);
        let single = derive_scalar_program_with_layout(&parsed.result, layout);
        assert!(single.diagnostics.is_empty(), "{:?}", single.diagnostics);
        let error = emit_scalar_llvm(&single).expect_err("missing expression length transport");
        assert!(
            error.contains("runtime-array length requires validated source transport"),
            "{error}"
        );
    }
}

#[test]
fn literal_forwarded_array_does_not_free_transferred_parameter() {
    let source = SourceIdentity::new(
        "project".into(),
        "package".into(),
        "src/main.w".into(),
        "r1".into(),
    );
    let text = "%%start\nu8[](u64) run = fn(count) { u8[](u8[]) forward = fn(input) { input }; u8[count] values; forward(values) };\n%%end";
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
        for result in [
            emit_scalar_llvm(&single),
            emit_scalar_project_llvm(&project),
        ] {
            let llvm = result.expect("literal borrowed forwarding").to_text();
            let code = llvm
                .split("define internal ptr @__wosy_closure_")
                .nth(1)
                .expect("literal code");
            let code = code.split("\n}").next().expect("literal body");
            assert!(code.contains("store i64 %array_length, ptr %3"), "{code}");
            assert!(!code.contains("call void @__wosy_core_free"), "{code}");
        }
    }
}

#[test]
fn indirect_callable_field_array_transport_and_nested_projection_on_both_targets() {
    let source = SourceIdentity::new(
        "project".into(),
        "package".into(),
        "src/main.w".into(),
        "r1".into(),
    );
    let text = "%%start\nstruct Dispatch { u8[](u8[], u64) relay; }\nstruct Wrapper { Dispatch nested; }\nu8[](u64) create = fn(count) { u8[count] fresh; fresh };\nu8[](u64) run = fn(count) { Wrapper wrapped = { .nested = { .relay = fn(input, unrelated) { unrelated; input }; }; }; Dispatch extracted = wrapped.nested; u8[] returned = extracted.relay(create(count), 91); returned };\nu8[]() widen = fn { Dispatch target = { .relay = fn(input, unrelated) { unrelated; input }; }; u8[3] fixed = [1, 2, 3]; target.relay(fixed, 73) };\n%%end";
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
        for result in [
            emit_scalar_llvm(&single),
            emit_scalar_project_llvm(&project),
        ] {
            let llvm = result.expect("indirect field length transport").to_text();
            assert!(
                llvm.contains("i64 %array_length, i64 91, ptr %closure_return_length_0)"),
                "{llvm}"
            );
            assert!(
                llvm.contains("i64 3, i64 73, ptr %closure_return_length_0)"),
                "{llvm}"
            );
            assert!(llvm.contains("callable_field"), "{llvm}");
        }
    }
}

#[test]
fn indirect_field_mixed_checked_owner_uses_signature_only_second_output() {
    let source = SourceIdentity::new(
        "project".into(),
        "package".into(),
        "src/main.w".into(),
        "r1".into(),
    );
    let text = "%%start\nstruct Dispatch { (*u8, u8[])(*u8, u8[], bool) choose; }\nu8[](u64) create = fn(count) { u8[count] fresh; fresh };\nu8[](u64) run = fn(count) { Dispatch runner = { .choose = fn(external, bytes, local) { *u8 selected = external; if (local) { u8 sample = 7; selected = &sample; }; selected, bytes }; }; u8 outside = 3; runner.choose(&outside, create(count), true); create(count) };\n%%end";
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
        for items in [&single.program.items, &project.project.modules[0].items] {
            let run = items
                .iter()
                .find_map(|item| match item {
                    ScalarItem::Function(function) if function.name == "run" => Some(function),
                    _ => None,
                })
                .expect("run");
            let calls = run
                .reference_cfg
                .call_points
                .iter()
                .filter(|call| {
                    matches!(
                        &run.reference_cfg.points[call.evaluated_point.0].kind,
                        CfgPointKind::Call {
                            target: crate::scalar::ResolvedCallTarget::CallableField { .. },
                            ..
                        }
                    )
                })
                .collect::<Vec<_>>();
            let [call] = calls.as_slice() else {
                panic!("one field call")
            };
            assert_eq!(call.projections.len(), 1);
            assert_eq!(call.projections[0].output, 0);
            assert_eq!(call.projections[0].point, call.evaluated_point);
        }
        for result in [
            emit_scalar_llvm(&single),
            emit_scalar_project_llvm(&project),
        ] {
            let text = result.expect("indirect mixed result").to_text();
            assert!(
                text.contains("ptr %field_owner_0, ptr %closure_return_length_1"),
                "{text}"
            );
        }
    }
}

#[test]
fn indirect_field_branch_array_without_resolved_binding_reports_scalar_error() {
    let source = SourceIdentity::new(
        "project".into(),
        "package".into(),
        "src/main.w".into(),
        "r1".into(),
    );
    let text = "%%start\nstruct Dispatch { u8[](u8[]) relay; }\nu8[](bool) run = fn(flag) { Dispatch target = { .relay = fn(input) { input }; }; u8[2] fixed_left = [1, 2]; u8[3] fixed_right = [3, 4, 5]; u8[] left = fixed_left; u8[] right = fixed_right; target.relay(if (flag) { left } else { right }) };\n%%end";
    for layout in [ScalarTargetLayout::WASM32, ScalarTargetLayout::NATIVE64] {
        let parsed = parse_source(source.clone(), text.to_owned(), &[]);
        assert!(parsed.diagnostics.is_empty(), "{:?}", parsed.diagnostics);
        let single = derive_scalar_program_with_layout(&parsed.result, layout);
        assert!(
            single
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.message == "missing resolved binding"),
            "{:?}",
            single.diagnostics
        );
        let project = validate_scalar_project(ScalarProject::new(
            vec![ScalarModule::from_program(
                single.program.clone(),
                Vec::new(),
            )],
            vec![source.clone()],
        ));
        assert!(
            project
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.message == "missing resolved binding"),
            "{:?}",
            project.diagnostics
        );
    }
}

#[test]
fn overwritten_callable_field_closure_returns_typed_literal_on_both_targets_and_pipelines() {
    let source = SourceIdentity::new(
        "project".into(),
        "package".into(),
        "src/main.w".into(),
        "r1".into(),
    );
    let text = "%%start\nstruct copy Captured { u8(u8) write; u8(u64) read; }\nCaptured(u64) make_counter = fn(count) { u8[count] slots; slots[0] = 6; slots[1] = 7; Captured made = { .write = fn(value) { slots[0] = value; slots[0] }; .read = fn(index) { slots[index] }; }; made };\nunit() run = fn { Captured kept = make_counter(2); Captured copy = kept; u8 first = kept.read(0); u8 wrote = kept.write(9); kept.read = fn(index) { index; 5 }; u8 second = copy.read(0); u8 other = kept.read(1); if (first != 6 || wrote != 9 || second != 9 || other != 5) { core.system_panic(); }; };\nrun();\n%%end";
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
            assert!(llvm.contains("ret i8 5"), "{llvm}");
        }
    }
}

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
fn moved_direct_runtime_array_remains_addressable_on_both_targets_and_pipelines() {
    let source = SourceIdentity::new(
        "project".into(),
        "package".into(),
        "src/main.w".into(),
        "r1".into(),
    );
    let text = "%%start\nu8(u64) read_after_move = fn(count) { u8[count] bytes; bytes[0] = 120; u8[] moved = bytes; unsafe { *?u8 data = &?bytes[0]; core.load<u8>(data); }; moved[0] };\n%%end";
    for layout in [ScalarTargetLayout::WASM32, ScalarTargetLayout::NATIVE64] {
        let parsed = parse_source(source.clone(), text.to_owned(), &[]);
        assert!(parsed.diagnostics.is_empty(), "{:?}", parsed.diagnostics);
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
                llvm.matches("getelementptr inbounds i8, ptr %allocation, i64 0")
                    .count()
                    == 2,
                "{llvm}"
            );
            assert!(!llvm.contains("load ptr, ptr %allocation, align"), "{llvm}");
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

fn prepared_return_before_cleanup(layout: ScalarTargetLayout, project_pipeline: bool) {
    let source = SourceIdentity::new(
        "project".into(),
        "package".into(),
        "src/main.w".into(),
        "prepared-return".into(),
    );
    let text = "%%start\nstruct copy Point { u8 value; }\n*Point(u64, *Point, bool) choose = fn(count, input, flag) { if (flag) { u8[count] bytes; bytes[0]; Point local = { .value = 1; }; &local } else { input } };\n%%end";
    let parsed = parse_source(source.clone(), text.to_owned(), &[]);
    assert!(parsed.diagnostics.is_empty(), "{:?}", parsed.diagnostics);
    let single = derive_scalar_program_with_layout(&parsed.result, layout);
    assert!(single.diagnostics.is_empty(), "{:?}", single.diagnostics);
    let llvm = if project_pipeline {
        let project = validate_scalar_project(ScalarProject::new(
            vec![ScalarModule::from_program(
                single.program.clone(),
                Vec::new(),
            )],
            vec![source],
        ));
        assert!(project.diagnostics.is_empty(), "{:?}", project.diagnostics);
        emit_scalar_project_llvm(&project)
            .expect("project LLVM")
            .to_text()
    } else {
        emit_scalar_llvm(&single).expect("single LLVM").to_text()
    };
    assert_eq!(
        llvm.matches("return_result_value = load [1 x i8]").count(),
        1,
        "{llvm}"
    );
    let copy = llvm
        .find("return_result_value = load [1 x i8]")
        .expect("prepared branch copy");
    let cleanup = llvm
        .find("call void @__wosy_core_free(")
        .expect("local backing cleanup");
    let owner = llvm
        .find("store i1 %return_owner_0")
        .expect("actual hidden owner flag store");
    let returned = llvm.find("ret i").expect("prepared return payload");
    assert!(copy < cleanup, "{llvm}");
    assert!(owner < returned, "{llvm}");
}

#[test]
fn prepared_return_before_cleanup_program_native64() {
    prepared_return_before_cleanup(ScalarTargetLayout::NATIVE64, false);
}

#[test]
fn prepared_return_before_cleanup_program_wasm32() {
    prepared_return_before_cleanup(ScalarTargetLayout::WASM32, false);
}

#[test]
fn prepared_return_before_cleanup_project_native64() {
    prepared_return_before_cleanup(ScalarTargetLayout::NATIVE64, true);
}

#[test]
fn prepared_return_before_cleanup_project_wasm32() {
    prepared_return_before_cleanup(ScalarTargetLayout::WASM32, true);
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
    let run = validation
        .program
        .items
        .iter()
        .find_map(|item| match item {
            ScalarItem::Function(function) if function.name == "run" => Some(function),
            _ => None,
        })
        .expect("run");
    assert_projected_loop_correspondence(&run.reference_cfg);

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
fn nested_top_level_calls_connect_before_bindings_in_both_pipelines_and_layouts() {
    let source = SourceIdentity::new(
        "project".into(),
        "package".into(),
        "src/main.w".into(),
        "r1".into(),
    );
    let text = "%%start\n*u32(*u32) transport = fn(value) { value };\nu32 number = 3;\nu32 loaded = *transport(&number);\n*transport(&number);\n%%end";
    let parsed = parse_source(source.clone(), text.to_owned(), &[]);
    assert!(parsed.diagnostics.is_empty(), "{:?}", parsed.diagnostics);
    for layout in [ScalarTargetLayout::WASM32, ScalarTargetLayout::NATIVE64] {
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
        for cfg in [
            &single.program.top_level_reference_cfg,
            &project.project.modules[0].top_level_reference_cfg,
        ] {
            let calls = cfg.call_points.iter().collect::<Vec<_>>();
            assert_eq!(calls.len(), 2);
            assert_ne!(calls[0].occurrence, calls[1].occurrence);
            for call in calls {
                let evaluated = call.evaluated_point;
                assert!(
                    matches!(&cfg.top_level.points[evaluated.0].execution, crate::scalar::ScalarTopLevelPointExecution::Connected { occurrence: crate::scalar::ScalarTopLevelOccurrence::Call(recorded), .. } if recorded == &call.occurrence)
                );
                super::holder_events::HolderEvents::new(cfg)
                    .expect("top-level holder events")
                    .validated(&cfg.points[evaluated.0])
                    .expect("connected call edge or exit");
            }
        }
        emit_scalar_llvm(&single).expect("single nested calls");
        emit_scalar_project_llvm(&project).expect("project nested calls");
    }
}

#[test]
fn typed_binding_roles_and_ordinary_non_holder_bindings_match_both_pipelines_and_layouts() {
    let source = SourceIdentity::new(
        "project".into(),
        "package".into(),
        "src/main.w".into(),
        "r1".into(),
    );
    let text = "%%start\nu8 first = 1; u8 second = 2; u8()[2] readers = [fn { first }, fn { second }]; u8()[2] copied = readers; u32 ordinary = 1 + 2;\n%%end";
    let parsed = parse_source(source.clone(), text.to_owned(), &[]);
    assert!(parsed.diagnostics.is_empty(), "{:?}", parsed.diagnostics);
    for layout in [ScalarTargetLayout::WASM32, ScalarTargetLayout::NATIVE64] {
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
        for (items, cfg) in [
            (
                &single.program.items,
                &single.program.top_level_reference_cfg,
            ),
            (
                &project.project.modules[0].items,
                &project.project.modules[0].top_level_reference_cfg,
            ),
        ] {
            let events = super::holder_events::HolderEvents::new(cfg).expect("binding events");
            for item in items {
                let ScalarItem::Binding(binding) = item else {
                    continue;
                };
                events
                    .binding(binding, 0, false)
                    .expect("exact binding roles");
                if binding.name == "readers" {
                    assert_eq!(
                        cfg.binding_points
                            .iter()
                            .filter(|event| event.occurrence == *binding.occurrence
                                && matches!(
                                    event.role,
                                    crate::scalar::ScalarBindingPointRole::ArrayElement(_)
                                ))
                            .count(),
                        2
                    );
                }
                if binding.name == "ordinary" {
                    assert!(matches!(
                        &cfg.top_level
                            .bindings
                            .iter()
                            .find(|entry| entry.occurrence == *binding.occurrence)
                            .expect("ordinary role")
                            .execution,
                        crate::scalar::ScalarTopLevelBindingExecution::EmittedWithoutHolderAction
                    ));
                }
            }
        }
        emit_scalar_llvm(&single).expect("single callable captures in array");
        emit_scalar_project_llvm(&project).expect("project callable captures in array");
    }
}

#[test]
fn emits_callable_array_literals_with_exact_capture_occurrences_on_both_targets_and_pipelines() {
    let source = SourceIdentity::new(
        "project".into(),
        "package".into(),
        "src/main.w".into(),
        "r1".into(),
    );
    let parsed = parse_source(
        source.clone(),
        "%%start\nu8()[2] readers = [fn { 1 }, fn { 2 }];\n%%end".into(),
        &[],
    );
    assert!(parsed.diagnostics.is_empty(), "{:?}", parsed.diagnostics);
    for layout in [ScalarTargetLayout::WASM32, ScalarTargetLayout::NATIVE64] {
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
        for cfg in [
            &single.program.top_level_reference_cfg,
            &project.project.modules[0].top_level_reference_cfg,
        ] {
            let events = super::holder_events::HolderEvents::new(cfg).expect("holder events");
            assert_eq!(cfg.capture_points.len(), 2);
            assert_ne!(
                cfg.capture_points[0].occurrence,
                cfg.capture_points[1].occurrence
            );
            for capture in &cfg.capture_points {
                assert_eq!(
                    events
                        .capture(&capture.occurrence)
                        .expect("exact capture")
                        .id,
                    capture.point
                );
            }
        }
        emit_scalar_llvm(&single).expect("single callable array emission");
        emit_scalar_project_llvm(&project).expect("project callable array emission");
    }
}

#[test]
fn capture_descriptors_follow_source_qualified_layout_on_both_targets_and_pipelines() {
    let source = SourceIdentity::new(
        "project".into(),
        "package".into(),
        "src/main.w".into(),
        "r1".into(),
    );
    let text = "%%start\nu8() make = fn { u8 first = 1; u64 second = 2; u8() a = fn { first }; u64() b = fn { second }; a(); second = b(); first };\nmake();\n%%end";
    let parsed = parse_source(source.clone(), text.into(), &[]);
    assert!(parsed.diagnostics.is_empty(), "{:?}", parsed.diagnostics);
    for layout in [ScalarTargetLayout::WASM32, ScalarTargetLayout::NATIVE64] {
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
        for items in [&single.program.items, &project.project.modules[0].items] {
            let ScalarItem::Function(function) = &items[0] else {
                panic!("make function")
            };
            let events = super::holder_events::HolderEvents::new(&function.reference_cfg).unwrap();
            let descriptors = function
                .reference_cfg
                .capture_points
                .iter()
                .filter_map(|capture| {
                    let point = events.capture(&capture.occurrence).unwrap();
                    let storage = match &point.kind {
                        crate::scalar::CfgPointKind::Capture { captures, .. } => captures
                            .iter()
                            .map(|capture| {
                                (
                                    capture.binding.clone(),
                                    super::ownership::CaptureCellResidence::Promoted,
                                )
                            })
                            .collect(),
                        _ => unreachable!(),
                    };
                    events.capture_descriptor(point, layout, &storage).unwrap()
                })
                .collect::<Vec<_>>();
            assert_eq!(descriptors.len(), 2);
            assert_ne!(descriptors[0].literal, descriptors[1].literal);
            assert_ne!(descriptors[0].signature, descriptors[1].signature);
            assert_eq!(descriptors[0].cells[0].cell_offset, 0);
            assert_eq!(descriptors[1].cells[0].cell_offset, 0);
            assert!(descriptors
                .iter()
                .all(|descriptor| descriptor.cells[0].residence
                    == super::ownership::CaptureCellResidence::Promoted));
        }
        emit_scalar_llvm(&single).expect("single descriptor IR");
        emit_scalar_project_llvm(&project).expect("project descriptor IR");
    }
}

#[test]
fn capture_descriptor_requires_typed_global_binding_and_classified_external_contract() {
    let source = SourceIdentity::new(
        "project".into(),
        "package".into(),
        "src/main.w".into(),
        "r1".into(),
    );
    let text = "%%start\nstruct copy Reader { u8() read; }\nu64 length = 2;\nu8[length] bytes;\nbytes[0] = 5;\nReader result = { .read = fn { bytes[0] }; };\nu8 observed = result.read();\n%%end";
    let parsed = parse_source(source.clone(), text.into(), &[]);
    for layout in [ScalarTargetLayout::WASM32, ScalarTargetLayout::NATIVE64] {
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
        for cfg in [
            &single.program.top_level_reference_cfg,
            &project.project.modules[0].top_level_reference_cfg,
        ] {
            let events = super::holder_events::HolderEvents::new(cfg).unwrap();
            let capture = cfg.capture_points.first().expect("top-level capture");
            let point = events.capture(&capture.occurrence).unwrap();
            let crate::scalar::CfgPointKind::Capture { captures, .. } = &point.kind else {
                panic!("capture")
            };
            let binding = captures[0].binding.clone();
            let global = std::collections::BTreeMap::from([(
                binding.clone(),
                super::ownership::CaptureCellResidence::Global,
            )]);
            assert_eq!(
                events
                    .capture_descriptor(point, layout, &global)
                    .unwrap()
                    .unwrap()
                    .cells[0]
                    .residence,
                super::ownership::CaptureCellResidence::Global
            );
            for (residence, diagnostic) in [
                (
                    super::ownership::CaptureCellResidence::Promoted,
                    "typed top-level binding",
                ),
                (
                    super::ownership::CaptureCellResidence::External,
                    "validated external ownership and trace/release contract",
                ),
            ] {
                let storage = std::collections::BTreeMap::from([(binding.clone(), residence)]);
                let error = events
                    .capture_descriptor(point, layout, &storage)
                    .err()
                    .unwrap();
                assert!(
                    error.contains(diagnostic) && error.contains("src/main.w"),
                    "{error}"
                );
            }
        }
        emit_scalar_llvm(&single).expect("single global capture");
        emit_scalar_project_llvm(&project).expect("project global capture");
    }
}

#[test]
fn capture_descriptor_tracks_callable_array_element_origins_on_both_targets() {
    let source = SourceIdentity::new(
        "project".into(),
        "package".into(),
        "src/main.w".into(),
        "r1".into(),
    );
    let text = "%%start\nunit() inspect = fn { u8 first = 1; u8 second = 2; u8()[2] readers = [fn { first }, fn { second }]; u8() observer = fn { u8() selected = readers[0]; selected() }; observer(); };\ninspect();\n%%end";
    let parsed = parse_source(source.clone(), text.into(), &[]);
    assert!(parsed.diagnostics.is_empty(), "{:?}", parsed.diagnostics);
    for layout in [ScalarTargetLayout::WASM32, ScalarTargetLayout::NATIVE64] {
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
        for items in [&single.program.items, &project.project.modules[0].items] {
            let function = items
                .iter()
                .find_map(|item| match item {
                    ScalarItem::Function(function) if function.name == "inspect" => Some(function),
                    _ => None,
                })
                .unwrap();
            let events = super::holder_events::HolderEvents::new(&function.reference_cfg).unwrap();
            let descriptors = function
                .reference_cfg
                .capture_points
                .iter()
                .filter_map(|capture| {
                    let point = events.capture(&capture.occurrence).unwrap();
                    let crate::scalar::CfgPointKind::Capture { captures, .. } = &point.kind else {
                        unreachable!()
                    };
                    let storage = captures
                        .iter()
                        .map(|capture| {
                            (
                                capture.binding.clone(),
                                super::ownership::CaptureCellResidence::Promoted,
                            )
                        })
                        .collect();
                    events.capture_descriptor(point, layout, &storage).unwrap()
                })
                .collect::<Vec<_>>();
            let reader = descriptors
                .iter()
                .find(|descriptor| {
                    descriptor
                        .cells
                        .iter()
                        .any(|cell| matches!(cell.ty, crate::scalar::ScalarType::Array { .. }))
                })
                .unwrap();
            assert_eq!(reader.cells.len(), 1);
            assert_eq!(reader.cells[0].initialized_origins.len(), 2);
            assert!(!reader.cells[0].initialized_cases.is_empty());
            assert_ne!(
                reader.cells[0].initialized_origins[0].literal,
                reader.cells[0].initialized_origins[1].literal
            );
        }
        emit_scalar_llvm(&single).expect("single callable element layout");
        emit_scalar_project_llvm(&project).expect("project callable element layout");
    }
}

#[test]
fn captured_callable_array_descriptor_has_connected_backing_and_owner_on_both_targets() {
    let source = SourceIdentity::new(
        "project".into(),
        "package".into(),
        "src/main.w".into(),
        "r1".into(),
    );
    let text = "%%start\nstruct copy Captured { u8(u8) write; u8(u64) read; }\nCaptured(u64) make = fn(count) { u8[count] slots; slots[0] = 6; Captured result = { .write = fn(value) { slots[0] = value; slots[0] }; .read = fn(index) { slots[index] }; }; result };\nunit() run = fn { Captured first = make(2); Captured second = make(3); first.write(7); second.read(0); };\nrun();\n%%end";
    let parsed = parse_source(source.clone(), text.into(), &[]);
    assert!(parsed.diagnostics.is_empty(), "{:?}", parsed.diagnostics);
    for layout in [ScalarTargetLayout::WASM32, ScalarTargetLayout::NATIVE64] {
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
        for items in [&single.program.items, &project.project.modules[0].items] {
            let function = items
                .iter()
                .find_map(|item| match item {
                    ScalarItem::Function(function) if function.name == "make" => Some(function),
                    _ => None,
                })
                .expect("maker function");
            let events = super::holder_events::HolderEvents::new(&function.reference_cfg).unwrap();
            let descriptors = function
                .reference_cfg
                .capture_points
                .iter()
                .filter_map(|capture| {
                    let point = events.capture(&capture.occurrence).unwrap();
                    let storage = match &point.kind {
                        crate::scalar::CfgPointKind::Capture { captures, .. } => captures
                            .iter()
                            .map(|capture| {
                                (
                                    capture.binding.clone(),
                                    super::ownership::CaptureCellResidence::Promoted,
                                )
                            })
                            .collect(),
                        _ => unreachable!(),
                    };
                    events.capture_descriptor(point, layout, &storage).unwrap()
                })
                .collect::<Vec<_>>();
            assert_eq!(descriptors.len(), 2);
            assert_ne!(descriptors[0].literal, descriptors[1].literal);
            for descriptor in descriptors {
                assert_eq!(descriptor.cells.len(), 1);
                let cell = &descriptor.cells[0];
                assert_eq!(cell.cell_offset, 0);
                assert_eq!(cell.length_offset, Some(8));
                assert_eq!(descriptor.slot_width, 8);
                assert_eq!(descriptor.payload_size, 16);
                assert!(!cell.backings.is_empty());
                assert!(!cell.owner_exits.is_empty());
            }
            let mut disconnected = function.reference_cfg.clone();
            for edge in &mut disconnected.holder_edges {
                edge.actions.retain(|action| {
                    !matches!(
                        action,
                        crate::scalar::CallableHolderAction::DropCaptureCellOwner { .. }
                    )
                });
            }
            for exit in &mut disconnected.holder_exits {
                exit.actions.retain(|action| {
                    !matches!(
                        action,
                        crate::scalar::CallableHolderAction::DropCaptureCellOwner { .. }
                    )
                });
            }
            let missing = super::holder_events::HolderEvents::new(&disconnected).unwrap();
            let capture = &disconnected.capture_points[0];
            let point = missing.capture(&capture.occurrence).unwrap();
            let storage = match &point.kind {
                crate::scalar::CfgPointKind::Capture { captures, .. } => captures
                    .iter()
                    .map(|capture| {
                        (
                            capture.binding.clone(),
                            super::ownership::CaptureCellResidence::Promoted,
                        )
                    })
                    .collect(),
                _ => unreachable!(),
            };
            let error = missing
                .capture_descriptor(point, layout, &storage)
                .err()
                .expect("owned capture requires connected exit");
            assert!(error.contains("connected cell owner exit"), "{error}");
            assert!(error.contains("src/main.w"), "{error}");
        }
        emit_scalar_llvm(&single).expect("single array descriptor IR");
        emit_scalar_project_llvm(&project).expect("project array descriptor IR");
    }
}

#[test]
fn emits_projected_same_span_callable_captures_on_both_targets_and_pipelines() {
    let source = SourceIdentity::new(
        "project".into(),
        "package".into(),
        "src/main.w".into(),
        "r1".into(),
    );
    let text = "%%start\n((u8()), (u8()))() choose = fn { if (true) { u8() local = fn { 5 }; fn { local() }, fn { local() } } else { u8() other = fn { 6 }; fn { other() }, fn { other() } } };\n%%end";
    let parsed = parse_source(source.clone(), text.to_owned(), &[]);
    assert!(parsed.diagnostics.is_empty(), "{:?}", parsed.diagnostics);
    for layout in [ScalarTargetLayout::WASM32, ScalarTargetLayout::NATIVE64] {
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
        for items in [&single.program.items, &project.project.modules[0].items] {
            let ScalarItem::Function(function) = &items[0] else {
                panic!("expected choose function")
            };
            let cfg = &function.reference_cfg;
            let events = super::holder_events::HolderEvents::new(cfg).expect("holder events");
            assert!(cfg.capture_points.len() >= 4, "{:?}", cfg.capture_points);
            assert!(
                cfg.capture_points
                    .iter()
                    .any(|left| cfg
                        .capture_points
                        .iter()
                        .any(
                            |right| left.occurrence.source_span == right.occurrence.source_span
                                && left.occurrence.output_projection
                                    != right.occurrence.output_projection
                        )),
                "{:?}",
                cfg.capture_points
            );
            for capture in &cfg.capture_points {
                assert_eq!(
                    events
                        .capture(&capture.occurrence)
                        .expect("projected capture")
                        .id,
                    capture.point
                );
            }
        }
        emit_scalar_llvm(&single).expect("single projected captures");
        emit_scalar_project_llvm(&project).expect("project projected captures");
    }
}

#[test]
fn callable_read_occurrences_cache_single_evaluation_on_both_targets_and_pipelines() {
    let source = SourceIdentity::new(
        "project".into(),
        "package".into(),
        "src/main.w".into(),
        "r1".into(),
    );
    for text in [
        "%%start\nu8() inspect = fn { u8() callback = fn { 1 }; *(u8()) shared = &callback; u8() copy = *shared; copy() };\ninspect();\n%%end",
        "%%start\nu8() inspect = fn { u8()[2] readers = [fn { 1 }, fn { 2 }]; u8() copy = readers[0]; copy() };\ninspect();\n%%end",
    ] {
        for layout in [ScalarTargetLayout::WASM32, ScalarTargetLayout::NATIVE64] {
            let parsed = parse_source(source.clone(), text.to_owned(), &[]);
            let single = derive_scalar_program_with_layout(&parsed.result, layout);
            assert!(single.diagnostics.is_empty(), "{:?}", single.diagnostics);
            let project = validate_scalar_project(ScalarProject::new(
                vec![ScalarModule::from_program(single.program.clone(), Vec::new())],
                vec![source.clone()],
            ));
            assert!(project.diagnostics.is_empty(), "{:?}", project.diagnostics);
            for llvm in [
                emit_scalar_llvm(&single).expect("single callable read LLVM").to_text(),
                emit_scalar_project_llvm(&project).expect("project callable read LLVM").to_text(),
            ] {
                assert!(llvm.contains("%read_callable_slot = alloca { ptr, ptr }"), "{llvm}");
                assert!(llvm.contains("%read_address_slot = alloca ptr"), "{llvm}");
                assert!(llvm.contains("store i1 false, ptr %read_present_slot"), "{llvm}");
                assert!(llvm.contains("store { ptr, ptr } %read_callable, ptr %read_callable_slot"), "{llvm}");
                assert!(llvm.contains("store i1 true, ptr %read_present_slot"), "{llvm}");
                assert_eq!(llvm.matches("store { ptr, ptr } %read_callable, ptr %read_callable_slot").count(), 1, "{llvm}");
            }
        }
    }
}

#[test]
fn callable_read_branch_and_loop_keep_dominating_occurrence_storage() {
    let source = SourceIdentity::new(
        "project".into(),
        "package".into(),
        "src/main.w".into(),
        "r1".into(),
    );
    for text in [
        "%%start\nunit(bool) inspect = fn(choose) { u8() callback = fn { 1 }; *(u8()) shared = &callback; if (choose) { u8() copy = *shared; copy(); }; };\ninspect(false);\n%%end",
        "%%start\nunit() inspect = fn { u8() callback = fn { 1 }; *(u8()) shared = &callback; u64 index = 0; while (index < 2) { u8() copy = *shared; copy(); index = index + 1; } };\ninspect();\n%%end",
    ] {
        for layout in [ScalarTargetLayout::WASM32, ScalarTargetLayout::NATIVE64] {
            let parsed = parse_source(source.clone(), text.to_owned(), &[]);
            let single = derive_scalar_program_with_layout(&parsed.result, layout);
            assert!(single.diagnostics.is_empty(), "{:?}", single.diagnostics);
            let project = validate_scalar_project(ScalarProject::new(
                vec![ScalarModule::from_program(single.program.clone(), Vec::new())],
                vec![source.clone()],
            ));
            assert!(project.diagnostics.is_empty(), "{:?}", project.diagnostics);
            for llvm in [
                emit_scalar_llvm(&single).expect("single callable branch/loop LLVM").to_text(),
                emit_scalar_project_llvm(&project).expect("project callable branch/loop LLVM").to_text(),
            ] {
                let init = llvm.find("store i1 false, ptr %read_present_slot").expect("entry absent");
                let executed = llvm.find("store i1 true, ptr %read_present_slot").expect("executed read");
                assert!(init < executed, "{llvm}");
                assert_eq!(llvm.matches("store { ptr, ptr } %read_callable, ptr %read_callable_slot").count(), 1, "{llvm}");
            }
        }
    }
}

#[test]
fn callable_read_same_span_output_clones_cache_the_executed_read() {
    let source = SourceIdentity::new(
        "project".into(),
        "package".into(),
        "src/main.w".into(),
        "r1".into(),
    );
    let text = "%%start\n(u8, u8)() pair = fn { u8() callback = fn { 1 }; *(u8()) shared = &callback; if (true) { *shared; 1, 2 } else { 3, 4 } };\n%%end";
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
            emit_scalar_llvm(&single)
                .expect("single cloned read LLVM")
                .to_text(),
            emit_scalar_project_llvm(&project)
                .expect("project cloned read LLVM")
                .to_text(),
        ] {
            let slots = llvm
                .lines()
                .filter(|line| {
                    line.trim_start().starts_with("%read_callable_slot")
                        && line.contains(" = alloca { ptr, ptr }")
                })
                .collect::<Vec<_>>();
            assert_eq!(slots.len(), 1, "{llvm}");
            assert_eq!(
                llvm.matches("store { ptr, ptr } %read_callable, ptr %read_callable_slot")
                    .count(),
                1,
                "{llvm}"
            );
        }
    }
}

#[test]
fn separate_callable_reads_have_separate_cached_values() {
    let source = SourceIdentity::new(
        "project".into(),
        "package".into(),
        "src/main.w".into(),
        "r1".into(),
    );
    let text = "%%start\nu8() inspect = fn { u8() callback = fn { 1 }; *(u8()) shared = &callback; u8() first = *shared; u8() second = *shared; first(); second() };\ninspect();\n%%end";
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
            emit_scalar_llvm(&single)
                .expect("single read LLVM")
                .to_text(),
            emit_scalar_project_llvm(&project)
                .expect("project read LLVM")
                .to_text(),
        ] {
            let slots = llvm
                .lines()
                .filter(|line| {
                    line.trim_start().starts_with("%read_callable_slot")
                        && line.contains(" = alloca { ptr, ptr }")
                })
                .collect::<Vec<_>>();
            assert_eq!(slots.len(), 2, "{llvm}");
            assert_ne!(slots[0], slots[1], "{llvm}");
        }
    }
}

#[test]
fn typed_read_place_occurrences_match_emitted_dereference_and_indexed_returns() {
    let source = SourceIdentity::new(
        "project".into(),
        "package".into(),
        "src/main.w".into(),
        "r1".into(),
    );
    let text = "%%start\nu8() read = fn { u8 value = 1; *u8 shared = &value; *shared }; u8() indexed = fn { u8[2] bytes = [1, 2]; bytes[0] };\n%%end";
    let parsed = parse_source(source.clone(), text.to_owned(), &[]);
    assert!(parsed.diagnostics.is_empty(), "{:?}", parsed.diagnostics);
    for layout in [ScalarTargetLayout::WASM32, ScalarTargetLayout::NATIVE64] {
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
        for items in [&single.program.items, &project.project.modules[0].items] {
            for item in items {
                let ScalarItem::Function(function) = item else {
                    continue;
                };
                let expression = &function.body.final_output_values[0].value;
                let events = super::holder_events::HolderEvents::new(&function.reference_cfg)
                    .expect("holder events");
                let point = events.read_place(expression).expect("exact read point");
                assert_eq!(function.reference_cfg.read_points[0].point, point.id);
                let mut supplemental = function.reference_cfg.clone();
                supplemental
                    .top_level
                    .points
                    .push(crate::scalar::ScalarTopLevelPoint {
                        point: point.id,
                        execution:
                            crate::scalar::ScalarTopLevelPointExecution::SupplementalAnalysisOnly,
                    });
                let rejected = super::holder_events::HolderEvents::new(&supplemental)
                    .expect("supplemental events");
                assert!(rejected
                    .read_place(expression)
                    .unwrap_err()
                    .contains("supplemental analysis only"));
            }
        }
        emit_scalar_llvm(&single).expect("single read emission");
        emit_scalar_project_llvm(&project).expect("project read emission");
    }
}

#[test]
fn typed_read_place_selects_each_same_span_projected_dereference() {
    let source = SourceIdentity::new(
        "project".into(),
        "package".into(),
        "src/main.w".into(),
        "r1".into(),
    );
    let text = "%%start\n(u8, u8)() pair = fn { u8 value = 1; *u8 shared = &value; if (true) { *shared; value, value } else { value, value } };\n%%end";
    let parsed = parse_source(source.clone(), text.to_owned(), &[]);
    for layout in [ScalarTargetLayout::WASM32, ScalarTargetLayout::NATIVE64] {
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
        for items in [&single.program.items, &project.project.modules[0].items] {
            let ScalarItem::Function(function) = &items[0] else {
                panic!("expected pair")
            };
            let events = super::holder_events::HolderEvents::new(&function.reference_cfg)
                .expect("holder events");
            let mut matched = Vec::new();
            for item in &function.body.items {
                let ScalarBlockItem::Expression(crate::ScalarExpression::If {
                    then_branch, ..
                }) = item
                else {
                    continue;
                };
                let ScalarBlockItem::Expression(ref read) = then_branch.items[0] else {
                    panic!("expected read")
                };
                matched.push(events.read_place(read).expect("projected ReadPlace").id);
            }
            assert_eq!(matched.len(), 2);
            assert_ne!(matched[0], matched[1]);
        }
        emit_scalar_llvm(&single).expect("single projected read emission");
        emit_scalar_project_llvm(&project).expect("project projected read emission");
    }
}

#[test]
fn top_level_unsafe_block_exit_and_branch_are_connected_on_both_targets_and_pipelines() {
    let source = SourceIdentity::new(
        "project".into(),
        "package".into(),
        "src/main.w".into(),
        "r1".into(),
    );
    let text = "%%start\n*?u8 base = null; *?u8 result = unsafe { core.pointer_cast<*?u8>(base) }; if (false) { core.system_panic(); };\n%%end";
    let parsed = parse_source(source.clone(), text.to_owned(), &[]);
    assert!(parsed.diagnostics.is_empty(), "{:?}", parsed.diagnostics);
    for layout in [ScalarTargetLayout::WASM32, ScalarTargetLayout::NATIVE64] {
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
        for cfg in [
            &single.program.top_level_reference_cfg,
            &project.project.modules[0].top_level_reference_cfg,
        ] {
            let events = super::holder_events::HolderEvents::new(cfg).expect("holder events");
            let exits = cfg.block_scope_exits.iter().collect::<Vec<_>>();
            assert!(!exits.is_empty());
            for exit in exits {
                let point = events
                    .block_exit(&exit.occurrence)
                    .expect("connected block exit");
                events.validated(point).expect("block exit edge");
            }
            let branches = cfg
                .conditional_points
                .iter()
                .filter(|event| event.role == crate::scalar::ScalarConditionalPointRole::Branch)
                .collect::<Vec<_>>();
            assert_eq!(branches.len(), 1);
            let branch = events
                .conditional_point(
                    &branches[0].then_block,
                    crate::scalar::ScalarConditionalPointRole::Branch,
                )
                .expect("connected top-level branch");
            events.arm(branch.id, 0).expect("then edge");
            events.arm(branch.id, 1).expect("skip edge");
        }
        emit_scalar_llvm(&single).expect("single structural top-level emission");
        emit_scalar_project_llvm(&project).expect("project structural top-level emission");
    }
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
    let run = validation.project.modules[0]
        .items
        .iter()
        .find_map(|item| match item {
            ScalarItem::Function(function) if function.name == "run" => Some(function),
            _ => None,
        })
        .expect("run");
    assert_projected_loop_correspondence(&run.reference_cfg);

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

fn assert_projected_loop_correspondence(cfg: &crate::scalar::ScalarReferenceCfg) {
    use crate::scalar::ScalarWhilePointRole;
    let events = super::holder_events::HolderEvents::new(cfg).expect("holder events");
    let tests = cfg
        .while_points
        .iter()
        .filter(|event| event.role == ScalarWhilePointRole::LoopTest)
        .collect::<Vec<_>>();
    assert_eq!(tests.len(), 2);
    assert_eq!(
        tests[0].occurrence.source_span,
        tests[1].occurrence.source_span
    );
    assert_ne!(
        tests[0].occurrence.output_projection,
        tests[1].occurrence.output_projection
    );
    assert_ne!(tests[0].point, tests[1].point);
    for test in tests {
        let head = events
            .while_point(&test.occurrence, ScalarWhilePointRole::LoopTest)
            .expect("projected loop head");
        assert_eq!(head.id, test.point);
        let branch = events
            .while_point(&test.occurrence, ScalarWhilePointRole::Branch)
            .expect("projected loop branch");
        events.arm(branch.id, 0).expect("body edge");
        events.arm(branch.id, 1).expect("exit edge");
        let back = events
            .while_point(&test.occurrence, ScalarWhilePointRole::BodyScopeExit)
            .expect("projected loop back edge");
        events
            .outgoing(back.id, head.id)
            .expect("validated back edge");
    }
}

#[test]
fn projected_loop_and_conditional_exits_match_both_layouts_and_pipelines() {
    let source = SourceIdentity::new(
        "project".into(),
        "package".into(),
        "src/main.w".into(),
        "r1".into(),
    );
    let text = "%%start\n(i32, bool)() run = fn { bool running = true; if (true) { while (running) { running = false; } 1, true } else { 2, false } };\n%%end";
    let parsed = parse_source(source.clone(), text.to_owned(), &[]);
    assert!(parsed.diagnostics.is_empty(), "{:?}", parsed.diagnostics);
    for layout in [ScalarTargetLayout::WASM32, ScalarTargetLayout::NATIVE64] {
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
        for items in [&single.program.items, &project.project.modules[0].items] {
            let run = items
                .iter()
                .find_map(|item| match item {
                    ScalarItem::Function(function) if function.name == "run" => Some(function),
                    _ => None,
                })
                .expect("run");
            assert_projected_loop_correspondence(&run.reference_cfg);
            let events =
                super::holder_events::HolderEvents::new(&run.reference_cfg).expect("holder events");
            for exit in &run.reference_cfg.block_scope_exits {
                assert_eq!(
                    events
                        .block_exit(&exit.occurrence)
                        .expect("structural exit")
                        .id,
                    exit.point
                );
            }
        }
        emit_scalar_llvm(&single).expect("single structural emission");
        emit_scalar_project_llvm(&project).expect("project structural emission");
    }
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
fn callable_array_prefix_uses_one_allocation_and_original_base_on_both_targets() {
    let source = SourceIdentity::new(
        "project".into(),
        "package".into(),
        "src/main.w".into(),
        "r1".into(),
    );
    let text = "%%start\nunit(u64) run = fn(count) { (u8())[count] items; items; };\n%%end";
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
        for result in [
            emit_scalar_llvm(&single),
            emit_scalar_project_llvm(&project),
        ] {
            let llvm = result.expect("callable array prefix LLVM").to_text();
            assert!(llvm.contains("callable_bitmap_bytes"), "{llvm}");
            assert!(llvm.contains("callable_allocation_fits"), "{llvm}");
            assert!(llvm.contains("callable_array_data"), "{llvm}");
            assert!(llvm.contains("callable_array_bitmap_field"), "{llvm}");
            assert!(llvm.contains("call void @llvm.memset"), "{llvm}");
            assert!(
                llvm.contains("call void @__wosy_core_free(ptr %callable_array_allocation_base)"),
                "{llvm}"
            );
            assert_eq!(
                llvm.matches("call ptr @__wosy_core_alloc").count(),
                1,
                "{llvm}"
            );
            assert_eq!(
                llvm.matches("call void @__wosy_core_free").count()
                    - llvm
                        .matches("call void @__wosy_core_free(ptr %environment_allocation_base)")
                        .count(),
                1,
                "{llvm}"
            );
            assert!(llvm.contains("callable_array.drop.installed:"), "{llvm}");
            assert!(llvm.contains("callable_array_walk_in_bounds"), "{llvm}");
            assert!(
                llvm.contains(if layout == ScalarTargetLayout::WASM32 {
                    "callable_bitmap_fits = icmp ule i64 %callable_bitmap_bytes, 429496"
                } else {
                    "callable_bitmap_fits = icmp ule i64 %callable_bitmap_bytes, -"
                }),
                "{llvm}"
            );
        }
    }
}

#[test]
fn callable_array_fixed_widening_creates_its_own_prefixed_backing() {
    let source = SourceIdentity::new(
        "project".into(),
        "package".into(),
        "src/main.w".into(),
        "r1".into(),
    );
    let text = "%%start\nunit() run = fn { u8()[2] fixed = [fn { 1 }, fn { 2 }]; (u8())[] copied = fixed; copied; fixed; };\n%%end";
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
        for result in [
            emit_scalar_llvm(&single),
            emit_scalar_project_llvm(&project),
        ] {
            let llvm = result.expect("fixed callable widening LLVM").to_text();
            assert!(llvm.contains("call ptr @__wosy_core_alloc(i64 "), "{llvm}");
            assert!(llvm.contains("call void @llvm.memcpy"), "{llvm}");
            assert!(llvm.contains("callable_array_bitmap_field"), "{llvm}");
            assert!(llvm.contains("%callable_flag_installed = or i8"), "{llvm}");
            assert!(
                llvm.contains("%fixed_copy_retained_environment = extractvalue"),
                "{llvm}"
            );
            assert!(
                llvm.contains("call void @__wosy_core_free(ptr %callable_array_allocation_base)"),
                "{llvm}"
            );
        }
    }
}

#[test]
fn callable_fixed_widening_drops_replaced_source_and_copied_elements_on_both_targets() {
    let source = SourceIdentity::new(
        "project".into(),
        "package".into(),
        "src/main.w".into(),
        "r1".into(),
    );
    let text = "%%start\nunit() run = fn { u8 first = 11; u8 second = 22; (u8())[2] fixed = [fn { first }, fn { second }]; (u8())[] copied = fixed; u8() from_source = fixed[1]; fixed[0] = fn { first }; u8() from_copy = copied[0]; u8 a = from_copy(); u8 b = from_source(); if (a != 11 || b != 22) { core.system_panic(); }; copied; };\nrun();\n%%end";
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
        for result in [
            emit_scalar_llvm(&single),
            emit_scalar_project_llvm(&project),
        ] {
            let llvm = result.expect("fixed callable lifecycle LLVM").to_text();
            assert!(llvm.contains("fixed_copy_retained_environment"), "{llvm}");
            assert!(llvm.contains("callable_array.drop.installed:"), "{llvm}");
            assert!(
                llvm.contains("call void @__wosy_core_free(ptr %callable_array_allocation_base)"),
                "{llvm}"
            );
            assert!(
                llvm.matches("getelementptr inbounds [2 x { ptr, ptr }], ptr %fixed, i32 0, i64 0")
                    .count()
                    >= 5,
                "{llvm}"
            );
        }
    }
}

#[test]
fn callable_array_prefix_checks_zero_two_and_overflow_before_allocator() {
    let source = SourceIdentity::new(
        "project".into(),
        "package".into(),
        "src/main.w".into(),
        "r1".into(),
    );
    let text = "%%start\nunit() empty = fn { u64 count = 0; (u8())[count] items; items; };\nunit() pair = fn { u64 count = 2; (u8())[count] items; items; };\nunit(u64) large = fn(count) { (u8())[count] items; items; };\n%%end";
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
        for result in [
            emit_scalar_llvm(&single),
            emit_scalar_project_llvm(&project),
        ] {
            let llvm = result.expect("callable prefix bounds LLVM").to_text();
            assert_eq!(
                llvm.matches("call ptr @__wosy_core_alloc(i64 %callable_allocation_size")
                    .count(),
                3,
                "{llvm}"
            );
            assert!(llvm.contains("callable_allocation_fits"), "{llvm}");
            assert!(llvm.contains("callable_allocation_rejected:"), "{llvm}");
            assert!(
                llvm.contains("callable_maximum_length = udiv i64"),
                "{llvm}"
            );
            assert!(llvm.contains("callable_bitmap_bytes"), "{llvm}");
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
fn direct_runtime_array_call_transports_evaluated_length_on_both_targets_and_pipelines() {
    let source = SourceIdentity::new(
        "project".into(),
        "package".into(),
        "src/main.w".into(),
        "r1".into(),
    );
    let text = "%%start\nu8[](u8[], u64) forward = fn(bytes, unrelated) { unrelated; bytes };\nu8[](u64) make = fn(count) { u8[count] bytes; forward(bytes, 99) };\nu8[](u64) run = fn(count) { u8[] bytes = make(count); bytes };\n%%end";
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
        for result in [
            emit_scalar_llvm(&validation),
            emit_scalar_project_llvm(&project),
        ] {
            let llvm = result.expect("runtime-array direct call LLVM").to_text();
            assert!(
                llvm.contains("(ptr %bytes, i64 %bytes_length, i64 %unrelated, ptr"),
                "{llvm}"
            );
            assert!(
                llvm.contains("ptr %allocation, i64 %array_length, i64 99, ptr"),
                "{llvm}"
            );
            assert!(llvm.contains("array_return_length_0"), "{llvm}");
        }
    }
}

#[test]
fn direct_fixed_widening_and_multi_output_array_length_slots_on_both_targets() {
    let source = SourceIdentity::new(
        "project".into(),
        "package".into(),
        "src/main.w".into(),
        "r1".into(),
    );
    let text = "%%start\n(u8[], u64)(u8[], u64) relay = fn(bytes, unrelated) { bytes, unrelated };\nunit() caller = fn { u8[3] fixed = [1, 2, 3]; u8[] received, u64 number = relay(fixed, 91); received[0] = 4; number; };\ncaller();\n%%end";
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
        for result in [
            emit_scalar_llvm(&validation),
            emit_scalar_project_llvm(&project),
        ] {
            let llvm = result.expect("fixed widening multi-output LLVM").to_text();
            assert!(
                llvm.contains("(ptr %bytes, i64 %bytes_length, i64 %unrelated, ptr"),
                "{llvm}"
            );
            assert!(llvm.contains("i64 3, i64 91, ptr"), "{llvm}");
            assert!(llvm.contains("store i64 %array_length, ptr %"), "{llvm}");
        }
    }
}

#[test]
fn direct_array_length_slot_follows_checked_owner_slot_on_both_targets() {
    let source = SourceIdentity::new(
        "project".into(),
        "package".into(),
        "src/main.w".into(),
        "r1".into(),
    );
    let text = "%%start\nstruct copy Point { u8 value; }\n(*Point, u8[])(*Point, u8[], bool) choose = fn(external, bytes, local) { *Point selected = external; if (local) { Point sample = { .value = 7; }; selected = &sample; }; selected, bytes };\n%%end";
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
        for result in [
            emit_scalar_llvm(&validation),
            emit_scalar_project_llvm(&project),
        ] {
            let llvm = result
                .expect("checked owner and array length ABI")
                .to_text();
            assert!(
                llvm.contains("ptr %bytes, i64 %bytes_length, i1 %local, ptr %0, ptr %1)"),
                "{llvm}"
            );
            assert!(llvm.contains("store i64 %array_length, ptr %1"), "{llvm}");
        }
    }
}

#[test]
fn instantiated_generic_array_forwarding_tracks_length_in_single_and_project_emission() {
    let source = SourceIdentity::new(
        "project".into(),
        "package".into(),
        "src/main.w".into(),
        "r1".into(),
    );
    let text = "%%start\ngeneric T;\nT(T, u64) relay = fn(values, unrelated) { unrelated; values };\nu8[](u64) produce = fn(count) { u8[count] fresh; relay<u8[]>(fresh, 91) };\n%%end";
    for layout in [ScalarTargetLayout::WASM32, ScalarTargetLayout::NATIVE64] {
        let parsed = parse_source(source.clone(), text.to_owned(), &[]);
        let single = crate::derive_scalar_program_with_layout(&parsed.result, layout);
        assert!(single.diagnostics.is_empty(), "{:?}", single.diagnostics);
        let project = validate_scalar_project(ScalarProject::new(
            vec![ScalarModule::from_program(
                single.program.clone(),
                Vec::new(),
            )],
            vec![source.clone()],
        ));
        assert!(project.diagnostics.is_empty(), "{:?}", project.diagnostics);
        for result in [
            emit_scalar_llvm(&single),
            emit_scalar_project_llvm(&project),
        ] {
            let llvm = result.expect("instantiated generic array LLVM").to_text();
            assert!(
                llvm.contains("(ptr %values, i64 %values_length, i64 %unrelated, ptr"),
                "{llvm}"
            );
            assert!(llvm.contains("i64 %array_length, i64 91, ptr"), "{llvm}");
            assert!(llvm.contains("call ptr @wosy_generic__"), "{llvm}");
        }
    }
}

#[test]
fn module_runtime_array_forwarding_preserves_callee_source_and_hidden_length() {
    let child_source = SourceIdentity::new(
        "project".into(),
        "package".into(),
        "src/child.w".into(),
        "r1".into(),
    );
    let root_source = SourceIdentity::new(
        "project".into(),
        "package".into(),
        "src/main.w".into(),
        "r1".into(),
    );
    let child_text =
        "%%start\nu8[](u8[], u64) relay = fn(bytes, unrelated) { unrelated; bytes };\n%%end";
    let root_text = "%%start\nchild = namespace package \"src/child.w\";\nu8[](u64) make = fn(count) { u8[count] fresh; child.relay(fresh, 73) };\nu8[] returned = make(5);\nreturned[0] = 7;\n%%end";
    for layout in [ScalarTargetLayout::WASM32, ScalarTargetLayout::NATIVE64] {
        let child = crate::derive_scalar_program_with_layout(
            &parse_source(child_source.clone(), child_text.to_owned(), &[]).result,
            layout,
        )
        .program;
        let root = crate::derive_scalar_program_with_layout(
            &parse_source(root_source.clone(), root_text.to_owned(), &[]).result,
            layout,
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
        let project = validate_scalar_project(ScalarProject::new(
            vec![
                ScalarModule::new(root_source.clone(), root.items, namespace_bindings),
                ScalarModule::new(child_source.clone(), child.items, Vec::new()),
            ],
            vec![root_source.clone(), child_source.clone()],
        ));
        assert!(project.diagnostics.is_empty(), "{:?}", project.diagnostics);
        let llvm = emit_scalar_project_llvm(&project)
            .expect("cross-module array LLVM")
            .to_text();
        let symbol = project_function_name(&child_source, "relay");
        assert!(
            llvm.contains(&format!(
                "define ptr @{symbol}(ptr %bytes, i64 %bytes_length, i64 %unrelated, ptr"
            )),
            "{llvm}"
        );
        assert!(
            llvm.contains(&format!(
                "call ptr @{symbol}(ptr %allocation, i64 %array_length, i64 73, ptr"
            )),
            "{llvm}"
        );
    }
}

#[test]
fn namespaced_generic_array_specialization_uses_child_source_identity() {
    let child_source = SourceIdentity::new(
        "project".into(),
        "package".into(),
        "src/child.w".into(),
        "r1".into(),
    );
    let root_source = SourceIdentity::new(
        "project".into(),
        "package".into(),
        "src/main.w".into(),
        "r1".into(),
    );
    let child_text = "%%start\ngeneric T;\nT(T, u64) relay = fn(values, unrelated) { unrelated; values };\n%%end";
    let root_text = "%%start\nchild = namespace package \"src/child.w\";\nu8[](u64) make = fn(count) { u8[count] fresh; child.relay<u8[]>(fresh, 73) };\n%%end";
    for layout in [ScalarTargetLayout::WASM32, ScalarTargetLayout::NATIVE64] {
        let child = crate::derive_scalar_program_with_layout(
            &parse_source(child_source.clone(), child_text.to_owned(), &[]).result,
            layout,
        )
        .program;
        let root = crate::derive_scalar_program_with_layout(
            &parse_source(root_source.clone(), root_text.to_owned(), &[]).result,
            layout,
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
        let project = validate_scalar_project(ScalarProject::new(
            vec![
                ScalarModule::new(root_source.clone(), root.items, namespace_bindings),
                ScalarModule::new(child_source.clone(), child.items, Vec::new()),
            ],
            vec![root_source.clone(), child_source.clone()],
        ));
        assert!(project.diagnostics.is_empty(), "{:?}", project.diagnostics);
        let llvm = emit_scalar_project_llvm(&project)
            .expect("cross-module generic array LLVM")
            .to_text();
        let symbol = generic_specialization_name(
            &project_function_name(&child_source, "relay"),
            &[ScalarType::RuntimeArray {
                element: Box::new(ScalarType::U8),
                span: wosy_syntax::ByteSpan::new(0, 0),
            }],
        );
        assert!(
            llvm.contains(&format!(
                "define ptr @{symbol}(ptr %values, i64 %values_length, i64 %unrelated, ptr"
            )),
            "{llvm}"
        );
        assert!(
            llvm.contains(&format!(
                "call ptr @{symbol}(ptr %allocation, i64 %array_length, i64 73, ptr"
            )),
            "{llvm}"
        );
    }
}

#[test]
fn module_fixed_widening_and_multi_output_keep_explicit_u64_independent() {
    let child_source = SourceIdentity::new(
        "project".into(),
        "package".into(),
        "src/child.w".into(),
        "r1".into(),
    );
    let root_source = SourceIdentity::new(
        "project".into(),
        "package".into(),
        "src/main.w".into(),
        "r1".into(),
    );
    let child_text =
        "%%start\n(u8[], u64)(u8[], u64) relay = fn(bytes, unrelated) { bytes, unrelated };\n%%end";
    let root_text = "%%start\nchild = namespace package \"src/child.w\";\nu8[]() make = fn { u8[3] fixed = [1, 2, 3]; u8[] bytes, u64 unrelated = child.relay(fixed, 73); unrelated; bytes };\n%%end";
    for layout in [ScalarTargetLayout::WASM32, ScalarTargetLayout::NATIVE64] {
        let child = crate::derive_scalar_program_with_layout(
            &parse_source(child_source.clone(), child_text.to_owned(), &[]).result,
            layout,
        )
        .program;
        let root = crate::derive_scalar_program_with_layout(
            &parse_source(root_source.clone(), root_text.to_owned(), &[]).result,
            layout,
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
        let project = validate_scalar_project(ScalarProject::new(
            vec![
                ScalarModule::new(root_source.clone(), root.items, namespace_bindings),
                ScalarModule::new(child_source.clone(), child.items, Vec::new()),
            ],
            vec![root_source.clone(), child_source.clone()],
        ));
        assert!(project.diagnostics.is_empty(), "{:?}", project.diagnostics);
        let llvm = emit_scalar_project_llvm(&project)
            .expect("cross-module multi-output array LLVM")
            .to_text();
        let symbol = project_function_name(&child_source, "relay");
        assert!(llvm.contains(&format!("define {{ ptr, i64 }} @{symbol}(ptr %bytes, i64 %bytes_length, i64 %unrelated, ptr")), "{llvm}");
        assert!(
            llvm.contains(&format!(
                "call {{ ptr, i64 }} @{symbol}(ptr %allocation, i64 3, i64 73, ptr"
            )),
            "{llvm}"
        );
    }
}

#[test]
fn generic_fixed_widening_multi_output_uses_instantiated_array_length() {
    let source = SourceIdentity::new(
        "project".into(),
        "package".into(),
        "src/main.w".into(),
        "r1".into(),
    );
    let text = "%%start\ngeneric T;\n(T, u64)(T, u64) relay = fn(bytes, unrelated) { bytes, unrelated };\nu8[]() make = fn { u8[3] fixed = [1, 2, 3]; u8[] bytes, u64 unrelated = relay<u8[]>(fixed, 91); unrelated; bytes };\n%%end";
    for layout in [ScalarTargetLayout::WASM32, ScalarTargetLayout::NATIVE64] {
        let parsed = parse_source(source.clone(), text.to_owned(), &[]);
        let single = crate::derive_scalar_program_with_layout(&parsed.result, layout);
        assert!(single.diagnostics.is_empty(), "{:?}", single.diagnostics);
        let specializations = super::generic_specializations(
            &single.program.items,
            |name| {
                single.program.items.iter().find_map(|item| match item {
                    ScalarItem::Function(function) if function.name == name => Some(function),
                    _ => None,
                })
            },
            |name| name.to_owned(),
        )
        .expect("generic specialization");
        assert_eq!(specializations.len(), 1, "{specializations:?}");
        assert!(
            matches!(&specializations[0].1.signature, ScalarType::Callable { parameters, .. } if matches!(&parameters[0], ScalarType::RuntimeArray { .. })),
            "{specializations:?}"
        );
        assert!(
            matches!(&specializations[0].1.signature, ScalarType::Callable { outputs, .. } if matches!(&outputs.outputs[0].ty, ScalarType::RuntimeArray { .. }) && outputs.outputs[1].ty == ScalarType::U64),
            "{specializations:?}"
        );
        assert!(
            specializations[0]
                .1
                .body
                .final_output_values
                .iter()
                .all(|output| output.ty != ScalarType::Unit),
            "{specializations:?}"
        );
        let project = validate_scalar_project(ScalarProject::new(
            vec![ScalarModule::from_program(
                single.program.clone(),
                Vec::new(),
            )],
            vec![source.clone()],
        ));
        assert!(project.diagnostics.is_empty(), "{:?}", project.diagnostics);
        for result in [
            emit_scalar_llvm(&single),
            emit_scalar_project_llvm(&project),
        ] {
            let llvm = result.expect("generic fixed multi-output LLVM").to_text();
            assert!(
                llvm.contains("(ptr %bytes, i64 %bytes_length, i64 %unrelated, ptr"),
                "{llvm}"
            );
            assert!(llvm.contains("i64 3, i64 91, ptr"), "{llvm}");
            assert!(llvm.contains("call { ptr, i64 } @wosy_generic__"), "{llvm}");
        }
    }
}

#[test]
fn module_array_multi_output_length_slot_follows_checked_owner_slot() {
    let child_source = SourceIdentity::new(
        "project".into(),
        "package".into(),
        "src/child.w".into(),
        "r1".into(),
    );
    let root_source = SourceIdentity::new(
        "project".into(),
        "package".into(),
        "src/main.w".into(),
        "r1".into(),
    );
    let child_text = "%%start\n(*u8, u8[])(*u8, u8[], bool) choose = fn(external, bytes, local) { *u8 selected = external; if (local) { u8 sample = 7; selected = &sample; }; selected, bytes };\n%%end";
    let root_text = "%%start\nchild = namespace package \"src/child.w\";\n%%end";
    for layout in [ScalarTargetLayout::WASM32, ScalarTargetLayout::NATIVE64] {
        let child = crate::derive_scalar_program_with_layout(
            &parse_source(child_source.clone(), child_text.to_owned(), &[]).result,
            layout,
        )
        .program;
        let root = crate::derive_scalar_program_with_layout(
            &parse_source(root_source.clone(), root_text.to_owned(), &[]).result,
            layout,
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
        let project = validate_scalar_project(ScalarProject::new(
            vec![
                ScalarModule::new(root_source.clone(), root.items, namespace_bindings),
                ScalarModule::new(child_source.clone(), child.items, Vec::new()),
            ],
            vec![root_source.clone(), child_source.clone()],
        ));
        assert!(project.diagnostics.is_empty(), "{:?}", project.diagnostics);
        let llvm = emit_scalar_project_llvm(&project)
            .expect("module checked-owner array ABI")
            .to_text();
        let symbol = project_function_name(&child_source, "choose");
        assert!(
            llvm.contains("ptr %bytes, i64 %bytes_length, i1 %local, ptr %0, ptr %1)"),
            "{llvm}"
        );
        assert!(
            llvm.contains(&format!("define {{ i32, ptr }} @{symbol}(i32"))
                || llvm.contains(&format!("define {{ i64, ptr }} @{symbol}(i64")),
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
        overload_selection,
        occurrence,
        ..
    } = &mut binding.value
    else {
        panic!("qualified generic overload call");
    };
    let original_occurrence = occurrence.as_ref().clone();
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
    let cfg = &validation.project.modules[0].top_level_reference_cfg;
    let [call] = cfg.call_points.as_slice() else {
        panic!("one evaluated overload call")
    };
    assert_eq!(call.occurrence, original_occurrence);
    assert!(
        matches!(&cfg.points[call.evaluated_point.0].kind, CfgPointKind::Call { expression: crate::ScalarExpression::Call { overload_selection: Some(selection), .. }, .. } if selection.arm_index == 1)
    );

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
    let pair = validation
        .program
        .items
        .iter()
        .find_map(|item| match item {
            ScalarItem::Function(function) if function.name == "pair" => Some(function),
            _ => None,
        })
        .expect("pair");
    let events =
        super::holder_events::HolderEvents::new(&pair.reference_cfg).expect("holder events");
    for exit in &pair.reference_cfg.block_scope_exits {
        assert_eq!(
            events
                .block_exit(&exit.occurrence)
                .expect("exact conditional exit")
                .id,
            exit.point
        );
    }
    for branch in pair
        .reference_cfg
        .conditional_points
        .iter()
        .filter(|event| event.role == crate::scalar::ScalarConditionalPointRole::Branch)
    {
        assert_eq!(
            events
                .conditional_point(
                    &branch.then_block,
                    crate::scalar::ScalarConditionalPointRole::Branch
                )
                .expect("exact conditional branch")
                .id,
            branch.point
        );
    }

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
    assert!(text.contains("source_filename = \"src/main.w\""), "{text}");
    assert!(text.contains("define void @touch()"), "{text}");
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
*Record record_shared = &item;
u32 shared_value = *shared;
u32 field_value = (*record_shared).count;
*u32 forwarded = transport(shared);
u32 forwarded_value = *transport(shared);
forwarded;
*!u32 mutable = &!item.count;
u32 mutable_value = *mutable;
u32 mutable_forwarded_value = *transport_mutable(&!(*mutable));
*!u32 mutable_forwarded = transport_mutable(mutable);
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
        let root_cfg = &validation.project.modules[0].top_level_reference_cfg;
        let nested = root_cfg
            .call_points
            .iter()
            .filter(|call| {
                call.occurrence.source_span.source == validation.project.modules[0].source
            })
            .collect::<Vec<_>>();
        assert_eq!(nested.len(), 1);
        let evaluated = &root_cfg.points[nested[0].evaluated_point.0];
        assert!(root_cfg.top_level.points.iter().any(|event| event.point == evaluated.id && matches!(&event.execution, crate::scalar::ScalarTopLevelPointExecution::Connected { occurrence: crate::scalar::ScalarTopLevelOccurrence::Call(call), .. } if call == &nested[0].occurrence)));
        assert!(root_cfg
            .holder_edges
            .iter()
            .any(|edge| edge.from == evaluated.id));
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

#[test]
fn closure_capture_uses_shared_cell_and_two_pointer_callable_layout_on_both_targets() {
    let source = SourceIdentity::new(
        "project".into(),
        "package".into(),
        "src/main.w".into(),
        "r1".into(),
    );
    let text = "%%start\nstruct copy Reader { u8() read; }\nReader(u64) make = fn(count) { u8[count] bytes; bytes[0] = 4; Reader result = { .read = fn { bytes[0] }; }; result };\nunit() run = fn { Reader kept = make(2); Reader copied = kept; kept.read(); copied.read(); };\n%%end";
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
            assert!(llvm.contains("__wosy_closure_"), "{llvm}");
            assert!(llvm.contains("insertvalue { ptr, ptr }"), "{llvm}");
            assert!(llvm.contains("closure_environment"), "{llvm}");
            assert!(llvm.contains("{ ptr, ptr }"), "{llvm}");
            assert!(llvm.contains("environment_cell"), "{llvm}");
            assert!(llvm.contains("capture_cell"), "{llvm}");
            assert!(llvm.contains("capture_address"), "{llvm}");
        }
    }
}

#[test]
fn callable_field_dispatch_uses_indirect_call_with_environment_on_both_targets_and_pipelines() {
    let source = SourceIdentity::new(
        "project".into(),
        "package".into(),
        "src/main.w".into(),
        "r1".into(),
    );
    let text = "%%start\nstruct Counter { i32(i32) advance; }\nCounter(i32) make = fn(start) { i32 total = start; Counter built = { .advance = fn(delta) { total = total + delta; total }; }; built };\nunit() run = fn { Counter held = make(40); i32 first = held.advance(1); first; };\nrun();\n%%end";
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
            assert!(llvm.contains("callable_field"), "{llvm}");
            assert!(llvm.contains("field_code"), "{llvm}");
            assert!(llvm.contains("field_environment"), "{llvm}");
            assert!(llvm.contains("field_call"), "{llvm}");
        }
    }
}

#[test]
fn top_level_callable_field_literal_captures_global_binding_on_both_targets() {
    let source = SourceIdentity::new(
        "project".into(),
        "package".into(),
        "src/main.w".into(),
        "r1".into(),
    );
    let text = "%%start\nstruct copy Reader { u8() read; }\nu64 length = 2;\nu8[length] bytes;\nbytes[0] = 5;\nReader result = { .read = fn { bytes[0] }; };\nu8 observed = result.read();\n%%end";
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
            assert!(llvm.contains("__wosy_closure_"), "{llvm}");
            assert!(llvm.contains("closure_environment"), "{llvm}");
            assert!(llvm.contains("environment_cell"), "{llvm}");
            assert!(llvm.contains("capture_address"), "{llvm}");
            assert!(llvm.contains("field_call"), "{llvm}");
        }
    }
}

#[test]
fn copyable_fixed_widening_materializes_independent_copy_on_both_targets_and_pipelines() {
    let source = SourceIdentity::new(
        "project".into(),
        "package".into(),
        "src/main.w".into(),
        "r1".into(),
    );
    let text = "%%start\nu8() run = fn { u8[2] fixed = [4, 5]; u8[] widened = fixed; u8 original = fixed[0]; widened[0] + original };\n%%end";
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
        for (llvm, name) in [
            (
                emit_scalar_llvm(&single).expect("single LLVM").to_text(),
                "run".to_owned(),
            ),
            (
                emit_scalar_project_llvm(&project)
                    .expect("project LLVM")
                    .to_text(),
                project_function_name(&source, "run"),
            ),
        ] {
            let body = &llvm[llvm.find(&format!("@{name}(")).unwrap()..];
            let body = &body[..body.find("\n}").unwrap()];
            assert!(body.contains("call ptr @__wosy_core_alloc"), "{llvm}");
            assert!(body.contains("llvm.memcpy"), "{llvm}");
            assert_eq!(
                body.matches("call void @__wosy_core_free").count(),
                1,
                "{llvm}"
            );
        }
    }
}

#[test]
fn move_only_fixed_transfer_moves_once_and_rejects_second_move_on_both_targets() {
    let source = SourceIdentity::new(
        "project".into(),
        "package".into(),
        "src/main.w".into(),
        "r1".into(),
    );
    let text = "%%start\nstruct Payload { u8 value; }\nPayload[](Payload[]) relay = fn(value) { value };\nunit() run = fn { Payload[2] fixed = [{ .value = 6; }, { .value = 7; }]; Payload[] moved = relay(fixed); moved; };\n%%end";
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
        for (llvm, name) in [
            (
                emit_scalar_llvm(&single).expect("single LLVM").to_text(),
                "run".to_owned(),
            ),
            (
                emit_scalar_project_llvm(&project)
                    .expect("project LLVM")
                    .to_text(),
                project_function_name(&source, "run"),
            ),
        ] {
            let body = &llvm[llvm.find(&format!("@{name}(")).unwrap()..];
            let body = &body[..body.find("\n}").unwrap()];
            assert_eq!(
                body.matches("call ptr @__wosy_core_alloc").count(),
                1,
                "{llvm}"
            );
            assert!(body.contains("llvm.memcpy"), "{llvm}");
            assert_eq!(
                body.matches("call void @__wosy_core_free").count(),
                1,
                "{llvm}"
            );
        }
    }
    let invalid = "%%start\nstruct Payload { u8 value; }\nPayload[](Payload[]) relay = fn(value) { value };\nunit() run = fn { Payload[2] fixed = [{ .value = 6; }, { .value = 7; }]; Payload[] moved = relay(fixed); Payload[] second = relay(fixed); moved; second; };\n%%end";
    for layout in [ScalarTargetLayout::WASM32, ScalarTargetLayout::NATIVE64] {
        let parsed = parse_source(source.clone(), invalid.to_owned(), &[]);
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
                        && diagnostic.labels.iter().any(|label| {
                            label.span.range.start
                                == invalid.rfind("relay(fixed)").unwrap() as u32 + 6
                        })
                }),
                "{diagnostics:?}"
            );
        }
    }
}

#[test]
fn local_callable_array_overwrite_and_copied_holder_release_on_both_targets() {
    let source = SourceIdentity::new(
        "project".into(),
        "package".into(),
        "src/main.w".into(),
        "r1".into(),
    );
    let text = "%%start\nunit(u64) inspect = fn(count) { (u8())[count] items; u8 first = 1; u8 second = 2; items[0] = fn { first }; items[1] = fn { second }; u8() kept = items[0]; items[0] = fn { first }; items[0] = fn { first }; u8() sibling = items[1]; u8 a = kept(); u8 b = sibling(); if (a != 1 || b != 2) { core.system_panic(); }; items; };\ninspect(2);\n%%end";
    for layout in [ScalarTargetLayout::WASM32, ScalarTargetLayout::NATIVE64] {
        let parsed = parse_source(source.clone(), text.to_owned(), &[]);
        assert!(parsed.diagnostics.is_empty(), "{:?}", parsed.diagnostics);
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
        for result in [
            emit_scalar_llvm(&single),
            emit_scalar_project_llvm(&project),
        ] {
            let llvm = result.expect("local callable array lifecycle").to_text();
            assert!(llvm.contains("selected_old_environment"), "{llvm}");
            assert!(llvm.contains("callable_flag_installed"), "{llvm}");
            assert!(llvm.contains("environment_holder_retained"), "{llvm}");
            assert!(llvm.contains("callable_array_private_length"), "{llvm}");
            assert!(
                llvm.contains("call void @__wosy_core_free(ptr %callable_array_allocation_base)"),
                "{llvm}"
            );
        }
    }
}

#[test]
fn local_callable_array_explicit_free_and_replacement_release_on_both_targets() {
    let source = SourceIdentity::new(
        "project".into(),
        "package".into(),
        "src/main.w".into(),
        "r1".into(),
    );
    for (text, expected_releases) in [
        ("%%start\nunit(u64) inspect = fn(count) { (u8())[count] items; u8 value = 1; items[0] = fn { value }; unsafe { core.free(&items); }; };\n%%end", 1),
        ("%%start\nunit(u64) inspect = fn(count) { (u8())[count] items; u8 value = 1; items[0] = fn { value }; (u8())[count] replacement; u8 other = 2; replacement[0] = fn { other }; items = replacement; items; };\n%%end", 2),
    ] {
        for layout in [ScalarTargetLayout::WASM32, ScalarTargetLayout::NATIVE64] {
            let parsed = parse_source(source.clone(), text.to_owned(), &[]);
            assert!(parsed.diagnostics.is_empty(), "{:?}", parsed.diagnostics);
            let single = derive_scalar_program_with_layout(&parsed.result, layout);
            assert!(single.diagnostics.is_empty(), "{:?}", single.diagnostics);
            let project = validate_scalar_project(ScalarProject::new(
                vec![ScalarModule::from_program(single.program.clone(), Vec::new())], vec![source.clone()],
            ));
            assert!(project.diagnostics.is_empty(), "{:?}", project.diagnostics);
            for result in [emit_scalar_llvm(&single), emit_scalar_project_llvm(&project)] {
                let llvm = result.expect("callable array release").to_text();
                assert!(llvm.contains("callable_array_allocation_base"), "{llvm}");
                assert!(llvm.contains("callable_array_private_length"), "{llvm}");
                assert_eq!(llvm.matches("call void @__wosy_core_free(ptr %callable_array_allocation_base").count(), expected_releases, "{llvm}");
            }
        }
    }
}

#[test]
fn promoted_array_cell_shares_replacement_length_between_returned_siblings() {
    let source = SourceIdentity::new(
        "project".into(),
        "package".into(),
        "src/main.w".into(),
        "r1".into(),
    );
    let text = "%%start\nstruct copy Twin { u8(u8) write; u8(u64) read; }\nTwin(u64) make = fn(count) { u8[count] slots; slots[0] = 1; slots[1] = 2; Twin result = { .write = fn(value) { slots[0] = value; slots[0] }; .read = fn(index) { slots[index] }; }; u64 extended = count + 1; u8[extended] next; next[0] = 4; next[1] = 5; next[2] = 6; slots = next; result };\nunit() verify = fn { Twin pair = make(2); if (pair.read(2) != 6 || pair.write(9) != 9 || pair.read(0) != 9) { core.system_panic(); }; };\nverify();\n%%end";
    for layout in [ScalarTargetLayout::NATIVE64, ScalarTargetLayout::WASM32] {
        let parsed = parse_source(source.clone(), text.to_owned(), &[]);
        assert!(parsed.diagnostics.is_empty(), "{:?}", parsed.diagnostics);
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
        let ScalarItem::Function(make) = &project.project.modules[0].items[1] else {
            panic!("maker function")
        };
        let restored: crate::scalar::ScalarReferenceCfg = serde_json::from_str(
            &serde_json::to_string(&make.reference_cfg).expect("serialize correlated facts"),
        )
        .expect("restore correlated facts");
        assert_eq!(restored, make.reference_cfg);
        for result in [
            emit_scalar_llvm(&single),
            emit_scalar_project_llvm(&project),
        ] {
            let llvm = result.expect("shared promoted array length").to_text();
            assert!(
                llvm.matches("capture_cell_current_length_address").count() >= 4,
                "{llvm}"
            );
            assert!(llvm.contains("capture_cell_incoming_length"), "{llvm}");
            assert!(llvm.contains("%capture_cell_backing = load ptr"), "{llvm}");
            let length = llvm
                .find("%capture_cell_incoming_length = load i64")
                .unwrap();
            let begin = llvm[..length]
                .rfind("call void @__wosy_begin_graph_mutation()")
                .unwrap();
            let end = llvm[length..]
                .find("call void @__wosy_end_graph_mutation()")
                .unwrap()
                + length;
            assert!(
                llvm[begin..end].contains("%capture_cell_previous_backing = load ptr"),
                "{llvm}"
            );
            assert!(
                llvm[begin..end].contains("capture_cell_incoming_length"),
                "{llvm}"
            );
            assert_eq!(
                llvm.matches("call void @__wosy_begin_graph_mutation()")
                    .count(),
                llvm.matches("call void @__wosy_end_graph_mutation()")
                    .count(),
                "{llvm}"
            );
        }
    }
}

#[test]
fn selected_callable_assignment_records_evaluated_index_and_address_on_both_targets() {
    let source = SourceIdentity::new(
        "project".into(),
        "package".into(),
        "src/main.w".into(),
        "r1".into(),
    );
    let text = "%%start\nunit(u64) inspect = fn(count) { (u8())[count] readers; u64 index = 0; while (index < count) { u8 captured = 7; readers[index] = fn { captured }; index = index + 1; } readers; };\ninspect(2);\n%%end";
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
            assert!(llvm.contains("%selected_index_slot = alloca i64"), "{llvm}");
            assert!(
                llvm.contains("%selected_address_slot = alloca ptr"),
                "{llvm}"
            );
            assert!(
                llvm.contains("%selected_present_slot = alloca i1"),
                "{llvm}"
            );
            assert!(
                llvm.contains("store i1 false, ptr %selected_present_slot"),
                "{llvm}"
            );
            assert!(
                llvm.contains("store ptr %array_element, ptr %selected_address_slot"),
                "{llvm}"
            );
            let selected = llvm
                .find("%array_element = getelementptr")
                .expect("assignment element address");
            let begin = llvm[..selected]
                .rfind("call void @__wosy_begin_graph_mutation()")
                .expect("assignment acquire");
            let length = llvm[begin..selected]
                .find("selected_current_length")
                .expect("assignment current length")
                + begin;
            let recorded = llvm[selected..]
                .find("store ptr %array_element, ptr %selected_address_slot")
                .expect("selected address publication")
                + selected;
            assert!(
                begin < length && length < selected && selected < recorded,
                "{llvm}"
            );
            assert!(
                llvm.contains("store i1 true, ptr %selected_present_slot"),
                "{llvm}"
            );
            assert_eq!(
                llvm.matches("store ptr %array_element, ptr %selected_address_slot")
                    .count(),
                1,
                "{llvm}"
            );
        }
    }
}

#[test]
fn selected_callable_index_call_is_emitted_once_per_write_on_both_targets() {
    let source = SourceIdentity::new(
        "project".into(),
        "package".into(),
        "src/main.w".into(),
        "r1".into(),
    );
    let text = "%%start\nu64 counter = 0;\nu64() next = fn { counter = counter + 1; counter };\nunit(u64) inspect = fn(count) { (u8())[count] readers; readers[next()] = fn { 1 }; readers; };\ninspect(2);\n%%end";
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
        for (llvm, next) in [
            (
                emit_scalar_llvm(&single).expect("single LLVM").to_text(),
                "next".to_owned(),
            ),
            (
                emit_scalar_project_llvm(&project)
                    .expect("project LLVM")
                    .to_text(),
                project_function_name(&source, "next"),
            ),
        ] {
            assert_eq!(
                llvm.matches("store ptr %array_element, ptr %selected_address_slot")
                    .count(),
                1,
                "{llvm}"
            );
            assert_eq!(
                llvm.matches(&format!("call i64 @{next}()")).count(),
                1,
                "{llvm}"
            );
            assert!(
                llvm.contains("store i64 %call, ptr %selected_index_slot"),
                "{llvm}"
            );
        }
    }
}

#[test]
fn selected_callable_branch_skips_initialized_slot_on_both_targets() {
    let source = SourceIdentity::new(
        "project".into(),
        "package".into(),
        "src/main.w".into(),
        "r1".into(),
    );
    let text = "%%start\nunit(u64, bool) inspect = fn(count, choose) { (u8())[count] readers; if (choose) { readers[0] = fn { 1 }; }; readers; };\ninspect(2, false);\n%%end";
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
            let init = llvm
                .find("store i1 false, ptr %selected_present_slot")
                .expect("entry initialization");
            let executed = llvm
                .find("store i1 true, ptr %selected_present_slot")
                .expect("executed write");
            assert!(init < executed, "{llvm}");
            assert_eq!(
                llvm.matches("store ptr %array_element, ptr %selected_address_slot")
                    .count(),
                1,
                "{llvm}"
            );
        }
    }
}

#[test]
fn selected_callable_assignments_have_distinct_typed_slots() {
    let source = SourceIdentity::new(
        "project".into(),
        "package".into(),
        "src/main.w".into(),
        "r1".into(),
    );
    let text = "%%start\nunit(u64) inspect = fn(count) { (u8())[count] readers; readers[0] = fn { 1 }; readers[1] = fn { 2 }; readers; };\ninspect(2);\n%%end";
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
            let slots = llvm
                .lines()
                .filter(|line| {
                    line.trim_start().starts_with("%selected_address_slot")
                        && line.contains(" = alloca ptr")
                })
                .collect::<Vec<_>>();
            assert_eq!(slots.len(), 2, "{llvm}");
            assert_ne!(slots[0], slots[1], "{llvm}");
        }
    }
}

#[test]
fn captured_environment_has_private_dynamic_header_on_both_targets_and_pipelines() {
    let source = SourceIdentity::new(
        "project".into(),
        "package".into(),
        "src/main.w".into(),
        "r1".into(),
    );
    let text = "%%start\nstruct copy Reader { u8() read; }\nReader(u8) make = fn(seed) { Reader result = { .read = fn { seed }; }; result };\nReader empty = { .read = fn { 1 }; };\nReader kept = make(3);\nkept.read();\nempty.read();\n%%end";
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
            assert!(llvm.contains("closure_environment_data = getelementptr inbounds i8, ptr %closure_allocation, i64 48"), "{llvm}");
            assert!(llvm.contains("environment_holder_count_address = getelementptr inbounds i8, ptr %closure_environment_data, i64 -24"), "{llvm}");
            assert!(llvm.contains("environment_allocation_base_address = getelementptr inbounds i8, ptr %closure_environment_data, i64 -16"), "{llvm}");
            assert!(llvm.contains("environment_descriptor_address = getelementptr inbounds i8, ptr %closure_environment_data, i64 -8"), "{llvm}");
            assert!(
                llvm.contains(&format!(
                    "ptr %environment_descriptor_address, align {}",
                    layout.pointer_alignment
                )),
                "{llvm}"
            );
            assert!(llvm.contains("store ptr @__wosy_environment_"), "{llvm}");
            assert!(
                llvm.contains("store i64 1, ptr %environment_holder_count_address"),
                "{llvm}"
            );
            assert_eq!(
                llvm.matches("store i64 1, ptr %environment_holder_count_address")
                    .count(),
                1,
                "{llvm}"
            );
            assert!(
                llvm.contains(
                    "store ptr %closure_allocation, ptr %environment_allocation_base_address"
                ),
                "{llvm}"
            );
            let size = 48 + layout.pointer_size;
            assert!(
                llvm.contains(&format!("call ptr @__wosy_core_alloc(i64 {size}, i64 8)")),
                "{llvm}"
            );
            assert!(llvm.contains("insertvalue { ptr, ptr } { ptr @"), "{llvm}");
            assert!(llvm.contains("ptr null"), "{llvm}");
            assert!(!llvm.contains("environment_allocation_release"), "{llvm}");
        }
    }
}

#[test]
fn captured_runtime_backing_returns_once_with_release_after_last_use_on_both_targets() {
    let source = SourceIdentity::new(
        "project".into(),
        "package".into(),
        "src/main.w".into(),
        "r1".into(),
    );
    let text = "%%start\nu8[2]() make = fn { u8[2] fixed = [4, 5]; fixed };\nstruct copy Reader { u8() read; }\nunit() run = fn { u8[2] local = make(); u8[] backing = local; Reader kept = { .read = fn { backing[0] }; }; kept.read(); local[0]; };\nrun();\n%%end";
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
        for (llvm, maker_name) in [
            (
                emit_scalar_llvm(&single).expect("single LLVM").to_text(),
                "make".to_owned(),
            ),
            (
                emit_scalar_project_llvm(&project)
                    .expect("project LLVM")
                    .to_text(),
                project_function_name(&source, "make"),
            ),
        ] {
            assert_eq!(
                llvm.matches("call void @__wosy_core_free").count()
                    - llvm
                        .matches("call void @__wosy_core_free(ptr %environment_allocation_base)")
                        .count(),
                4,
                "adopted backing frees once and its counted cell frees once: {llvm}"
            );
            let maker = &llvm[llvm.find(&format!("@{maker_name}(")).unwrap()..];
            let maker = &maker[..maker.find("\n}").unwrap()];
            assert!(!maker.contains("call void @__wosy_core_free"), "{llvm}");
        }
    }
}

#[test]
fn copied_closures_share_capture_cells_with_single_release_per_backing_on_both_targets() {
    let source = SourceIdentity::new(
        "project".into(),
        "package".into(),
        "src/main.w".into(),
        "r1".into(),
    );
    let text = "%%start\nstruct copy Probe { u8() read; }\nProbe(u8) outer = fn(seed) { seed; u8[2] initial = [4, 5]; u8[] bytes = initial; Probe alpha = { .read = fn { bytes[0] }; }; Probe beta = alpha; u8[2] other = [9, 8]; u8[] next = other; bytes = next; alpha.read(); beta.read(); alpha };\nProbe held = outer(0);\nheld.read();\n%%end";
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
        for (llvm, name) in [
            (
                emit_scalar_llvm(&single).expect("single LLVM").to_text(),
                "outer".to_owned(),
            ),
            (
                emit_scalar_project_llvm(&project)
                    .expect("project LLVM")
                    .to_text(),
                project_function_name(&source, "outer"),
            ),
        ] {
            let body = &llvm[llvm.find(&format!("@{name}(")).unwrap()..];
            let body = &body[..body.find("\n}").unwrap()];
            assert_eq!(
                llvm.matches("define internal i8 @__wosy_closure_").count(),
                1,
                "{llvm}"
            );
            assert_eq!(body.matches("llvm.memcpy").count(), 2, "{llvm}");
            assert_eq!(
                body.matches("call void @__wosy_core_free").count(),
                2,
                "shared backing frees once and its counted cell frees once: {llvm}"
            );
        }
    }
}

#[test]
fn borrowed_checked_reference_capture_emits_no_backing_release_on_both_targets() {
    let source = SourceIdentity::new(
        "project".into(),
        "package".into(),
        "src/main.w".into(),
        "r1".into(),
    );
    let text = "%%start\nstruct copy Meter { u8() read; }\nu8(u8) apply = fn(value) { *u8 reference = &value; Meter gauge = { .read = fn { *reference }; }; gauge.read() };\nu8 outcome = apply(4);\n%%end";
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
            assert!(llvm.contains("__wosy_closure_"), "{llvm}");
            assert!(!llvm.contains("llvm.memcpy"), "{llvm}");
            assert!(
                !llvm.contains("call void @__wosy_core_free(ptr %captured_backing)"),
                "{llvm}"
            );
        }
    }
}

#[test]
fn callable_field_checked_results_validate_owner_slots_on_both_targets_and_pipelines() {
    let source = SourceIdentity::new(
        "project".into(),
        "package".into(),
        "src/main.w".into(),
        "r1".into(),
    );
    let text = "%%start\nstruct Holder { *u8(*u8) get; }\nunit(u8) run = fn(limit) { limit; Holder held = { .get = fn(input) { &(*input) }; }; u8 cell = 1; *u8 result = held.get(&cell); u8 value = *result; value; };\nrun(0);\n%%end";
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
            assert!(llvm.contains("field_call"), "{llvm}");
            assert!(llvm.contains("field_owner_0"), "{llvm}");
            assert!(llvm.contains("field_return_owned_0"), "{llvm}");
        }
    }
}

#[test]
fn captured_binding_overwrite_releases_each_backing_once_on_both_targets() {
    let source = SourceIdentity::new(
        "project".into(),
        "package".into(),
        "src/main.w".into(),
        "r1".into(),
    );
    let text = "%%start\nstruct copy Probe { u8() read; }\nunit() outer = fn { u8[2] initial = [4, 5]; u8[] bytes = initial; Probe alpha = { .read = fn { bytes[0] }; }; Probe beta = alpha; u8[2] other = [9, 8]; u8[] next = other; bytes = next; alpha.read(); beta.read(); };\nouter();\n%%end";
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
            // Collector subunit 5: the replaced backing's cell carries two
            // owner releases on the terminal exit (creator plus installing
            // environment), so each adopted backing and the shared counted
            // cell free once across five release sites.
            assert_eq!(
                llvm.matches("call void @__wosy_core_free").count()
                    - llvm
                        .matches("call void @__wosy_core_free(ptr %environment_allocation_base)")
                        .count(),
                5,
                "each adopted backing frees once and the shared counted cell frees once: {llvm}"
            );
        }
    }
}

#[test]
fn move_only_parameter_owns_transferred_backing_on_both_targets_and_pipelines() {
    let source = SourceIdentity::new(
        "project".into(),
        "package".into(),
        "src/main.w".into(),
        "r1".into(),
    );
    let text = "%%start\nstruct Payload { u8 value; }\nunit(Payload[]) consume = fn(value) { Payload[] kept = value; kept; };\nPayload[1] fixed = [{ .value = 1; }];\nPayload[] moved = fixed;\nconsume(moved);\n%%end";
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
        for (llvm, consume_name) in [
            (
                emit_scalar_llvm(&single).expect("single LLVM").to_text(),
                "consume".to_owned(),
            ),
            (
                emit_scalar_project_llvm(&project)
                    .expect("project LLVM")
                    .to_text(),
                project_function_name(&source, "consume"),
            ),
        ] {
            assert_eq!(
                llvm.matches("call void @__wosy_core_free").count(),
                1,
                "{llvm}"
            );
            let consume = &llvm[llvm.find(&format!("@{consume_name}(")).unwrap()..];
            let consume = &consume[..consume.find("\n}").unwrap()];
            assert_eq!(
                consume.matches("call void @__wosy_core_free").count(),
                1,
                "{llvm}"
            );
            let main = &llvm[llvm.find("define i32 @main").unwrap()..];
            let main = &main[..main.find("\n}").unwrap()];
            assert!(main.contains("call ptr @__wosy_core_alloc"), "{llvm}");
            assert!(main.contains("llvm.memcpy"), "{llvm}");
            assert!(!main.contains("call void @__wosy_core_free"), "{llvm}");
        }
    }
}

#[test]
fn branch_valued_runtime_array_materializes_each_copy_once_on_both_targets() {
    let source = SourceIdentity::new(
        "project".into(),
        "package".into(),
        "src/main.w".into(),
        "r1".into(),
    );
    let text = "%%start\nu8[](bool) select = fn(flag) { u8[1] left = [1]; u8[2] right = [2, 3]; if (flag) { u8[] value = left; value } else { u8[] value = right; value } };\nu8[] selected = select(true);\nselected[0];\n%%end";
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
        for (llvm, name) in [
            (
                emit_scalar_llvm(&single).expect("single LLVM").to_text(),
                "select".to_owned(),
            ),
            (
                emit_scalar_project_llvm(&project)
                    .expect("project LLVM")
                    .to_text(),
                project_function_name(&source, "select"),
            ),
        ] {
            let body = &llvm[llvm.find(&format!("@{name}(")).unwrap()..];
            let body = &body[..body.find("\n}").unwrap()];
            assert_eq!(
                body.matches("call ptr @__wosy_core_alloc").count(),
                2,
                "{llvm}"
            );
            assert_eq!(body.matches("llvm.memcpy").count(), 2, "{llvm}");
            assert_eq!(
                body.matches("call void @__wosy_core_free").count(),
                1,
                "{llvm}"
            );
            assert_eq!(
                llvm.matches("call void @__wosy_core_free").count(),
                2,
                "{llvm}"
            );
        }
    }
}

#[test]
fn call_received_maker_captured_reader_releases_once_at_top_level_on_both_targets() {
    let source = SourceIdentity::new(
        "project".into(),
        "package".into(),
        "src/main.w".into(),
        "r1".into(),
    );
    let text = "%%start\nstruct copy Reader { u8() read; }\nReader(u64) make = fn(count) { u8[count] bytes; bytes[0] = 4; Reader result = { .read = fn { bytes[0] }; }; result };\nReader kept = make(2);\nu8 observed = kept.read();\n%%end";
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
        for (llvm, maker_name) in [
            (
                emit_scalar_llvm(&single).expect("single LLVM").to_text(),
                "make".to_owned(),
            ),
            (
                emit_scalar_project_llvm(&project)
                    .expect("project LLVM")
                    .to_text(),
                project_function_name(&source, "make"),
            ),
        ] {
            assert!(
                llvm.contains("%import_holder_environment = alloca ptr"),
                "{llvm}"
            );
            assert!(
                llvm.contains("store ptr null, ptr %import_holder_environment"),
                "{llvm}"
            );
            assert!(
                llvm.contains("store ptr %import_environment, ptr %import_holder_environment"),
                "{llvm}"
            );
            assert!(
                llvm.contains("store i1 true, ptr %import_holder_present"),
                "{llvm}"
            );
            assert_eq!(
                llvm.matches("call void @__wosy_core_free").count()
                    - llvm
                        .matches("call void @__wosy_core_free(ptr %environment_allocation_base)")
                        .count(),
                2,
                "adopted backing frees once and its counted cell frees once: {llvm}"
            );
            let maker = &llvm[llvm.find(&format!("@{maker_name}(")).unwrap()..];
            let maker = &maker[..maker.find("\n}").unwrap()];
            assert!(
                maker.contains("capture_cell_owner_dropped"),
                "maker exit drops its local cell ownership: {llvm}"
            );
        }
    }
}

#[test]
fn imported_maker_captured_reader_releases_backing_and_cell_once_on_both_targets() {
    let source = SourceIdentity::new(
        "project".into(),
        "package".into(),
        "src/main.w".into(),
        "r1".into(),
    );
    let text = "%%start\nstruct copy Reader { u8() read; }\nReader(u64) make = fn(count) { u8[count] bytes; bytes[0] = 4; Reader result = { .read = fn { bytes[0] }; }; result };\nReader kept = make(2);\nu8 observed = kept.read();\n%%end";
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
        for (llvm, maker_name) in [
            (
                emit_scalar_llvm(&single).expect("single LLVM").to_text(),
                "make".to_owned(),
            ),
            (
                emit_scalar_project_llvm(&project)
                    .expect("project LLVM")
                    .to_text(),
                project_function_name(&source, "make"),
            ),
        ] {
            assert!(
                !llvm.contains("capture_release.check"),
                "synthetic captured release must be gone: {llvm}"
            );
            assert!(
                !llvm.contains("imported_capture_cell_slot"),
                "synthetic import cell slot must be gone: {llvm}"
            );
            assert_eq!(
                llvm.matches("call void @__wosy_core_free").count()
                    - llvm
                        .matches("call void @__wosy_core_free(ptr %environment_allocation_base)")
                        .count(),
                2,
                "adopted backing frees once and its counted cell frees once: {llvm}"
            );
            let maker = &llvm[llvm.find(&format!("@{maker_name}(")).unwrap()..];
            let maker = &maker[..maker.find("\n}").unwrap()];
            assert!(
                maker.contains("capture_cell_owner_dropped"),
                "maker exit drops its local cell ownership: {llvm}"
            );
            assert!(
                llvm.contains("terminal_holder_environment"),
                "terminal root drops the installed holder: {llvm}"
            );
        }
    }
}

#[test]
fn global_array_capture_retires_backing_once_at_terminal_on_both_targets() {
    let source = SourceIdentity::new(
        "project".into(),
        "package".into(),
        "src/main.w".into(),
        "r1".into(),
    );
    let text = "%%start\nstruct copy Reader { u8() read; }\nu64 length = 2;\nu8[length] bytes;\nbytes[0] = 5;\nReader result = { .read = fn { bytes[0] }; };\nu8 observed = result.read();\nobserved;\n%%end";
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
                !llvm.contains("capture_release.check"),
                "global backing retires at terminal, not via synthetic release: {llvm}"
            );
            assert!(
                !llvm.contains("captured_cell_pointer_slot"),
                "top-level remainder cell needs no synthetic capture slot: {llvm}"
            );
            assert_eq!(
                llvm.matches("call void @__wosy_core_free").count()
                    - llvm
                        .matches("call void @__wosy_core_free(ptr %environment_allocation_base)")
                        .count(),
                3,
                "scope backing release plus terminal cell backing/base blocks; runtime executes exactly once each: {llvm}"
            );
            assert_eq!(
                llvm.matches("capture_cell_owner_dropped = sub i64").count(),
                1,
                "terminal drops the top-level remainder cell once: {llvm}"
            );
            assert!(
                llvm.contains("terminal_holder_environment"),
                "terminal root drops the installed holder: {llvm}"
            );
        }
    }
}

#[test]
fn global_capture_import_skips_synthetic_registration_on_both_targets() {
    let source = SourceIdentity::new(
        "project".into(),
        "package".into(),
        "src/main.w".into(),
        "r1".into(),
    );
    let text = "%%start\nstruct copy Reader { u8() read; }\nu64 length = 2;\nu8[length] gbytes;\ngbytes[0] = 5;\nReader(u64) make = fn(count) { count; Reader result = { .read = fn { gbytes[0] }; }; result };\nReader kept = make(2);\nu8 observed = kept.read();\nobserved;\n%%end";
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
        for (llvm, maker_name) in [
            (
                emit_scalar_llvm(&single).expect("single LLVM").to_text(),
                "make".to_owned(),
            ),
            (
                emit_scalar_project_llvm(&project)
                    .expect("project LLVM")
                    .to_text(),
                project_function_name(&source, "make"),
            ),
        ] {
            assert!(
                !llvm.contains("capture_release.check"),
                "global backing retires at terminal, not via synthetic release: {llvm}"
            );
            assert!(
                !llvm.contains("imported_capture_cell_slot"),
                "global import takes no synthetic ownership: {llvm}"
            );
            assert_eq!(
                llvm.matches("call void @__wosy_core_free").count()
                    - llvm
                        .matches("call void @__wosy_core_free(ptr %environment_allocation_base)")
                        .count(),
                1,
                "global backing frees exactly once at terminal: {llvm}"
            );
            let maker = &llvm[llvm.find(&format!("@{maker_name}(")).unwrap()..];
            let maker = &maker[..maker.find("\n}").unwrap()];
            assert!(
                !maker.contains("call void @__wosy_core_free"),
                "maker takes no ownership of the global backing: {llvm}"
            );
            assert!(
                llvm.contains("terminal_holder_environment"),
                "terminal root drops the installed holder: {llvm}"
            );
        }
    }
}

#[test]
fn imported_holder_slots_distinguish_calls_and_loop_updates_on_both_targets() {
    let source = SourceIdentity::new(
        "project".into(),
        "package".into(),
        "src/main.w".into(),
        "r1".into(),
    );
    for (body, count) in [
        ("Reader first = make(2); Reader second = make(3); first.read(); second.read();", 2),
        ("u64 index = 0; while (index < 2) { Reader item = make(2); item.read(); index = index + 1; }", 1),
        ("Reader first = { .read = fn { 1 }; }; Reader second = { .read = fn { 2 }; }; u64 index = 0; while (index < 2) { Reader item = make(2); if (index == 0) { first = item; } else { second = item; }; index = index + 1; } first.read(); second.read();", 1),
        ("bool choose = false; if (choose) { Reader item = make(2); item.read(); };", 1),
    ] {
        let text = format!("%%start\nstruct copy Reader {{ u8() read; }}\nReader(u64) make = fn(count) {{ u8[count] bytes; bytes[0] = 4; Reader result = {{ .read = fn {{ bytes[0] }}; }}; result }};\nunit() run = fn {{ {body} }};\nrun();\n%%end");
        for layout in [ScalarTargetLayout::WASM32, ScalarTargetLayout::NATIVE64] {
            let parsed = parse_source(source.clone(), text.clone(), &[]);
            let single = derive_scalar_program_with_layout(&parsed.result, layout);
            assert!(single.diagnostics.is_empty(), "{:?}", single.diagnostics);
            let project = validate_scalar_project(ScalarProject::new(
                vec![ScalarModule::from_program(single.program.clone(), Vec::new())],
                vec![source.clone()],
            ));
            assert!(project.diagnostics.is_empty(), "{:?}", project.diagnostics);
            for items in [&single.program.items, &project.project.modules[0].items] {
                let maker = items.iter().find_map(|item| match item {
                    ScalarItem::Function(function) if function.name == "make" => Some(function),
                    _ => None,
                }).expect("maker");
                let run = items.iter().find_map(|item| match item {
                    ScalarItem::Function(function) if function.name == "run" => Some(function),
                    _ => None,
                }).expect("run");
                let ScalarType::Callable { outputs, .. } = &maker.signature else { panic!("maker signature") };
                let types = outputs.outputs.iter().map(|output| output.ty.clone()).collect::<Vec<_>>();
                let events = super::holder_events::HolderEvents::new(&run.reference_cfg).expect("typed call edges");
                let mut matched = 0;
                for call in &run.reference_cfg.call_points {
                    if !matches!(&run.reference_cfg.points[call.evaluated_point.0].kind,
                        CfgPointKind::Call { expression: ScalarExpression::Call { name, .. }, .. } if name == "make") {
                        continue;
                    }
                    events.evaluated_call(&call.occurrence, &types).expect("typed maker call");
                    let (point, holder) = events.imported_output_holder(0, &[0]).expect("projected captured holder");
                    assert_eq!(point, call.evaluated_point);
                    let crate::scalar::CallableHolderPlace::Projection(place) = holder else { panic!("field holder") };
                    assert_eq!(place.source, source);
                    assert_eq!(place.binding.function_span, run.span);
                    matched += 1;
                }
                assert_eq!(matched, count);
            }
            if count == 1 {
                continue;
            }
            for (llvm, name) in [
                (emit_scalar_llvm(&single).expect("single imported holder IR").to_text(), "run".to_owned()),
                (emit_scalar_project_llvm(&project).expect("project imported holder IR").to_text(), project_function_name(&source, "run")),
            ] {
                let body = &llvm[llvm.find(&format!("@{name}(")).expect("run function")..];
                let body = &body[..body.find("\n}").expect("run end")];
                assert_eq!(body.lines().filter(|line| line.trim_start().starts_with("%import_holder_environment")
                    && line.contains(" = alloca ptr")).count(), count, "{body}");
                assert_eq!(body.matches("store i1 false, ptr %import_holder_present").count(), count * 2, "{body}");
                assert_eq!(body.matches("store i1 true, ptr %import_holder_present").count(), count, "{body}");
            }
        }
    }
}

#[test]
fn final_environment_holder_drops_unique_promoted_cells_through_cell_owner_count() {
    let source = SourceIdentity::new(
        "project".into(),
        "package".into(),
        "src/main.w".into(),
        "r1".into(),
    );
    let parsed = parse_source(
        source.clone(),
        "%%start\nu8() run = fn { 1 };\n%%end".to_owned(),
        &[],
    );
    for layout in [ScalarTargetLayout::WASM32, ScalarTargetLayout::NATIVE64] {
        let single = derive_scalar_program_with_layout(&parsed.result, layout);
        assert!(single.diagnostics.is_empty(), "{:?}", single.diagnostics);
        let function = single
            .program
            .items
            .iter()
            .find_map(|item| match item {
                ScalarItem::Function(function) if function.name == "run" => Some(function),
                _ => None,
            })
            .expect("test function");
        let context = Context::create();
        let builder = context.create_builder();
        let functions = BTreeMap::new();
        let call_targets = BTreeMap::new();
        let signatures = BTreeMap::new();
        let dynamic_owner_outputs = BTreeMap::new();
        let literal_signatures = std::collections::HashMap::new();
        let literal_captures = std::collections::HashMap::new();
        let literal_reference_cfgs = std::collections::HashMap::new();
        let globals = BTreeMap::new();
        let module = context.create_module("drop_shared_capture");
        super::declare_core_runtime(&context, &module);
        let test = module.add_function(
            "retire_holder",
            context.void_type().fn_type(&[], false),
            None,
        );
        builder.position_at_end(context.append_basic_block(test, "entry"));
        let mut state = super::EmitState {
            builder: &builder,
            module: &module,
            functions: &functions,
            call_targets: &call_targets,
            signatures: &signatures,
            dynamic_owner_outputs: &dynamic_owner_outputs,
            literal_signatures: &literal_signatures,
            literal_captures: &literal_captures,
            literal_reference_cfgs: &literal_reference_cfgs,
            covered_literals: std::collections::HashSet::new(),
            cyclic_literals: Vec::new(),
            counted_fixed_cells: std::collections::BTreeSet::new(),
            top_level_remainder_cells: Vec::new(),
            counted_cell_backings: Vec::new(),
            resolved_calls: BTreeMap::new(),
            source: source.clone(),
            capture_cells: BTreeMap::new(),
            backing: super::ownership::validated_backing_maps(&function.reference_cfg),
            holder_events: super::holder_events::HolderEvents::new(&function.reference_cfg)
                .expect("events"),
            escaping: BTreeMap::new(),
            backing_values: BTreeMap::new(),
            backing_slots: BTreeMap::new(),
            imported_cell_slots: BTreeMap::new(),
            backing_captured: std::collections::BTreeSet::new(),
            values: BTreeMap::new(),
            storage: BTreeMap::new(),
            fresh_return_outputs: std::collections::BTreeSet::new(),
            branch_return_outputs: BTreeMap::new(),
            joined_return_bindings: BTreeMap::new(),
            joined_assignment_owners: BTreeMap::new(),
            return_output_types: Vec::new(),
            return_body_span: Some(function.body.span),
            checked_call_owners: BTreeMap::new(),
            checked_owner_flags: BTreeMap::new(),
            call_owner_flags: BTreeMap::new(),
            return_owner_flags: BTreeMap::new(),
            return_owner_slots: BTreeMap::new(),
            return_length_slots: BTreeMap::new(),
            call_length_slots: BTreeMap::new(),
            runtime_array_owners: BTreeMap::new(),
            runtime_array_allocations: BTreeMap::new(),
            runtime_array_lengths: BTreeMap::new(),
            runtime_array_live: BTreeMap::new(),
            mutable_runtime_allocations: std::collections::BTreeSet::new(),
            invalidations: BTreeMap::new(),
            automatic_return_result_owners: BTreeMap::new(),
            globals: globals.clone(),
            all_globals: globals,
            structs: &single.program.structs,
            target_layout: layout,
            next_literal: 0,
            next_block: 0,
        };
        let foreign =
            wosy_syntax::SourceSpan::new(source.clone(), wosy_syntax::ByteSpan::new(200, 201));
        let error = super::imports::imported_producer_descriptor(&state, &foreign)
            .err()
            .expect("foreign producer must be rejected");
        assert!(
            error.contains("src/main.w") && error.contains("lacks a Wosy producer"),
            "{error}"
        );
        let base = builder
            .build_alloca(context.i8_type().array_type(128), "environment_base")
            .expect("environment base");
        let environment = unsafe {
            builder.build_in_bounds_gep(
                context.i8_type(),
                base,
                &[context.i64_type().const_int(24, false)],
                "environment",
            )
        }
        .expect("environment payload");
        builder
            .build_store(
                super::ownership::environment_count_address(&context, &state, environment)
                    .expect("count"),
                context.i64_type().const_int(1, false),
            )
            .expect("holder count");
        builder
            .build_store(
                super::ownership::environment_base_address(&context, &state, environment)
                    .expect("base slot"),
                base,
            )
            .expect("base pointer");
        let pointer = context.ptr_type(inkwell::AddressSpace::default());
        let trace = module.add_function(
            "test_trace",
            context
                .void_type()
                .fn_type(&[pointer.into(), pointer.into()], false),
            None,
        );
        let dispose = module.add_function(
            "test_dispose",
            context.void_type().fn_type(&[pointer.into()], false),
            None,
        );
        let descriptor_type = context.struct_type(&[pointer.into(), pointer.into()], false);
        let descriptor = module.add_global(descriptor_type, None, "test_descriptor");
        descriptor.set_initializer(&descriptor_type.const_named_struct(&[
            trace.as_global_value().as_pointer_value().into(),
            dispose.as_global_value().as_pointer_value().into(),
        ]));
        builder
            .build_store(
                super::ownership::environment_descriptor_address(&context, &state, environment)
                    .expect("descriptor slot"),
                descriptor.as_pointer_value(),
            )
            .expect("descriptor pointer");
        builder.position_at_end(context.append_basic_block(trace, "entry"));
        builder.build_return(None).expect("trace return");
        builder.position_at_end(context.append_basic_block(dispose, "entry"));
        builder.build_return(None).expect("dispose return");
        builder.position_at_end(test.get_first_basic_block().expect("test entry"));
        let cell_base = builder
            .build_alloca(context.i8_type().array_type(64), "capture_base")
            .expect("cell base");
        let cell = unsafe {
            builder.build_in_bounds_gep(
                context.i8_type(),
                cell_base,
                &[context.i64_type().const_int(24, false)],
                "capture",
            )
        }
        .expect("cell payload");
        builder
            .build_store(
                super::ownership::capture_cell_count_address(&context, &state, cell)
                    .expect("cell count"),
                context.i64_type().const_int(1, false),
            )
            .expect("cell owner");
        builder
            .build_store(
                super::ownership::capture_cell_base_address(&context, &state, cell)
                    .expect("cell base"),
                cell_base,
            )
            .expect("cell allocation base");
        builder
            .build_store(
                super::ownership::capture_cell_owned_address(&context, &state, cell)
                    .expect("owned flag"),
                context.i64_type().const_zero(),
            )
            .expect("borrowed backing");
        let pointer_size = layout.pointer_size;
        for offset in [0, pointer_size] {
            let slot = unsafe {
                builder.build_in_bounds_gep(
                    context.i8_type(),
                    environment,
                    &[context.i64_type().const_int(offset, false)],
                    "capture_slot",
                )
            }
            .expect("capture slot");
            builder
                .build_store(slot, cell)
                .expect("shared cell pointer");
        }
        let captures = [
            super::ownership::EnvironmentCaptureCell {
                cell_offset: 0,
                element: ScalarType::U8,
                residence: super::ownership::CaptureCellResidence::Promoted,
            },
            super::ownership::EnvironmentCaptureCell {
                cell_offset: pointer_size,
                element: ScalarType::U8,
                residence: super::ownership::CaptureCellResidence::Promoted,
            },
            super::ownership::EnvironmentCaptureCell {
                cell_offset: pointer_size * 2,
                element: ScalarType::U8,
                residence: super::ownership::CaptureCellResidence::Global,
            },
            super::ownership::EnvironmentCaptureCell {
                cell_offset: pointer_size * 3,
                element: ScalarType::U8,
                residence: super::ownership::CaptureCellResidence::External,
            },
        ];
        assert_eq!(captures.len(), 4);
        super::ownership::drop_environment(&context, &mut state, environment)
            .expect("typed final holder");
        builder.build_return(None).expect("return");
        module.verify().expect("valid final-holder IR");
        let ir = module.print_to_string().to_string();
        assert!(ir.contains("environment_descriptor_address"), "{ir}");
        assert!(ir.contains("environment_dispose_address"), "{ir}");
        assert!(
            ir.contains("call void %environment_dispose(ptr %environment)"),
            "{ir}"
        );
    }
}

#[test]
fn imported_shared_capture_cells_survive_branch_and_repeated_loop_site_on_both_targets() {
    let source = SourceIdentity::new(
        "project".into(),
        "package".into(),
        "src/main.w".into(),
        "r1".into(),
    );
    let text = "%%start\nstruct copy Captured { u8(u8) write; u8(u64) read; }\nCaptured(u64) make = fn(count) { u8[count] slots; slots[0] = 6; Captured result = { .write = fn(value) { slots[0] = value; slots[0] }; .read = fn(index) { slots[index] }; }; result };\nunit() run = fn { Captured first = make(2); Captured second = first; u64 index = 0; while (index < 2) { Captured next = make(2); if (index == 0) { first = next; } else { second = next; }; index = index + 1; } if (false) { Captured skipped = make(2); skipped.read(0); }; if (true) { Captured taken = make(2); taken.write(8); }; first.write(9); second.read(0); };\nrun();\n%%end";
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
        for (ir, run) in [
            (
                emit_scalar_llvm(&single)
                    .expect("single branch/loop holder IR")
                    .to_text(),
                "run".to_owned(),
            ),
            (
                emit_scalar_project_llvm(&project)
                    .expect("project branch/loop holder IR")
                    .to_text(),
                project_function_name(&source, "run"),
            ),
        ] {
            assert!(ir.contains("environment_holder_retained"), "{layout:?}");
            assert!(ir.contains("environment_holder_dropped"), "{layout:?}");
            assert!(ir.contains("capture_cell_owner_dropped"), "{layout:?}");
            assert!(ir.contains("environment.trace.unique"), "{ir}");
            assert!(ir.contains("environment.cell.unique"), "{ir}");
            let body = &ir[ir.find(&format!("@{run}(")).expect("run definition")..];
            let body = &body[..body.find("\n}").expect("run body")];
            assert!(body.contains("import_literal_matches"), "{layout:?}");
            assert!(body.contains("environment_last_holder"), "{body}");
            assert!(body.contains("environment_descriptor = load ptr"), "{body}");
            assert!(body.contains("call void %environment_dispose"), "{body}");
            assert!(!body.contains("environment_capture_cell_address"), "{body}");
            assert!(
                !body.contains("capture_release.check"),
                "classified import still uses synthetic cleanup: {layout:?}"
            );
        }
    }
}

#[test]
fn literal_maker_imported_shared_capture_requires_connected_output_transport() {
    let source = SourceIdentity::new(
        "project".into(),
        "package".into(),
        "src/main.w".into(),
        "r1".into(),
    );
    let text = "%%start\nstruct copy Captured { u8(u8) write; u8(u64) read; }\nunit() run = fn { Captured(u64) factory = fn(count) { u8[count] slots; slots[0] = 6; Captured result = { .write = fn(value) { slots[0] = value; slots[0] }; .read = fn(index) { slots[index] }; }; result }; Captured first = factory(2); Captured copy = first; copy.write(9); first.read(0); };\nrun();\n%%end";
    for layout in [ScalarTargetLayout::WASM32, ScalarTargetLayout::NATIVE64] {
        let parsed = parse_source(source.clone(), text.to_owned(), &[]);
        assert!(parsed.diagnostics.is_empty(), "{:?}", parsed.diagnostics);
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
        for result in [
            emit_scalar_llvm(&single),
            emit_scalar_project_llvm(&project),
        ] {
            let error = result
                .expect_err("literal maker needs connected imported output/field holder transport");
            assert!(
                error.contains("unsupported source-qualified captured callable literal maker"),
                "{layout:?}: {error}"
            );
            assert!(error.contains("src/main.w"), "{layout:?}: {error}");
        }
    }
}

#[test]
fn indirect_imported_shared_capture_requires_typed_scalar_occurrence() {
    let source = SourceIdentity::new(
        "project".into(),
        "package".into(),
        "src/main.w".into(),
        "r1".into(),
    );
    let text = "%%start\nstruct copy Captured { u8(u8) write; u8(u64) read; }\nCaptured(u64) make = fn(count) { u8[count] slots; slots[0] = 6; Captured result = { .write = fn(value) { slots[0] = value; slots[0] }; .read = fn(index) { slots[index] }; }; result };\nunit() run = fn { Captured(u64) factory = make; Captured kept = factory(2); Captured copy = kept; copy.write(9); kept.read(0); };\nrun();\n%%end";
    for layout in [ScalarTargetLayout::WASM32, ScalarTargetLayout::NATIVE64] {
        let parsed = parse_source(source.clone(), text.to_owned(), &[]);
        assert!(parsed.diagnostics.is_empty(), "{:?}", parsed.diagnostics);
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
        for result in [
            emit_scalar_llvm(&single),
            emit_scalar_project_llvm(&project),
        ] {
            let error =
                result.expect_err("indirect callable transport requires connected lowering");
            assert!(
                error.contains("unsupported source-qualified indirect callable value `make`"),
                "{layout:?}: {error}"
            );
            assert!(error.contains("src/main.w"), "{layout:?}: {error}");
        }
    }
}

#[test]
fn project_module_callable_struct_import_requires_typed_scalar_transport() {
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
    let child_text = "%%start\nstruct copy Captured { u8(u8) write; u8(u64) read; }\nCaptured(u64) make = fn(count) { u8[count] slots; slots[0] = 6; Captured result = { .write = fn(value) { slots[0] = value; slots[0] }; .read = fn(index) { slots[index] }; }; result };\n%%end";
    let root_text = "%%start\nchild = namespace package \"src/child.w\";\nunit() run = fn { child.Captured first = child.make(2); child.Captured kept = first; if (kept.write(9) != 9 || first.read(0) != 9) { core.system_panic(); }; };\nrun();\n%%end";
    for layout in [ScalarTargetLayout::WASM32, ScalarTargetLayout::NATIVE64] {
        let root = derive_scalar_program_with_layout(
            &parse_source(root_source.clone(), root_text.to_owned(), &[]).result,
            layout,
        );
        let child = derive_scalar_program_with_layout(
            &parse_source(child_source.clone(), child_text.to_owned(), &[]).result,
            layout,
        );
        assert!(child.diagnostics.is_empty(), "{:?}", child.diagnostics);
        let namespace_bindings = root
            .program
            .items
            .iter()
            .filter_map(|item| match item {
                ScalarItem::Namespace(namespace) => Some(crate::ScalarNamespaceBinding {
                    binding: namespace.binding.clone(),
                    target: child_source.clone(),
                    span: namespace.span,
                }),
                _ => None,
            })
            .collect();
        let project = validate_scalar_project(ScalarProject::new(
            vec![
                ScalarModule::new(root_source.clone(), root.program.items, namespace_bindings),
                ScalarModule::new(child_source.clone(), child.program.items, Vec::new()),
            ],
            vec![root_source.clone(), child_source.clone()],
        ));
        assert!(
            project
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.code == "B0003"),
            "module callable-struct transport unexpectedly validated on {layout:?}: {:?}",
            project.diagnostics
        );
    }
}

#[test]
fn imported_shared_capture_multi_output_has_typed_holder_cleanup_on_both_targets() {
    let source = SourceIdentity::new(
        "project".into(),
        "package".into(),
        "src/main.w".into(),
        "r1".into(),
    );
    let text = "%%start\nstruct copy Captured { u8(u8) write; u8(u64) read; }\n(Captured, u64)(u64) make = fn(count) { u8[count] slots; slots[0] = 6; Captured result = { .write = fn(value) { slots[0] = value; slots[0] }; .read = fn(index) { slots[index] }; }; result, count };\nunit() run = fn { Captured first, u64 length = make(2); Captured copy = first; if (length != 2 || copy.write(9) != 9 || first.read(0) != 9) { core.system_panic(); }; };\nrun();\n%%end";
    for layout in [ScalarTargetLayout::WASM32, ScalarTargetLayout::NATIVE64] {
        let parsed = parse_source(source.clone(), text.to_owned(), &[]);
        assert!(parsed.diagnostics.is_empty(), "{:?}", parsed.diagnostics);
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
        for ir in [
            emit_scalar_llvm(&single)
                .expect("single typed multi-output capture")
                .to_text(),
            emit_scalar_project_llvm(&project)
                .expect("project typed multi-output capture")
                .to_text(),
        ] {
            assert!(ir.contains("environment_holder_dropped"), "{layout:?}");
            assert!(!ir.contains("capture_release.check"), "{layout:?}");
        }
    }
}

#[test]
fn branch_local_capture_field_alternatives_retire_with_counted_holders() {
    let source = SourceIdentity::new(
        "project".into(),
        "package".into(),
        "src/main.w".into(),
        "r1".into(),
    );
    let text = "%%start\nstruct copy Holder { u8() action; }\nHolder(bool) choose = fn(flag) { if (flag) { u64 length = 2; u8[length] first; first[0] = 4; Holder left = { .action = fn { first[0] }; }; left } else { u64 length = 3; u8[length] second; second[0] = 7; Holder right = { .action = fn { second[0] }; }; right } };\nunit() run = fn { Holder left = choose(true); Holder copy = left; Holder right = choose(false); left.action = right.action; if (copy.action() != 4 || left.action() != 7) { core.system_panic(); }; };\nrun();\n%%end";
    for layout in [ScalarTargetLayout::NATIVE64, ScalarTargetLayout::WASM32] {
        let parsed = parse_source(source.clone(), text.to_owned(), &[]);
        assert!(parsed.diagnostics.is_empty(), "{:?}", parsed.diagnostics);
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
        for ir in [
            emit_scalar_llvm(&single)
                .expect("single counted branch cells")
                .to_text(),
            emit_scalar_project_llvm(&project)
                .expect("project counted branch cells")
                .to_text(),
        ] {
            assert!(ir.contains("environment_descriptor"), "{layout:?}");
            assert!(ir.contains("environment_holder_dropped"), "{layout:?}");
            assert!(ir.contains("environment.cell.unique"), "{layout:?}");
            assert!(ir.contains("capture_cell_owner_dropped"), "{layout:?}");
            assert!(
                ir.matches(" = private constant { ptr, ptr }").count() >= 2,
                "{layout:?}"
            );
            assert!(!ir.contains("capture_release.check"), "{layout:?}");
        }
    }
}

#[test]
fn shared_capture_holder_actions_retire_one_cell_after_two_returned_environments() {
    let source = SourceIdentity::new(
        "project".into(),
        "package".into(),
        "src/main.w".into(),
        "r1".into(),
    );
    let text = "%%start\nstruct copy Captured { u8(u8) write; u8(u64) read; }\nCaptured(u64) make = fn(count) { u8[count] slots; slots[0] = 6; Captured result = { .write = fn(value) { slots[0] = value; slots[0] }; .read = fn(index) { slots[index] }; }; u8[count] replacement; replacement[0] = 7; slots = replacement; result };\nunit() run = fn { Captured first = make(2); Captured kept = first; first.read = fn(index) { index; 8 }; kept.write(9); kept.read(0); };\nrun();\n%%end";
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
        for ir in [
            emit_scalar_llvm(&single)
                .expect("single owned holders")
                .to_text(),
            emit_scalar_project_llvm(&project)
                .expect("project owned holders")
                .to_text(),
        ] {
            assert!(
                ir.contains("environment_holder_retained"),
                "missing connected copied-holder retain for {layout:?}"
            );
            assert!(
                ir.contains("environment_holder_dropped"),
                "missing connected holder drop for {layout:?}"
            );
            assert!(
                ir.contains("capture_cell_owner_dropped"),
                "missing outer capture-cell owner exit for {layout:?}"
            );
            assert!(
                ir.contains("capture_cell_previous_backing"),
                "missing connected replacement for {layout:?}"
            );
        }
    }
}

#[test]
fn promoted_shared_capture_cell_has_two_environment_owners_on_both_targets() {
    let source = SourceIdentity::new(
        "project".into(),
        "package".into(),
        "src/main.w".into(),
        "r1".into(),
    );
    let text = "%%start\nstruct copy Captured { u8(u8) write; u8(u64) read; }\nCaptured(u64) make = fn(count) { u8[count] slots; slots[0] = 6; Captured result = { .write = fn(value) { slots[0] = value; slots[0] }; .read = fn(index) { slots[index] }; }; result };\nunit() run = fn { Captured kept = make(2); Captured copied = kept; copied.read(0); kept.write(9); };\nrun();\n%%end";
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
        for (llvm, name) in [
            (
                emit_scalar_llvm(&single)
                    .expect("single shared capture IR")
                    .to_text(),
                "make".to_owned(),
            ),
            (
                emit_scalar_project_llvm(&project)
                    .expect("project shared capture IR")
                    .to_text(),
                project_function_name(&source, "make"),
            ),
        ] {
            let body = &llvm[llvm.find(&format!("@{name}(")).expect("maker")..];
            let body = &body[..body.find("\n}").expect("maker end")];
            assert_eq!(
                body.lines()
                    .filter(|line| line.contains("promoted_capture_cell = getelementptr"))
                    .count(),
                1,
                "{body}"
            );
            assert_eq!(
                body.lines()
                    .filter(|line| line.contains("capture_cell_owner_retained")
                        && line.contains(" = add i64"))
                    .count(),
                2,
                "{body}"
            );
            for (name, offset) in [
                ("capture_cell_owner_count_address", "-24"),
                ("capture_cell_allocation_base_address", "-16"),
                ("capture_cell_owns_backing_address", "-8"),
            ] {
                assert!(
                    body.lines().any(|line| line.contains(name)
                        && line.contains("getelementptr inbounds i8")
                        && line.ends_with(&format!("i64 {offset}"))),
                    "{body}"
                );
            }
            assert!(
                body.lines().any(|line| line
                    .contains("store i64 1, ptr %capture_cell_owner_count_address")
                    && line.ends_with("align 8")),
                "{body}"
            );
            assert!(
                body.lines().any(|line| line
                    .contains("store i64 1, ptr %capture_cell_owns_backing_address")
                    && line.ends_with("align 8")),
                "{body}"
            );
            assert!(
                body.lines()
                    .any(|line| line.contains("store ptr %closure_allocation")
                        && line.contains("%capture_cell_allocation_base_address")
                        && line.ends_with("align 8")),
                "{body}"
            );
        }
    }
}

#[test]
fn callable_terminal_and_return_edges_resolve_exact_capture_cell_actions() {
    let source = SourceIdentity::new(
        "project".into(),
        "package".into(),
        "src/main.w".into(),
        "r1".into(),
    );
    for (text, function_name, terminal) in [
        ("%%start\nstruct copy Captured { u8(u8) write; u8(u64) read; }\nCaptured(u64) make = fn(count) { u8[count] slots; slots[0] = 6; Captured result = { .write = fn(value) { slots[0] = value; slots[0] }; .read = fn(index) { slots[index] }; }; result };\n%%end", "make", false),
        ("%%start\nunit() inspect = fn { u8[2] slots = [1, 2]; u8() read = fn { slots[0] }; read(); };\n%%end", "inspect", true),
        ("%%start\nunit(bool) inspect = fn(enabled) { u8[2] slots = [1, 2]; if (enabled) { u8() read = fn { slots[0] }; read(); }; slots[0]; };\n%%end", "inspect", false),
    ] {
        for layout in [ScalarTargetLayout::WASM32, ScalarTargetLayout::NATIVE64] {
            let parsed = parse_source(source.clone(), text.to_owned(), &[]);
            let single = derive_scalar_program_with_layout(&parsed.result, layout);
            assert!(single.diagnostics.is_empty(), "{:?}", single.diagnostics);
            let project = validate_scalar_project(ScalarProject::new(
                vec![ScalarModule::from_program(single.program.clone(), Vec::new())], vec![source.clone()],
            ));
            assert!(project.diagnostics.is_empty(), "{:?}", project.diagnostics);
            for (program_items, llvm) in [
                (&single.program.items, emit_scalar_llvm(&single).expect("single terminal IR").to_text()),
                (&project.project.modules[0].items, emit_scalar_project_llvm(&project).expect("project terminal IR").to_text()),
            ] {
                assert!(llvm.contains("define"), "{llvm}");
                let function = program_items.iter().find_map(|item| match item {
                    ScalarItem::Function(function) if function.name == function_name => Some(function),
                    _ => None,
                }).expect("function");
                let cfg = &function.reference_cfg;
                let events = super::holder_events::HolderEvents::new(cfg).expect("connected events");
                let exit = events.block_exit(&function.body.occurrence).expect("body scope exit");
                let edge = if terminal {
                    assert!(exit.successors.is_empty());
                    super::holder_events::HolderEmissionEdge::Terminal { from: exit.id }
                } else if let [to] = exit.successors.as_slice() {
                    super::holder_events::HolderEmissionEdge::Outgoing { from: exit.id, to: *to }
                } else {
                    super::holder_events::HolderEmissionEdge::Terminal { from: exit.id }
                };
                let actions = match edge {
                    super::holder_events::HolderEmissionEdge::Outgoing { from, to } => cfg.holder_edges.iter()
                        .filter(|fact| fact.from == from && fact.to == to)
                        .flat_map(|fact| &fact.actions).collect::<Vec<_>>(),
                    super::holder_events::HolderEmissionEdge::Terminal { from } => cfg.holder_exits.iter()
                        .filter(|fact| fact.from == from)
                        .flat_map(|fact| &fact.actions).collect::<Vec<_>>(),
                };
                let cells = actions.iter().filter(|action| matches!(action, crate::scalar::CallableHolderAction::DropCaptureCellOwner { .. }))
                    .collect::<Vec<_>>();
                assert!(!cells.is_empty(), "{edge:?}: {actions:?}");
                for action in &cells {
                    events.action(edge, action).expect("one exact connected cell action");
                }
                if terminal {
                    for action in &actions {
                        if let crate::scalar::CallableHolderAction::Drop { holder, .. } = action {
                            events.action(edge, action).expect("terminal holder drop action");
                            events.connected_holder_edge(edge, holder).expect("terminal holder relation");
                        }
                    }
                    let wrong = super::holder_events::HolderEmissionEdge::Terminal { from: cfg.points[0].id };
                    assert!(events.action(wrong, cells[0]).is_err());
                }
                let context = Context::create();
                let builder = context.create_builder();
                let module = context.create_module("terminal_cell");
                let test = module.add_function("resolve_terminal", context.void_type().fn_type(&[], false), None);
                builder.position_at_end(context.append_basic_block(test, "entry"));
                let cell = builder.build_alloca(context.ptr_type(inkwell::AddressSpace::default()), "source_qualified_cell")
                    .expect("cell storage");
                let crate::scalar::CallableHolderAction::DropCaptureCellOwner { binding, .. } = *cells[0] else { unreachable!() };
                let map = std::collections::BTreeMap::from([(binding.clone(), cell)]);
                assert_eq!(events.capture_cell_owner(edge, cells[0], &map).expect("exact capture cell"), cell);
                let mut other = binding.clone();
                other.source.path.push_str(".other");
                assert!(events.capture_cell_owner(edge, cells[0], &std::collections::BTreeMap::from([(other, cell)])).is_err());
                let mut unrelated = (*cells[0]).clone();
                if let crate::scalar::CallableHolderAction::DropCaptureCellOwner { span, .. } = &mut unrelated {
                    span.source.path.push_str(".other");
                }
                assert!(events.action(edge, &unrelated).is_err());
                assert!(events.action(edge, cells[0]).is_ok());
                builder.build_return(None).expect("terminal return");
                module.verify().expect("terminal lookup IR");
                if function_name == "make" {
                    let transfers = cfg.holder_edges.iter().flat_map(|fact| &fact.actions)
                        .filter(|action| matches!(action, crate::scalar::CallableHolderAction::ReturnTransfer { .. })).collect::<Vec<_>>();
                    assert_eq!(transfers.len(), 2);
                    for action in transfers {
                        let fact = cfg.holder_edges.iter().find(|fact| fact.actions.contains(action)).expect("return fact");
                        events.action(super::holder_events::HolderEmissionEdge::Outgoing { from: fact.from, to: fact.to }, action).expect("exact return transfer");
                    }
                }
            }
        }
    }
}

#[test]
fn captured_callable_array_installs_typed_trace_and_dispose_on_both_targets() {
    let source = SourceIdentity::new(
        "project".into(),
        "package".into(),
        "src/main.w".into(),
        "r1".into(),
    );
    let text = "%%start\nstruct copy Pair { u8(u64) read; u8(u64) write; }\nPair(u64) make = fn(count) { (u8())[count] items; u8 first = 1; u8 second = 2; items[0] = fn { first }; items[1] = fn { second }; Pair result = { .read = fn(index) { u8() picked = items[index]; picked() }; .write = fn(index) { u8() picked = items[index]; picked() }; }; result };\n%%end";
    for layout in [ScalarTargetLayout::WASM32, ScalarTargetLayout::NATIVE64] {
        let parsed = parse_source(source.clone(), text.to_owned(), &[]);
        assert!(parsed.diagnostics.is_empty(), "{:?}", parsed.diagnostics);
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
        for ir in [
            emit_scalar_llvm(&single)
                .expect("single captured callable array IR")
                .to_text(),
            emit_scalar_project_llvm(&project)
                .expect("project captured callable array IR")
                .to_text(),
        ] {
            assert!(ir.contains("capture_cell.walk.installed"), "{ir}");
            assert!(ir.contains("capture_cell_dispose_address"), "{ir}");
            assert!(ir.contains("callable_array_allocation_base"), "{ir}");
            for destructor in ir.split("_dispose(ptr %0) {").skip(1) {
                let body = &destructor[..destructor.find("\n}").unwrap()];
                assert!(
                    !body.contains("call void @__wosy_begin_graph_mutation()"),
                    "{body}"
                );
            }
        }
    }
}

#[test]
fn copied_captured_callable_slot_releases_temporary_at_literal_exit() {
    let source = SourceIdentity::new(
        "project".into(),
        "package".into(),
        "src/main.w".into(),
        "r1".into(),
    );
    let text = "%%start\nstruct copy Pair { u8(u64) read; u8(u64) sibling; }\nPair(u64) make = fn(count) { (u8())[count] items; u8 first = 3; u8 second = 7; items[0] = fn { first }; items[1] = fn { second }; Pair result = { .read = fn(index) { u8() copy = items[index]; copy() }; .sibling = fn(index) { u8() copy = items[index]; copy() }; }; result };\n%%end";
    for layout in [ScalarTargetLayout::WASM32, ScalarTargetLayout::NATIVE64] {
        let parsed = parse_source(source.clone(), text.to_owned(), &[]);
        assert!(parsed.diagnostics.is_empty(), "{:?}", parsed.diagnostics);
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
        for items in [&single.program.items, &project.project.modules[0].items] {
            let maker = items
                .iter()
                .find_map(|item| match item {
                    ScalarItem::Function(function) if function.name == "make" => Some(function),
                    _ => None,
                })
                .expect("maker");
            let mut verified = 0;
            for literal in &maker.reference_cfg.literal_cfgs {
                if literal.literal.range.start != text.find("fn(index)").unwrap() as u32
                    && literal.literal.range.start != text.rfind("fn(index)").unwrap() as u32
                {
                    continue;
                }
                let events = super::holder_events::HolderEvents::new(&literal.reference_cfg)
                    .expect("connected selected read");
                let reads = literal
                    .reference_cfg
                    .holder_edges
                    .iter()
                    .flat_map(|fact| &fact.actions)
                    .filter(|action| {
                        matches!(
                            action,
                            crate::scalar::CallableHolderAction::RetainCheckedSelected { .. }
                        )
                    })
                    .count();
                assert!(reads >= 2, "{reads} selected alternatives");
                let drops = literal.reference_cfg.holder_edges.iter().flat_map(|fact| {
                    let edge = super::holder_events::HolderEmissionEdge::Outgoing { from: fact.from, to: fact.to };
                    fact.actions.iter().map(move |action| (edge, action))
                }).chain(literal.reference_cfg.holder_exits.iter().flat_map(|fact| {
                    let edge = super::holder_events::HolderEmissionEdge::Terminal { from: fact.from };
                    fact.actions.iter().map(move |action| (edge, action))
                })).filter(|(edge, action)| {
                    let crate::scalar::CallableHolderAction::Drop { environment, holder: crate::scalar::CallableHolderPlace::Temporary(read), .. } = action else { return false };
                    if !events.counted_environment(environment) || !matches!(literal.reference_cfg.points[read.0].kind, CfgPointKind::ReadPlace { .. }) {
                        return false;
                    }
                    events.action(*edge, action).expect("connected temporary drop");
                    literal.reference_cfg.holder_edges.iter().any(|fact| fact.from == *read && fact.actions.iter().any(|action| matches!(action,
                        crate::scalar::CallableHolderAction::RetainCheckedSelected { old: crate::scalar::CallableCellCaseFact::Initialized { environment_origin, .. }, to: crate::scalar::CallableHolderPlace::Temporary(selected), .. }
                            if environment_origin == environment && selected == read)))
                }).count();
                assert!(drops >= 2, "{drops} selected alternative drops");
                verified += 1;
            }
            assert_eq!(verified, 2);
        }
        for llvm in [
            emit_scalar_llvm(&single)
                .expect("single copied selected IR")
                .to_text(),
            emit_scalar_project_llvm(&project)
                .expect("project copied selected IR")
                .to_text(),
        ] {
            assert!(
                llvm.contains("%holder_environment = extractvalue { ptr, ptr }"),
                "{llvm}"
            );
        }
    }
}

#[test]
fn captured_callable_external_field_copy_has_connected_retain_and_drop() {
    let source = SourceIdentity::new(
        "project".into(),
        "package".into(),
        "src/main.w".into(),
        "r1".into(),
    );
    let text =
        include_str!("../../../../tests/fixtures/build/captured-callable-array/project/src/main.w");
    let parsed = parse_source(source.clone(), text.to_owned(), &[]);
    assert!(parsed.diagnostics.is_empty(), "{:?}", parsed.diagnostics);
    for layout in [ScalarTargetLayout::NATIVE64, ScalarTargetLayout::WASM32] {
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
        for items in [&single.program.items, &project.project.modules[0].items] {
            let make = items
                .iter()
                .find_map(|item| match item {
                    ScalarItem::Function(function) if function.name == "make" => Some(function),
                    _ => None,
                })
                .expect("make");
            let replacements = make
                .reference_cfg
                .holder_edges
                .iter()
                .flat_map(|edge| {
                    edge.actions.iter().filter_map(move |action| match action {
                        crate::scalar::CallableHolderAction::ReplaceCaptureCellValue {
                            binding,
                            previous,
                            incoming,
                            ..
                        } => Some((edge, binding, previous, incoming)),
                        _ => None,
                    })
                })
                .collect::<Vec<_>>();
            let [(replacement_edge, binding, previous, incoming)] = replacements.as_slice() else {
                panic!("expected one source-qualified replacement: {replacements:?}")
            };
            assert!(
                matches!(&previous.length, crate::scalar::RuntimeArrayLengthFact::Evaluated { expression: ScalarExpression::Name { name, .. }, .. } if name == "count")
            );
            assert!(
                matches!(&incoming.length, crate::scalar::RuntimeArrayLengthFact::Evaluated { expression: ScalarExpression::Integer { value, .. }, .. } if value.to_string() == "3")
            );
            assert_ne!(previous.value, incoming.value);
            assert_eq!(previous.owner, incoming.owner);
            let events = super::holder_events::HolderEvents::new(&make.reference_cfg)
                .expect("connected replacement");
            let capture = make.reference_cfg.capture_points.iter().find(|capture| matches!(&make.reference_cfg.points[capture.point.0].kind,
                CfgPointKind::Capture { captures, .. } if captures.iter().any(|input| &input.binding == *binding))).expect("capturing sibling");
            let point = events
                .capture(&capture.occurrence)
                .expect("source-qualified capture");
            let descriptor = events
                .capture_descriptor(
                    point,
                    layout,
                    &std::collections::BTreeMap::from([(
                        (*binding).clone(),
                        super::ownership::CaptureCellResidence::Promoted,
                    )]),
                )
                .expect("typed descriptor")
                .expect("captured backing");
            let cell = descriptor
                .cells
                .iter()
                .find(|cell| &cell.binding == *binding)
                .expect("shared cell");
            assert_eq!(cell.backings.len(), 2);
            assert_eq!(cell.initialized_origins.len(), 5);
            assert!(cell.backings.contains(previous) && cell.backings.contains(incoming));
            assert!(cell.replacements.iter().any(|(edge, old, new)| *edge
                == super::holder_events::HolderEmissionEdge::Outgoing {
                    from: replacement_edge.from,
                    to: replacement_edge.to
                }
                && old == *previous
                && new == *incoming));
            let inspect = items
                .iter()
                .find_map(|item| match item {
                    ScalarItem::Function(function) if function.name == "inspect" => Some(function),
                    _ => None,
                })
                .expect("inspect");
            let actions = inspect
                .reference_cfg
                .holder_edges
                .iter()
                .flat_map(|edge| &edge.actions)
                .collect::<Vec<_>>();
            let external = text.find("fn { first }").unwrap() as u32;
            assert!(actions.iter().any(|action| matches!(action,
                crate::scalar::CallableHolderAction::Retain { environment, from: crate::scalar::CallableHolderPlace::Projection(_), to: crate::scalar::CallableHolderPlace::Binding(_), .. }
                    if environment.literal.range.start == external)));
            assert!(actions.iter().any(|action| matches!(action,
                crate::scalar::CallableHolderAction::Drop { environment, holder: crate::scalar::CallableHolderPlace::Projection(_), .. }
                    if environment.literal.range.start == external)));
        }
        for llvm in [
            emit_scalar_llvm(&single)
                .expect("single external copy IR")
                .to_text(),
            emit_scalar_project_llvm(&project)
                .expect("project external copy IR")
                .to_text(),
        ] {
            assert!(llvm.contains("store i64 %capture_cell_incoming_length, ptr %capture_cell_current_length_address"), "{llvm}");
            assert!(llvm.contains("capture_cell.replace.old"));
            assert!(llvm.contains("capture_cell.walk.installed"));
            assert!(llvm.contains("%environment_holder_retained"));
            assert!(llvm.contains("%environment_holder_dropped"));
        }
    }
}

#[test]
fn nested_producer_capture_layout_mismatch_rejects_descriptor_on_both_targets() {
    let source = SourceIdentity::new(
        "project".into(),
        "package".into(),
        "src/main.w".into(),
        "r1".into(),
    );
    let text =
        include_str!("../../../../tests/fixtures/build/captured-callable-array/project/src/main.w");
    let parsed = parse_source(source.clone(), text.to_owned(), &[]);
    assert!(parsed.diagnostics.is_empty(), "{:?}", parsed.diagnostics);
    for layout in [ScalarTargetLayout::NATIVE64, ScalarTargetLayout::WASM32] {
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
        for items in [&single.program.items, &project.project.modules[0].items] {
            let make = items
                .iter()
                .find_map(|item| match item {
                    ScalarItem::Function(function) if function.name == "make" => Some(function),
                    _ => None,
                })
                .expect("make");
            let binding = make
                .reference_cfg
                .holder_edges
                .iter()
                .flat_map(|edge| &edge.actions)
                .find_map(|action| match action {
                    crate::scalar::CallableHolderAction::ReplaceCaptureCellValue {
                        binding,
                        ..
                    } => Some(binding.clone()),
                    _ => None,
                })
                .expect("replaced cell binding");
            let capture_environment = make
                .reference_cfg
                .points
                .iter()
                .find_map(|point| match &point.kind {
                    CfgPointKind::Capture {
                        environment,
                        captures,
                        ..
                    } if captures.iter().any(|input| input.binding == binding) => {
                        Some(environment.clone())
                    }
                    _ => None,
                })
                .expect("capturing environment");
            let storage = std::collections::BTreeMap::from([(
                binding.clone(),
                super::ownership::CaptureCellResidence::Promoted,
            )]);
            let describe = |cfg: &crate::scalar::ScalarReferenceCfg, context: &str| {
                let events = super::holder_events::HolderEvents::new(cfg)
                    .unwrap_or_else(|error| panic!("{context} connected holder events: {error}"));
                let occurrence = cfg
                    .capture_points
                    .iter()
                    .find(|capture| {
                        matches!(&cfg.points[capture.point.0].kind,
                            CfgPointKind::Capture { captures, .. } if captures.iter().any(|input| input.binding == binding))
                    })
                    .expect("capture occurrence")
                    .occurrence
                    .clone();
                let point = events
                    .capture(&occurrence)
                    .expect("source-qualified capture");
                events
                    .capture_descriptor(point, layout, &storage)
                    .map(|descriptor| descriptor.is_some())
            };
            assert_eq!(
                describe(&make.reference_cfg, "valid"),
                Ok(true),
                "valid nested producer keeps a typed descriptor"
            );
            for (corrupt, reason) in [
                ("origin", "lacks producer backing"),
                ("layout", "differing from producer"),
            ] {
                let mut corrupted = make.reference_cfg.clone();
                let fact = corrupted
                    .environments
                    .get_mut(capture_environment.index)
                    .expect("capture fact");
                assert_eq!(fact.literal, capture_environment.literal);
                let input = fact
                    .inputs
                    .iter_mut()
                    .find(|input| input.binding == binding)
                    .expect("capture input");
                let mut nested_origin: Option<crate::scalar::CallableEnvironmentId> = None;
                let mut corrupted_case = false;
                for backing in input.backings.iter_mut().chain(
                    input
                        .projected_backings
                        .iter_mut()
                        .map(|projected| &mut projected.backing),
                ) {
                    for family in &mut backing.callable_family {
                        for partition in &mut family.partitions {
                            for representative in &mut partition.representatives {
                                for case in &mut representative.cell.alternatives {
                                    if let crate::scalar::CallableCellCaseFact::Initialized {
                                        environment_origin,
                                        capture_paths,
                                        ..
                                    } = case
                                    {
                                        if corrupt == "origin" {
                                            environment_origin.index = usize::MAX;
                                        } else {
                                            assert!(
                                                !capture_paths.is_empty(),
                                                "nested case keeps capture layout"
                                            );
                                            nested_origin = Some(environment_origin.clone());
                                        }
                                        corrupted_case = true;
                                        break;
                                    }
                                }
                                if corrupted_case {
                                    break;
                                }
                            }
                            if corrupted_case {
                                break;
                            }
                        }
                        if corrupted_case {
                            break;
                        }
                    }
                    if corrupted_case {
                        break;
                    }
                }
                assert!(corrupted_case, "expected a nested installed case");
                if corrupt == "layout" {
                    let nested_origin = nested_origin.expect("nested producer");
                    let producer = corrupted
                        .environments
                        .get_mut(nested_origin.index)
                        .expect("nested producer fact");
                    assert_eq!(producer.literal, nested_origin.literal);
                    producer.capture_paths.clear();
                }
                let error = match super::holder_events::HolderEvents::new(&corrupted) {
                    Ok(events) => {
                        let occurrence = corrupted
                            .capture_points
                            .iter()
                            .find(|capture| {
                                matches!(&corrupted.points[capture.point.0].kind,
                                    CfgPointKind::Capture { captures, .. } if captures.iter().any(|input| input.binding == binding))
                            })
                            .expect("capture occurrence")
                            .occurrence
                            .clone();
                        let point = events.capture(&occurrence).expect("capture");
                        events
                            .capture_descriptor(&point, layout, &storage)
                            .map(|descriptor| descriptor.is_some())
                            .expect_err("mismatched nested producer rejects")
                    }
                    Err(error) => error,
                };
                assert!(error.contains(reason), "corrupt={corrupt}: {error}");
                assert!(error.contains("src/main.w"), "corrupt={corrupt}: {error}");
            }
        }
    }
}

#[test]
fn replacement_action_selects_connected_shared_capture_cell_on_both_targets() {
    let source = SourceIdentity::new(
        "project".into(),
        "package".into(),
        "src/main.w".into(),
        "r1".into(),
    );
    let text = "%%start\nstruct copy Captured { u8(u8) write; u8(u64) read; }\nCaptured(u64) make = fn(count) { u8[count] slots; slots[0] = 6; Captured result = { .write = fn(value) { slots[0] = value; slots[0] }; .read = fn(index) { slots[index] }; }; u8[count] replacement; replacement[0] = 9; slots = replacement; result };\n%%end";
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
        for items in [&single.program.items, &project.project.modules[0].items] {
            let function = items
                .iter()
                .find_map(|item| match item {
                    ScalarItem::Function(function) if function.name == "make" => Some(function),
                    _ => None,
                })
                .expect("maker");
            let cfg = &function.reference_cfg;
            let events = super::holder_events::HolderEvents::new(cfg).expect("connected facts");
            let replacements = cfg
                .holder_edges
                .iter()
                .flat_map(|fact| {
                    fact.actions.iter().filter_map(move |action| match action {
                        crate::scalar::CallableHolderAction::ReplaceCaptureCellValue { .. } => {
                            Some((fact, action))
                        }
                        _ => None,
                    })
                })
                .collect::<Vec<_>>();
            let [(fact, action)] = replacements.as_slice() else {
                panic!(
                    "expected one connected replacement, found {}",
                    replacements.len()
                );
            };
            let edge = super::holder_events::HolderEmissionEdge::Outgoing {
                from: fact.from,
                to: fact.to,
            };
            let crate::scalar::CallableHolderAction::ReplaceCaptureCellValue { binding, .. } =
                action
            else {
                unreachable!()
            };
            let capture = cfg
                .capture_points
                .iter()
                .find(|capture| {
                    matches!(&cfg.points[capture.point.0].kind,
                    crate::scalar::CfgPointKind::Capture { captures, .. }
                    if captures.iter().any(|input| &input.binding == binding))
                })
                .expect("captured replaced binding");
            let point = events.capture(&capture.occurrence).unwrap();
            let storage = std::collections::BTreeMap::from([(
                binding.clone(),
                super::ownership::CaptureCellResidence::Promoted,
            )]);
            let descriptor = events
                .capture_descriptor(point, layout, &storage)
                .unwrap()
                .unwrap();
            let cell_descriptor = &descriptor.cells[0];
            assert!(cell_descriptor
                .replacements
                .iter()
                .any(|(known, previous, incoming)| {
                    known == &edge
                        && cell_descriptor.backings.contains(previous)
                        && cell_descriptor.backings.contains(incoming)
                }));
            let context = Context::create();
            let builder = context.create_builder();
            let module = context.create_module("connected_replacement");
            let function =
                module.add_function("resolve", context.void_type().fn_type(&[], false), None);
            builder.position_at_end(context.append_basic_block(function, "entry"));
            let cell = builder
                .build_alloca(
                    context.ptr_type(inkwell::AddressSpace::default()),
                    "promoted_cell",
                )
                .expect("cell storage");
            let cells = std::collections::BTreeMap::from([(binding.clone(), cell)]);
            assert_eq!(
                events
                    .capture_cell_owner(edge, action, &cells)
                    .expect("typed replacement cell"),
                cell
            );
            let wrong_edge = super::holder_events::HolderEmissionEdge::Outgoing {
                from: fact.to,
                to: fact.from,
            };
            assert!(events
                .capture_cell_owner(wrong_edge, action, &cells)
                .is_err());
            let mut foreign = (*action).clone();
            if let crate::scalar::CallableHolderAction::ReplaceCaptureCellValue {
                incoming, ..
            } = &mut foreign
            {
                incoming.value.source.path.push_str(".foreign");
            }
            assert!(events.capture_cell_owner(edge, &foreign, &cells).is_err());
            builder.build_return(None).expect("return");
            module.verify().expect("connected lookup IR");
        }
    }
}

#[test]
fn borrowed_and_global_captures_do_not_own_their_backing_on_both_targets() {
    let source = SourceIdentity::new(
        "project".into(),
        "package".into(),
        "src/main.w".into(),
        "r1".into(),
    );
    let text = "%%start\nu8 global = 4;\nu8(*u8) read_borrowed = fn(address) { u8() reader = fn { *address }; reader() };\nu8() read_global = fn { global };\nu8 local = 7; read_borrowed(&local); read_global();\n%%end";
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
            emit_scalar_llvm(&single)
                .expect("single borrowed/global capture")
                .to_text(),
            emit_scalar_project_llvm(&project)
                .expect("project borrowed/global capture")
                .to_text(),
        ] {
            assert!(
                llvm.contains("store i64 0, ptr %capture_cell_owns_backing_address, align 8"),
                "{llvm}"
            );
            assert_eq!(
                llvm.lines()
                    .filter(|line| line.contains("capture_cell_owner_retained")
                        && line.contains(" = add i64"))
                    .count(),
                1,
                "{llvm}"
            );
        }
    }
}

#[test]
fn imported_projected_holder_rejects_incorrect_producer_descriptor_on_both_targets() {
    let source = SourceIdentity::new(
        "project".into(),
        "package".into(),
        "src/main.w".into(),
        "r1".into(),
    );
    let text = "%%start\nstruct copy Reader { u8() read; }\nReader(u64) make = fn(count) { u8[count] bytes; bytes[0] = 4; Reader result = { .read = fn { bytes[0] }; }; result };\nunit() run = fn { Reader item = make(2); item.read(); };\nrun();\n%%end";
    let parsed = parse_source(source.clone(), text.to_owned(), &[]);
    for layout in [ScalarTargetLayout::WASM32, ScalarTargetLayout::NATIVE64] {
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
        for items in [&single.program.items, &project.project.modules[0].items] {
            let maker = items
                .iter()
                .find_map(|item| match item {
                    ScalarItem::Function(function) if function.name == "make" => Some(function),
                    _ => None,
                })
                .unwrap();
            let run = items
                .iter()
                .find_map(|item| match item {
                    ScalarItem::Function(function) if function.name == "run" => Some(function),
                    _ => None,
                })
                .unwrap();
            let escape = super::ownership::validated_backing_maps(&maker.reference_cfg)
                .escaping
                .into_iter()
                .find(|escape| !escape.backings.is_empty())
                .unwrap();
            let capture = maker
                .reference_cfg
                .capture_points
                .iter()
                .find(|capture| {
                    maker.reference_cfg.points[capture.point.0].source_span == escape.literal
                })
                .unwrap();
            let maker_events =
                super::holder_events::HolderEvents::new(&maker.reference_cfg).unwrap();
            let point = maker_events.capture(&capture.occurrence).unwrap();
            let storage = std::collections::BTreeMap::from([(
                escape.binding.clone(),
                super::ownership::CaptureCellResidence::Promoted,
            )]);
            let descriptor = maker_events
                .capture_descriptor(point, layout, &storage)
                .unwrap()
                .unwrap();
            let mut events = super::holder_events::HolderEvents::new(&run.reference_cfg).unwrap();
            let call = run.reference_cfg.call_points.iter().find(|call| matches!(&run.reference_cfg.points[call.evaluated_point.0].kind, CfgPointKind::Call { expression: ScalarExpression::Call { name, .. }, .. } if name == "make")).unwrap();
            let ScalarType::Callable { outputs, .. } = &maker.signature else {
                unreachable!()
            };
            events
                .evaluated_call(
                    &call.occurrence,
                    &outputs
                        .outputs
                        .iter()
                        .map(|output| output.ty.clone())
                        .collect::<Vec<_>>(),
                )
                .unwrap();
            let (call_point, holder) = events.imported_output_holder(0, &[0]).unwrap();
            let key = super::ownership::ImportedHolderKey {
                source: source.clone(),
                function_span: run.span,
                call_point,
                output: 0,
                fields: vec![0],
            };
            let context = Context::create();
            let pointer = context
                .ptr_type(inkwell::AddressSpace::default())
                .const_null();
            events.imported_holders.insert(
                key.clone(),
                super::ownership::ImportedCaptureHolder {
                    holder,
                    code: pointer,
                    environment: pointer,
                    present: pointer,
                    owned: Vec::new(),
                },
            );
            let mut wrong = descriptor.clone();
            wrong.cells.clear();
            let error = events
                .imported_environment_covered(&key, &wrong, &escape.binding, &escape.backings[0])
                .err()
                .unwrap();
            assert!(
                error.contains("producer binding") && error.contains("src/main.w"),
                "{error}"
            );
        }
        emit_scalar_llvm(&single).expect("single projected producer");
        emit_scalar_project_llvm(&project).expect("project projected producer");
    }
}

#[test]
fn imported_projected_branch_alternatives_have_scalar_return_facts() {
    let source = SourceIdentity::new(
        "project".into(),
        "package".into(),
        "src/main.w".into(),
        "r1".into(),
    );
    let text = "%%start\nstruct copy Reader { u8() read; }\n(Reader, u64)(u64, bool) make = fn(count, pick) { u8[count] first; u8[count] second; first[0] = 3; second[0] = 4; Reader a = { .read = fn { first[0] }; }; Reader b = { .read = fn { second[0] }; }; if (pick) { a, count } else { b, count } };\nunit() run = fn { Reader a, u64 length = make(2, true); a.read(); length; };\nrun();\n%%end";
    let parsed = parse_source(source.clone(), text.to_owned(), &[]);
    assert!(parsed.diagnostics.is_empty(), "{:?}", parsed.diagnostics);
    for layout in [ScalarTargetLayout::WASM32, ScalarTargetLayout::NATIVE64] {
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
        let maker = single
            .program
            .items
            .iter()
            .find_map(|item| match item {
                ScalarItem::Function(function) if function.name == "make" => Some(function),
                _ => None,
            })
            .unwrap();
        let escaping = super::ownership::validated_backing_maps(&maker.reference_cfg).escaping;
        assert_eq!(escaping.len(), 2);
        assert_ne!(escaping[0].literal, escaping[1].literal);
        assert_ne!(escaping[0].binding, escaping[1].binding);
        assert!(escaping.iter().all(|fact| fact.literal.source == source
            && fact.binding.source == source
            && fact.backings.len() == 1));
        let alternatives = maker
            .reference_cfg
            .points
            .iter()
            .filter(|point| matches!(point.kind, CfgPointKind::ReturnOutput { output: 0, .. }))
            .collect::<Vec<_>>();
        let [output] = alternatives.as_slice() else {
            panic!("one projected branch return output")
        };
        assert_eq!(
            output
                .possible_projected_environments
                .iter()
                .flat_map(|projection| &projection.environments)
                .collect::<std::collections::BTreeSet<_>>()
                .len(),
            2
        );
        assert_eq!(output.source_span.source, source);
        for (ir, name) in [
            (
                emit_scalar_llvm(&single)
                    .expect("single branch alternatives")
                    .to_text(),
                "run".to_owned(),
            ),
            (
                emit_scalar_project_llvm(&project)
                    .expect("project branch alternatives")
                    .to_text(),
                project_function_name(&source, "run"),
            ),
        ] {
            let body = &ir[ir.find(&format!("@{name}(")).unwrap()..];
            let body = &body[..body.find("\n}").unwrap()];
            assert!(body.contains("import_literal_matches"), "{body}");
        }
    }
}

#[test]
fn branch_local_captured_arrays_import_projected_and_second_output() {
    let source = SourceIdentity::new(
        "project".into(),
        "package".into(),
        "src/main.w".into(),
        "r1".into(),
    );
    let text = "%%start\nstruct copy Reader { u8() read; }\n(Reader, (u8()))(u64, bool) make = fn(count, pick) { if (pick) { u8[count] first; first[0] = 3; Reader a = { .read = fn { first[0] }; }; u8() other = fn { first[0] }; a, other } else { u8[count] second; second[0] = 4; Reader b = { .read = fn { second[0] }; }; u8() other = fn { second[0] }; b, other } };\nunit() run = fn { Reader a, u8() other = make(2, true); Reader copied = a; u64 count = 2; u8[count] replacement; replacement[0] = 5; copied.read = fn { replacement[0] }; a.read(); other(); copied.read(); Reader b, u8() next = make(3, false); b.read(); next(); };\nrun();\n%%end";
    let parsed = parse_source(source.clone(), text.to_owned(), &[]);
    assert!(parsed.diagnostics.is_empty(), "{:?}", parsed.diagnostics);
    for layout in [ScalarTargetLayout::WASM32, ScalarTargetLayout::NATIVE64] {
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
        for items in [&single.program.items, &project.project.modules[0].items] {
            let run = items
                .iter()
                .find_map(|item| match item {
                    ScalarItem::Function(function) if function.name == "run" => Some(function),
                    _ => None,
                })
                .expect("run");
            let cfg = &run.reference_cfg;
            let copy = cfg.points.iter().find(|point| matches!(&point.kind, CfgPointKind::CopyCallableFields { source: crate::scalar::CallableFieldSource::Evaluated(source), target }
                if matches!(&cfg.initial_points[source.point.0].kind, CfgPointKind::Call { output: 0, .. })
                    && cfg.evaluated_values.binding_inputs.iter().filter(|input| input.producer == cfg.producer_owner
                        && input.source == *source && matches!(&cfg.initial_points[input.consumer.0].kind,
                            CfgPointKind::Bind { binding } if binding == target)).count() == 1)).expect("call output fields copied into a");
            let actions = cfg
                .holder_edges
                .iter()
                .filter(|edge| edge.from == copy.id)
                .collect::<Vec<_>>();
            assert!(
                actions
                    .iter()
                    .flat_map(|edge| &edge.actions)
                    .any(|action| matches!(
                        action,
                        crate::scalar::CallableHolderAction::Move {
                            to: crate::scalar::CallableHolderPlace::Projection(_),
                            ..
                        }
                    )),
                "missing a.read transport at {:?}, edges: {actions:#?}; call outputs: {:#?}",
                copy,
                cfg.points
                    .iter()
                    .filter(|point| matches!(point.kind, CfgPointKind::Call { .. }))
                    .collect::<Vec<_>>()
            );
            assert!(actions.iter().all(|edge| edge.after.iter().filter(|relation| relation.holders.iter().any(|holder| matches!(holder, crate::scalar::CallableHolderPlace::Projection(place) if matches!(place.binding.kind, crate::scalar::ReferenceBindingKind::Declared)))).count() == 2), "branch alternatives lost on copy: {actions:#?}");
            assert!(actions.iter().all(|edge| !edge.actions.iter().any(|action| matches!(action, crate::scalar::CallableHolderAction::Drop { holder: crate::scalar::CallableHolderPlace::Projection(place), .. } if matches!(place.binding.kind, crate::scalar::ReferenceBindingKind::Declared)))), "newly moved a.read was dropped on the same edge: {actions:#?}");
        }
        for ir in [
            emit_scalar_llvm(&single)
                .expect("single branch-local captures")
                .to_text(),
            emit_scalar_project_llvm(&project)
                .expect("project branch-local captures")
                .to_text(),
        ] {
            assert!(
                !ir.contains("captured_cell_pointer_slot"),
                "branch captures retire through counted cells, not synthetic capture slots: {ir}"
            );
            assert!(
                !ir.contains("imported_capture_cell"),
                "branch imports reference provenanced counted cells, not synthetic import slots: {ir}"
            );
            assert!(
                ir.contains("capture_cell_owner_dropped"),
                "branch captures retire through counted cell drops: {ir}"
            );
            assert!(
                ir.matches(" = private constant { ptr, ptr }").count() >= 4,
                "{ir}"
            );
            assert!(ir.contains("closure_environment_data = getelementptr inbounds i8, ptr %closure_allocation, i64 48"), "{ir}");
            assert!(ir.contains("environment_holder_count_address = getelementptr inbounds i8, ptr %closure_environment_data, i64 -24"), "{ir}");
            assert!(ir.contains("environment_allocation_base_address = getelementptr inbounds i8, ptr %closure_environment_data, i64 -16"), "{ir}");
            assert!(
                ir.contains(&format!(
                    "ptr %environment_descriptor_address, align {}",
                    layout.pointer_alignment
                )),
                "{ir}"
            );
        }
    }
}

#[test]
fn private_environment_descriptors_are_source_qualified_across_modules() {
    let first = SourceIdentity::new(
        "same-project".into(),
        "first-package".into(),
        "src/main.w".into(),
        "r1".into(),
    );
    let second = SourceIdentity::new(
        "same-project".into(),
        "second-package".into(),
        "src/main.w".into(),
        "r1".into(),
    );
    for other in [
        SourceIdentity::new(
            "other-project".into(),
            first.package.clone(),
            first.path.clone(),
            first.revision.clone(),
        ),
        SourceIdentity::new(
            first.project.clone(),
            first.package.clone(),
            first.path.clone(),
            "other-revision".into(),
        ),
        SourceIdentity::new(
            first.project.clone(),
            first.package.clone(),
            "src/main-w".into(),
            first.revision.clone(),
        ),
    ] {
        let range = wosy_syntax::ByteSpan::new(55, 67);
        let one = wosy_syntax::SourceSpan::new(first.clone(), range);
        let two = wosy_syntax::SourceSpan::new(other, range);
        assert_ne!(
            super::private_literal_symbol("__wosy_environment", &one),
            super::private_literal_symbol("__wosy_environment", &two)
        );
        assert_ne!(
            super::private_literal_symbol("__wosy_closure", &one),
            super::private_literal_symbol("__wosy_closure", &two)
        );
    }
    let text =
        "%%start\n(u8())() make = fn { u8 value = 7; u8() read = fn { value }; read };\n%%end";
    for layout in [ScalarTargetLayout::WASM32, ScalarTargetLayout::NATIVE64] {
        let modules = [first.clone(), second.clone()]
            .into_iter()
            .map(|source| {
                let parsed = parse_source(source, text.to_owned(), &[]);
                assert!(parsed.diagnostics.is_empty(), "{:?}", parsed.diagnostics);
                let scalar = derive_scalar_program_with_layout(&parsed.result, layout);
                assert!(scalar.diagnostics.is_empty(), "{:?}", scalar.diagnostics);
                scalar.program
            })
            .collect::<Vec<_>>();
        for program in &modules {
            let single = derive_scalar_program_with_layout(
                &parse_source(program.source.clone(), text.to_owned(), &[]).result,
                layout,
            );
            let ir = emit_scalar_llvm(&single)
                .expect("single source descriptor")
                .to_text();
            assert!(ir.contains("environment_descriptor_address"), "{ir}");
        }
        let project = validate_scalar_project(ScalarProject::new(
            modules
                .into_iter()
                .map(|program| ScalarModule::from_program(program, Vec::new()))
                .collect(),
            vec![first.clone(), second.clone()],
        ));
        assert!(project.diagnostics.is_empty(), "{:?}", project.diagnostics);
        let ir = emit_scalar_project_llvm(&project)
            .expect("project source-qualified descriptors")
            .to_text();
        for source in [&first, &second] {
            let point = project
                .project
                .modules
                .iter()
                .find(|module| &module.source == source)
                .unwrap()
                .items
                .iter()
                .find_map(|item| match item {
                    ScalarItem::Function(function) if function.name == "make" => function
                        .reference_cfg
                        .points
                        .iter()
                        .find(|point| matches!(point.kind, CfgPointKind::Capture { .. })),
                    _ => None,
                })
                .unwrap();
            let name = super::encoded_llvm_name(
                "__wosy_environment",
                [
                    source.project.as_str(),
                    source.package.as_str(),
                    source.path.as_str(),
                    source.revision.as_str(),
                    &point.source_span.range.start.to_string(),
                    &point.source_span.range.end.to_string(),
                ],
            );
            assert!(ir.contains(&format!("@{name} = private constant")), "{ir}");
            assert!(
                ir.contains(&format!(
                    "store ptr @{name}, ptr %environment_descriptor_address"
                )),
                "{ir}"
            );
            let code = super::private_literal_symbol("__wosy_closure", &point.source_span);
            assert!(ir.contains(&format!("define internal i8 @{code}(")), "{ir}");
        }
        assert_eq!(
            ir.matches(" = private constant { ptr, ptr }").count(),
            2,
            "{ir}"
        );
    }
}

#[test]
fn imported_branch_returned_second_projected_output_verifies_on_both_targets() {
    let source = SourceIdentity::new(
        "project".into(),
        "package".into(),
        "src/main.w".into(),
        "r1".into(),
    );
    let text = "%%start\nstruct copy Holder { u8() action; }\n((u8()), Holder)(bool) choose = fn(flag) { u8 first = 1; u8 second = 2; u8() left = fn { first }; u8() right = fn { second }; Holder a = { .action = fn { first }; }; Holder b = { .action = fn { second }; }; if (flag) { left, a } else { right, b } };\nunit() run = fn { u8() direct, Holder box = choose(true); direct(); box.action(); };\nrun();\n%%end";
    let parsed = parse_source(source.clone(), text.to_owned(), &[]);
    assert!(parsed.diagnostics.is_empty(), "{:?}", parsed.diagnostics);
    for layout in [ScalarTargetLayout::WASM32, ScalarTargetLayout::NATIVE64] {
        let single = derive_scalar_program_with_layout(&parsed.result, layout);
        let project = validate_scalar_project(ScalarProject::new(
            vec![ScalarModule::from_program(
                single.program.clone(),
                Vec::new(),
            )],
            vec![source.clone()],
        ));
        assert!(single.diagnostics.is_empty(), "{:?}", single.diagnostics);
        assert!(project.diagnostics.is_empty(), "{:?}", project.diagnostics);
        for items in [&single.program.items, &project.project.modules[0].items] {
            let run = items
                .iter()
                .find_map(|item| match item {
                    ScalarItem::Function(function) if function.name == "run" => Some(function),
                    _ => None,
                })
                .expect("run");
            assert!(run
                .reference_cfg
                .binding_points
                .iter()
                .any(|point| point.output == 1
                    && point.role == crate::scalar::ScalarBindingPointRole::CopyCallableFields));
            let calls = run
                .reference_cfg
                .points
                .iter()
                .filter(|point| matches!(point.kind, CfgPointKind::Call { .. }))
                .collect::<Vec<_>>();
            assert!(calls.iter().any(|point| matches!(
                point.kind,
                CfgPointKind::Call { output: 1, .. }
            ) && point
                .possible_projected_environments
                .iter()
                .flat_map(|projection| &projection.environments)
                .collect::<std::collections::BTreeSet<_>>()
                .len()
                == 2));
        }
        for llvm in [
            emit_scalar_llvm(&single)
                .expect("single branch multi-output imported capture")
                .to_text(),
            emit_scalar_project_llvm(&project)
                .expect("project branch multi-output imported capture")
                .to_text(),
        ] {
            let run = llvm
                .split("define ")
                .find(|body| {
                    body.starts_with("void @")
                        && (body
                            .lines()
                            .next()
                            .expect("signature")
                            .contains("__72756e(")
                            || body.starts_with("void @run("))
                })
                .expect("run body");
            let call = run.find(" = call {").expect("two-output call");
            let direct = run[call..].find("ptr %direct").expect("first output store") + call;
            let boxed = run[direct..]
                .find("ptr %box")
                .expect("projected second output store")
                + direct;
            let begin = run[call..direct]
                .rfind("call void @__wosy_begin_graph_mutation()")
                .expect("binding transaction")
                + call;
            let end = run[boxed..]
                .find("call void @__wosy_end_graph_mutation()")
                .expect("binding release")
                + boxed;
            assert!(
                call < begin && begin < direct && direct < boxed && boxed < end,
                "{run}"
            );
            assert!(!run[begin..end].contains(" = call {"), "{run}");
        }
    }
}

#[test]
fn imported_copied_callable_fields_release_each_captured_array_once() {
    let source = SourceIdentity::new(
        "project".into(),
        "package".into(),
        "src/main.w".into(),
        "r1".into(),
    );
    let text = "%%start\nstruct copy Captured { u8(u8) write; u8(u64) read; }\nCaptured(u64) make = fn(count) { u8[count] slots; slots[0] = 6; Captured result = { .write = fn(value) { slots[0] = value; slots[0] }; .read = fn(index) { slots[index] }; }; result };\nunit() run = fn { Captured kept = make(2); Captured copied = kept; u8 before = kept.read(0); u8 written = kept.write(9); u8 shared = copied.read(0); u64 count = 2; u8[count] replacement; replacement[0] = 5; kept.read = fn(index) { replacement[index] }; u8 old = copied.read(0); u8 new = kept.read(0); before; written; shared; old; new; };\nrun();\n%%end";
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
        for (llvm, run_name) in [
            (
                emit_scalar_llvm(&single).expect("single LLVM").to_text(),
                "run".to_owned(),
            ),
            (
                emit_scalar_project_llvm(&project)
                    .expect("project LLVM")
                    .to_text(),
                project_function_name(&source, "run"),
            ),
        ] {
            let body = &llvm[llvm.find(&format!("@{run_name}(")).unwrap()..];
            let body = &body[..body.find("\n}").unwrap()];
            assert!(body.contains("import_literal_matches"), "{body}");
            assert_eq!(
                body.lines()
                    .filter(
                        |line| line.trim_start().starts_with("%import_holder_environment")
                            && line.contains(" = alloca ptr")
                    )
                    .count(),
                2,
                "{body}"
            );
            assert!(
                body.contains("store ptr null, ptr %import_holder_environment"),
                "{body}"
            );
            assert!(
                body.contains("store i1 false, ptr %import_holder_present"),
                "{body}"
            );
            assert!(body.contains("environment_holder_dropped"), "{body}");
            assert!(llvm.contains("capture_cell_owner_dropped"), "{llvm}");
            assert!(
                body.contains("capture_cell_owner_dropped"),
                "the replacement capture retires through its counted cell in run: {body}"
            );
            assert!(
                !body.contains("capture_release.check"),
                "imported arrays retire through their provenanced counted cells, not synthetic release: {body}"
            );
            assert!(
                !body.contains("store ptr null, ptr %import_selected_cell"),
                "{body}"
            );
        }
    }
}

#[test]
fn constant_callable_array_holder_sources_are_distinct_on_both_targets() {
    let source = SourceIdentity::new(
        "project".into(),
        "package".into(),
        "src/main.w".into(),
        "r1".into(),
    );
    for text in [
        "%%start\nunit() inspect = fn { u8 first = 1; u8 second = 2; u8()[2] readers = [fn { first }, fn { second }]; u8() a = readers[0]; u8() b = readers[1]; a(); b(); };\ninspect();\n%%end",
        "%%start\nunit() inspect = fn { u8()[2] fixed = [fn { 1 }, fn { 2 }]; (u8())[] copied = fixed; u8() a = copied[0]; u8() b = copied[1]; a(); b(); };\ninspect();\n%%end",
        "%%start\nstruct copy Readers { u8()[2] callbacks; }\nunit() inspect = fn { Readers bundle = { .callbacks = [fn { 1 }, fn { 2 }]; }; u8() a = bundle.callbacks[0]; u8() b = bundle.callbacks[1]; a(); b(); };\ninspect();\n%%end",
    ] {
        for layout in [ScalarTargetLayout::WASM32, ScalarTargetLayout::NATIVE64] {
            let parsed = parse_source(source.clone(), text.to_owned(), &[]);
            assert!(parsed.diagnostics.is_empty(), "{text}: {:?}", parsed.diagnostics);
            let single = derive_scalar_program_with_layout(&parsed.result, layout);
            assert!(single.diagnostics.is_empty(), "{text}: {:?}", single.diagnostics);
            let project = validate_scalar_project(ScalarProject::new(
                vec![ScalarModule::from_program(single.program.clone(), Vec::new())],
                vec![source.clone()],
            ));
            assert!(project.diagnostics.is_empty(), "{text}: {:?}", project.diagnostics);
            for items in [&single.program.items, &project.project.modules[0].items] {
                let function = items.iter().find_map(|item| match item {
                    ScalarItem::Function(function) if function.name == "inspect" => Some(function),
                    _ => None,
                }).expect("inspect function");
                let places = function.reference_cfg.holder_edges.iter()
                    .flat_map(|edge| edge.before.iter().chain(&edge.after))
                    .flat_map(|relation| &relation.holders)
                    .filter_map(|holder| match holder {
                        crate::scalar::CallableHolderPlace::Projection(place)
                            if place.projections.iter().any(|projection| matches!(projection,
                                crate::scalar::ReferencePlaceProjection::Index(ScalarExpression::Integer { .. }))) => Some(place),
                        _ => None,
                    }).collect::<Vec<_>>();
                if text.contains("struct copy Readers") {
                    assert!(places.is_empty(), "nested indexed reads use typed ReadPlace temporaries: {places:?}");
                    let candidates = function.reference_cfg.points.iter().filter_map(|point| match &point.kind {
                        CfgPointKind::ReadPlace { candidates, .. } if candidates.iter().any(|candidate| candidate.projections.iter().any(|projection| matches!(projection, crate::scalar::ReferencePlaceProjection::Index(_)))) => Some(candidates),
                        _ => None,
                    }).collect::<Vec<_>>();
                    assert!(!candidates.is_empty(), "nested indexed read candidates: {candidates:?}");
                    continue;
                }
                assert!(places.iter().any(|place| place.projections.iter().any(|projection| matches!(projection,
                    crate::scalar::ReferencePlaceProjection::Index(ScalarExpression::Integer { value, .. }) if *value == 0.into()))), "{text}: {places:?}");
                assert!(places.iter().any(|place| place.projections.iter().any(|projection| matches!(projection,
                    crate::scalar::ReferencePlaceProjection::Index(ScalarExpression::Integer { value, .. }) if *value == 1.into()))), "{text}: {places:?}");
                assert!(places.iter().all(|place| place.binding.source == source), "{text}: {places:?}");
            }
            emit_scalar_llvm(&single).expect("single constant index LLVM");
            emit_scalar_project_llvm(&project).expect("project constant index LLVM");
        }
    }
}

#[test]
fn constant_index_selected_projection_emits_one_counted_install_on_both_targets() {
    let source = SourceIdentity::new(
        "project".into(),
        "package".into(),
        "src/main.w".into(),
        "r1".into(),
    );
    for layout in [ScalarTargetLayout::WASM32, ScalarTargetLayout::NATIVE64] {
        for (text, retained_install) in [
            ("%%start\nunit(u64) inspect = fn(count) { (u8())[count] readers; readers[0] = fn { 1 }; readers; };\n%%end", false),
            ("%%start\nunit(u64) inspect = fn(count) { (u8())[count] readers; u8() callback = fn { 1 }; readers[0] = callback; readers; };\n%%end", true),
        ] {
            let parsed = parse_source(source.clone(), text.to_owned(), &[]);
            assert!(parsed.diagnostics.is_empty(), "{:?}", parsed.diagnostics);
            let single = derive_scalar_program_with_layout(&parsed.result, layout);
            assert!(single.diagnostics.is_empty(), "{:?}", single.diagnostics);
            let project = validate_scalar_project(ScalarProject::new(
                vec![ScalarModule::from_program(single.program.clone(), Vec::new())],
                vec![source.clone()],
            ));
            assert!(project.diagnostics.is_empty(), "{:?}", project.diagnostics);
            for ir in [
                emit_scalar_llvm(&single).expect("single selected projection").to_text(),
                emit_scalar_project_llvm(&project).expect("project selected projection").to_text(),
            ] {
                assert_eq!(ir.matches(" = extractvalue { ptr, ptr } %selected_incoming_callable, 1").count(), usize::from(retained_install), "{layout:?}: {ir}");
                assert_eq!(ir.matches("%environment_holder_retained = add i64").count(), usize::from(retained_install), "{layout:?}: {ir}");
                assert_eq!(ir.matches("%environment_holder_dropped = sub i64").count(), 1, "{layout:?}: {ir}");
            }
        }
    }
}

#[test]
fn constant_holder_bridge_uses_source_qualified_array_slot_and_distinct_offsets() {
    let source = SourceIdentity::new(
        "project".into(),
        "package".into(),
        "src/main.w".into(),
        "r1".into(),
    );
    for (text, root_name) in [
        ("%%start\nunit() inspect = fn { u8()[2] readers = [fn { 1 }, fn { 2 }]; u8() left = readers[0]; u8() right = readers[1]; left(); right(); };\n%%end", "readers"),
        ("%%start\nunit(u64) inspect = fn(count) { (u8())[count] readers; readers[0] = fn { 1 }; readers[1] = fn { 2 }; readers; };\n%%end", "readers"),
        ("%%start\nunit() inspect = fn { u8()[2] fixed = [fn { 1 }, fn { 2 }]; (u8())[] copied = fixed; u8() left = copied[0]; u8() right = copied[1]; left(); right(); };\n%%end", "copied"),
        ("%%start\nstruct copy Readers { u8()[2] callbacks; }\nunit() inspect = fn { Readers bundle = { .callbacks = [fn { 1 }, fn { 2 }]; }; u8() left = bundle.callbacks[0]; u8() right = bundle.callbacks[1]; left(); right(); };\n%%end", "bundle"),
    ] {
    for layout in [ScalarTargetLayout::WASM32, ScalarTargetLayout::NATIVE64] {
        let parsed = parse_source(source.clone(), text.to_owned(), &[]);
        let single = derive_scalar_program_with_layout(&parsed.result, layout);
        assert!(single.diagnostics.is_empty(), "{:?}", single.diagnostics);
        let project = validate_scalar_project(ScalarProject::new(
            vec![ScalarModule::from_program(single.program.clone(), Vec::new())], vec![source.clone()],
        ));
        assert!(project.diagnostics.is_empty(), "{:?}", project.diagnostics);
        for items in [&single.program.items, &project.project.modules[0].items] {
            let function = items.iter().find_map(|item| match item {
                ScalarItem::Function(function) if function.name == "inspect" => Some(function),
                _ => None,
            }).expect("inspect");
            let binding = function.body.items.iter().find_map(|item| match item {
                ScalarBlockItem::LocalBinding(binding) if binding.name == root_name => Some(binding),
                _ => None,
            }).expect("array binding");
            let mut places = function.reference_cfg.holder_edges.iter()
                .flat_map(|edge| edge.before.iter().chain(&edge.after))
                .flat_map(|relation| &relation.holders)
                .filter_map(|holder| match holder {
                    crate::scalar::CallableHolderPlace::Projection(place)
                        if place.binding.declaration_span == binding.name_span
                        && matches!(place.projections.as_slice(), [crate::scalar::ReferencePlaceProjection::Index(ScalarExpression::Integer { .. })]) => Some(place.clone()),
                    _ => None,
                }).collect::<Vec<_>>();
            if root_name == "bundle" || root_name == "copied" {
                places = function.reference_cfg.points.iter().flat_map(|point| match &point.kind {
                    CfgPointKind::ReadPlace { candidates, .. } => candidates.as_slice(),
                    _ => &[],
                }).filter(|place| place.binding.declaration_span == binding.name_span
                    && matches!(place.projections.last(), Some(crate::scalar::ReferencePlaceProjection::Index(ScalarExpression::Integer { .. }))))
                    .cloned().collect();
            }
            let context = Context::create();
            let builder = context.create_builder();
            let functions = BTreeMap::new();
            let call_targets = BTreeMap::new();
            let signatures = BTreeMap::new();
            let dynamic_owner_outputs = BTreeMap::new();
            let literal_signatures = std::collections::HashMap::new();
            let literal_captures = std::collections::HashMap::new();
            let literal_reference_cfgs = std::collections::HashMap::new();
            let structs = &single.program.structs;
            let globals = BTreeMap::new();
            let module = context.create_module("constant_holder");
            super::declare_core_runtime(&context, &module);
            let function_value = module.add_function("test_holder", context.void_type().fn_type(&[], false), None);
            builder.position_at_end(context.append_basic_block(function_value, "entry"));
            let mut state = super::EmitState {
                builder: &builder, module: &module, functions: &functions, call_targets: &call_targets,
                signatures: &signatures, dynamic_owner_outputs: &dynamic_owner_outputs,
                literal_signatures: &literal_signatures, literal_captures: &literal_captures,
                literal_reference_cfgs: &literal_reference_cfgs, covered_literals: std::collections::HashSet::new(), cyclic_literals: Vec::new(), counted_fixed_cells: std::collections::BTreeSet::new(), top_level_remainder_cells: Vec::new(), counted_cell_backings: Vec::new(), resolved_calls: BTreeMap::new(),
                source: source.clone(), capture_cells: BTreeMap::new(),
                backing: super::ownership::validated_backing_maps(&function.reference_cfg),
                holder_events: super::holder_events::HolderEvents::new(&function.reference_cfg).expect("holder events"),
                escaping: BTreeMap::new(), backing_values: BTreeMap::new(), backing_slots: BTreeMap::new(), imported_cell_slots: BTreeMap::new(),
                backing_captured: std::collections::BTreeSet::new(), values: BTreeMap::new(), storage: BTreeMap::new(),
                fresh_return_outputs: std::collections::BTreeSet::new(), branch_return_outputs: BTreeMap::new(),
                joined_return_bindings: BTreeMap::new(), joined_assignment_owners: BTreeMap::new(),
                return_output_types: Vec::new(), return_body_span: Some(function.body.span),
                checked_call_owners: BTreeMap::new(), checked_owner_flags: BTreeMap::new(),
                call_owner_flags: BTreeMap::new(), return_owner_flags: BTreeMap::new(),
                return_owner_slots: BTreeMap::new(), return_length_slots: BTreeMap::new(),
                call_length_slots: BTreeMap::new(), runtime_array_owners: BTreeMap::new(),
                runtime_array_allocations: BTreeMap::new(), runtime_array_lengths: BTreeMap::new(),
                runtime_array_live: BTreeMap::new(), mutable_runtime_allocations: std::collections::BTreeSet::new(),
                invalidations: BTreeMap::new(), automatic_return_result_owners: BTreeMap::new(),
                globals: globals.clone(), all_globals: globals, structs, target_layout: layout,
                next_literal: 0, next_block: 0,
            };
            let runtime = matches!(binding.declared_type, ScalarType::RuntimeArray { .. });
            let slot = if let ScalarType::RuntimeArray { element, .. } = &binding.declared_type {
                let element = storage_type(&context, element, structs, layout).expect("runtime element type");
                let data = builder.build_array_alloca(element, context.i64_type().const_int(2, false), "source_array")
                    .expect("runtime source storage");
                if root_name == "copied" {
                    let slot = builder.build_alloca(context.ptr_type(inkwell::AddressSpace::default()), "copied_pointer")
                        .expect("copied pointer storage");
                    builder.build_store(slot, data).expect("copied data pointer");
                    slot
                } else {
                    data
                }
            } else {
                let array = storage_type(&context, &binding.declared_type, structs, layout).expect("array type");
                builder.build_alloca(array, "source_array").expect("source storage")
            };
            if runtime {
                let length = builder.build_alloca(context.i64_type(), "copied_length").expect("length storage");
                builder.build_store(length, context.i64_type().const_int(2, false)).expect("length value");
                state.runtime_array_lengths.insert(root_name.to_owned(), length);
                state.runtime_array_allocations.insert(root_name.to_owned(), root_name != "copied");
            }
            state.storage.insert(root_name.to_owned(), (slot, binding.declared_type.clone()));
            state.holder_events.register_binding(binding, 0, slot).expect("typed binding storage");
            let mut seen = std::collections::BTreeSet::new();
            let addresses = places.iter().filter_map(|place| {
                let Some(crate::scalar::ReferencePlaceProjection::Index(ScalarExpression::Integer { value, .. })) = place.projections.last() else { return None };
                let index = u64::try_from(value).expect("literal index");
                if index > 1 || !seen.insert(index) { return None; }
                Some((index, super::memory::constant_callable_projection_pointer(&context, &mut state, place)
                    .expect("constant holder address")))
            }).collect::<Vec<_>>();
            let zero = addresses.iter().find(|(index, _)| *index == 0).unwrap_or_else(|| panic!("first holder {root_name}: {places:?}")).1;
            let one = addresses.iter().find(|(index, _)| *index == 1).expect("second holder").1;
            assert_ne!(zero, one);
            if root_name == "readers" {
                let place = places.iter().find(|place| matches!(place.projections.last(),
                    Some(crate::scalar::ReferencePlaceProjection::Index(ScalarExpression::Integer { value, .. }))
                        if *value == 0.into())).expect("first projected holder");
                let holder = crate::scalar::CallableHolderPlace::Projection(place.clone());
                let edge = function.reference_cfg.holder_edges.iter().find(|edge| edge.before.iter().chain(&edge.after)
                    .any(|relation| relation.holders.contains(&holder))).expect("connected projected holder");
                let concrete = super::memory::concrete_callable_holder(&context, &mut state, super::holder_events::HolderEmissionEdge::Outgoing { from: edge.from, to: edge.to }, &holder)
                    .expect("concrete constant projected holder");
                assert!(concrete.present.is_const());
                assert!(concrete.environment.as_instruction().is_some());
                assert!(concrete.address.as_instruction().is_some());
            }
            builder.build_return(None).expect("return");
            module.verify().expect("constant holder IR");
            let llvm = module.print_to_string().to_string();
            assert!(llvm.contains("holder_constant_element"), "{llvm}");
            if root_name == "readers" {
                assert!(llvm.contains("holder_environment = extractvalue { ptr, ptr }"), "{llvm}");
                if !runtime {
                    assert!(llvm.contains("ptr %source_array, i32 0, i64 0"), "{llvm}");
                }
            }
            if root_name == "bundle" {
                assert!(llvm.contains("holder_field"), "{llvm}");
            }
            if runtime {
                assert!(llvm.contains("holder_constant_in_bounds"), "{llvm}");
                assert!(llvm.contains("holder.constant.out_of_bounds"), "{llvm}");
            }
            if root_name == "copied" {
                assert!(llvm.contains("holder_array_data"), "{llvm}");
            }
            assert!(!llvm.contains("selected_address"), "{llvm}");
        }
    }
    }
}

#[test]
fn callable_return_holder_slots_cover_struct_and_callable_outputs_in_both_pipelines() {
    let source = SourceIdentity::new(
        "project".into(),
        "package".into(),
        "src/main.w".into(),
        "r1".into(),
    );
    for (text, count) in [
        ("%%start\nstruct copy Reader { u8() read; }\nReader() make = fn { Reader result = { .read = fn { 1 }; }; result };\n%%end", 1),
        ("%%start\nstruct copy Reader { u8() read; }\n(u8())() make = fn { Reader result = { .read = fn { 1 }; }; result.read };\n%%end", 1),
        ("%%start\nstruct copy Reader { u8() read; }\n(Reader, Reader)() make = fn { Reader first = { .read = fn { 1 }; }; Reader second = { .read = fn { 2 }; }; first, second };\n%%end", 2),
    ] {
        for layout in [ScalarTargetLayout::WASM32, ScalarTargetLayout::NATIVE64] {
            let parsed = parse_source(source.clone(), text.to_owned(), &[]);
            assert!(parsed.diagnostics.is_empty(), "{:?}", parsed.diagnostics);
            let single = derive_scalar_program_with_layout(&parsed.result, layout);
            assert!(single.diagnostics.is_empty(), "{text}: {:?}", single.diagnostics);
            let project = validate_scalar_project(ScalarProject::new(
                vec![ScalarModule::from_program(single.program.clone(), Vec::new())],
                vec![source.clone()],
            ));
            assert!(project.diagnostics.is_empty(), "{:?}", project.diagnostics);
            for items in [&single.program.items, &project.project.modules[0].items] {
                let function = items.iter().find_map(|item| match item {
                    ScalarItem::Function(function) if function.name == "make" => Some(function),
                    _ => None,
                }).expect("maker");
                let events = super::holder_events::HolderEvents::new(&function.reference_cfg)
                    .expect("return events");
                for edge in &function.reference_cfg.holder_edges {
                    for action in &edge.actions {
                        if let crate::scalar::CallableHolderAction::ReturnTransfer { to: holder, .. } = action {
                            let output = match holder {
                                crate::scalar::CallableHolderPlace::Output(output)
                                | crate::scalar::CallableHolderPlace::OutputProjection { output, .. } => *output,
                                _ => continue,
                            };
                            events.return_holder_point(edge.from, edge.to, output)
                                .expect("exact connected return output");
                            events.connected_holder_edge(super::holder_events::HolderEmissionEdge::Outgoing { from: edge.from, to: edge.to }, holder)
                                .expect("transferred holder on return edge");
                        }
                    }
                }
            }
            for llvm in [
                emit_scalar_llvm(&single).expect("single return slots").to_text(),
                emit_scalar_project_llvm(&project).expect("project return slots").to_text(),
            ] {
                assert_eq!(llvm.lines().filter(|line| line.trim_start().starts_with("%return_holder_address") && line.contains(" = alloca ptr,")).count(), count, "{llvm}");
                assert_eq!(llvm.matches("store i1 false, ptr %return_holder_present").count(), count, "{llvm}");
                assert_eq!(llvm.matches("store i1 true, ptr %return_holder_present").count(), count, "{llvm}");
            }
        }
    }
}

#[test]
fn returned_and_imported_callable_slots_publish_inside_graph_transactions() {
    let source = SourceIdentity::new(
        "project".into(),
        "package".into(),
        "src/main.w".into(),
        "r1".into(),
    );
    let text = "%%start\nstruct copy Reader { u8() read; }\nReader(u64) make = fn(count) { u8[count] bytes; bytes[0] = 4; Reader result = { .read = fn { bytes[0] }; }; result };\nunit() run = fn { Reader first = make(2); Reader second = make(3); first.read(); second.read(); };\nrun();\n%%end";
    for layout in [ScalarTargetLayout::WASM32, ScalarTargetLayout::NATIVE64] {
        let parsed = parse_source(source.clone(), text.to_owned(), &[]);
        assert!(parsed.diagnostics.is_empty(), "{:?}", parsed.diagnostics);
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
            emit_scalar_llvm(&single)
                .expect("single return/import transaction")
                .to_text(),
            emit_scalar_project_llvm(&project)
                .expect("project return/import transaction")
                .to_text(),
        ] {
            let maker = llvm
                .split("define ")
                .find(|body| {
                    body.starts_with("ptr @")
                        && (body
                            .lines()
                            .next()
                            .expect("signature")
                            .contains("__6d616b65(")
                            || body.starts_with("ptr @make("))
                })
                .expect("maker IR");
            let output_store = maker
                .find("store i1 true, ptr %return_holder_present")
                .expect("return publication");
            let begin = maker[..output_store]
                .rfind("call void @__wosy_begin_graph_mutation()")
                .expect("return lock");
            let end = maker[output_store..]
                .find("call void @__wosy_end_graph_mutation()")
                .expect("return unlock")
                + output_store;
            assert!(
                begin
                    < maker
                        .find("store ptr %result, ptr %return_holder_address")
                        .expect("output address")
                    && output_store < end,
                "{maker}"
            );
            assert!(!maker[begin..end].contains("call ptr @"), "{maker}");

            let run = llvm
                .split("define ")
                .find(|body| {
                    body.starts_with("void @")
                        && (body
                            .lines()
                            .next()
                            .expect("signature")
                            .contains("__72756e(")
                            || body.starts_with("void @run("))
                })
                .expect("caller IR");
            let call = run.find("call ptr @").expect("maker call");
            let imported = run[call..]
                .find(", ptr %import_holder_code")
                .expect("imported code store")
                + call;
            let begin = run[call..imported]
                .find("call void @__wosy_begin_graph_mutation()")
                .expect("import lock")
                + call;
            let end = run[imported..]
                .find("call void @__wosy_end_graph_mutation()")
                .expect("import unlock")
                + imported;
            assert!(call < begin && begin < imported && imported < end, "{run}");
            assert!(!run[begin..end].contains("call ptr @"), "{run}");
        }
    }
}

#[test]
fn expression_body_callable_return_and_imported_binding_share_protected_handoffs() {
    let source = SourceIdentity::new(
        "project".into(),
        "package".into(),
        "src/main.w".into(),
        "r1".into(),
    );
    let text = "%%start\n(u8())() make = fn { fn { 7 } };\nunit() run = fn { u8() reader = make(); reader(); };\nrun();\n%%end";
    for layout in [ScalarTargetLayout::WASM32, ScalarTargetLayout::NATIVE64] {
        let parsed = parse_source(source.clone(), text.to_owned(), &[]);
        assert!(parsed.diagnostics.is_empty(), "{:?}", parsed.diagnostics);
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
            emit_scalar_llvm(&single)
                .expect("single expression return")
                .to_text(),
            emit_scalar_project_llvm(&project)
                .expect("project expression return")
                .to_text(),
        ] {
            let maker = llvm
                .split("define ")
                .find(|body| {
                    body.starts_with("{ ptr, ptr } @")
                        && body.lines().next().expect("signature").contains("6d616b65")
                        || body.starts_with("{ ptr, ptr } @make(")
                })
                .expect("maker");
            let present = maker
                .find("store i1 true, ptr %return_holder_present")
                .expect("return present");
            let begin = maker[..present]
                .rfind("call void @__wosy_begin_graph_mutation()")
                .expect("return begin");
            let end = maker[present..]
                .find("call void @__wosy_end_graph_mutation()")
                .expect("return end")
                + present;
            assert!(maker[begin..end].contains("store { ptr, ptr }"), "{maker}");
            assert!(
                !maker[begin..end].contains("call { ptr, ptr } @"),
                "{maker}"
            );
            let run = llvm
                .split("define ")
                .find(|body| {
                    body.starts_with("void @")
                        && (body
                            .lines()
                            .next()
                            .expect("signature")
                            .contains("__72756e(")
                            || body.starts_with("void @run("))
                })
                .expect("caller");
            let call = run.find("call { ptr, ptr } @").expect("maker call");
            let store = run[call..].find("ptr %reader").expect("reader store") + call;
            let begin = run[call..store]
                .rfind("call void @__wosy_begin_graph_mutation()")
                .expect("binding begin")
                + call;
            let end = run[store..]
                .find("call void @__wosy_end_graph_mutation()")
                .expect("binding end")
                + store;
            assert!(call < begin && begin < store && store < end, "{run}");
            assert!(!run[begin..end].contains("call { ptr, ptr } @"), "{run}");
        }
    }
}

#[test]
fn literal_and_copied_callable_initial_roots_publish_inside_transaction() {
    let source = SourceIdentity::new(
        "project".into(),
        "package".into(),
        "src/main.w".into(),
        "r1".into(),
    );
    let text = "%%start\nstruct copy Reader { u8() read; }\nu8 base = 9;\nReader global = { .read = fn { base }; };\nu8()[2] readers = [fn { base }, fn { base }];\nu8() run = fn { u8() local = fn { base }; u8() copied = local; copied() };\nrun(); global.read();\n%%end";
    for layout in [ScalarTargetLayout::WASM32, ScalarTargetLayout::NATIVE64] {
        let parsed = parse_source(source.clone(), text.to_owned(), &[]);
        assert!(parsed.diagnostics.is_empty(), "{:?}", parsed.diagnostics);
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
            emit_scalar_llvm(&single).expect("single roots").to_text(),
            emit_scalar_project_llvm(&project)
                .expect("project roots")
                .to_text(),
        ] {
            let run = llvm
                .split("define ")
                .find(|body| {
                    body.lines().next().expect("signature").contains("72756e")
                        || body.starts_with("i8 @run(")
                })
                .expect("run");
            for slot in ["local", "copied"] {
                let store = run.find(&format!("ptr %{slot}")).expect("binding store");
                let begin = run[..store]
                    .rfind("call void @__wosy_begin_graph_mutation()")
                    .expect("binding begin");
                let end = run[store..]
                    .find("call void @__wosy_end_graph_mutation()")
                    .expect("binding end")
                    + store;
                assert!(begin < store && store < end, "{run}");
            }
            let main = llvm
                .split("define ")
                .find(|body| body.starts_with("i32 @main("))
                .expect("main");
            let global_stores = main
                .match_indices("store [")
                .filter_map(|(offset, _)| {
                    main[offset..]
                        .lines()
                        .next()
                        .unwrap()
                        .contains(", ptr @")
                        .then_some(offset)
                })
                .collect::<Vec<_>>();
            assert_eq!(global_stores.len(), 2, "{main}");
            for store in global_stores {
                let begin = main[..store]
                    .rfind("call void @__wosy_begin_graph_mutation()")
                    .expect("global begin");
                let end = main[store..]
                    .find("call void @__wosy_end_graph_mutation()")
                    .expect("global end")
                    + store;
                assert!(begin < store && store < end, "{main}");
            }
        }
    }
}

#[test]
fn callable_return_holder_slots_cover_joined_branch_and_same_span_outputs() {
    let source = SourceIdentity::new(
        "project".into(),
        "package".into(),
        "src/main.w".into(),
        "r1".into(),
    );
    let text = "%%start\nstruct copy Reader { u8() read; }\nReader(bool) make = fn(choose) { if (choose) { Reader left = { .read = fn { 1 }; }; left } else { Reader right = { .read = fn { 2 }; }; right } };\n%%end";
    for layout in [ScalarTargetLayout::WASM32, ScalarTargetLayout::NATIVE64] {
        let parsed = parse_source(source.clone(), text.to_owned(), &[]);
        assert!(parsed.diagnostics.is_empty(), "{:?}", parsed.diagnostics);
        let single = derive_scalar_program_with_layout(&parsed.result, layout);
        assert!(
            single.diagnostics.is_empty(),
            "{text}: {:?}",
            single.diagnostics
        );
        let project = validate_scalar_project(ScalarProject::new(
            vec![ScalarModule::from_program(
                single.program.clone(),
                Vec::new(),
            )],
            vec![source.clone()],
        ));
        assert!(project.diagnostics.is_empty(), "{:?}", project.diagnostics);
        for llvm in [
            emit_scalar_llvm(&single)
                .expect("single joined return slots")
                .to_text(),
            emit_scalar_project_llvm(&project)
                .expect("project joined return slots")
                .to_text(),
        ] {
            assert_eq!(
                llvm.lines()
                    .filter(
                        |line| line.trim_start().starts_with("%return_holder_address")
                            && line.contains(" = alloca ptr,")
                    )
                    .count(),
                1,
                "{llvm}"
            );
            assert_eq!(
                llvm.matches("store i1 false, ptr %return_holder_present")
                    .count(),
                1,
                "{llvm}"
            );
            assert_eq!(
                llvm.matches("store i1 true, ptr %return_holder_present")
                    .count(),
                1,
                "{llvm}"
            );
        }
    }
}

#[test]
fn duplicate_holder_across_outputs_reports_source_diagnostic_before_descriptor() {
    let source = SourceIdentity::new(
        "project".into(),
        "package".into(),
        "src/main.w".into(),
        "r1".into(),
    );
    let text = "%%start\nstruct copy Reader { u8() read; }\n(Reader, Reader)() make = fn { Reader shared = { .read = fn { 1 }; }; shared, shared };\n%%end";
    let second = text.find("shared, shared").unwrap() as u32 + "shared, ".len() as u32;
    let parsed = parse_source(source.clone(), text.to_owned(), &[]);
    assert!(parsed.diagnostics.is_empty(), "{:?}", parsed.diagnostics);
    for layout in [ScalarTargetLayout::WASM32, ScalarTargetLayout::NATIVE64] {
        let single = derive_scalar_program_with_layout(&parsed.result, layout);
        let project = validate_scalar_project(ScalarProject::new(
            vec![ScalarModule::from_program(
                single.program.clone(),
                Vec::new(),
            )],
            vec![source.clone()],
        ));
        for diagnostics in [&single.diagnostics, &project.diagnostics] {
            let mut spans = diagnostics
                .iter()
                .filter(|diagnostic| diagnostic.code == "B0003")
                .map(|diagnostic| {
                    assert_eq!(diagnostic.labels.len(), 1, "{diagnostic:?}");
                    diagnostic.labels[0].span.range
                })
                .collect::<Vec<_>>();
            spans.sort_by_key(|span| (span.start, span.end));
            spans.dedup();
            assert_eq!(
                spans,
                vec![ByteSpan::new(second, second + "shared".len() as u32)],
                "{text}: {diagnostics:?}"
            );
        }
    }
}

#[test]
fn callable_binding_addresses_follow_typed_holder_identity_on_both_targets() {
    let source = SourceIdentity::new(
        "project".into(),
        "package".into(),
        "src/main.w".into(),
        "r1".into(),
    );
    let text = "%%start\nu8() inspect = fn { u8() first = fn { 1 }; u8() copied = first; copied() };\ninspect();\n%%end";
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
        for items in [&single.program.items, &project.project.modules[0].items] {
            let ScalarItem::Function(function) = &items[0] else {
                panic!("inspect")
            };
            let context = Context::create();
            let module = context.create_module("holder_identity");
            let pointer = context.ptr_type(inkwell::AddressSpace::default());
            let first = module
                .add_global(pointer, None, "first_slot")
                .as_pointer_value();
            let copied = module
                .add_global(pointer, None, "copied_slot")
                .as_pointer_value();
            let mut events = super::holder_events::HolderEvents::new(&function.reference_cfg)
                .expect("holder events");
            let ScalarBlockItem::LocalBinding(first_binding) = &function.body.items[0] else {
                panic!("first")
            };
            let ScalarBlockItem::LocalBinding(copied_binding) = &function.body.items[1] else {
                panic!("copied")
            };
            events
                .register_binding(first_binding, 0, first)
                .expect("first holder");
            events
                .register_binding(copied_binding, 0, copied)
                .expect("copied holder");
            assert_eq!(events.bindings.len(), 2);
            assert!(events.bindings.values().any(|address| *address == first));
            assert!(events.bindings.values().any(|address| *address == copied));
            let connected = function
                .reference_cfg
                .holder_edges
                .iter()
                .find_map(|edge| {
                    edge.actions.iter().find_map(|action| match action {
                        crate::scalar::CallableHolderAction::Retain {
                            from: holder @ crate::scalar::CallableHolderPlace::Binding(_),
                            ..
                        } => Some((edge, holder)),
                        _ => None,
                    })
                })
                .expect("copied callable retain edge");
            events
                .connected_holder_edge(
                    super::holder_events::HolderEmissionEdge::Outgoing {
                        from: connected.0.from,
                        to: connected.0.to,
                    },
                    connected.1,
                )
                .expect("source holder on connected edge");
            assert!(events
                .connected_holder_edge(
                    super::holder_events::HolderEmissionEdge::Outgoing {
                        from: connected.0.to,
                        to: connected.0.from
                    },
                    connected.1
                )
                .is_err());
        }
    }
}

#[test]
fn holder_event_ids_and_edge_alternatives_match_both_pipelines() {
    use super::holder_events::{HolderEvents, HolderPointKind};

    let source = SourceIdentity::new(
        "project".into(),
        "package".into(),
        "src/main.w".into(),
        "r1".into(),
    );
    let text = "%%start\nstruct copy Reader { u8() read; }\nReader(u64) make = fn(count) { u8[count] bytes; bytes[0] = 4; Reader result = { .read = fn { bytes[0] }; }; result };\nunit() run = fn { Reader kept = make(2); Reader copied = kept; copied = make(3); if (true) { Reader branch = make(4); branch.read(); }; u64 index = 0; while (index < 2) { Reader iterated = make(5); iterated.read(); index = index + 1; } kept.read(); };\nrun();\n%%end";
    let parsed = parse_source(source.clone(), text.to_owned(), &[]);
    assert!(parsed.diagnostics.is_empty(), "{:?}", parsed.diagnostics);
    let single = derive_scalar_program_with_layout(&parsed.result, ScalarTargetLayout::WASM32);
    assert!(single.diagnostics.is_empty(), "{:?}", single.diagnostics);
    let project = validate_scalar_project(ScalarProject::new(
        vec![ScalarModule::from_program(
            single.program.clone(),
            Vec::new(),
        )],
        vec![source.clone()],
    ));
    assert!(project.diagnostics.is_empty(), "{:?}", project.diagnostics);
    for items in [&single.program.items, &project.project.modules[0].items] {
        let run = items
            .iter()
            .find_map(|item| match item {
                ScalarItem::Function(function) if function.name == "run" => Some(function),
                _ => None,
            })
            .expect("run");
        let maker = items
            .iter()
            .find_map(|item| match item {
                ScalarItem::Function(function) if function.name == "make" => Some(function),
                _ => None,
            })
            .expect("make");
        let events = HolderEvents::new(&run.reference_cfg).expect("indexed holder events");
        let at = |fragment: &str| {
            let start = text.find(fragment).unwrap() as u32;
            wosy_syntax::ByteSpan::new(start, start + fragment.len() as u32)
        };
        let calls = run
            .reference_cfg
            .call_points
            .iter()
            .filter(|call| {
                call.occurrence.source_span
                    == wosy_syntax::SourceSpan::new(source.clone(), at("make(4)"))
            })
            .collect::<Vec<_>>();
        let [call] = calls.as_slice() else {
            panic!("expected one branch call occurrence")
        };
        let ScalarType::Callable { outputs, .. } = &maker.signature else {
            panic!("maker signature")
        };
        let types = outputs
            .outputs
            .iter()
            .map(|output| output.ty.clone())
            .collect::<Vec<_>>();
        events
            .evaluated_call(&call.occurrence, &types)
            .expect("exact branch call and its output projection");
        assert_eq!(call.projections.len(), 1);
        assert_eq!(call.projections[0].output, 0);
        assert_eq!(call.projections[0].point, call.evaluated_point);
        let branch = events
            .point(&source, at("true"), HolderPointKind::Branch)
            .expect("branch point");
        let then_edges = events.arm(branch.id, 0).expect("taken arm");
        let skip_edges = events.arm(branch.id, 1).expect("skipped arm");
        assert!(then_edges
            .iter()
            .all(|edge| edge.to == branch.successors[0]));
        assert!(skip_edges
            .iter()
            .all(|edge| edge.to == branch.successors[1]));
        assert_ne!(branch.successors[0], branch.successors[1]);
        let loop_test = events
            .point(&source, at("index < 2"), HolderPointKind::LoopTest)
            .expect("loop head");
        let loop_branch = events
            .point(&source, at("index < 2"), HolderPointKind::Branch)
            .expect("loop branch");
        assert!(events.arm(loop_branch.id, 0).is_ok());
        assert!(events.arm(loop_branch.id, 1).is_ok());
        let body = events
            .point(
                &source,
                run.body
                    .items
                    .iter()
                    .find_map(|item| match item {
                        ScalarBlockItem::While(expression) => Some(expression.body.span),
                        _ => None,
                    })
                    .expect("loop body"),
                HolderPointKind::ScopeExit,
            )
            .expect("body exit");
        assert!(events.outgoing(body.id, loop_test.id).is_ok());
        for (fragment, kind) in [
            ("kept", HolderPointKind::Bind),
            ("copied", HolderPointKind::Bind),
            ("copied", HolderPointKind::CopyCallableFields),
        ] {
            let point = events
                .point(&source, at(fragment), kind)
                .expect("binding event");
            events.validated(point).expect("correlated edges");
        }
        let overwrite = at("copied = make(3)");
        let assign = events
            .point(
                &source,
                wosy_syntax::ByteSpan::new(overwrite.start, overwrite.start + 6),
                HolderPointKind::Assign,
            )
            .expect("overwrite event");
        events.validated(assign).expect("overwrite alternatives");
        let capture = maker
            .reference_cfg
            .points
            .iter()
            .find(|point| matches!(point.kind, CfgPointKind::Capture { .. }))
            .expect("capture");
        let maker_events = HolderEvents::new(&maker.reference_cfg).expect("maker events");
        assert_eq!(
            maker_events
                .point(&source, capture.source_span.range, HolderPointKind::Capture)
                .unwrap()
                .id,
            capture.id
        );
        for point in maker
            .reference_cfg
            .points
            .iter()
            .filter(|point| matches!(point.kind, CfgPointKind::ReturnOutput { .. }))
        {
            let CfgPointKind::ReturnOutput { output, .. } = point.kind else {
                unreachable!()
            };
            assert_eq!(
                maker_events
                    .return_output(&source, point.source_span.range, output)
                    .unwrap()
                    .id,
                point.id
            );
        }
    }
}

#[test]
fn imported_capture_release_keeps_distinct_calls_and_branch_loop_lifetimes() {
    let source = SourceIdentity::new(
        "project".into(),
        "package".into(),
        "src/main.w".into(),
        "r1".into(),
    );
    let text = "%%start\nstruct copy Reader { u8() read; }\nReader(u64) make = fn(count) { u8[count] bytes; bytes[0] = 4; Reader result = { .read = fn { bytes[0] }; }; result };\nunit() run = fn { Reader kept = make(2); Reader another = make(3); kept.read(); another.read(); if (true) { Reader branch = make(4); branch.read(); }; u64 index = 0; while (index < 2) { Reader iterated = make(5); iterated.read(); index = index + 1; } };\nrun();\n%%end";
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
        for (llvm, run_name) in [
            (
                emit_scalar_llvm(&single).expect("single LLVM").to_text(),
                "run".to_owned(),
            ),
            (
                emit_scalar_project_llvm(&project)
                    .expect("project LLVM")
                    .to_text(),
                project_function_name(&source, "run"),
            ),
        ] {
            let body = &llvm[llvm.find(&format!("@{run_name}(")).unwrap()..];
            let body = &body[..body.find("\n}").unwrap()];
            assert_eq!(body.matches(" = call ptr @").count(), 4, "{body}");
            assert!(
                !body.contains("imported_capture_cell_slot"),
                "provenanced imports need no synthetic import slots: {body}"
            );
            assert!(
                !body.contains("capture_backing_present"),
                "provenanced imports need no synthetic backing release: {body}"
            );
            assert!(
                llvm.contains("capture_cell_owner_dropped"),
                "each generation retires through its counted cell: {llvm}"
            );
            assert!(
                body.contains("if.then") && body.contains("while.body"),
                "{body}"
            );
            assert_eq!(
                body.matches("call void @__wosy_begin_graph_mutation()")
                    .count(),
                body.matches("call void @__wosy_end_graph_mutation()")
                    .count(),
                "{body}"
            );
        }
    }
}

#[test]
fn two_hop_make_run_use_with_param_capture_releases_once_on_both_targets() {
    let source = SourceIdentity::new(
        "project".into(),
        "package".into(),
        "src/main.w".into(),
        "r1".into(),
    );
    let text = "%%start\nstruct copy Reader { u8() read; }\nReader(u64) make = fn(count) { u8[count] bytes; bytes[0] = 4; Reader result = { .read = fn { bytes[0] }; }; result };\nu8(Reader) run = fn(item) { item.read() };\nunit() use = fn { Reader kept = make(2); u8 value = run(kept); value; };\nuse();\n%%end";
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
        for (llvm, use_name, maker_name) in [
            (
                emit_scalar_llvm(&single).expect("single LLVM").to_text(),
                "use".to_owned(),
                "make".to_owned(),
            ),
            (
                emit_scalar_project_llvm(&project)
                    .expect("project LLVM")
                    .to_text(),
                project_function_name(&source, "use"),
                project_function_name(&source, "make"),
            ),
        ] {
            assert_eq!(
                llvm.matches("call void @__wosy_core_free").count()
                    - llvm
                        .matches("call void @__wosy_core_free(ptr %environment_allocation_base)")
                        .count(),
                2,
                "adopted backing frees once and its counted cell frees once: {llvm}"
            );
            let maker = &llvm[llvm.find(&format!("@{maker_name}(")).unwrap()..];
            let maker = &maker[..maker.find("\n}").unwrap()];
            assert!(
                maker.contains("capture_cell_owner_dropped"),
                "maker exit drops its local cell ownership: {llvm}"
            );
            let use_body = &llvm[llvm.find(&format!("@{use_name}(")).unwrap()..];
            let use_body = &use_body[..use_body.find("\n}").unwrap()];
            assert_eq!(
                use_body.matches("call void @__wosy_core_free").count(),
                0,
                "use holds only holders; the adopted cell retires through the maker exit drop plus holder-dispose chain: {llvm}"
            );
        }
    }
}

#[test]
fn cross_function_global_overwrite_installs_new_generation_for_caller_on_both_targets() {
    let source = SourceIdentity::new(
        "project".into(),
        "package".into(),
        "src/main.w".into(),
        "r1".into(),
    );
    let text = "%%start\nstruct Dispatch { u8() action; }\nDispatch global_field = { .action = fn { 3 }; };\nunit() replace = fn { global_field.action = fn { 4 }; };\nreplace();\nDispatch kept = global_field;\nu8 r = kept.action();\nr;\n%%end";
    for layout in [ScalarTargetLayout::WASM32, ScalarTargetLayout::NATIVE64] {
        let parsed = parse_source(source.clone(), text.to_owned(), &[]);
        let single = derive_scalar_program_with_layout(&parsed.result, layout);
        assert!(single.diagnostics.is_empty(), "{:?}", single.diagnostics);
        let owner = single
            .program
            .terminal_owners
            .iter()
            .find(|owner| owner.global_name == "global_field")
            .expect("terminal owner");
        assert_eq!(
            owner.environment.literal.range.start,
            text.find("fn { 4 }").unwrap() as u32,
            "{owner:?}"
        );
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
            assert!(llvm.contains("@__wosy_begin_graph_mutation"), "{llvm}");
            assert!(llvm.contains("@__wosy_end_graph_mutation"), "{llvm}");
            assert!(
                llvm.contains("terminal_holder_environment"),
                "missing program-terminal owner tied to the installed global: {llvm}"
            );
            assert!(
                llvm.contains("environment.retain.increment"),
                "post-call copy requires a connected retain: {llvm}"
            );
        }
        let llvm = emit_scalar_llvm(&single).expect("single LLVM").to_text();
        assert_eq!(
            llvm.matches("call void %environment_dispose").count(),
            3,
            "overwritten old drop plus kept-copy drop plus installed global terminal drop: {llvm}"
        );
    }
}

#[test]
fn collector_uncovered_env_drop_twin_red_vector() {
    let source = SourceIdentity::new(
        "project".into(),
        "package".into(),
        "src/main.w".into(),
        "r1".into(),
    );
    let text = "%%start\nstruct Counter { i32(i32) advance; }\nstruct copy Captured { u8(u8) write; u8(u64) read; }\nCounter(i32) make_counter = fn(start) { i32 total = start; Counter made = { .advance = fn(delta) { total = total + delta; total }; }; total = total + 2; made };\nCaptured(u64) make_captured = fn(count) { u8[count] slots; slots[0] = 6; slots[1] = 7; Captured result = { .write = fn(value) { slots[0] = value; slots[0] }; .read = fn(index) { slots[index] }; }; u64 longer = count + 1; u8[longer] next; next[0] = 4; next[1] = 5; next[2] = 6; slots = next; result };\nunit() run = fn { Counter held = make_counter(40); i32 first = held.advance(1); i32 second = held.advance(3); Captured kept = make_captured(2); Captured copied = kept; u8 before = kept.read(0); u8 written = kept.write(9); u8 shared = copied.read(0); u64 length = 2; u8[length] replacement; replacement[0] = 5; replacement[1] = 8; kept.read = fn(index) { replacement[index] }; u8 old_after_overwrite = copied.read(0); u8 new_after_overwrite = kept.read(1); Captured separate = make_captured(2); u8 second_write = separate.write(4); u8 second_read = separate.read(0); u8 expanded_read = separate.read(2); if (first != 43 || second != 46 || before != 4 || written != 9 || shared != 9 || old_after_overwrite != 9 || new_after_overwrite != 8 || second_write != 4 || second_read != 4 || expanded_read != 6) { core.system_panic(); }; };\nrun();\n%%end";
    for layout in [ScalarTargetLayout::NATIVE64, ScalarTargetLayout::WASM32] {
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
        for item in &single.program.items {
            let crate::ScalarItem::Function(function) = item else {
                continue;
            };
            if function.name != "run" {
                continue;
            }
            let events = super::holder_events::HolderEvents::new(&function.reference_cfg)
                .expect("holder events");
            let mut counted = Vec::new();
            for (index, fact) in function.reference_cfg.environments.iter().enumerate() {
                if events.counted_environment(&crate::scalar::CallableEnvironmentId {
                    literal: fact.literal.clone(),
                    index,
                }) {
                    counted.push(fact.literal.range.start);
                }
            }
            counted.sort();
            // advance, both write/read generations, and the overwrite
            // literal installed via Assign are all counted: the overwrite
            // carries a connected install Retain+Drop pair on the nearest
            // downstream same-block Bind edge, so the LLVM skip no longer
            // applies to it and its terminal Drop retires the installed
            // generation.
            for literal in [
                text.find("fn(delta)").unwrap() as u32,
                text.find("fn(value)").unwrap() as u32,
                text.find("fn(index) { slots").unwrap() as u32,
                text.find("fn(index) { replacement").unwrap() as u32,
            ] {
                assert!(
                    counted.contains(&literal),
                    "counted literals {counted:?} miss {literal}"
                );
            }
        }
        for llvm in [
            emit_scalar_llvm(&single).expect("single LLVM").to_text(),
            emit_scalar_project_llvm(&project)
                .expect("project LLVM")
                .to_text(),
        ] {
            let allocs = llvm.matches("call ptr @__wosy_core_alloc").count();
            let frees = llvm.matches("call void @__wosy_core_free").count();
            let disposes = llvm.matches("environment_dispose").count();
            let retains = llvm.matches("environment.retain.increment").count();
            // D-0022 site accounting for this vector, identical on both
            // targets and both validators: one allocation site per live
            // generation (4 closure envs + 5 array backings + 1 capture cell
            // = 10); releases retire every generation exactly once,
            // including the Assign-installed overwrite generation whose
            // install pair plus terminal Drop now emit.
            assert_eq!(allocs, 10, "allocation sites per live generation: {allocs}");
            assert_eq!(
                frees, 19,
                "release sites cover every generation exactly once: {frees}"
            );
            assert_eq!(
                disposes, 48,
                "dispose traffic for counted generations: {disposes}"
            );
            assert_eq!(
                retains, 18,
                "retain sites incl. counted-domain entries: {retains}"
            );
        }
    }
}

#[test]
fn global_overwrite_keeps_saved_copy_callable_and_retires_each_generation_once_on_both_targets() {
    let source = SourceIdentity::new(
        "project".into(),
        "package".into(),
        "src/main.w".into(),
        "r1".into(),
    );
    let text = "%%start\nstruct Dispatch { u8() action; }\nDispatch global_field = { .action = fn { 3 }; };\nunit() invoke = fn { Dispatch saved = global_field; global_field.action = fn { 4 }; u8 old = saved.action(); u8 current = global_field.action(); old; current; };\ninvoke();\n%%end";
    for layout in [ScalarTargetLayout::WASM32, ScalarTargetLayout::NATIVE64] {
        let parsed = parse_source(source.clone(), text.to_owned(), &[]);
        let single = derive_scalar_program_with_layout(&parsed.result, layout);
        assert!(single.diagnostics.is_empty(), "{:?}", single.diagnostics);
        assert!(
            !single.program.terminal_owners.is_empty(),
            "terminal owners: {:?}; exits: {:?}",
            single.program.terminal_owners,
            single.program.top_level_reference_cfg.holder_exits
        );
        let project = validate_scalar_project(ScalarProject::new(
            vec![ScalarModule::from_program(
                single.program.clone(),
                Vec::new(),
            )],
            vec![source.clone()],
        ));
        assert!(project.diagnostics.is_empty(), "{:?}", project.diagnostics);
        for (llvm, invoke_name) in [
            (
                emit_scalar_llvm(&single).expect("single LLVM").to_text(),
                "invoke".to_owned(),
            ),
            (
                emit_scalar_project_llvm(&project)
                    .expect("project LLVM")
                    .to_text(),
                project_function_name(&source, "invoke"),
            ),
        ] {
            assert!(llvm.contains("@__wosy_begin_graph_mutation"), "{llvm}");
            assert!(llvm.contains("@__wosy_end_graph_mutation"), "{llvm}");
            assert!(
                llvm.contains("terminal_holder_environment"),
                "missing program-terminal owner tied to the installed global: {llvm}"
            );
            let define = format!("define void @{invoke_name}(");
            let body = &llvm[llvm
                .find(&define)
                .unwrap_or_else(|| panic!("{define}:\n{llvm}"))..];
            let end = body
                .find("\n}\n")
                .map(|index| index + 3)
                .unwrap_or(body.len());
            let body = &body[..end];
            assert!(
                body.contains("environment.retain.increment"),
                "saved copy requires a connected retain in the callee body: {body}"
            );
            assert_eq!(
                body.matches("call void %environment_dispose").count(),
                2,
                "overwritten old holder drop plus saved copy terminal drop in callee body: {body}"
            );
            assert_eq!(
                llvm.matches("call void %environment_dispose").count(),
                3,
                "each generation retires once including the final global drop: {llvm}"
            );
        }
    }
}

#[test]
fn overwrite_of_returned_callable_releases_old_backing_once_on_both_targets() {
    let source = SourceIdentity::new(
        "project".into(),
        "package".into(),
        "src/main.w".into(),
        "r1".into(),
    );
    let text = "%%start\nstruct copy Reader { u8() read; }\nReader(u64) make = fn(count) { u8[count] bytes; bytes[0] = 4; Reader result = { .read = fn { bytes[0] }; }; result };\nunit() run = fn { Reader kept = make(2); kept.read(); kept = make(3); kept.read(); };\nrun();\n%%end";
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
        for (llvm, run_name) in [
            (
                emit_scalar_llvm(&single).expect("single LLVM").to_text(),
                "run".to_owned(),
            ),
            (
                emit_scalar_project_llvm(&project)
                    .expect("project LLVM")
                    .to_text(),
                project_function_name(&source, "run"),
            ),
        ] {
            let body = &llvm[llvm.find(&format!("@{run_name}(")).unwrap()..];
            let body = &body[..body.find("\n}").unwrap()];
            assert_eq!(
                body.matches("call void @__wosy_core_free").count(),
                0,
                "run holds only holders; both generations retire through the maker exit drop plus holder-dispose chain: {body}"
            );
            assert_eq!(
                llvm.matches("call void @__wosy_core_free").count()
                    - llvm
                        .matches("call void @__wosy_core_free(ptr %environment_allocation_base)")
                        .count(),
                2,
                "{llvm}"
            );
        }
    }
}

#[test]
fn borrowed_call_received_reader_emits_no_backing_release_on_both_targets() {
    let source = SourceIdentity::new(
        "project".into(),
        "package".into(),
        "src/main.w".into(),
        "r1".into(),
    );
    let text = "%%start\nstruct copy Meter { u8() read; }\nu8(u8) apply = fn(value) { *u8 reference = &value; Meter gauge = { .read = fn { *reference }; }; gauge.read() };\nu8 outcome = apply(4);\n%%end";
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
            assert!(llvm.contains("__wosy_closure_"), "{llvm}");
            assert!(
                !llvm.contains("call void @__wosy_core_free(ptr %captured_backing)"),
                "{llvm}"
            );
        }
    }
}

#[test]
fn capture_descriptor_accepts_valid_facts_and_rejects_index_action_case_corruption_on_both_targets()
{
    fn reject_corruption(
        cfg: &crate::scalar::ScalarReferenceCfg,
        layout: ScalarTargetLayout,
        storage: &BTreeMap<
            crate::scalar::ReferenceBindingId,
            super::ownership::CaptureCellResidence,
        >,
        binding: &crate::scalar::ReferenceBindingId,
        arm: &str,
        reason: &str,
        mutate: impl FnOnce(&mut crate::scalar::ScalarReferenceCfg),
    ) {
        let mut corrupted = cfg.clone();
        mutate(&mut corrupted);
        let error = match super::holder_events::HolderEvents::new(&corrupted) {
            Ok(events) => {
                let occurrence = corrupted
                    .capture_points
                    .iter()
                    .find(|capture| {
                        matches!(&corrupted.points[capture.point.0].kind,
                            CfgPointKind::Capture { captures, .. } if captures.iter().any(|input| input.binding == *binding))
                    })
                    .expect("capture occurrence")
                    .occurrence
                    .clone();
                let point = events
                    .capture(&occurrence)
                    .expect("source-qualified capture");
                events
                    .capture_descriptor(&point, layout, storage)
                    .map(|descriptor| descriptor.is_some())
                    .expect_err(&format!("{arm} corruption must reject"))
            }
            Err(error) => error,
        };
        assert!(error.contains(reason), "arm={arm}: {error}");
        assert!(error.contains("src/main.w"), "arm={arm}: {error}");
    }

    fn capture_input<'a>(
        corrupted: &'a mut crate::scalar::ScalarReferenceCfg,
        env_index: usize,
        binding: &crate::scalar::ReferenceBindingId,
    ) -> &'a mut crate::scalar::CallableCaptureInput {
        let fact = corrupted
            .environments
            .get_mut(env_index)
            .expect("capture fact");
        fact.inputs
            .iter_mut()
            .find(|input| input.binding == *binding)
            .expect("capture input")
    }

    fn consistent_binding_backings<F: FnMut(&mut crate::scalar::RuntimeBackingFact)>(
        cfg: &mut crate::scalar::ScalarReferenceCfg,
        binding: &crate::scalar::ReferenceBindingId,
        mut mutate: F,
    ) -> usize {
        let mut count = 0usize;
        for env in &mut cfg.environments {
            for input in env
                .inputs
                .iter_mut()
                .filter(|input| input.binding == *binding)
            {
                for backing in input.backings.iter_mut().chain(
                    input
                        .projected_backings
                        .iter_mut()
                        .map(|projected| &mut projected.backing),
                ) {
                    mutate(backing);
                    count += 1;
                }
            }
        }
        let mut visit = |action: &mut crate::scalar::CallableHolderAction| match action {
            crate::scalar::CallableHolderAction::DropCaptureCellOwner {
                binding: actual,
                backing,
                ..
            } if *actual == *binding => {
                mutate(backing);
                count += 1;
            }
            crate::scalar::CallableHolderAction::ReplaceCaptureCellValue {
                binding: actual,
                previous,
                incoming,
                ..
            } if *actual == *binding => {
                mutate(previous);
                mutate(incoming);
                count += 2;
            }
            crate::scalar::CallableHolderAction::RetainArrayElements {
                source, backing, ..
            } => {
                if source.owner == *binding {
                    mutate(source);
                    count += 1;
                }
                if backing.owner == *binding {
                    mutate(backing);
                    count += 1;
                }
            }
            crate::scalar::CallableHolderAction::DropArrayElements { backing, .. } => {
                if backing.owner == *binding {
                    mutate(backing);
                    count += 1;
                }
            }
            _ => {}
        };
        for edge in &mut cfg.holder_edges {
            for action in &mut edge.actions {
                visit(action);
            }
        }
        for exit in &mut cfg.holder_exits {
            for action in &mut exit.actions {
                visit(action);
            }
        }
        count
    }

    fn first_installed_case_mut<'a>(
        backings: &'a mut [crate::scalar::RuntimeBackingFact],
    ) -> Option<&'a mut crate::scalar::CallableCellCaseFact> {
        for backing in backings.iter_mut() {
            for family in &mut backing.callable_family {
                for partition in &mut family.partitions {
                    for representative in &mut partition.representatives {
                        for case in &mut representative.cell.alternatives {
                            if matches!(
                                case,
                                crate::scalar::CallableCellCaseFact::Initialized { .. }
                            ) {
                                return Some(case);
                            }
                        }
                    }
                }
            }
        }
        None
    }

    let source = SourceIdentity::new(
        "project".into(),
        "package".into(),
        "src/main.w".into(),
        "r1".into(),
    );
    let text =
        include_str!("../../../../tests/fixtures/build/captured-callable-array/project/src/main.w");
    let parsed = parse_source(source.clone(), text.to_owned(), &[]);
    assert!(parsed.diagnostics.is_empty(), "{:?}", parsed.diagnostics);
    for layout in [ScalarTargetLayout::NATIVE64, ScalarTargetLayout::WASM32] {
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
        for items in [&single.program.items, &project.project.modules[0].items] {
            let make = items
                .iter()
                .find_map(|item| match item {
                    ScalarItem::Function(function) if function.name == "make" => Some(function),
                    _ => None,
                })
                .expect("make");
            assert_eq!(make.reference_cfg.producer_return_cases.len(), 3);
            let binding = make
                .reference_cfg
                .holder_edges
                .iter()
                .flat_map(|edge| &edge.actions)
                .find_map(|action| match action {
                    crate::scalar::CallableHolderAction::ReplaceCaptureCellValue {
                        binding,
                        ..
                    } => Some(binding.clone()),
                    _ => None,
                })
                .expect("replaced cell binding");
            let capture_environment = make
                .reference_cfg
                .points
                .iter()
                .find_map(|point| match &point.kind {
                    CfgPointKind::Capture {
                        environment,
                        captures,
                        ..
                    } if captures.iter().any(|input| input.binding == binding) => {
                        Some(environment.clone())
                    }
                    _ => None,
                })
                .expect("capturing environment");
            let env_index = capture_environment.index;
            let storage = BTreeMap::from([(
                binding.clone(),
                super::ownership::CaptureCellResidence::Promoted,
            )]);
            let events = super::holder_events::HolderEvents::new(&make.reference_cfg).unwrap();
            let occurrence = make
                .reference_cfg
                .capture_points
                .iter()
                .find(|capture| {
                    matches!(&make.reference_cfg.points[capture.point.0].kind,
                        CfgPointKind::Capture { captures, .. } if captures.iter().any(|input| input.binding == binding))
                })
                .expect("capture occurrence")
                .occurrence
                .clone();
            let point = events.capture(&occurrence).unwrap();
            let descriptor = events
                .capture_descriptor(&point, layout, &storage)
                .unwrap()
                .unwrap();
            assert_eq!(descriptor.cells.len(), 1);
            let cell = &descriptor.cells[0];
            assert_eq!(cell.binding, binding);
            assert!(!cell.backings.is_empty());
            assert!(!cell.owner_exits.is_empty());
            assert!(!cell.replacements.is_empty());
            assert!(cell
                .replacements
                .iter()
                .all(|(_, previous, incoming)| cell.backings.contains(previous)
                    && cell.backings.contains(incoming)));
            assert_eq!(cell.initialized_origins.len(), 5);
            for (position, origin) in cell.initialized_origins.iter().enumerate() {
                assert!(
                    cell.initialized_origins[..position]
                        .iter()
                        .all(|known| known.literal != origin.literal),
                    "duplicate producer origin: {origin:?}"
                );
                assert!(
                    make.reference_cfg
                        .environments
                        .get(origin.index)
                        .is_some_and(|fact| fact.literal == origin.literal),
                    "unresolved producer origin: {origin:?}"
                );
            }
            assert!(!cell.initialized_cases.is_empty());
            assert_eq!(descriptor.slot_width, 8);
            assert_eq!(descriptor.payload_size, 16);
            assert_eq!(cell.length_offset, Some(8));
            reject_corruption(
                &make.reference_cfg,
                layout,
                &storage,
                &binding,
                "direct-length",
                "connected backing length",
                |corrupted| {
                    let count = consistent_binding_backings(corrupted, &binding, |backing| {
                        if let crate::scalar::RuntimeArrayLengthFact::Evaluated { point, .. } =
                            &mut backing.length
                        {
                            *point = crate::scalar::CfgPointId(usize::MAX);
                        }
                    });
                    assert!(count > 0, "expected backing copies");
                },
            );
            reject_corruption(
                &make.reference_cfg,
                layout,
                &storage,
                &binding,
                "direct-place",
                "unrelated direct place",
                |corrupted| {
                    let count = consistent_binding_backings(corrupted, &binding, |backing| {
                        if let Some(element) = backing
                            .callable_family
                            .iter_mut()
                            .flat_map(|family| &mut family.elements)
                            .next()
                        {
                            element.place.source.path.push_str(".foreign");
                        }
                    });
                    assert!(count > 0, "expected backing copies");
                },
            );
            reject_corruption(
                &make.reference_cfg,
                layout,
                &storage,
                &binding,
                "projected-env",
                "projected producer",
                |corrupted| {
                    let input = capture_input(corrupted, env_index, &binding);
                    let projected = input.projected.iter_mut().next().expect("projected input");
                    let mut foreign_source = source.clone();
                    foreign_source.path.push_str(".foreign");
                    projected
                        .environments
                        .insert(crate::scalar::CallableEnvironmentId {
                            literal: wosy_syntax::SourceSpan::new(
                                foreign_source,
                                wosy_syntax::ByteSpan::new(0, 1),
                            ),
                            index: usize::MAX,
                        });
                },
            );
            reject_corruption(
                &make.reference_cfg,
                layout,
                &storage,
                &binding,
                "projected-place",
                "projected place",
                |corrupted| {
                    let input = capture_input(corrupted, env_index, &binding);
                    input
                        .projected
                        .iter_mut()
                        .next()
                        .expect("projected input")
                        .place
                        .source
                        .path
                        .push_str(".foreign");
                },
            );
            reject_corruption(
                &make.reference_cfg,
                layout,
                &storage,
                &binding,
                "installed-key",
                "connected installed index",
                |corrupted| {
                    let count = consistent_binding_backings(corrupted, &binding, |backing| {
                        for family in &mut backing.callable_family {
                            for partition in &mut family.partitions {
                                for representative in &mut partition.representatives {
                                    if let crate::scalar::CallableIndexKeyFact::Evaluated {
                                        point,
                                        ..
                                    } = &mut representative.key
                                    {
                                        *point = crate::scalar::CfgPointId(usize::MAX);
                                    }
                                }
                            }
                        }
                    });
                    assert!(count > 0, "expected backing copies");
                },
            );
            reject_corruption(
                &make.reference_cfg,
                layout,
                &storage,
                &binding,
                "installed-anchor",
                "connected installed anchor",
                |corrupted| {
                    let count = consistent_binding_backings(corrupted, &binding, |backing| {
                        for family in &mut backing.callable_family {
                            for partition in &mut family.partitions {
                                for representative in &mut partition.representatives {
                                    for anchor in &mut representative.anchors {
                                        if let crate::scalar::CallableIndexAnchorFact::InFlightSelection(
                                            index_point,
                                        ) = anchor
                                        {
                                            *index_point = crate::scalar::CfgPointId(usize::MAX);
                                        }
                                    }
                                }
                            }
                        }
                    });
                    assert!(count > 0, "expected backing copies");
                },
            );
            reject_corruption(
                &make.reference_cfg,
                layout,
                &storage,
                &binding,
                "action-span",
                "another source",
                |corrupted| {
                    let mut found = false;
                    for edge in &mut corrupted.holder_edges {
                        for action in &mut edge.actions {
                            if let crate::scalar::CallableHolderAction::ReplaceCaptureCellValue {
                                binding: actual,
                                span,
                                ..
                            } = action
                            {
                                if actual == &binding {
                                    span.source.path.push_str(".foreign");
                                    found = true;
                                }
                            }
                        }
                    }
                    assert!(found, "expected a connected replacement action");
                },
            );
            reject_corruption(
                &make.reference_cfg,
                layout,
                &storage,
                &binding,
                "action-incoming",
                "unrelated backing alternative",
                |corrupted| {
                    let mut found = false;
                    for edge in &mut corrupted.holder_edges {
                        for action in &mut edge.actions {
                            if let crate::scalar::CallableHolderAction::ReplaceCaptureCellValue {
                                binding: actual,
                                incoming,
                                ..
                            } = action
                            {
                                if actual == &binding {
                                    incoming.value.source.path.push_str(".foreign");
                                    found = true;
                                }
                            }
                        }
                    }
                    assert!(found, "expected a connected replacement action");
                },
            );
            reject_corruption(
                &make.reference_cfg,
                layout,
                &storage,
                &binding,
                "case-origin",
                "unrelated installed case origin",
                |corrupted| {
                    let count = consistent_binding_backings(corrupted, &binding, |backing| {
                        if let Some(case) = first_installed_case_mut(std::slice::from_mut(backing))
                        {
                            if let crate::scalar::CallableCellCaseFact::Initialized {
                                origin, ..
                            } = case
                            {
                                origin.source.path.push_str(".foreign");
                            }
                        }
                    });
                    assert!(count > 0, "expected backing copies");
                },
            );
            reject_corruption(
                &make.reference_cfg,
                layout,
                &storage,
                &binding,
                "case-alias",
                "case alias",
                |corrupted| {
                    let count = consistent_binding_backings(corrupted, &binding, |backing| {
                        if let Some(case) = first_installed_case_mut(std::slice::from_mut(backing))
                        {
                            if let crate::scalar::CallableCellCaseFact::Initialized {
                                holder_aliases,
                                ..
                            } = case
                            {
                                let mut foreign_binding = binding.clone();
                                foreign_binding.source.path.push_str(".foreign");
                                holder_aliases.push(crate::scalar::CallableHolderPlace::Binding(
                                    foreign_binding,
                                ));
                            }
                        }
                    });
                    assert!(count > 0, "expected backing copies");
                },
            );
            reject_corruption(
                &make.reference_cfg,
                layout,
                &storage,
                &binding,
                "case-loan",
                "unrelated loan",
                |corrupted| {
                    let count = consistent_binding_backings(corrupted, &binding, |backing| {
                        if let Some(case) = first_installed_case_mut(std::slice::from_mut(backing))
                        {
                            if let crate::scalar::CallableCellCaseFact::Initialized {
                                loans, ..
                            } = case
                            {
                                loans.insert(crate::scalar::CallableLoanId {
                                    origin: crate::scalar::ReferenceOriginId(usize::MAX),
                                });
                            }
                        }
                    });
                    assert!(count > 0, "expected backing copies");
                },
            );
        }
        emit_scalar_llvm(&single).expect("single callable array IR");
        emit_scalar_project_llvm(&project).expect("project callable array IR");
    }
}
#[test]
fn scalar_capture_cycle_retires_through_trial_deletion_on_both_targets() {
    let source = SourceIdentity::new(
        "project".into(),
        "package".into(),
        "src/main.w".into(),
        "r1".into(),
    );
    let scalar_cycle = "%%start\n(unit())() make = fn { unit() inner = fn { 1; }; inner = fn { inner(); }; inner };\nunit() run = fn { unit() first = make(); unit() copied = first; unit() second = make(); copied; second; };\n%%end";
    let installed_cycle = "%%start\n(unit())() make = fn { (unit())[1] slots; unit() inner = fn { unit() picked = slots[0]; picked(); }; slots[0] = inner; inner };\nunit() run = fn { unit() first = make(); unit() copied = first; unit() second = make(); copied; second; };\n%%end";
    for layout in [ScalarTargetLayout::NATIVE64, ScalarTargetLayout::WASM32] {
        let parsed = parse_source(source.clone(), scalar_cycle.to_owned(), &[]);
        assert!(parsed.diagnostics.is_empty(), "{:?}", parsed.diagnostics);
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
        for (llvm, make_name, run_name) in [
            (
                emit_scalar_llvm(&single)
                    .expect("single scalar cycle IR")
                    .to_text(),
                "make".to_owned(),
                "run".to_owned(),
            ),
            (
                emit_scalar_project_llvm(&project)
                    .expect("project scalar cycle IR")
                    .to_text(),
                project_function_name(&source, "make"),
                project_function_name(&source, "run"),
            ),
        ] {
            assert!(
                llvm.contains("define internal void @__wosy_gc_trial("),
                "{layout:?}"
            );
            assert!(llvm.contains("call void @__wosy_gc_trial()"), "{layout:?}");
            assert!(llvm.contains("graph_mutation.collect"), "{layout:?}");
            assert!(llvm.contains("__wosy_scalar_cell"), "{layout:?}");
            assert!(llvm.contains("environment.trace.unique"), "{layout:?}");
            assert!(llvm.contains("scalar_cell.present"), "{layout:?}");
            assert!(
                llvm.contains("call void @__wosy_gc_register_env"),
                "{layout:?}"
            );
            assert!(
                llvm.contains("call void @__wosy_gc_register_cell"),
                "{layout:?}"
            );
            assert!(
                llvm.contains("call void @__wosy_gc_unregister_env"),
                "{layout:?}"
            );
            assert!(
                llvm.contains("call void @__wosy_gc_unregister_cell"),
                "{layout:?}"
            );
            let make_body = &llvm[llvm
                .find(&format!("@{make_name}("))
                .unwrap_or_else(|| panic!("{make_name}:\n{llvm}"))..];
            let make_body = &make_body[..make_body.find("\n}\n").expect("make body")];
            assert_eq!(
                make_body
                    .matches("= add i64 %environment_holder_count")
                    .count(),
                1,
                "cell install retains its cyclic environment: {make_body}"
            );
            assert_eq!(
                make_body
                    .matches("= sub i64 %capture_cell_owner_count")
                    .count(),
                1,
                "maker exit drops its captured cell owner: {make_body}"
            );
            let run_body = &llvm[llvm
                .find(&format!("@{run_name}("))
                .unwrap_or_else(|| panic!("{run_name}:\n{llvm}"))..];
            let run_body = &run_body[..run_body.find("\n}\n").expect("run body")];
            assert_eq!(
                run_body
                    .matches("= add i64 %environment_holder_count")
                    .count(),
                1,
                "copied holder retains its environment: {run_body}"
            );
            assert_eq!(
                run_body
                    .matches("= sub i64 %environment_holder_count")
                    .count(),
                3,
                "each dropped holder releases its environment: {run_body}"
            );
        }
        let installed = parse_source(source.clone(), installed_cycle.to_owned(), &[]);
        assert!(
            installed.diagnostics.is_empty(),
            "{:?}",
            installed.diagnostics
        );
        let derived = derive_scalar_program_with_layout(&installed.result, layout);
        assert!(derived.diagnostics.is_empty(), "{:?}", derived.diagnostics);
        let project = validate_scalar_project(ScalarProject::new(
            vec![ScalarModule::from_program(
                derived.program.clone(),
                Vec::new(),
            )],
            vec![source.clone()],
        ));
        assert!(project.diagnostics.is_empty(), "{:?}", project.diagnostics);
        for (llvm, make_name, run_name) in [
            (
                emit_scalar_llvm(&derived)
                    .expect("single installed cycle IR")
                    .to_text(),
                "make".to_owned(),
                "run".to_owned(),
            ),
            (
                emit_scalar_project_llvm(&project)
                    .expect("project installed cycle IR")
                    .to_text(),
                project_function_name(&source, "make"),
                project_function_name(&source, "run"),
            ),
        ] {
            assert!(
                llvm.contains("define internal void @__wosy_gc_trial("),
                "{layout:?}"
            );
            assert!(llvm.contains("call void @__wosy_gc_trial()"), "{layout:?}");
            assert!(llvm.contains("__wosy_fixed_cell"), "{layout:?}");
            assert!(llvm.contains("environment.trace.unique"), "{layout:?}");
            assert!(llvm.contains("capture_cell.walk.installed"), "{layout:?}");
            let make_body = &llvm[llvm
                .find(&format!("@{make_name}("))
                .unwrap_or_else(|| panic!("{make_name}:\n{llvm}"))..];
            let make_body = &make_body[..make_body.find("\n}\n").expect("make body")];
            assert_eq!(
                make_body
                    .matches("= add i64 %environment_holder_count")
                    .count(),
                1,
                "installed slot retains its cyclic environment: {make_body}"
            );
            assert_eq!(
                make_body
                    .matches("= sub i64 %capture_cell_owner_count")
                    .count(),
                1,
                "maker exit drops its captured array cell owner: {make_body}"
            );
            assert_eq!(
                make_body
                    .matches("= sub i64 %environment_holder_count")
                    .count(),
                0,
                "installed slot edge survives its projection loan: {make_body}"
            );
            let run_body = &llvm[llvm
                .find(&format!("@{run_name}("))
                .unwrap_or_else(|| panic!("{run_name}:\n{llvm}"))..];
            let run_body = &run_body[..run_body.find("\n}\n").expect("run body")];
            assert_eq!(
                run_body
                    .matches("= add i64 %environment_holder_count")
                    .count(),
                1,
                "copied holder retains its environment: {run_body}"
            );
            assert_eq!(
                run_body
                    .matches("= sub i64 %environment_holder_count")
                    .count(),
                3,
                "each dropped holder releases its environment: {run_body}"
            );
        }
    }
}

#[test]
fn fixed_array_module_dynamic_store_uses_i32_index_on_both_targets() {
    let source = SourceIdentity::new(
        "project".into(),
        "package".into(),
        "src/main.w".into(),
        "r1".into(),
    );
    let text = "%%start\nu8[2] buf = [0, 0];\nu64 idx = 1;\nbuf[idx] = 7;\n%%end";
    for layout in [ScalarTargetLayout::WASM32, ScalarTargetLayout::NATIVE64] {
        let parsed = parse_source(source.clone(), text.to_owned(), &[]);
        let validation = derive_scalar_program_with_layout(&parsed.result, layout);
        assert!(
            validation.diagnostics.is_empty(),
            "{:?}",
            validation.diagnostics
        );
        let llvm = emit_scalar_llvm(&validation)
            .expect("module dynamic store LLVM")
            .to_text();
        assert!(llvm.contains("trunc i64"), "{llvm}");
        assert!(!llvm.contains("i32 0, i64"), "{llvm}");
        let dynamic = llvm
            .lines()
            .filter(|line| {
                line.contains("getelementptr inbounds [2 x i8]")
                    && line.contains("array_element")
                    && line.contains("i32 %array_index")
            })
            .collect::<Vec<_>>();
        assert_eq!(dynamic.len(), 1, "{llvm}");
        for address in dynamic {
            assert!(address.contains("i32 0, i32"), "{address}");
        }
    }
}

#[test]
fn fixed_array_function_dynamic_store_load_roundtrip_uses_i32_index_on_both_targets() {
    let source = SourceIdentity::new(
        "project".into(),
        "package".into(),
        "src/main.w".into(),
        "r1".into(),
    );
    let text = "%%start\nu8(u64) run = fn(index) { u8[2] buf = [0, 0]; buf[index] = 7; buf[index] };\nrun(1);\n%%end";
    for layout in [ScalarTargetLayout::WASM32, ScalarTargetLayout::NATIVE64] {
        let parsed = parse_source(source.clone(), text.to_owned(), &[]);
        let validation = derive_scalar_program_with_layout(&parsed.result, layout);
        assert!(
            validation.diagnostics.is_empty(),
            "{:?}",
            validation.diagnostics
        );
        let llvm = emit_scalar_llvm(&validation)
            .expect("function dynamic store-load LLVM")
            .to_text();
        assert!(llvm.contains("trunc i64"), "{llvm}");
        assert!(!llvm.contains("i32 0, i64"), "{llvm}");
        let dynamic = llvm
            .lines()
            .filter(|line| {
                line.contains("getelementptr inbounds [2 x i8]")
                    && line.contains("array_element")
                    && line.contains("i32 %array_index")
            })
            .collect::<Vec<_>>();
        assert_eq!(dynamic.len(), 2, "{llvm}");
        for address in dynamic {
            assert!(address.contains("i32 0, i32"), "{address}");
        }
    }
}

#[test]
fn runtime_array_address_after_checked_call_retains_direct_base_on_both_targets() {
    let source = SourceIdentity::new(
        "project".into(),
        "package".into(),
        "src/main.w".into(),
        "r1".into(),
    );
    let text = "%%start\nu8(*u8, u64) peek = fn(data, length) { length; *data };\n*?u8 saved = null;\nu8(u64, u8) run = fn(max, first) { u8[max] buf; buf[0] = first; u64 index = 0; while (index < 1) { u8 seen = peek(&buf[0], 1); seen; index = index + 1; }; unsafe { saved = &?buf[0]; }; first };\nrun(4, 120);\n%%end";
    for layout in [ScalarTargetLayout::WASM32, ScalarTargetLayout::NATIVE64] {
        let parsed = parse_source(source.clone(), text.to_owned(), &[]);
        let validation = derive_scalar_program_with_layout(&parsed.result, layout);
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
                .expect("address-after-checked-call LLVM")
                .to_text(),
            emit_scalar_project_llvm(&project)
                .expect("project address-after-checked-call LLVM")
                .to_text(),
        ] {
            assert!(!llvm.contains("load ptr, ptr %allocation"), "{llvm}");
            assert!(
                llvm.contains("getelementptr inbounds i8, ptr %allocation, i64 0"),
                "{llvm}"
            );
        }
    }
}

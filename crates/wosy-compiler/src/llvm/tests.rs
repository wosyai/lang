use std::collections::BTreeMap;

use inkwell::context::Context;
use inkwell::values::{BasicValue, BasicValueEnum, InstructionOpcode, InstructionValue};

use super::emit_scalar_llvm;
use super::emit_scalar_project_llvm;
use super::function_value_result;
use super::project_function_name;
use super::project_global_name;
use super::storage_type;
use super::{LlvmFunction, LlvmFunctionAttributes, LlvmPartition, LlvmValueType};
use crate::scalar::{CfgPointKind, CoreOperationId, ReleaseTarget, ScalarOverloadSelection};
use crate::{
    analyze_scalar_build, derive_scalar_declarations_from_cst, derive_scalar_program,
    derive_scalar_program_with_layout, emit_scalar_build_project_llvm,
    emit_scalar_build_single_module_llvm, parse_source, prepare_scalar_project,
    validate_scalar_project, ScalarBlockItem, ScalarExpression, ScalarItem, ScalarModule, ScalarNamespaceBinding, ScalarOutput,
    ScalarOutputSequence, ScalarPreparationInputs, ScalarProject, ScalarTargetLayout, ScalarType,
};
use wosy_syntax::{ByteSpan, SourceIdentity};

fn prepare_initializer_test(
    text: &str,
    target_layout: ScalarTargetLayout,
) -> crate::PreparedScalarProject {
    let source = SourceIdentity::new(
        "project".into(),
        "package".into(),
        "src/main.w".into(),
        "r1".into(),
    );
    let parsed = parse_source(source.clone(), text.into(), &[]);
    assert!(parsed.diagnostics.is_empty(), "{:?}", parsed.diagnostics);
    prepare_scalar_project(
        vec![(
            derive_scalar_declarations_from_cst(parsed.result.canonical_cst()),
            Vec::new(),
        )],
        vec![source],
        ScalarPreparationInputs { target_layout },
    )
    .unwrap_or_else(|error| panic!("{target_layout:?}: {:?}", error.diagnostics))
}

#[test]
fn storage_scope_undo_restores_shadowed_bindings_and_removes_locals() {
    let context = Context::create();
    let module = context.create_module("storage_scope");
    let pointer = |name| module.add_global(context.i32_type(), None, name).as_pointer_value();
    let outer = pointer("outer");
    let shadow = pointer("shadow");
    let inner = pointer("inner");
    let temporary = pointer("temporary");
    let mut storage = super::StorageBindings::default();
    storage.insert("value".into(), (outer, ScalarType::I32));

    let read_only_scope = storage.enter_scope();
    assert_eq!(storage.get("value"), Some(&(outer, ScalarType::I32)));
    storage.leave_scope(read_only_scope);
    assert_eq!(storage.undo.capacity(), 0);

    let then_scope = storage.enter_scope();
    storage.insert("value".into(), (shadow, ScalarType::I32));
    storage.insert("local".into(), (temporary, ScalarType::U32));
    let inner_scope = storage.enter_scope();
    storage.insert("value".into(), (inner, ScalarType::U32));
    storage.insert("value".into(), (temporary, ScalarType::U32));
    storage.insert("inner".into(), (inner, ScalarType::U32));
    storage.leave_scope(inner_scope);
    assert_eq!(storage.get("value"), Some(&(shadow, ScalarType::I32)));
    assert_eq!(storage.get("local"), Some(&(temporary, ScalarType::U32)));
    assert!(storage.get("inner").is_none());
    storage.leave_scope(then_scope);

    let else_scope = storage.enter_scope();
    assert_eq!(storage.get("value"), Some(&(outer, ScalarType::I32)));
    assert!(storage.get("local").is_none());
    storage.insert("other".into(), (inner, ScalarType::I32));
    storage.leave_scope(else_scope);
    assert!(storage.get("other").is_none());
    assert_eq!(storage.get("value"), Some(&(outer, ScalarType::I32)));
    assert!(storage.undo.is_empty());
    storage.insert("value".into(), (inner, ScalarType::U32));
    assert!(storage.undo.is_empty());
    assert_eq!(storage.get("value"), Some(&(inner, ScalarType::U32)));
}

#[test]
fn consuming_llvm_text_preserves_exact_header_formatting() {
    for (raw, expected) in [
        ("source_filename = \"x\"\n", "source_filename = \"x\"\n"),
        ("source_filename = \"x\"\n\n", "source_filename = \"x\"\n\n"),
        ("source_filename = \"x\"\n\n\n", "source_filename = \"x\"\n\n"),
        ("préface\nsource_filename = \"日\"\n\n\n\n; finé",
         "préface\nsource_filename = \"日\"\n\n\n; finé"),
        ("source_filename = \"x\"\r\n\r\n\r\n; body\r\n",
         "source_filename = \"x\"\n\n; body\n"),
        ("source_filename = \"x\"\n\n\n\r", "source_filename = \"x\"\n\n\r"),
        ("; no header\n\n\n", "; no header\n\n\n"),
    ] {
        let partition = LlvmPartition {
            module_name: String::new(),
            declarations: Vec::new(),
            functions: Vec::new(),
            text: raw.to_owned(),
        };
        let mut streamed = Vec::new();
        partition.write_text(&mut streamed).expect("write LLVM partition");
        assert_eq!(streamed, expected.as_bytes(), "streamed: {raw:?}");
        assert_eq!(partition.to_text(), expected, "borrowed: {raw:?}");
        assert_eq!(partition.into_text(), expected, "consuming: {raw:?}");
    }
}

#[test]
fn text_only_emission_matches_structured_partitions_on_both_targets() {
    let source = SourceIdentity::new(
        "project".into(),
        "package".into(),
        "src/main.w".into(),
        "r1".into(),
    );
    for layout in [ScalarTargetLayout::WASM32, ScalarTargetLayout::NATIVE64] {
        for external_items in [
            "",
            "scene = extern wasm \"showcase\" { unit(u8) mesh; };\nscene.mesh(7);",
        ] {
            let text = format!(
                "%%start\nu8[2] row = [1, 2];\nu8[2][2] grid = [row, row];\nu8(u8) ordinary = fn(value) {{ value }};\nu8 result = ordinary(7);\n{external_items}\n%%end"
            );
            let parsed = parse_source(source.clone(), text.clone(), &[]);
            let raw = derive_scalar_program_with_layout(&parsed.result, layout);
            let raw_partition = emit_scalar_llvm(&raw).unwrap();
            assert!(raw_partition.functions.iter().any(|function| function.parameters.len() == 1));
            assert_eq!(
                crate::emit_scalar_llvm_text(&raw).unwrap(),
                raw_partition.to_text()
            );
            let prepared = prepare_initializer_test(&text, layout);
            let validation = analyze_scalar_build(&prepared);
            assert!(validation.diagnostics().is_empty(), "{:?}", validation.diagnostics());
            for (partition, text) in [
                (
                    emit_scalar_build_single_module_llvm(&validation).unwrap(),
                    crate::emit_scalar_build_single_module_llvm_text(&validation).unwrap(),
                ),
                (
                    emit_scalar_build_project_llvm(&validation).unwrap(),
                    crate::emit_scalar_build_project_llvm_text(&validation).unwrap(),
                ),
            ] {
                assert!(partition.functions.iter().any(|function| function.parameters.len() == 1));
                assert_eq!(partition.declarations.is_empty(), external_items.is_empty());
                assert_eq!(text, partition.to_text());
            }
        }
    }
}

#[test]
fn raw_address_of_checked_dereference_preserves_the_referent() {
    let text = "%%start
unit(*?u8) read_into = fn(destination) { destination; };
unit(*!u8) step = fn(view) { unsafe { read_into(&?*view); }; };
unit(*?u8) raw = fn(view) { unsafe { read_into(&?*view); }; };
unit() wrapper = fn { u8[1] scratch = [0]; step(&!scratch[0]); };
%%end";
    let mut rejected = Vec::new();
    for layout in [ScalarTargetLayout::NATIVE64, ScalarTargetLayout::WASM32] {
        let source = SourceIdentity::new(
            "project".into(),
            "package".into(),
            "src/main.w".into(),
            "r1".into(),
        );
        let parsed = parse_source(source.clone(), text.into(), &[]);
        assert!(parsed.diagnostics.is_empty(), "{:?}", parsed.diagnostics);
        let standalone = derive_scalar_program_with_layout(&parsed.result, layout);
        if !standalone.diagnostics.is_empty() {
            rejected.push(format!(
                "{layout:?} standalone: {:?}",
                standalone.diagnostics
            ));
        }
        let prepared = prepare_scalar_project(
            vec![(
                derive_scalar_declarations_from_cst(parsed.result.canonical_cst()),
                Vec::new(),
            )],
            vec![source.clone()],
            ScalarPreparationInputs {
                target_layout: layout,
            },
        );
        let prepared = match prepared {
            Ok(prepared) => prepared,
            Err(error) => {
                rejected.push(format!("{layout:?} project: {:?}", error.diagnostics));
                continue;
            }
        };
        let build = analyze_scalar_build(&prepared);
        assert!(build.diagnostics().is_empty(), "{:?}", build.diagnostics());
        let cfg = build.reference_cfg(0, 1, None).unwrap();
        assert_eq!(cfg.facts.len(), 1, "{cfg:?}");
        let [(pointer, candidates)] = cfg
            .points
            .iter()
            .filter_map(|point| match &point.kind {
                CfgPointKind::ReadPlace {
                    pointer_point: Some(pointer),
                    candidates,
                    reads_origin: false,
                    ..
                } => Some((pointer, candidates)),
                _ => None,
            })
            .collect::<Vec<_>>()[..]
        else {
            panic!("one checked access: {cfg:?}");
        };
        let CfgPointKind::Read { binding } = &cfg.points[pointer.0].kind else {
            panic!("one pointer read: {cfg:?}");
        };
        let [candidate] = candidates.as_slice() else {
            panic!("one checked referent: {cfg:?}");
        };
        assert_eq!(&candidate.binding, binding);
        assert!(!cfg.points.iter().any(|point| matches!(
            point.kind,
            CfgPointKind::Borrow { .. }
                | CfgPointKind::Move { .. }
                | CfgPointKind::MoveThrough { .. }
        )));
        let raw_cfg = build.reference_cfg(0, 2, None).unwrap();
        assert!(!raw_cfg
            .points
            .iter()
            .any(|point| matches!(point.kind, CfgPointKind::ReadPlace { .. })));
        let width = layout.pointer_size * 8;
        for (project, partition) in [
            (false, emit_scalar_build_single_module_llvm(&build)),
            (true, emit_scalar_build_project_llvm(&build)),
            (false, emit_scalar_llvm(&standalone)),
        ] {
            let ir = partition.unwrap().to_text();
            let symbol = |name| self_tail_test_symbol(&source, project, name);
            for (name, checked) in [("step", true), ("raw", false)] {
                let body = self_tail_test_body(&ir, "void", &symbol(name));
                assert_eq!(body.contains("icmp eq"), checked, "{body}");
                assert!(body.contains(&format!("inttoptr i{width}")), "{body}");
                assert!(
                    body.contains(&format!("%address = ptrtoint ptr %deref to i{width}")),
                    "{body}"
                );
                assert!(
                    body.contains(&format!(
                        "call void @{}(i{width} %address)",
                        symbol("read_into")
                    )),
                    "{body}"
                );
                assert!(!body.contains("load i8, ptr"), "{body}");
            }
        }
    }
    assert!(rejected.is_empty(), "{rejected:#?}");
}

#[test]
fn raw_address_of_checked_dereference_preserves_unsafe_and_invalidation_diagnostics() {
    let outside = "%%start\nunit(*!u8) step = fn(view) { &?*view; };\n%%end";
    let wrong = "%%start\nunit(u8) step = fn(view) { unsafe { &?*view; }; };\n%%end";
    let expired = "%%start
unit(u64) step = fn(count) {
    u8[count] bytes;
    *!u8 view = &!bytes[0];
    unsafe { core.invalidate(&bytes); &?*view; };
};
%%end";
    for layout in [ScalarTargetLayout::NATIVE64, ScalarTargetLayout::WASM32] {
        for (text, build_error, code, message, label) in [
            (
                outside,
                false,
                "B0012",
                "raw address requires an unsafe block",
                "&?*view",
            ),
            (wrong, false, "B0003", "cannot dereference value", "*view"),
            (
                expired,
                true,
                "B0003",
                "missing checked-reference origin",
                "view",
            ),
        ] {
            let source = SourceIdentity::new(
                "project".into(),
                "package".into(),
                "src/main.w".into(),
                "r1".into(),
            );
            let parsed = parse_source(source.clone(), text.into(), &[]);
            assert!(parsed.diagnostics.is_empty(), "{:?}", parsed.diagnostics);
            let standalone = derive_scalar_program_with_layout(&parsed.result, layout);
            let project = prepare_scalar_project(
                vec![(
                    derive_scalar_declarations_from_cst(parsed.result.canonical_cst()),
                    Vec::new(),
                )],
                vec![source.clone()],
                ScalarPreparationInputs {
                    target_layout: layout,
                },
            );
            let project_diagnostics = if build_error {
                let prepared = project.unwrap();
                let build = analyze_scalar_build(&prepared);
                assert!(emit_scalar_build_single_module_llvm(&build).is_err());
                assert!(emit_scalar_build_project_llvm(&build).is_err());
                build.diagnostics().to_vec()
            } else {
                project
                    .err()
                    .expect("preparation error")
                    .into_ordered_diagnostics()
            };
            assert_eq!(project_diagnostics, standalone.diagnostics);
            let start = text.rfind(label).unwrap() as u32;
            let end = start + label.len() as u32;
            if !build_error {
                assert_eq!(project_diagnostics.len(), 1, "{project_diagnostics:?}");
            }
            assert_eq!(
                project_diagnostics
                    .iter()
                    .filter(|diagnostic| {
                        diagnostic.code == code
                            && diagnostic.message == message
                            && diagnostic.labels[0].span.source == source
                            && diagnostic.labels[0].span.range == ByteSpan::new(start, end)
                    })
                    .count(),
                1,
                "{project_diagnostics:?}"
            );
        }
    }
}

#[test]
fn prepared_self_tail_stages_crossed_arguments_before_reusing_entry_slots() {
    let text = "%%start
i32 phase = 0;
i32(i32) first = fn(value) { phase = 1; value };
i32(i32) second = fn(value) { phase = 2; value };
unit(i32) observe = fn(value) { phase = value; };
i32(i32, i32, i32) swap = fn(left, right, count) {
    observe(count);
    if (count == 0) { left + right } else {
        swap(first(right), second(left), count - 1)
    }
};
u8(u8, bool) narrow = fn(value, done) {
    if (done) { value } else { unsafe { narrow(17, true) } }
};
%%end";
    for layout in [ScalarTargetLayout::NATIVE64, ScalarTargetLayout::WASM32] {
        let prepared = prepare_initializer_test(text, layout);
        let build = analyze_scalar_build(&prepared);
        assert!(build.diagnostics().is_empty(), "{:?}", build.diagnostics());
        let source = &prepared.project().modules[0].source;
        for (project, result) in [
            (false, emit_scalar_build_single_module_llvm(&build)),
            (true, emit_scalar_build_project_llvm(&build)),
        ] {
            let symbol = |name| self_tail_test_symbol(source, project, name);
            let ir = result.unwrap().to_text();
            let body = self_tail_test_body(&ir, "i32", &symbol("swap"));
            let (entry, repeated) = body
                .split_once("self_tail.header:")
                .unwrap_or_else(|| panic!("{body}"));
            assert_eq!(entry.matches("alloca i32").count(), 3, "{body}");
            assert!(!repeated.contains("alloca"), "{body}");
            assert_eq!(
                body.matches("br label %self_tail.header").count(),
                2,
                "{body}"
            );
            assert!(
                !repeated.contains(&format!("call i32 @{}(", symbol("swap"))),
                "{body}"
            );
            assert!(repeated.contains("ret i32"), "{body}");
            let expected_order = [
                "load i32, ptr %right".to_owned(),
                format!("call i32 @{}(", symbol("first")),
                "load i32, ptr %left".to_owned(),
                format!("call i32 @{}(", symbol("second")),
                "sub i32".to_owned(),
            ];
            let backedge = repeated.split_once("self_tail.else:").unwrap().1;
            assert!(
                repeated.contains(&format!("call void @{}(", symbol("observe"))),
                "{body}"
            );
            let (mut remaining, stores) = backedge.split_once("store i32").unwrap();
            for needle in expected_order {
                remaining = remaining
                    .split_once(&needle)
                    .unwrap_or_else(|| panic!("{needle}: {body}"))
                    .1;
            }
            assert!(stores.contains("br label %self_tail.header"), "{body}");
            assert_eq!(backedge.matches("store i32").count(), 3, "{body}");
            let narrow = self_tail_test_body(&ir, "i8", &symbol("narrow"));
            assert!(narrow.contains("store i8 17, ptr %value"), "{narrow}");
            assert_eq!(narrow.matches("br label %self_tail.header").count(), 2);
        }
        let legacy = derive_scalar_program_with_layout(
            &parse_source(source.clone(), text.into(), &[]).result,
            layout,
        );
        let ir = emit_scalar_llvm(&legacy).unwrap().to_text();
        let body = self_tail_test_body(&ir, "i32", "swap");
        assert!(body.contains("call i32 @swap("), "{body}");
        assert!(!body.contains("self_tail.header"), "{body}");
    }
}

fn self_tail_test_symbol(source: &SourceIdentity, project: bool, name: &str) -> String {
    if project {
        project_function_name(source, name)
    } else {
        name.to_owned()
    }
}

#[test]
fn prepared_self_tail_writes_checked_u64_width() {
    let text = "%%start
u64(u64, bool) wide = fn(value, done) {
    if (done) { value } else { wide(0 + 1, true) }
};
u64(u64, bool) overflow = fn(value, done) {
    if (done) { value } else { overflow((2147483647 + 2147483647) + 3, true) }
};
i32(i32, bool) ordinary = fn(value, done) {
    if (done) { value } else { ordinary(-value, true) }
};
u64 result = wide(4294967296, false);
if (result != 1) { core.system_panic(); };
%%end";
    let mut failures = Vec::new();
    for layout in [ScalarTargetLayout::NATIVE64, ScalarTargetLayout::WASM32] {
        let prepared = prepare_initializer_test(text, layout);
        let build = analyze_scalar_build(&prepared);
        assert!(build.diagnostics().is_empty(), "{:?}", build.diagnostics());
        let source = &prepared.project().modules[0].source;
        for (project, partition) in [
            (false, emit_scalar_build_single_module_llvm(&build)),
            (true, emit_scalar_build_project_llvm(&build)),
        ] {
            let ir = partition.unwrap().to_text();
            let symbol = self_tail_test_symbol(source, project, "wide");
            let body = self_tail_test_body(&ir, "i64", &symbol);
            if !body.contains("store i64 1, ptr %value")
                || body.contains("store i32")
                || body.matches("br label %self_tail.header").count() != 2
                || !ir.contains(&format!("call i64 @{symbol}(i64 4294967296, i1 false)"))
            {
                failures.push(format!("{layout:?} project={project}: {body}"));
            }
            let overflow = self_tail_test_symbol(source, project, "overflow");
            let body = self_tail_test_body(&ir, "i64", &overflow);
            if !body.contains("store i64 4294967297, ptr %value") {
                failures.push(format!("{layout:?} project={project}: {body}"));
            }
            let ordinary = self_tail_test_symbol(source, project, "ordinary");
            let body = self_tail_test_body(&ir, "i32", &ordinary);
            assert!(body.contains(&format!("call i32 @{ordinary}(")), "{body}");
            assert!(!body.contains("self_tail.header"), "{body}");
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

fn self_tail_test_body<'a>(ir: &'a str, ty: &str, symbol: &str) -> &'a str {
    ir.split_once(&format!("define {ty} @{symbol}("))
        .unwrap_or_else(|| panic!("missing definition {symbol}: {ir}"))
        .1
        .split_once("\n}")
        .unwrap()
        .0
}

#[test]
fn prepared_project_checked_forwarding_stages_the_complete_backedge() {
    let text = "%%start
i32(*u8) step = fn(view) { if (*view == 10) { 1 } else { 0 } };
i32(*u8, i32) drive = fn(view, outcome) {
    if (outcome == 0) { drive(view, step(view)) } else { outcome }
};
i32(*u8, i32) ordinary = fn(view, outcome) {
    if (outcome == 0) { step(view); ordinary(view, step(view)) } else { outcome }
};
%%end";
    for layout in [ScalarTargetLayout::NATIVE64, ScalarTargetLayout::WASM32] {
        let prepared = prepare_initializer_test(text, layout);
        let build = analyze_scalar_build(&prepared);
        assert!(build.diagnostics().is_empty(), "{:?}", build.diagnostics());
        let source = &prepared.project().modules[0].source;
        let symbol = |name| project_function_name(source, name);
        let ir = emit_scalar_build_project_llvm(&build).unwrap().to_text();
        let drive = self_tail_test_body(&ir, "i32", &symbol("drive"));
        let (entry, repeated) = drive
            .split_once("self_tail.header:")
            .unwrap_or_else(|| panic!("{drive}"));
        let width = layout.pointer_size * 8;
        assert!(
            drive.starts_with(&format!("i{width} %view, i32 %outcome)")),
            "{drive}"
        );
        assert!(
            entry.contains(&format!("%view1 = alloca i{width}")),
            "{drive}"
        );
        assert!(entry.contains("%outcome2 = alloca i32"), "{drive}");
        assert_eq!(entry.matches("alloca ").count(), 2, "{drive}");
        assert!(!repeated.contains("alloca "), "{drive}");
        assert!(
            !drive.contains(&format!("call i32 @{}(", symbol("drive"))),
            "{drive}"
        );
        assert_eq!(
            drive
                .matches(&format!("call i32 @{}(", symbol("step")))
                .count(),
            1,
            "{drive}"
        );
        let backedge = repeated.split_once("self_tail.then:").unwrap().1;
        let mut lines = backedge.lines().skip(1);
        let first = lines.next().unwrap().trim();
        let second = lines.next().unwrap().trim();
        let call = lines.next().unwrap().trim();
        let (carried, first_load) = first.split_once(" = ").unwrap();
        let (stepped, second_load) = second.split_once(" = ").unwrap();
        let (outcome, step_call) = call.split_once(" = ").unwrap();
        let load = format!("load i{width}, ptr %view1, align ");
        assert_ne!(carried, stepped);
        assert!(
            first_load.starts_with(&load) && second_load.starts_with(&load),
            "{drive}"
        );
        assert_eq!(
            step_call,
            format!("call i32 @{}(i{width} {stepped})", symbol("step"))
        );
        assert!(
            lines
                .next()
                .unwrap()
                .trim()
                .starts_with(&format!("store i{width} {carried}, ptr %view1, align ")),
            "{drive}"
        );
        assert!(
            lines
                .next()
                .unwrap()
                .trim()
                .starts_with(&format!("store i32 {outcome}, ptr %outcome2, align ")),
            "{drive}"
        );
        assert_eq!(lines.next().unwrap().trim(), "br label %self_tail.header");
        let ordinary = self_tail_test_body(&ir, "i32", &symbol("ordinary"));
        assert!(
            ordinary.contains(&format!("call i32 @{}(", symbol("ordinary"))),
            "{ordinary}"
        );
        assert!(!ordinary.contains("self_tail.header"), "{ordinary}");

        let single = emit_scalar_build_single_module_llvm(&build)
            .unwrap()
            .to_text();
        let drive = self_tail_test_body(&single, "i32", "drive");
        assert!(drive.contains("call i32 @drive("), "{drive}");
        assert!(!drive.contains("self_tail.header"), "{drive}");
        let legacy = validate_scalar_project(prepared.project().clone());
        let legacy = emit_scalar_project_llvm(&legacy).unwrap().to_text();
        let drive = self_tail_test_body(&legacy, "i32", &symbol("drive"));
        assert!(
            drive.contains(&format!("call i32 @{}(", symbol("drive"))),
            "{drive}"
        );
        assert!(!drive.contains("self_tail.header"), "{drive}");
    }
}

#[test]
fn checked_forwarding_reports_primitive_call_owner_gate_evidence() {
    let text = "%%start
i32(*u8) step = fn(view) { if (*view == 10) { 1 } else { 0 } };
i32(*u8, i32) drive = fn(view, outcome) {
    if (outcome == 0) { drive(view, step(view)) } else { outcome }
};
%%end";
    for layout in [ScalarTargetLayout::NATIVE64, ScalarTargetLayout::WASM32] {
        let prepared = prepare_initializer_test(text, layout);
        let build = analyze_scalar_build(&prepared);
        assert!(build.diagnostics().is_empty(), "{:?}", build.diagnostics());
        let ScalarItem::Function(function) = &prepared.project().modules[0].items[1] else {
            panic!("driver")
        };
        assert!(build.checked_self_tail_forwarding(0, 1).is_some());
        let nodes = build.bind_original_project_body(0, 1, function).unwrap();
        assert!(nodes.has_checked_self_tail());
        let cfg = build.reference_cfg(0, 1, None).unwrap();
        let owners = super::checked_call_owners(cfg).unwrap();
        let mut evidence = Vec::new();
        for point in &cfg.points {
            let CfgPointKind::Call { output, .. } = &point.kind else {
                continue;
            };
            let span = point.source_span.range;
            let owner = owners[&(span.start, span.end, *output)];
            assert!(matches!(owner, super::CheckedCallOwner::Borrowed));
            assert_eq!(
                point.possible_origins,
                Some([crate::scalar::ReferenceOriginId(0)].into())
            );
            evidence.push((point.id.0, span.start, span.end, *output));
        }
        assert_eq!(owners.len(), 2);
        assert_eq!(evidence.len(), 2);
        println!("{layout:?}: bound proof with two Borrowed call-owner entries {evidence:?}");
        let owner_gate = |cfg: &_, owners: &_| {
            super::self_tail::call_owners_allow_reuse(Some(&nodes), cfg, owners)
        };
        assert!(owner_gate(cfg, &owners));
        let key = *owners.keys().next().unwrap();
        let mut missing = owners.clone();
        missing.remove(&key);
        assert!(!owner_gate(cfg, &missing));
        let mut extra = owners.clone();
        extra.insert((0, 0, 0), super::CheckedCallOwner::Borrowed);
        assert!(!owner_gate(cfg, &extra));
        let mut inconsistent = owners.clone();
        inconsistent.insert(key, super::CheckedCallOwner::Fresh);
        assert!(!owner_gate(cfg, &inconsistent));
        inconsistent.insert(key, super::CheckedCallOwner::Mixed);
        assert!(!owner_gate(cfg, &inconsistent));
        let mut missing_mapping = cfg.clone();
        missing_mapping.primary_call_points.clear();
        assert!(!owner_gate(&missing_mapping, &owners));
        let ir = emit_scalar_build_project_llvm(&build).unwrap().to_text();
        let source = &prepared.project().modules[0].source;
        let symbol = project_function_name(source, "drive");
        let drive = self_tail_test_body(&ir, "i32", &symbol);
        assert!(drive.contains("self_tail.header"), "{drive}");
    }
}

#[test]
fn prepared_self_tail_preserves_ordinary_edges_and_resource_cleanup() {
    let text = "%%start
i32(i32) non_tail = fn(n) { if (n == 0) { n } else { non_tail(n - 1) + n } };
i32(i32) mutual_a = fn(n) { mutual_b(n) };
i32(i32) mutual_b = fn(n) { mutual_a(n) };
i32(i32) resource = fn(n) {
    if (n != 0) {
        u64 length = 2;
        u8[length] scratch;
        scratch[0] = 7;
        scratch[0];
    };
    if (n == 0) { 7 } else { resource(n - 1) }
};
i32(i32) local = fn(n) {
    i32 next = n - 1;
    if (n == 0) { next } else { local(next) }
};
u8(*!u8, i32) view = fn(cell, n) {
    if (n == 0) { *cell } else { view(cell, n - 1) }
};
%%end";
    for layout in [ScalarTargetLayout::NATIVE64, ScalarTargetLayout::WASM32] {
        let prepared = prepare_initializer_test(text, layout);
        let build = analyze_scalar_build(&prepared);
        assert!(build.diagnostics().is_empty(), "{:?}", build.diagnostics());
        let module = &prepared.project().modules[0];
        for (item, expected) in [(0, false), (3, true), (4, true), (5, false)] {
            let ScalarItem::Function(function) = &module.items[item] else {
                panic!("function {item}");
            };
            let nodes = prepared
                .bind_callable_body_call_nodes(0, item, None, &function.body)
                .unwrap();
            assert_eq!(nodes.has_self_tail_candidates(), expected);
        }
        assert!(!build
            .reference_cfg(0, 3, None)
            .unwrap()
            .allocations
            .is_empty());
        for (project, result) in [
            (false, emit_scalar_build_single_module_llvm(&build)),
            (true, emit_scalar_build_project_llvm(&build)),
        ] {
            let symbol = |name| self_tail_test_symbol(&module.source, project, name);
            let ir = result.unwrap().to_text();
            for (ty, function, callee) in [
                ("i32", "non_tail", "non_tail"),
                ("i32", "mutual_a", "mutual_b"),
                ("i32", "mutual_b", "mutual_a"),
                ("i32", "resource", "resource"),
                ("i32", "local", "local"),
                ("i8", "view", "view"),
            ] {
                let body = self_tail_test_body(&ir, ty, &symbol(function));
                assert!(!body.contains("self_tail.header"), "{body}");
                assert!(
                    body.contains(&format!("call {ty} @{}(", symbol(callee))),
                    "{body}"
                );
            }
            let body = self_tail_test_body(&ir, "i32", &symbol("resource"));
            let alloc = body.find("call ptr @__wosy_core_alloc(").unwrap();
            let recur = body
                .find(&format!("call i32 @{}(", symbol("resource")))
                .unwrap();
            let free = body
                .find("call void @__wosy_core_free(")
                .unwrap_or_else(|| panic!("missing scoped cleanup: {body}"));
            assert!(alloc < free && free < recur, "{body}");
        }
    }
}

fn emit_initializer_test_input(
    build: &crate::ScalarBuildValidation<'_>,
    project: &ScalarProject,
    region: &crate::scalar::control::ModuleInitializationRegion,
) -> [Result<LlvmPartition, String>; 2] {
    let module = &project.modules[0];
    let input = super::GenericInitializerInput::PreparedRows(build.prepared());
    [
        super::emit_single_module_llvm(
            &module.source,
            &module.items,
            region,
            &module.structs,
            &module.layouts,
            module.target_layout,
            input,
            &|reservations| build.aggregate(reservations),
            &|item, arm, _| build.reference_cfg(0, item, arm),
            super::PartitionMetadata::Full,
            &[],
        ),
        super::emit_project_llvm(
            project,
            &|_| Some(region),
            input,
            &|reservations| build.aggregate(reservations),
            &|module, item, arm, _| build.reference_cfg(module, item, arm),
            None,
            super::PartitionMetadata::Full,
            &[],
        ),
    ]
}

const BODY_AUTHORITY_SOURCE: &str = "%%start
disk = extern wasm \"disk_abi\" { i32(i32) read; };
u64(u64) length = fn(n) { n };
i32(i32) echo = fn(n) { n };
generic T;
T(T) identity = fn(n) { echo(77); n };
pick = overload { i32(i32) => fn(n) { disk.read(n) }; };
choose = overload { generic T; T(T) => fn(n) { echo(88); n }; };
i64(i32) main_body = fn(n) { i32 value = echo(identity<i32>(n)); value = echo(value); core.int_extend<i64>(pick(value)) };
bool(bool) demand_generic_arm = fn(n) { choose(n) };
(i32, i32)(i32) multi = fn(n) { i32 first, i32 second = echo(n) + 1, echo(99); echo(first), echo(second) };
arrays = overload { u64(u64) => fn(n) { u8[length(n)] bytes; bytes; length(n) }; };
i32 generic_seed = identity<i32>(1);
%%end";

#[test]
fn prepared_body_dispatch_uses_rows_when_call_spelling_differs() {
    use crate::ScalarExpression;

    fn corrupt_call(expression: &mut ScalarExpression) {
        if let ScalarExpression::Binary { left, .. } = expression {
            corrupt_call(left);
            return;
        }
        let ScalarExpression::Call {
            receiver,
            name,
            arguments,
            overload_selection,
            ..
        } = expression
        else {
            panic!("canonical call");
        };
        *receiver = Some("not_a_namespace".into());
        *name = "not_a_callable".into();
        *overload_selection = None;
        for argument in arguments {
            if matches!(argument, ScalarExpression::Call { .. }) {
                corrupt_call(argument);
            }
        }
    }

    for layout in [ScalarTargetLayout::NATIVE64, ScalarTargetLayout::WASM32] {
        let prepared = prepare_initializer_test(BODY_AUTHORITY_SOURCE, layout);
        let build = analyze_scalar_build(&prepared);
        assert!(build.diagnostics().is_empty(), "{:?}", build.diagnostics());
        let baseline = [
            emit_scalar_build_single_module_llvm(&build),
            emit_scalar_build_project_llvm(&build),
        ];
        let mut project = prepared.project().clone();
        for item in &mut project.modules[0].items {
            let ScalarItem::Function(function) = item else {
                continue;
            };
            for body in std::iter::once(&mut function.body)
                .chain(function.overload_arms.iter_mut().map(|arm| &mut arm.body))
            {
                for item in &mut body.items {
                    match item {
                        crate::ScalarBlockItem::Expression(
                            expression @ ScalarExpression::Call { .. },
                        ) => corrupt_call(expression),
                        crate::ScalarBlockItem::LocalBinding(binding) if binding.is_allocation => {
                            corrupt_call(binding.receivers[0].allocation_length.as_mut().unwrap());
                        }
                        crate::ScalarBlockItem::LocalBinding(binding) => {
                            corrupt_call(&mut binding.value);
                            for output in binding.output_values.iter_mut().skip(1) {
                                corrupt_call(&mut output.value);
                            }
                        }
                        crate::ScalarBlockItem::Assignment(assignment) => {
                            corrupt_call(&mut assignment.values[0]);
                        }
                        _ => {}
                    }
                }
            }
        }
        let region = prepared.initialization_region(0).unwrap();
        let changed = emit_initializer_test_input(&build, &project, region);
        for (expected, actual) in baseline.into_iter().zip(changed) {
            let expected = expected.unwrap().to_text();
            let actual = actual.unwrap().to_text();
            assert_eq!(expected, actual, "{layout:?}");
            assert!(actual.contains("sext i32"), "{actual}");
            assert!(actual.contains("(i32 77)"), "{actual}");
            assert!(actual.contains("(i32 88)"), "{actual}");
            assert!(actual.contains("(i32 99)"), "{actual}");
            assert!(actual.contains("call ptr @__wosy_core_alloc"), "{actual}");
        }
    }
}

#[test]
fn prepared_body_dispatch_rejects_missing_allocation_length_target() {
    for layout in [ScalarTargetLayout::NATIVE64, ScalarTargetLayout::WASM32] {
        let mut prepared = prepare_initializer_test(BODY_AUTHORITY_SOURCE, layout);
        let item = prepared.project().modules[0]
            .items
            .iter()
            .position(
                |item| matches!(item, ScalarItem::Function(function) if function.name == "arrays"),
            )
            .unwrap();
        let (span, ordinal) = prepared.remove_body_allocation_length_target(0, item, 0);
        let source = &prepared.project().modules[0].source;
        let build = analyze_scalar_build(&prepared);
        assert!(build.diagnostics().is_empty(), "{:?}", build.diagnostics());
        for result in [
            emit_scalar_build_single_module_llvm(&build),
            emit_scalar_build_project_llvm(&build),
        ] {
            assert_eq!(result.unwrap_err(), format!(
                "internal error: missing callable body call target in {source:?} at {}..{} (occurrence {ordinal})",
                span.start, span.end,
            ));
        }
    }
}

#[test]
fn prepared_initializer_dispatch_uses_rows_when_call_spelling_differs() {
    let text = "%%start
disk = extern wasm \"disk_abi\" { i32(i32) read; };
i32(i32) echo = fn(value) { value };
pick = overload { i32(i32) => fn(value) { value }; };
choose = overload { generic T; T(T) => fn(value) { value }; };
generic T;
T(T) identity = fn(value) { value };
i32 direct = echo(11);
i32 overloaded = pick(22);
bool generic_arm = choose(true);
i32 generic = identity<i32>(33);
i32 external = disk.read(44);
i64 extended = core.int_extend<i64>(echo(55));
%%end";
    for layout in [ScalarTargetLayout::NATIVE64, ScalarTargetLayout::WASM32] {
        let prepared = prepare_initializer_test(text, layout);
        let build = analyze_scalar_build(&prepared);
        let baseline = [
            emit_scalar_build_single_module_llvm(&build),
            emit_scalar_build_project_llvm(&build),
        ];
        let mut project = prepared.project().clone();
        for item in &mut project.modules[0].items {
            if let ScalarItem::Binding(binding) = item {
                let crate::ScalarExpression::Call {
                    receiver,
                    name,
                    overload_selection,
                    ..
                } = &mut binding.value
                else {
                    panic!("call initializer");
                };
                *receiver = Some("not_a_namespace".into());
                *name = "not_a_callable".into();
                *overload_selection = None;
            }
        }
        let region = prepared.initialization_region(0).unwrap();
        let changed = emit_initializer_test_input(&build, &project, region);
        for (expected, actual) in baseline.into_iter().zip(changed) {
            let expected = expected.unwrap().to_text();
            let actual = actual.unwrap().to_text();
            assert_eq!(expected, actual, "{layout:?}");
            for argument in [11, 22, 33, 44, 55] {
                assert!(actual.contains(&format!("(i32 {argument})")), "{actual}");
            }
            assert!(actual.contains("sext i32"), "{actual}");
            assert!(actual.contains("(i1 true)"), "{actual}");
        }
    }
}

#[test]
fn prepared_initializer_preserves_unsupported_indirect_call_diagnostics() {
    let text = "%%start\ni32() direct = fn { 11 };\ni32() indirect = direct;\ni32 result = indirect();\n%%end";
    for layout in [ScalarTargetLayout::NATIVE64, ScalarTargetLayout::WASM32] {
        let prepared = prepare_initializer_test(text, layout);
        let build = analyze_scalar_build(&prepared);
        assert!(build.diagnostics().is_empty(), "{:?}", build.diagnostics());
        let mut project = prepared.project().clone();
        let module = &mut project.modules[0];
        let ScalarItem::Binding(binding) = &mut module.items[1] else {
            panic!("indirect binding");
        };
        binding.declared_type = ScalarType::I32;
        binding.receivers[0].ty = ScalarType::I32;
        binding.value = crate::ScalarExpression::Integer {
            value: 0.into(),
            span: binding.span,
        };
        let [single, project] = emit_initializer_test_input(
            &build,
            &project,
            prepared.initialization_region(0).unwrap(),
        );
        assert_eq!(single.err().unwrap(), "unknown LLVM callable indirect",);
        let source = &prepared.project().modules[0].source;
        assert_eq!(
            project.err().unwrap(),
            format!(
                "unknown LLVM callable {}",
                project_function_name(source, "indirect")
            ),
        );
    }
}

#[test]
fn prepared_initializer_missing_row_is_rejected_by_both_emitters() {
    let text = "%%start\ni32(i32) echo = fn(value) { value };\ni32 result = echo(11);\n%%end";
    for layout in [ScalarTargetLayout::NATIVE64, ScalarTargetLayout::WASM32] {
        let prepared = prepare_initializer_test(text, layout);
        let build = analyze_scalar_build(&prepared);
        let mut region = prepared.initialization_region(0).unwrap().clone();
        for result in emit_initializer_test_input(&build, prepared.project(), &region) {
            assert!(result.is_ok());
        }
        region.remove_only_call_target();
        let start = text.find("echo(11)").unwrap();
        let expected = format!(
            "internal error: missing initializer call target in {:?} at {start}..{} (occurrence 0)",
            region.source,
            start + "echo(11)".len(),
        );
        for result in emit_initializer_test_input(&build, prepared.project(), &region) {
            assert_eq!(
                result
                    .err()
                    .expect("missing selected row must fail emission"),
                expected,
                "{layout:?}"
            );
        }
    }
}

#[test]
fn prepared_initializer_member_authority_on_both_emitters_and_layouts() {
    use crate::ScalarExpression as Expr;
    let text = "%%start
struct Record { u8 small; i32 signed_value; f32 floating; }
struct Boxed { u8 payload; }
enum Mode { waiting; ready; }
Record data = { .small = 200; .signed_value = -7; .floating = 1.5; };
Mode status = Mode.ready;
Boxed wrapped = { .payload = unsafe { data.small }; };
bool comparison = data.small < 201;
u16 widened = core.int_extend<u16>(data.small);
u32 bits = core.bitcast<u32>(data.signed_value);
f64 extended = core.float_extend<f64>(data.floating);
%%end";
    fn mutate(expr: &mut Expr) {
        match expr {
            Expr::Member {
                receiver,
                name,
                enum_tag,
                ..
            } => {
                *receiver = "decoy_receiver".into();
                *name = "decoy_member".into();
                *enum_tag = Some(99);
            }
            Expr::StructLiteral { fields, .. } => {
                for field in fields {
                    mutate(&mut field.value);
                }
            }
            Expr::Block(block) => {
                for item in &mut block.items {
                    if let crate::ScalarBlockItem::Expression(expr) = item {
                        mutate(expr);
                    }
                }
            }
            Expr::Binary { left, right, .. } => {
                mutate(left);
                mutate(right);
            }
            Expr::Call { arguments, .. } => {
                for argument in arguments {
                    mutate(argument);
                }
            }
            _ => {}
        }
    }
    for layout in [ScalarTargetLayout::NATIVE64, ScalarTargetLayout::WASM32] {
        let prepared = prepare_initializer_test(text, layout);
        let build = analyze_scalar_build(&prepared);
        let region = prepared.initialization_region(0).unwrap();
        let baseline = emit_initializer_test_input(&build, prepared.project(), region);
        let mut mutated = prepared.project().clone();
        for item in &mut mutated.modules[0].items {
            if let ScalarItem::Binding(binding) = item {
                mutate(&mut binding.value);
            }
        }
        for (expected, changed) in baseline
            .into_iter()
            .zip(emit_initializer_test_input(&build, &mutated, region))
        {
            let expected = expected.unwrap().to_text();
            assert_eq!(changed.unwrap().to_text(), expected);
            for fragment in ["icmp ult i8", "zext i8", "fpext float", "store i32 1"] {
                assert!(expected.contains(fragment), "{fragment}: {expected}");
            }
        }
        for (remove_type, fact) in [(false, "identity"), (true, "type")] {
            let mut corrupt = region.clone();
            let (span, ordinal) = corrupt.remove_first_member_fact(remove_type);
            let start = (text.find("Mode.ready").unwrap() + "Mode.".len()) as u32;
            assert_eq!(span, ByteSpan::new(start, start + 5));
            let expected = format!("internal error: missing initializer member {fact} in {:?} at {start}..{} (occurrence {ordinal})", region.source, start + 5);
            for result in emit_initializer_test_input(&build, &mutated, &corrupt) {
                assert_eq!(result.unwrap_err(), expected);
            }
        }
    }
}

#[test]
fn prepared_initializer_namespace_member_uses_selected_storage_on_both_layouts() {
    let root_text = "%%start\nmath = namespace package \"math.w\";\ni32 result = math.value;\nstruct Boxed { i32 field; }\nBoxed nested = { .field = math.value; };\n%%end";
    let source = |path: &str| {
        SourceIdentity::new("project".into(), "package".into(), path.into(), "r1".into())
    };
    let main = source("main.w");
    let math = source("math.w");
    let declarations = |source, text: &str| {
        let parsed = parse_source(source, text.into(), &[]);
        assert!(parsed.diagnostics.is_empty(), "{:?}", parsed.diagnostics);
        derive_scalar_declarations_from_cst(parsed.result.canonical_cst())
    };
    for target_layout in [ScalarTargetLayout::NATIVE64, ScalarTargetLayout::WASM32] {
        let root = declarations(main.clone(), root_text);
        let namespace = root.namespaces().next().unwrap();
        let binding = ScalarNamespaceBinding {
            binding: namespace.binding.clone(),
            span: namespace.span,
            target: math.clone(),
        };
        let prepared = prepare_scalar_project(
            vec![
                (root, vec![binding]),
                (
                    declarations(math.clone(), "%%start\ni32 value = 7;\n%%end"),
                    Vec::new(),
                ),
            ],
            vec![main.clone(), math.clone()],
            ScalarPreparationInputs { target_layout },
        )
        .unwrap();
        let build = analyze_scalar_build(&prepared);
        let baseline = emit_scalar_build_project_llvm(&build).unwrap().to_text();
        assert!(
            baseline.contains(&format!(
                "load i32, ptr @{}",
                project_global_name(&math, "value")
            )),
            "{baseline}"
        );
        let mut mutated = prepared.project().clone();
        for item in &mut mutated.modules[0].items[1..] {
            let ScalarItem::Binding(binding) = item else {
                panic!("initializer binding")
            };
            let expression = match &mut binding.value {
                crate::ScalarExpression::StructLiteral { fields, .. } => &mut fields[0].value,
                expression => expression,
            };
            let crate::ScalarExpression::Member {
                receiver,
                name,
                enum_tag,
                ..
            } = expression
            else {
                panic!("namespace member")
            };
            *receiver = "decoy_receiver".into();
            *name = "decoy_member".into();
            *enum_tag = Some(99);
        }
        let input = super::GenericInitializerInput::PreparedRows(&prepared);
        let emit = |region: &crate::scalar::control::ModuleInitializationRegion| {
            super::emit_project_llvm(
                &mutated,
                &|module| {
                    if module == 0 {
                        Some(region)
                    } else {
                        prepared.initialization_region(module)
                    }
                },
                input,
                &|reservations| build.aggregate(reservations),
                &|module, item, arm, _| build.reference_cfg(module, item, arm),
                None,
                super::PartitionMetadata::Full,
                &[],
            )
        };
        let region = prepared.initialization_region(0).unwrap();
        assert_eq!(emit(region).unwrap().to_text(), baseline);
        let mut corrupt = region.clone();
        let (span, ordinal) = corrupt.remove_first_member_fact(false);
        let start = (root_text.find("math.value").unwrap() + "math.".len()) as u32;
        assert_eq!(span, ByteSpan::new(start, start + 5));
        assert_eq!(emit(&corrupt).unwrap_err(), format!("internal error: missing initializer member identity in {main:?} at {start}..{} (occurrence {ordinal})", start + 5));
    }
}

#[test]
fn prepared_initializer_emits_canonical_binding_and_block_outputs() {
    use crate::ScalarExpression;

    let source = SourceIdentity::new(
        "project".into(),
        "package".into(),
        "src/main.w".into(),
        "r1".into(),
    );
    let parsed = parse_source(
        source.clone(),
        "%%start
i32(i32) echo = fn(value) { value };
i32 first, i32 second = echo(11) + 1, echo(22);
i32 nested = unsafe { echo(33) };
%%end"
            .into(),
        &[],
    );
    assert!(parsed.diagnostics.is_empty(), "{:?}", parsed.diagnostics);
    for target_layout in [ScalarTargetLayout::NATIVE64, ScalarTargetLayout::WASM32] {
        let prepared = prepare_scalar_project(
            vec![(
                derive_scalar_declarations_from_cst(parsed.result.canonical_cst()),
                Vec::new(),
            )],
            vec![source.clone()],
            ScalarPreparationInputs { target_layout },
        )
        .unwrap_or_else(|error| panic!("{target_layout:?}: {:?}", error.diagnostics));
        let build = analyze_scalar_build(&prepared);
        assert!(build.diagnostics().is_empty(), "{:?}", build.diagnostics());
        let original = [
            emit_scalar_build_single_module_llvm(&build)
                .unwrap()
                .to_text(),
            emit_scalar_build_project_llvm(&build).unwrap().to_text(),
        ];

        let mut project = prepared.project().clone();
        let module = &mut project.modules[0];
        let [_, ScalarItem::Binding(binding), ScalarItem::Binding(nested)] =
            module.items.as_mut_slice()
        else {
            panic!("expected function and two bindings");
        };
        let ScalarExpression::Block(block) = &mut nested.value else {
            panic!("expected initializer block");
        };
        for mirror in [
            &mut binding.output_values[0],
            &mut block.final_output_values[0],
        ] {
            mirror.value = ScalarExpression::Integer {
                value: 99.into(),
                span: mirror.span,
            };
        }

        let region = prepared.initialization_region(0).unwrap();
        let input = super::GenericInitializerInput::PreparedRows(build.prepared());
        let emitted = [
            super::emit_single_module_llvm(
                &source,
                &module.items,
                region,
                &module.structs,
                &module.layouts,
                target_layout,
                input,
                &|reservations| build.aggregate(reservations),
                &|item, arm, _| build.reference_cfg(0, item, arm),
                super::PartitionMetadata::Full,
                &[],
            )
            .unwrap()
            .to_text(),
            super::emit_project_llvm(
                &project,
                &|_| Some(region),
                input,
                &|reservations| build.aggregate(reservations),
                &|module, item, arm, _| build.reference_cfg(module, item, arm),
                None,
                super::PartitionMetadata::Full,
                &[],
            )
            .unwrap()
            .to_text(),
        ];
        for (route, (original, emitted)) in original.iter().zip(&emitted).enumerate() {
            for argument in [11, 22, 33] {
                assert!(
                    original.contains(&format!("(i32 {argument})")),
                    "{original}"
                );
            }
            assert_eq!(emitted, original, "{target_layout:?}, route {route}");
        }
    }
}

#[test]
fn prepared_initializer_emits_generic_runtime_array_length_on_both_targets() {
    let source = SourceIdentity::new(
        "project".into(),
        "package".into(),
        "src/main.w".into(),
        "r1".into(),
    );
    let text = "%%start\ngeneric T;\nT(T) echo = fn(value) { value };\n\
                u8 initial = echo<u8>(1);\n\
                u8[echo<u64>(2)] module_bytes;\n%%end";
    let parsed = parse_source(source.clone(), text.into(), &[]);
    assert!(parsed.diagnostics.is_empty(), "{:?}", parsed.diagnostics);
    let mut failures = Vec::new();
    for target_layout in [ScalarTargetLayout::NATIVE64, ScalarTargetLayout::WASM32] {
        let prepared = prepare_scalar_project(
            vec![(
                derive_scalar_declarations_from_cst(parsed.result.canonical_cst()),
                Vec::new(),
            )],
            vec![source.clone()],
            ScalarPreparationInputs { target_layout },
        )
        .unwrap_or_else(|error| panic!("{target_layout:?}: {:?}", error.diagnostics));
        let build = analyze_scalar_build(&prepared);
        assert!(build.diagnostics().is_empty(), "{:?}", build.diagnostics());
        for (route, result) in [
            ("single", emit_scalar_build_single_module_llvm(&build)),
            ("project", emit_scalar_build_project_llvm(&build)),
        ] {
            match result {
                Ok(llvm) => {
                    let ir = llvm.to_text();
                    assert!(ir.contains("call i64 @"), "{target_layout:?} {route}: {ir}");
                    assert!(ir.contains("(i64 2)"), "{target_layout:?} {route}: {ir}");
                }
                Err(error) => failures.push(format!("{target_layout:?} {route}: {error}")),
            }
        }
    }
    assert!(failures.is_empty(), "{failures:#?}");
}

#[test]
fn prepared_project_initializer_shares_specialization_across_aliases() {
    let source = |path: &str| {
        SourceIdentity::new("project".into(), "package".into(), path.into(), "r1".into())
    };
    let root = source("src/main.w");
    let library = source("src/library.w");
    let main = parse_source(
        root.clone(),
        "%%start
left = namespace package \"library.w\";
right = namespace package \"library.w\";
u64 first = left.echo<u64>(1);
u8[right.echo<u64>(2)] bytes;
%%end"
            .into(),
        &[],
    );
    let implementation = parse_source(
        library.clone(),
        "%%start\ngeneric T;\nT(T) echo = fn(value) { value };\n%%end".into(),
        &[],
    );
    assert!(main.diagnostics.is_empty(), "{:?}", main.diagnostics);
    assert!(
        implementation.diagnostics.is_empty(),
        "{:?}",
        implementation.diagnostics,
    );
    for target_layout in [ScalarTargetLayout::NATIVE64, ScalarTargetLayout::WASM32] {
        let declarations = derive_scalar_declarations_from_cst(main.result.canonical_cst());
        let namespaces = declarations
            .namespaces()
            .map(|namespace| ScalarNamespaceBinding {
                binding: namespace.binding.clone(),
                target: library.clone(),
                span: namespace.binding_span,
            })
            .collect();
        let prepared = prepare_scalar_project(
            vec![
                (declarations, namespaces),
                (
                    derive_scalar_declarations_from_cst(implementation.result.canonical_cst()),
                    Vec::new(),
                ),
            ],
            vec![root.clone(), library.clone()],
            ScalarPreparationInputs { target_layout },
        )
        .unwrap_or_else(|error| panic!("{target_layout:?}: {:?}", error.diagnostics));
        let build = analyze_scalar_build(&prepared);
        let ir = emit_scalar_build_project_llvm(&build)
            .expect("qualified generic initializer emission")
            .to_text();
        let symbol = super::generic_specialization_name(
            &project_function_name(&library, "echo"),
            &[ScalarType::U64],
        );
        assert_eq!(
            ir.matches(&format!("define i64 @{symbol}(")).count(),
            1,
            "{ir}"
        );
        for argument in [1, 2] {
            assert!(
                ir.contains(&format!("call i64 @{symbol}(i64 {argument})")),
                "{ir}"
            );
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
            assert!(fixed_aggregate_move_count(&llvm, "%deref", 8) != 0, "{llvm}");
            assert!(fixed_aggregate_move_count(&llvm, "%deref", 2) != 0, "{llvm}");
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
            assert!(fixed_aggregate_move_count(&llvm, "%deref", 8) != 0, "{llvm}");
            assert!(fixed_aggregate_move_count(&llvm, "%deref", 2) != 0, "{llvm}");
            assert!(
                fixed_aggregate_move_count(&llvm, "%return_result_source", 2) != 0,
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
            assert_eq!(fixed_aggregate_move_count(&llvm, "%return_result_source", 1), 3, "{llvm}");
            assert_eq!(fixed_aggregate_move_count(&llvm, "%return_result_source", 2), 2, "{llvm}");
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
                fixed_aggregate_move_count(&llvm, "%return_result_source", 1),
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
    let text = "%%start\nstruct copy Point { u8 value; }\n*Point(bool) allocate = fn(flag) { Point local = { .value = 22; }; if (flag) { &local } else { null } };\n*Point(bool, bool) choose = fn(local, external) { *Point result = null; Point owned = { .value = 11; }; if (local) { result = &owned; }; if (external) { result = allocate(true); }; result };\n*Point(*Point, bool, bool) choose_borrow = fn(input, local, borrowed) { *Point result = null; Point owned = { .value = 44; }; if (local) { result = &owned; }; if (borrowed) { result = input; }; result };\nunit() caller = fn { *Point first = choose(true, false); Point one = *first; *Point second = choose(false, true); Point two = *second; *Point absent = choose(false, false); Point outside = { .value = 33; }; *Point local = choose_borrow(&outside, true, false); Point three = *local; *Point borrowed = choose_borrow(&outside, false, true); Point four = *borrowed; *Point replaced = choose_borrow(&outside, true, true); Point five = *replaced; *Point empty = choose_borrow(&outside, false, false); if (one.value != 11 || two.value != 22 || absent != null || three.value != 44 || four.value != 33 || five.value != 33 || empty != null || outside.value != 33) { core.system_panic(); }; };\ncaller();\n%%end";
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
fn joined_assignment_previous_values_are_captured_before_group_writes_on_both_targets() {
    let source = SourceIdentity::new(
        "project".into(),
        "package".into(),
        "src/main.w".into(),
        "r1".into(),
    );
    let text = "%%start\nstruct copy Point { u8 value; }\n*Point(u8) create = fn(value) { Point backing = { .value = value; }; &backing };\n(*Point, *Point)(*Point, bool, bool) choose = fn(input, local, borrowed) { *Point left = null; *Point right = null; u8 marker = 0; *!u8 marker_writer = &!marker; if (local) { left, right, *marker_writer = create(11), create(22), 3; }; if (borrowed) { left, right, *marker_writer = input, input, 4; }; marker_writer; left, right };\n%%end";
    for (layout, width) in [
        (ScalarTargetLayout::WASM32, "i32"),
        (ScalarTargetLayout::NATIVE64, "i64"),
    ] {
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
        for llvm in [
            emit_scalar_llvm(&single).unwrap().to_text(),
            emit_scalar_project_llvm(&project).unwrap().to_text(),
        ] {
            let lines = llvm.lines().collect::<Vec<_>>();
            let owners = lines
                .iter()
                .enumerate()
                .filter(|(_, line)| {
                    line.contains("joined_previous_owner") && line.contains(" = load i1,")
                })
                .collect::<Vec<_>>();
            let addresses = lines
                .iter()
                .enumerate()
                .filter(|(_, line)| {
                    line.contains("joined_previous_address")
                        && line.contains(&format!(" = load {width},"))
                })
                .collect::<Vec<_>>();
            assert_eq!(owners.len(), 4, "{llvm}");
            assert_eq!(addresses.len(), 4, "{llvm}");
            let selectors = lines
                .iter()
                .enumerate()
                .filter(|(_, line)| {
                    if !line.contains(&format!(" = load {width}, ptr %marker_writer,")) {
                        return false;
                    }
                    let value = line.split_whitespace().next().unwrap();
                    lines.iter().any(|candidate| {
                        candidate.contains(&format!("inttoptr {width} {value} to ptr"))
                    })
                })
                .collect::<Vec<_>>();
            assert_eq!(selectors.len(), 2, "{llvm}");
            for group in 0..2 {
                let first_capture = owners[2 * group].0;
                let last_capture = addresses[2 * group + 1].0;
                let rhs_loads = lines
                    .iter()
                    .enumerate()
                    .filter(|(index, line)| {
                        *index < first_capture
                            && line.contains(&format!(" = load {width}, ptr %assignment_value"))
                    })
                    .collect::<Vec<_>>();
                assert!(rhs_loads.len() >= 2, "{llvm}");
                assert!(rhs_loads[rhs_loads.len() - 1].0 < first_capture, "{llvm}");
                let (selector_index, selector_line) = selectors[group];
                let selector_value = selector_line.split_whitespace().next().unwrap();
                let destination = lines
                    .iter()
                    .enumerate()
                    .find(|(_, line)| {
                        line.contains(&format!("inttoptr {width} {selector_value} to ptr"))
                    })
                    .expect("retained projected destination");
                assert!(rhs_loads[rhs_loads.len() - 1].0 < selector_index, "{llvm}");
                assert!(
                    selector_index < destination.0 && destination.0 < first_capture,
                    "{llvm}"
                );
                assert!(owners[2 * group].0 < addresses[2 * group].0, "{llvm}");
                assert!(addresses[2 * group].0 < owners[2 * group + 1].0, "{llvm}");
                assert!(owners[2 * group + 1].0 < last_capture, "{llvm}");
                let mut prior_write = last_capture;
                for (position, name) in ["left", "right"].into_iter().enumerate() {
                    let (owner_index, owner_line) = owners[2 * group + position];
                    let (address_index, address_line) = addresses[2 * group + position];
                    assert!(
                        owner_line.contains(&format!(", ptr %{name}_return_owner,")),
                        "{llvm}"
                    );
                    assert!(address_line.contains(&format!(", ptr %{name},")), "{llvm}");
                    let owner = owner_line.split_whitespace().next().unwrap();
                    let address = address_line.split_whitespace().next().unwrap();
                    let different = lines
                        .iter()
                        .enumerate()
                        .find(|(_, line)| {
                            line.contains("joined_owner_replaced")
                                && line.contains(&format!("icmp ne {width} {address},"))
                        })
                        .expect("replacement predicate");
                    let live = lines
                        .iter()
                        .enumerate()
                        .find(|(_, line)| {
                            line.contains("joined_previous_live")
                                && line.contains(&format!("icmp ne {width} {address}, 0"))
                        })
                        .expect("previous live predicate");
                    assert!(last_capture < different.0, "{llvm}");
                    assert!(prior_write < different.0, "{llvm}");
                    assert!(
                        address_index < live.0 && owner_index < different.0,
                        "{llvm}"
                    );
                    let different_value = different.1.split_whitespace().next().unwrap();
                    let live_value = live.1.split_whitespace().next().unwrap();
                    let owned = lines
                        .iter()
                        .enumerate()
                        .find(|(_, line)| {
                            line.contains(&format!("and i1 {owner}, {different_value}"))
                        })
                        .expect("previous-owned predicate");
                    let owned_value = owned.1.split_whitespace().next().unwrap();
                    let release = lines
                        .iter()
                        .enumerate()
                        .find(|(_, line)| {
                            line.contains(&format!("and i1 {owned_value}, {live_value}"))
                        })
                        .expect("release predicate");
                    let release_value = release.1.split_whitespace().next().unwrap();
                    let branch = lines
                        .iter()
                        .enumerate()
                        .find(|(_, line)| line.contains(&format!("br i1 {release_value},")))
                        .expect("conditional release");
                    assert!(last_capture < branch.0, "{llvm}");
                    let pointer = lines
                        .iter()
                        .enumerate()
                        .find(|(_, line)| {
                            line.contains("joined_previous_pointer")
                                && line.contains(&format!("inttoptr {width} {address} to ptr"))
                        })
                        .expect("released previous address");
                    let pointer_value = pointer.1.split_whitespace().next().unwrap();
                    let free = lines
                        .iter()
                        .enumerate()
                        .find(|(_, line)| {
                            line.contains(&format!(
                                "call void @__wosy_core_free(ptr {pointer_value})"
                            ))
                        })
                        .expect("previous owner free");
                    assert!(branch.0 < pointer.0 && pointer.0 < free.0, "{llvm}");
                    let owner_write = lines
                        .iter()
                        .enumerate()
                        .find(|(index, line)| {
                            *index > branch.0
                                && line.contains("store i1 ")
                                && line.contains(&format!(", ptr %{name}_return_owner,"))
                        })
                        .expect("ordered owner install");
                    let value_write = lines
                        .iter()
                        .enumerate()
                        .find(|(index, line)| {
                            *index > owner_write.0
                                && line.contains(&format!("store {width} "))
                                && line.contains(&format!(", ptr %{name},"))
                        })
                        .expect("ordered checked-address install");
                    assert!(
                        last_capture < owner_write.0 && owner_write.0 < value_write.0,
                        "{llvm}"
                    );
                    prior_write = value_write.0;
                }
            }
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
            assert!(fixed_aggregate_move_count(&llvm, "%deref", 1) != 0, "{llvm}");
            assert!(fixed_aggregate_move_count(&llvm, "%deref", 2) != 0, "{llvm}");
            assert!(
                !llvm.contains("call void @__wosy_core_free(ptr %deref"),
                "{llvm}"
            );
        }
    }
}

#[test]
fn joined_group_transfer_does_not_release_the_address_transferred_to_left() {
    fn release_on_owned_distinct_live_path(
        value: &str,
        definitions: &BTreeMap<String, String>,
        previous_owner: &str,
        different: &str,
        live: &str,
        transferred_owner: &str,
    ) -> bool {
        if value == previous_owner
            || value == different
            || value == live
            || value == transferred_owner
            || value == "true"
        {
            return true;
        }
        if value == "false" {
            return false;
        }
        if let Some(operands) = definitions[value].strip_prefix("xor i1 ") {
            let (left, right) = operands.split_once(", ").unwrap();
            return release_on_owned_distinct_live_path(
                left,
                definitions,
                previous_owner,
                different,
                live,
                transferred_owner,
            ) ^ release_on_owned_distinct_live_path(
                right,
                definitions,
                previous_owner,
                different,
                live,
                transferred_owner,
            );
        }
        let operands = definitions[value]
            .strip_prefix("and i1 ")
            .expect("release conjunction");
        let (left, right) = operands.split_once(", ").unwrap();
        release_on_owned_distinct_live_path(
            left,
            definitions,
            previous_owner,
            different,
            live,
            transferred_owner,
        ) && release_on_owned_distinct_live_path(
            right,
            definitions,
            previous_owner,
            different,
            live,
            transferred_owner,
        )
    }

    let source = SourceIdentity::new(
        "project".into(),
        "package".into(),
        "src/main.w".into(),
        "r1".into(),
    );
    let text = "%%start\nstruct copy Point { u8 value; }\n*Point(u8) create = fn(value) { Point backing = { .value = value; }; &backing };\n(*Point, *Point)(*Point, bool, bool, bool) choose = fn(input, local, borrowed, transfer) { *Point left = null; *Point right = null; if (local) { left, right = create(11), create(22); }; if (borrowed) { left, right = input, input; }; if (transfer) { left, right = right, input; }; left, right };\nunit() caller = fn { Point outside = { .value = 33; }; *Point first, *Point second = choose(&outside, true, false, true); Point one = *first; Point two = *second; one; two; };\ncaller();\n%%end";
    let mut unsafe_routes = Vec::new();
    for (layout, width) in [
        (ScalarTargetLayout::WASM32, "i32"),
        (ScalarTargetLayout::NATIVE64, "i64"),
    ] {
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
        for (route, llvm) in [
            (
                "standalone",
                emit_scalar_llvm(&single)
                    .expect("accepted standalone emission")
                    .to_text(),
            ),
            (
                "project",
                emit_scalar_project_llvm(&project)
                    .expect("accepted project emission")
                    .to_text(),
            ),
        ] {
            let lines = llvm
                .lines()
                .skip_while(|line| !line.starts_with(&format!("define {{ {width}, {width} }} @")))
                .take_while(|line| *line != "}")
                .collect::<Vec<_>>();
            assert!(!lines.is_empty(), "{llvm}");
            let definitions = lines
                .iter()
                .filter_map(|line| {
                    line.split_once(" = ").map(|(name, expression)| {
                        (
                            name.split_whitespace().next().unwrap().to_owned(),
                            expression.to_owned(),
                        )
                    })
                })
                .collect::<BTreeMap<_, _>>();
            let (owner_index, owner_line) = lines
                .iter()
                .enumerate()
                .filter(|(_, line)| {
                    line.contains("joined_previous_owner")
                        && line.contains(" = load i1, ptr %right_return_owner,")
                })
                .last()
                .expect("captured previous right owner");
            let (address_index, address_line) = lines
                .iter()
                .enumerate()
                .filter(|(_, line)| {
                    line.contains("joined_previous_address")
                        && line.contains(&format!(" = load {width}, ptr %right,"))
                })
                .last()
                .expect("captured previous right address");
            let owner = owner_line.split_whitespace().next().unwrap();
            let address = address_line.split_whitespace().next().unwrap();
            let (left_write, left_store) = lines
                .iter()
                .enumerate()
                .filter(|(_, line)| {
                    line.contains(&format!("store {width} ")) && line.contains(", ptr %left,")
                })
                .last()
                .expect("transferred right payload installed in left");
            let replacement_value = left_store
                .split_once(&format!("store {width} "))
                .unwrap()
                .1
                .split(',')
                .next()
                .unwrap();
            let replacement_slot = definitions[replacement_value]
                .strip_prefix(&format!("load {width}, ptr "))
                .unwrap()
                .split(',')
                .next()
                .unwrap();
            let (rhs_store, rhs_line) = lines
                .iter()
                .enumerate()
                .filter(|(index, line)| {
                    *index < owner_index
                        && line.contains(&format!("store {width} "))
                        && line.contains(&format!(", ptr {replacement_slot},"))
                })
                .last()
                .expect("staged right payload");
            let right_value = rhs_line
                .split_once(&format!("store {width} "))
                .unwrap()
                .1
                .split(',')
                .next()
                .unwrap();
            assert!(
                definitions[right_value].starts_with(&format!("load {width}, ptr %right,")),
                "{llvm}"
            );
            assert!(
                rhs_store < owner_index && owner_index < address_index,
                "{llvm}"
            );
            assert!(
                lines[rhs_store + 1..address_index]
                    .iter()
                    .all(|line| !(line.contains("store ") && line.contains(", ptr %right,"))),
                "right address changed before capture: {llvm}"
            );
            let (source_owner_read, source_owner_line) = lines
                .iter()
                .enumerate()
                .filter(|(_, line)| {
                    line.contains("joined_assignment_owned")
                        && line.contains(" = load i1, ptr %right_return_owner,")
                })
                .last()
                .expect("joined RHS owner transfer");
            let source_owner = source_owner_line.split_whitespace().next().unwrap();
            let left_owner_write = lines
                .iter()
                .position(|line| {
                    line.contains(&format!("store i1 {source_owner}, ptr %left_return_owner,"))
                })
                .expect("left receives right ownership");
            let source_clear = lines
                .iter()
                .enumerate()
                .filter(|(index, line)| {
                    *index < left_write && line.contains("store i1 false, ptr %right_return_owner,")
                })
                .last()
                .expect("source owner clear")
                .0;
            assert!(
                source_owner_read < rhs_store && source_owner_read < owner_index,
                "{llvm}"
            );
            assert!(
                address_index < source_clear
                    && source_clear < left_owner_write
                    && left_owner_write < left_write,
                "{llvm}"
            );
            assert!(
                lines[source_owner_read + 1..owner_index]
                    .iter()
                    .all(|line| !(line.contains("store ")
                        && line.contains(", ptr %right_return_owner,"))),
                "right owner changed before capture: {llvm}"
            );
            let different = lines
                .iter()
                .find(|line| {
                    line.contains("joined_owner_replaced")
                        && line.contains(&format!("icmp ne {width} {address},"))
                })
                .expect("old right versus distinct input comparison");
            let different_value = different.split_whitespace().next().unwrap();
            let new_right = different
                .split_once(&format!("icmp ne {width} {address}, "))
                .unwrap()
                .1;
            let input_slot = definitions[new_right]
                .strip_prefix(&format!("load {width}, ptr "))
                .unwrap()
                .split(',')
                .next()
                .unwrap();
            let input_store = lines
                .iter()
                .enumerate()
                .filter(|(index, line)| {
                    *index < owner_index
                        && line.contains(&format!(", ptr {input_slot},"))
                        && line.contains(&format!("store {width} "))
                })
                .last()
                .expect("staged distinct input")
                .1;
            let input_value = input_store
                .split_once(&format!("store {width} "))
                .unwrap()
                .1
                .split(',')
                .next()
                .unwrap();
            assert!(
                definitions[input_value].starts_with(&format!("load {width}, ptr %input")),
                "{llvm}"
            );
            let live = lines
                .iter()
                .find(|line| {
                    line.contains("joined_previous_live")
                        && line.contains(&format!("icmp ne {width} {address}, 0"))
                })
                .expect("old right live predicate");
            let live_value = live.split_whitespace().next().unwrap();
            let owned = lines
                .iter()
                .find(|line| {
                    line.contains(" = and i1 ") && line.ends_with(&format!(", {different_value}"))
                })
                .expect("captured right owner release input");
            let owned_value = owned.split_whitespace().next().unwrap();
            let release = lines
                .iter()
                .find(|line| line.contains(&format!("and i1 {owned_value}, {live_value}")))
                .expect("right replacement release predicate");
            let release_value = release.split_whitespace().next().unwrap();
            let branch = lines
                .iter()
                .position(|line| line.contains(&format!("br i1 {release_value},")))
                .expect("right release branch");
            assert!(source_clear < left_write && left_write < branch, "{llvm}");
            let pointer = lines
                .iter()
                .find(|line| {
                    line.contains("joined_previous_pointer")
                        && line.contains(&format!("inttoptr {width} {address} to ptr"))
                })
                .expect("old right release address");
            let pointer_value = pointer.split_whitespace().next().unwrap();
            assert!(
                lines.iter().any(|line| line
                    .contains(&format!("call void @__wosy_core_free(ptr {pointer_value})"))),
                "{llvm}"
            );
            let releases_transferred_address = release_on_owned_distinct_live_path(
                release_value,
                &definitions,
                owner,
                different_value,
                live_value,
                source_owner,
            );
            println!("{layout:?} {route}: validation=accepted emission=accepted; old_right={address} old_owner={owner}; left_payload={replacement_value}; clear_line={source_clear}; release={release_value}; frees_transferred_right={releases_transferred_address}");
            println!(
                "{}\n{}\n{}\n{}\n{}\n{}\n{}\n{}",
                source_owner_line,
                owner_line,
                address_line,
                lines[source_clear],
                lines[left_owner_write],
                left_store,
                release,
                lines[branch]
            );
            if releases_transferred_address {
                unsafe_routes.push(format!("{layout:?} {route}"));
            }
        }
    }
    assert!(unsafe_routes.is_empty(), "right replacement frees the owned address transferred to left on local=true, borrowed=false, transfer=true: {unsafe_routes:?}");
}

#[test]
fn joined_group_crossed_self_and_ordinary_replacements_preserve_owner_authority() {
    fn predicate(
        value: &str,
        definitions: &BTreeMap<String, String>,
        inputs: &BTreeMap<String, bool>,
    ) -> bool {
        if value == "true" {
            return true;
        }
        if value == "false" {
            return false;
        }
        if let Some(value) = inputs.get(value) {
            return *value;
        }
        let expression = &definitions[value];
        if let Some(operands) = expression.strip_prefix("xor i1 ") {
            let (left, right) = operands.split_once(", ").unwrap();
            return predicate(left, definitions, inputs) ^ predicate(right, definitions, inputs);
        }
        let operands = expression
            .strip_prefix("and i1 ")
            .expect("joined release conjunction");
        let (left, right) = operands.split_once(", ").unwrap();
        predicate(left, definitions, inputs) && predicate(right, definitions, inputs)
    }

    for (case, group, destination_order, source_names, expected_owner_bits, expected_releases) in [
        (
            "crossed",
            "left, right = right, left;",
            ["left", "right"],
            ["right", "left"],
            [true, true],
            [false, false],
        ),
        (
            "self_then_transfer",
            "right, left = right, right;",
            ["right", "left"],
            ["right", "right"],
            [false, true],
            [false, true],
        ),
        (
            "ordinary",
            "left, right = input, input;",
            ["left", "right"],
            ["input", "input"],
            [false, false],
            [true, true],
        ),
    ] {
        let source = SourceIdentity::new(
            "project".into(),
            "package".into(),
            "src/main.w".into(),
            "r1".into(),
        );
        let text = format!("%%start\nstruct copy Point {{ u8 value; }}\n*Point(u8) create = fn(value) {{ Point backing = {{ .value = value; }}; &backing }};\n(*Point, *Point)(*Point, bool, bool, bool) choose = fn(input, local, borrowed, replace) {{ *Point left = null; *Point right = null; if (local) {{ left, right = create(11), create(22); }}; if (borrowed) {{ left, right = input, input; }}; if (replace) {{ {group} }}; left, right }};\n%%end");
        for (layout, width) in [
            (ScalarTargetLayout::WASM32, "i32"),
            (ScalarTargetLayout::NATIVE64, "i64"),
        ] {
            let parsed = parse_source(source.clone(), text.clone(), &[]);
            assert!(
                parsed.diagnostics.is_empty(),
                "{case}: {:?}",
                parsed.diagnostics
            );
            let single = derive_scalar_program_with_layout(&parsed.result, layout);
            assert!(
                single.diagnostics.is_empty(),
                "{case}: {:?}",
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
                project.diagnostics.is_empty(),
                "{case}: {:?}",
                project.diagnostics
            );
            for (route, llvm) in [
                ("standalone", emit_scalar_llvm(&single).unwrap().to_text()),
                (
                    "project",
                    emit_scalar_project_llvm(&project).unwrap().to_text(),
                ),
            ] {
                let lines = llvm
                    .lines()
                    .skip_while(|line| {
                        !line.starts_with(&format!("define {{ {width}, {width} }} @"))
                    })
                    .take_while(|line| *line != "}")
                    .collect::<Vec<_>>();
                let definitions = lines
                    .iter()
                    .filter_map(|line| {
                        line.split_once(" = ").map(|(name, expression)| {
                            (
                                name.split_whitespace().next().unwrap().to_owned(),
                                expression.to_owned(),
                            )
                        })
                    })
                    .collect::<BTreeMap<_, _>>();
                let captures = destination_order.map(|name| {
                    let owner = lines
                        .iter()
                        .enumerate()
                        .filter(|(_, line)| {
                            line.contains("joined_previous_owner")
                                && line.contains(&format!(" = load i1, ptr %{name}_return_owner,"))
                        })
                        .last()
                        .unwrap();
                    let address = lines
                        .iter()
                        .enumerate()
                        .filter(|(_, line)| {
                            line.contains("joined_previous_address")
                                && line.contains(&format!(" = load {width}, ptr %{name},"))
                        })
                        .last()
                        .unwrap();
                    (owner, address)
                });
                let last_capture = captures.iter().map(|(_, address)| address.0).max().unwrap();
                let staged_owners = lines
                    .iter()
                    .enumerate()
                    .filter(|(_, line)| {
                        line.contains("joined_assignment_owned") && line.contains(" = load i1,")
                    })
                    .collect::<Vec<_>>();
                assert_eq!(
                    staged_owners.len(),
                    if case == "ordinary" { 0 } else { 2 },
                    "{case} {route}: {llvm}"
                );
                let mut inputs = BTreeMap::new();
                for (_, line) in &staged_owners {
                    inputs.insert(line.split_whitespace().next().unwrap().to_owned(), true);
                }
                let mut prior_write = last_capture;
                let mut released = 0;
                for position in 0..2 {
                    let name = destination_order[position];
                    let source_name = source_names[position];
                    let ((owner_index, owner_line), (address_index, address_line)) =
                        captures[position];
                    let owner = owner_line.split_whitespace().next().unwrap();
                    let address = address_line.split_whitespace().next().unwrap();
                    inputs.insert(owner.to_owned(), true);
                    let (write_index, write) = lines
                        .iter()
                        .enumerate()
                        .filter(|(_, line)| {
                            line.contains(&format!("store {width} "))
                                && line.contains(&format!(", ptr %{name},"))
                        })
                        .last()
                        .unwrap();
                    let payload = write
                        .split_once(&format!("store {width} "))
                        .unwrap()
                        .1
                        .split(',')
                        .next()
                        .unwrap();
                    let temporary = definitions[payload]
                        .strip_prefix(&format!("load {width}, ptr "))
                        .unwrap()
                        .split(',')
                        .next()
                        .unwrap();
                    let (rhs_index, rhs) = lines
                        .iter()
                        .enumerate()
                        .filter(|(index, line)| {
                            *index < owner_index
                                && line.contains(&format!("store {width} "))
                                && line.contains(&format!(", ptr {temporary},"))
                        })
                        .last()
                        .unwrap();
                    let source_payload = rhs
                        .split_once(&format!("store {width} "))
                        .unwrap()
                        .1
                        .split(',')
                        .next()
                        .unwrap();
                    assert!(
                        definitions[source_payload]
                            .starts_with(&format!("load {width}, ptr %{source_name}")),
                        "{case}: {llvm}"
                    );
                    assert!(
                        rhs_index < owner_index && owner_index < address_index,
                        "{case}: {llvm}"
                    );
                    let owner_writes = lines
                        .iter()
                        .enumerate()
                        .filter(|(index, line)| {
                            *index > last_capture
                                && line.contains("store i1 ")
                                && line.contains(&format!(", ptr %{name}_return_owner,"))
                        })
                        .collect::<Vec<_>>();
                    let expected_clear =
                        case == "crossed" || (case == "self_then_transfer" && name == "right");
                    assert_eq!(
                        owner_writes.len(),
                        if expected_clear { 2 } else { 1 },
                        "{case}: {llvm}"
                    );
                    let install = owner_writes.last().unwrap();
                    let installed_bit = install
                        .1
                        .split_once("store i1 ")
                        .unwrap()
                        .1
                        .split(',')
                        .next()
                        .unwrap();
                    assert_eq!(
                        predicate(installed_bit, &definitions, &inputs),
                        expected_owner_bits[position],
                        "{case} owner {name}: {llvm}"
                    );
                    if case != "ordinary" {
                        let staged = staged_owners[position];
                        assert!(
                            staged
                                .1
                                .contains(&format!(", ptr %{source_name}_return_owner,")),
                            "{case}: {llvm}"
                        );
                        assert!(
                            staged.0 < rhs_index && staged.0 < owner_index,
                            "{case}: {llvm}"
                        );
                        if expected_owner_bits[position] {
                            assert_eq!(
                                installed_bit,
                                staged.1.split_whitespace().next().unwrap(),
                                "{case}: {llvm}"
                            );
                        }
                    }
                    if expected_clear {
                        assert!(
                            owner_writes[0].1.contains("store i1 false,"),
                            "{case}: {llvm}"
                        );
                        assert!(
                            last_capture < owner_writes[0].0 && owner_writes[0].0 < install.0,
                            "{case}: {llvm}"
                        );
                        for other in destination_order {
                            let first_install = lines
                                .iter()
                                .enumerate()
                                .filter(|(index, line)| {
                                    *index > last_capture
                                        && line.contains("store i1 ")
                                        && line.contains(&format!(", ptr %{other}_return_owner,"))
                                })
                                .last()
                                .unwrap()
                                .0;
                            assert!(
                                owner_writes[0].0 < first_install,
                                "late source clear in {case}: {llvm}"
                            );
                        }
                    }
                    assert!(
                        prior_write < install.0 && install.0 < write_index,
                        "{case}: {llvm}"
                    );
                    prior_write = write_index;
                    let different = lines
                        .iter()
                        .find(|line| {
                            line.contains("joined_owner_replaced")
                                && line.contains(&format!("icmp ne {width} {address},"))
                        })
                        .unwrap();
                    let different_value = different.split_whitespace().next().unwrap();
                    let live = lines
                        .iter()
                        .find(|line| {
                            line.contains("joined_previous_live")
                                && line.contains(&format!("icmp ne {width} {address}, 0"))
                        })
                        .unwrap();
                    let live_value = live.split_whitespace().next().unwrap();
                    inputs.insert(
                        different_value.to_owned(),
                        !(case == "self_then_transfer" && name == "right"),
                    );
                    inputs.insert(live_value.to_owned(), true);
                    let owned = lines
                        .iter()
                        .find(|line| {
                            line.contains(" = and i1 ")
                                && line.ends_with(&format!(", {different_value}"))
                        })
                        .unwrap();
                    let owned_value = owned.split_whitespace().next().unwrap();
                    let release = lines
                        .iter()
                        .find(|line| line.contains(&format!("and i1 {owned_value}, {live_value}")))
                        .unwrap();
                    let release_value = release.split_whitespace().next().unwrap();
                    let release_bit = predicate(release_value, &definitions, &inputs);
                    assert_eq!(
                        release_bit, expected_releases[position],
                        "{case} release {name}: {llvm}"
                    );
                    released += usize::from(release_bit);
                    let pointer = lines
                        .iter()
                        .find(|line| {
                            line.contains("joined_previous_pointer")
                                && line.contains(&format!("inttoptr {width} {address} to ptr"))
                        })
                        .unwrap();
                    let pointer_value = pointer.split_whitespace().next().unwrap();
                    assert_eq!(
                        lines
                            .iter()
                            .filter(|line| line.contains(&format!(
                                "call void @__wosy_core_free(ptr {pointer_value})"
                            )))
                            .count(),
                        1,
                        "{case}: {llvm}"
                    );
                }
                assert_eq!(
                    released,
                    expected_releases.into_iter().filter(|value| *value).count()
                );
                println!("{case} {layout:?} {route}: staged joined bits={}, eligible old-owner releases={released}; payload/slot/order/clear checks passed", staged_owners.len());
            }
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
fn detached_project_llvm_matches_attached_for_invalidation_and_selected_overload() {
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
    let root_cst = parse_source(
        root.clone(),
        "%%start
child = namespace package \"src/child.w\";
unit(u64) discard = fn(count) { u8[count] direct; direct[0] = 3; unsafe { core.invalidate(&direct); } };
u8 outside = 2;
*u8 outside_ref = &outside;
bool use_fresh = true;
*u8 selected = child.select(outside_ref, use_fresh);
%%end".into(),
        &[],
    );
    let child_cst = parse_source(
        child.clone(),
        "%%start
*u8(*u8, bool) choose = fn(input, flag) { u8 local = 1; if (flag) { &local } else { input } };
select = overload {
    i64(i64) => fn(value) { value };
    *u8(*u8, bool) => fn(input, flag) { choose(input, flag) };
};
%%end"
            .into(),
        &[],
    );
    assert!(
        root_cst.diagnostics.is_empty(),
        "{:?}",
        root_cst.diagnostics
    );
    assert!(
        child_cst.diagnostics.is_empty(),
        "{:?}",
        child_cst.diagnostics
    );
    for target_layout in [ScalarTargetLayout::WASM32, ScalarTargetLayout::NATIVE64] {
        let declarations = derive_scalar_declarations_from_cst(root_cst.result.canonical_cst());
        let namespace = declarations.namespaces().next().unwrap();
        let binding = ScalarNamespaceBinding {
            binding: namespace.binding.clone(),
            span: namespace.span,
            target: child.clone(),
        };
        let prepared = prepare_scalar_project(
            vec![
                (
                    derive_scalar_declarations_from_cst(child_cst.result.canonical_cst()),
                    Vec::new(),
                ),
                (declarations, vec![binding]),
            ],
            vec![root.clone(), child.clone()],
            ScalarPreparationInputs { target_layout },
        )
        .unwrap_or_else(|invalid| panic!("{:?}", invalid.diagnostics));
        let build = analyze_scalar_build(&prepared);
        assert!(build.diagnostics().is_empty(), "{:?}", build.diagnostics());
        let legacy = validate_scalar_project(prepared.project().clone());
        assert!(legacy.diagnostics.is_empty(), "{:?}", legacy.diagnostics);

        let detached_ir = emit_scalar_build_project_llvm(&build).unwrap().to_text();
        assert_eq!(
            detached_ir,
            emit_scalar_project_llvm(&legacy).unwrap().to_text()
        );
        assert!(
            detached_ir.contains("store i1 false, ptr %direct_live"),
            "{detached_ir}"
        );
        assert!(
            detached_ir.lines().any(|line| line.contains("call ")
                && line.contains("@wosy_generic__")
                && line.contains("ptr %checked_return_owner_0")),
            "{detached_ir}"
        );

        let ScalarItem::Function(overload) = &legacy.project.modules[0].items[1] else {
            panic!("expected overload at its original project position");
        };
        let arm = &overload.overload_arms[1];
        assert!(!arm.reference_cfg.points.is_empty());
        let missing = super::emit_project_llvm(
            &legacy.project,
            &|module| prepared.initialization_region(module),
            super::GenericInitializerInput::RawProject(&prepared),
            &|_| Ok(crate::scalar::ClosedAggregates::default()),
            &|module, item, arm, function| {
                if (module, item, arm) == (0, 1, Some(1)) {
                    build.reference_cfg(module, item, Some(overload.overload_arms.len()))
                } else {
                    Some(&function.reference_cfg)
                }
            },
            None,
            super::PartitionMetadata::Full,
            &[],
        )
        .unwrap_err();
        assert!(
            missing.starts_with("internal error: missing reference CFG"),
            "{missing}"
        );
        assert!(missing.contains(&format!("{child:?}")), "{missing}");
        assert!(
            missing.contains(&format!(
                "at {}..{} (module 0, item 1, arm Some(1))",
                arm.span.start, arm.span.end
            )),
            "{missing}"
        );
    }
}

#[test]
fn detached_single_module_llvm_matches_legacy_on_both_targets() {
    let source = SourceIdentity::new(
        "project".into(),
        "package".into(),
        "src/main.w".into(),
        "r1".into(),
    );
    for (target_layout, pointer_width) in [
        (ScalarTargetLayout::WASM32, "i32"),
        (ScalarTargetLayout::NATIVE64, "i64"),
    ] {
        let dereference = format!("inttoptr {pointer_width}");
        for (text, ir_witnesses, main_stores) in [
            (
                "%%start
i32 counter = 10;
unit(u64) discard = fn(count) { u8[count] bytes; bytes[0] = 3; unsafe { core.invalidate(&bytes); } };
*u8(*u8, bool) choose = fn(input, flag) { u8 local = 1; if (flag) { &local } else { input } };
select = overload {
    i64(i64) => fn(value) { value };
    *u8(*u8, bool) => fn(input, flag) { choose(input, flag) };
};
counter = 11;
i32 after = 12;
counter = 13;
u8 outside = 2;
*u8 outside_ref = &outside;
u8 observed = *outside_ref;
bool use_fresh = true;
*u8 selected = select(outside_ref, use_fresh);
%%end",
                [
                    "store i1 false, ptr %bytes_live",
                    "ptr %checked_return_owner_0",
                    dereference.as_str(),
                ]
                .as_slice(),
                [
                    "store i32 10, ptr @counter,",
                    "store i32 11,",
                    "store i32 12, ptr @after,",
                    "store i32 13,",
                ]
                .as_slice(),
            ),
            (
                "%%start
generic T;
T(T) identity = fn(value) { value };
cast = overload {
    i32(i64) => fn(value) { 1 };
    generic T;
    T(T) => fn(value) { value };
};
i64 wide = identity<i64>(1);
i32 narrow = cast(wide);
bool flag = true;
bool copied = cast(flag);
%%end",
                ["call i64 @wosy_generic__", "call i1 @wosy_generic__"].as_slice(),
                &[],
            ),
        ] {
            let parsed = parse_source(source.clone(), text.into(), &[]);
            assert!(parsed.diagnostics.is_empty(), "{:?}", parsed.diagnostics);
            let prepared = prepare_scalar_project(
                vec![(
                    derive_scalar_declarations_from_cst(parsed.result.canonical_cst()),
                    Vec::new(),
                )],
                vec![source.clone()],
                ScalarPreparationInputs { target_layout },
            )
            .unwrap_or_else(|invalid| panic!("{:?}", invalid.diagnostics));
            let build = analyze_scalar_build(&prepared);
            assert!(build.diagnostics().is_empty(), "{:?}", build.diagnostics());
            let legacy = derive_scalar_program_with_layout(&parsed.result, target_layout);
            assert!(legacy.diagnostics.is_empty(), "{:?}", legacy.diagnostics);

            let detached_ir = emit_scalar_build_single_module_llvm(&build).unwrap().to_text();
            assert_eq!(detached_ir, emit_scalar_llvm(&legacy).unwrap().to_text());
            for witness in ir_witnesses {
                assert!(
                    detached_ir.contains(witness),
                    "missing {witness}: {detached_ir}"
                );
            }
            let main = detached_ir.split("define i32 @main").nth(1).expect("main");
            let mut next = 0;
            for &store in main_stores {
                assert_eq!(main.matches(store).count(), 1, "{store}: {main}");
                next += main[next..]
                    .find(store)
                    .unwrap_or_else(|| panic!("{store}: {main}"))
                    + store.len();
            }
        }
    }
}

#[test]
fn detached_single_module_llvm_rejects_non_single_cardinality() {
    for count in [0, 2] {
        let sources = (0..count)
            .map(|index| {
                SourceIdentity::new(
                    "project".into(),
                    "package".into(),
                    format!("src/module{index}.w"),
                    "r1".into(),
                )
            })
            .collect::<Vec<_>>();
        let declarations = sources
            .iter()
            .map(|source| {
                let parsed =
                    parse_source(source.clone(), "%%start\nu8 value = 1;\n%%end".into(), &[]);
                assert!(parsed.diagnostics.is_empty(), "{:?}", parsed.diagnostics);
                (
                    derive_scalar_declarations_from_cst(parsed.result.canonical_cst()),
                    Vec::new(),
                )
            })
            .collect();
        let prepared = prepare_scalar_project(
            declarations,
            sources,
            ScalarPreparationInputs {
                target_layout: ScalarTargetLayout::NATIVE64,
            },
        )
        .unwrap_or_else(|invalid| panic!("{:?}", invalid.diagnostics));
        let build = analyze_scalar_build(&prepared);
        assert!(build.diagnostics().is_empty(), "{:?}", build.diagnostics());
        assert_eq!(
            emit_scalar_build_single_module_llvm(&build).unwrap_err(),
            format!(
                "single-module LLVM emission requires exactly one prepared module, found {count}"
            )
        );
    }
}

#[test]
fn detached_project_llvm_rejects_build_diagnostics() {
    let source = SourceIdentity::new(
        "project".into(),
        "package".into(),
        "src/main.w".into(),
        "r1".into(),
    );
    let parsed = parse_source(
        source.clone(),
        "%%start\nstruct Payload { u8 value; }\nPayload(Payload) consume = fn(input) { next = input; next; next };\n%%end".into(),
        &[],
    );
    for target_layout in [ScalarTargetLayout::WASM32, ScalarTargetLayout::NATIVE64] {
        let prepared = prepare_scalar_project(
            vec![(
                derive_scalar_declarations_from_cst(parsed.result.canonical_cst()),
                Vec::new(),
            )],
            vec![source.clone()],
            ScalarPreparationInputs { target_layout },
        )
        .unwrap_or_else(|invalid| panic!("{:?}", invalid.diagnostics));
        let build = analyze_scalar_build(&prepared);
        assert_eq!(build.diagnostics()[0].code, "B0003");
        assert_eq!(
            emit_scalar_build_project_llvm(&build).unwrap_err(),
            "cannot emit LLVM for an invalid scalar project"
        );
        assert_eq!(
            emit_scalar_build_single_module_llvm(&build).unwrap_err(),
            "cannot emit LLVM for an invalid scalar program"
        );
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
        storage_type(
            &context,
            &array,
            &crate::scalar::layout::ScalarLayoutIndex::project(&[]),
            ScalarTargetLayout::NATIVE64
        )
        .unwrap_err(),
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
    assert!(storage_type(
        &context,
        &array,
        &crate::scalar::layout::ScalarLayoutIndex::project(&[]),
        ScalarTargetLayout::NATIVE64
    )
    .is_ok());
}

#[test]
fn lowers_nested_project_conditionals_with_runtime_array_operations() {
    let source = SourceIdentity::new(
        "project".into(),
        "package".into(),
        "src/main.w".into(),
        "r1".into(),
    );
    let scalar = derive_scalar_program(
            &parse_source(
                source.clone(),
                "%%start\ni32(u64) count = fn(length) { u8[length] bytes; i32 outer = 0; if (outer < 1) { i32 index = 0; if (index < 1) { if (index == 0) { bytes[0] = 1; }; index = index + 1; }; outer = outer + 1; }; outer };\n%%end".into(),
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
        .expect("project nested conditional LLVM")
        .to_text();
    assert!(
        llvm.lines().filter(|line| line.contains("br i1 ")).count() >= 3,
        "{llvm}"
    );
    assert!(llvm.contains("store i8 1"), "{llvm}");
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
fn emits_contextual_integer_operators_with_literals_on_either_side() {
    let code = r#"%%start
u8(u32) stamp = fn(depth) {
    u8[1] scratch = [0];
    scratch[0] = 16 + core.int_trunc<u8>(depth);
    scratch[0]
};
u8(u8) divide_left = fn(value) { 233 / value };
u8(u8) divide_right = fn(value) { value / 233 };
i8(i8) signed_left = fn(value) { -7 / value };
u8(u8) shift_unsigned = fn(value) { 233 >> value };
i8(i8) shift_signed = fn(value) { -7 >> value };
u8 seed = stamp(3);
u8 quotient = 233 / seed;
%%end"#;
    for layout in [ScalarTargetLayout::NATIVE64, ScalarTargetLayout::WASM32] {
        let project = prepare_initializer_test(code, layout);
        let checked = analyze_scalar_build(&project);
        assert!(checked.diagnostics().is_empty(), "{:?}", checked.diagnostics());
        for llvm in [
            emit_scalar_build_single_module_llvm(&checked).unwrap().to_text(),
            emit_scalar_build_project_llvm(&checked).unwrap().to_text(),
        ] {
            assert!(llvm.contains("add i8 16, %int_trunc"), "{llvm}");
            let left_divisions = llvm.lines().filter(|line| {
                line.contains("udiv i8 -23, ") || line.contains("udiv i8 233, ")
            }).count();
            assert_eq!(left_divisions, 2, "{llvm}");
            assert!(llvm.lines().any(|line| line.contains("udiv i8 %")
                && (line.ends_with(", -23") || line.ends_with(", 233"))), "{llvm}");
            assert!(llvm.contains("sdiv i8 -7, "), "{llvm}");
            assert!(llvm.contains("lshr i8 -23, %value")
                || llvm.contains("lshr i8 233, %value"), "{llvm}");
            assert!(llvm.contains("ashr i8 -7, %value"), "{llvm}");
        }
    }
}

#[test]
fn emits_multiple_unit_conditionals_in_a_branch() {
    let source = SourceIdentity::new(
        "project".into(),
        "package".into(),
        "src/main.w".into(),
        "r1".into(),
    );
    let validation = derive_scalar_program(
            &parse_source(
                source,
                "%%start\n(i32, bool)() run = fn { bool running = true; if (true) { if (running) { if (running) { running = false; } else { running = false; }; if (running) { running = running; } else { running = running; }; }; running = false; 1, true } else { 2, false } };\n%%end".into(),
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
        .expect("multiple nested unit conditional LLVM")
        .to_text();
    assert_eq!(
        llvm.lines()
            .filter(|line| line.contains("br i1 "))
            .count(),
        4,
        "{llvm}"
    );
}

#[test]
fn discards_terminated_call_result_but_preserves_effects_and_value_conditional() {
    let text = r#"%%start
i32 saved = 0;
i32() touch = fn {
    saved = 5;
    saved
};
unit(bool) attempt = fn(enabled) {
    if (enabled) {
        touch();
    } else {
        saved = 2;
    };
};
i32(bool) choose = fn(enabled) {
    if (enabled) { saved } else { 3 }
};
unit done = attempt(true);
i32 result = choose(true);
%%end"#;
    for layout in [ScalarTargetLayout::NATIVE64, ScalarTargetLayout::WASM32] {
        let prepared = prepare_initializer_test(text, layout);
        let checked = analyze_scalar_build(&prepared);
        assert!(checked.diagnostics().is_empty(), "{:?}", checked.diagnostics());
        let module = &prepared.project().modules[0];
        let attempt = module.items.iter().find_map(|item| match item {
            ScalarItem::Function(function) if function.name == "attempt" => Some(function),
            _ => None,
        }).unwrap();
        let [ScalarBlockItem::Expression(ScalarExpression::If { then_branch, .. })] =
            attempt.body.items.as_slice()
        else {
            panic!("expected conditional statement");
        };
        assert_eq!(then_branch.terminated_items, [true]);
        assert!(then_branch.final_output_values.is_empty());
        let source = SourceIdentity::new("project".into(), "package".into(), "src/main.w".into(), "r1".into());
        let scalar = derive_scalar_program_with_layout(&parse_source(source, text.into(), &[]).result, layout);
        assert!(scalar.diagnostics.is_empty(), "{:?}", scalar.diagnostics);
        for partition in [
            emit_scalar_llvm(&scalar),
            emit_scalar_build_single_module_llvm(&checked),
            emit_scalar_build_project_llvm(&checked),
        ] {
            let ir = partition.unwrap().to_text();
            let effects = ir.lines().filter(|line| line.contains(" = call i32 @")).count();
            assert_eq!(effects, 2, "{layout:?}: {ir}");
            assert_eq!(ir.matches("phi i32").count(), 1, "{layout:?}: {ir}");
            assert!(ir.contains("store i32 5"), "{layout:?}: {ir}");
            assert!(ir.contains("store i32 2"), "{layout:?}: {ir}");
            let ctx = Context::create();
            let memory = inkwell::memory_buffer::MemoryBuffer::create_from_memory_range_copy(ir.as_bytes(), "discarded-call");
            ctx.create_module_from_ir(memory).unwrap().verify().unwrap();
        }
    }
}

#[test]
fn emits_project_multiple_unit_conditionals_in_a_branch_once() {
    let source = SourceIdentity::new(
        "project".into(),
        "package".into(),
        "src/main.w".into(),
        "r1".into(),
    );
    let scalar = derive_scalar_program(
            &parse_source(
                source.clone(),
                "%%start\n(i32, bool)() run = fn { bool running = true; if (true) { if (running) { if (running) { running = false; } else { running = false; }; if (running) { running = running; } else { running = running; }; }; running = false; 1, true } else { 2, false } };\n%%end".into(),
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
        .expect("project multiple nested unit conditional LLVM")
        .to_text();
    assert_eq!(
        llvm.lines()
            .filter(|line| line.contains("br i1 "))
            .count(),
        4,
        "{llvm}"
    );
}

fn fixed_aggregate_move_count(ir: &str, source: &str, bytes: u64) -> usize {
    let source = format!(", ptr align 1 {source}");
    let size = format!(" {bytes}, i1 false)");
    ir.lines()
        .filter(|line| {
            line.contains("call void @llvm.memmove")
                && line.contains(&source)
                && line.ends_with(&size)
        })
        .count()
}

#[test]
fn emits_bulk_transfer_for_large_fixed_array_literals_on_both_targets() {
    let source = SourceIdentity::new(
        "project".into(),
        "package".into(),
        "src/main.w".into(),
        "r1".into(),
    );
    let text = "%%start
f32[6] point = [1.0, 2.0, 3.0, 4.0, 5.0, 6.0];
POINTS
%%end"
        .replace(
            "POINTS",
            &[("point", "row", 24), ("row", "plane", 24), ("plane", "layer", 24), ("layer", "grid", 12)]
                .into_iter()
                .scan(String::from("f32[6]"), |ty, (item, name, count)| {
                    ty.push_str(&format!("[{count}]"));
                    Some(format!("{ty} {name} = [{}];", vec![item; count].join(", ")))
                })
                .collect::<Vec<_>>()
                .join("\n"),
        );
    for layout in [ScalarTargetLayout::WASM32, ScalarTargetLayout::NATIVE64] {
        let parsed = parse_source(source.clone(), text.clone(), &[]);
        let single = derive_scalar_program_with_layout(&parsed.result, layout);
        assert!(single.diagnostics.is_empty(), "{:?}", single.diagnostics);
        let single_ir = emit_scalar_llvm(&single).unwrap().to_text();
        let project = validate_scalar_project(ScalarProject::new(
            vec![ScalarModule::from_program(single.program, Vec::new())],
            vec![source.clone()],
        ));
        assert!(project.diagnostics.is_empty(), "{:?}", project.diagnostics);
        let project_ir = emit_scalar_project_llvm(&project).unwrap().to_text();
        for ir in [single_ir, project_ir] {
            assert_eq!(fixed_aggregate_move_count(&ir, "%array_literal", 3_981_312), 1, "{ir}");
            assert!(ir.lines().all(|line| !line.contains("load [") && !line.contains("store [")), "{ir}");
            if layout == ScalarTargetLayout::WASM32 {
                assert!(ir.contains("@llvm.memmove.p0.p0.i32"), "{ir}");
                assert!(ir.contains("\"target-features\"=\"+bulk-memory\""), "{ir}");
            } else {
                assert!(ir.contains("@llvm.memmove.p0.p0.i64"), "{ir}");
                assert!(!ir.contains("+bulk-memory"), "{ir}");
            }
        }
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

fn length_loaded_for_runtime_array(llvm: &str) -> &str {
    let slot = llvm
        .lines()
        .find_map(|line| {
            line.trim_start()
                .strip_prefix("store i64 %length, ptr ")
                .map(|rest| rest.split(',').next().unwrap())
        })
        .expect("length parameter storage");
    llvm.lines()
        .find_map(|line| {
            let (value, load) = line.trim_start().split_once(" = load i64, ptr ")?;
            (load.split(',').next() == Some(slot)).then_some(value)
        })
        .expect("length parameter load")
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
            let length = length_loaded_for_runtime_array(&llvm);
            assert!(
                llvm.contains(&format!("call ptr @__wosy_core_alloc(i64 {length}, i64 1)")),
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
fn checks_runtime_array_byte_products_before_calling_the_allocator() {
    let text = "%%start
unit(u64) wide = fn(length) { u64[length] words; words; };
unit(u64) narrow = fn(length) { u8[length] bytes; bytes; };
%%end";
    let source = SourceIdentity::new(
        "project".into(),
        "package".into(),
        "src/main.w".into(),
        "r1".into(),
    );
    for layout in [ScalarTargetLayout::WASM32, ScalarTargetLayout::NATIVE64] {
        let parsed = parse_source(source.clone(), text.into(), &[]);
        assert!(parsed.diagnostics.is_empty(), "{:?}", parsed.diagnostics);
        let single = derive_scalar_program_with_layout(&parsed.result, layout);
        assert!(single.diagnostics.is_empty(), "{:?}", single.diagnostics);
        let project = validate_scalar_project(ScalarProject::new(
            vec![ScalarModule::from_program(single.program.clone(), Vec::new())],
            vec![source.clone()],
        ));
        assert!(project.diagnostics.is_empty(), "{:?}", project.diagnostics);
        for (project_route, llvm) in [
            (false, emit_scalar_llvm(&single).unwrap().to_text()),
            (true, emit_scalar_project_llvm(&project).unwrap().to_text()),
        ] {
            let body = |name: &str| {
                let name = if project_route {
                    project_function_name(&source, name)
                } else {
                    name.to_owned()
                };
                llvm.split_once(&format!("@{name}("))
                    .and_then(|(_, rest)| rest.split_once("\n}"))
                    .map(|(body, _)| body)
                    .unwrap_or_else(|| panic!("{name}: {llvm}"))
            };
            let wide = body("wide");
            let length = length_loaded_for_runtime_array(wide);
            assert!(
                wide.contains(&format!("icmp ugt i64 {length}, {}", u64::MAX / 8)),
                "{wide}"
            );
            assert!(
                wide.contains("br i1 %allocation_size_overflow, label %allocation_panic, label %allocation_size_valid"),
                "{wide}"
            );
            let (_, valid) = wide.split_once("\nallocation_size_valid:").expect("valid size");
            let size = valid
                .lines()
                .find_map(|line| {
                    let (value, operands) = line.trim_start().split_once(" = mul nuw i64 ")?;
                    (operands == format!("{length}, 8")).then_some(value)
                })
                .expect("unsigned product uses guarded length");
            assert!(
                valid.contains(&format!("call ptr @__wosy_core_alloc(i64 {size}, i64 8)")),
                "{valid}"
            );
            let (_, panic) = wide.split_once("\nallocation_panic:").expect("shared panic");
            let (panic, _) = panic.split_once("\nallocation_size_valid:").expect("panic end");
            assert!(panic.contains("call void @__wosy_core_system_panic()"), "{panic}");
            assert!(panic.contains("unreachable"), "{panic}");
            assert!(!panic.contains("@__wosy_core_alloc("), "{panic}");
            let narrow = body("narrow");
            let length = length_loaded_for_runtime_array(narrow);
            assert!(
                narrow.contains(&format!("call ptr @__wosy_core_alloc(i64 {length}, i64 1)")),
                "{narrow}"
            );
            assert!(!narrow.contains("allocation_size_overflow"), "{narrow}");
            assert!(!narrow.contains("mul "), "{narrow}");
        }
    }
}

#[test]
fn releases_runtime_array_backing_after_copying_primitive_returns() {
    let text = "%%start
u8(u64) indexed = fn(length) { u8[length] bytes; bytes[0] = 7; bytes[0] };
u8(u64) copied = fn(length) { u8[length] bytes; bytes[0] = 7; u8 result = bytes[0]; result };
u8(u64) indirect = fn(length) { u8[length] bytes; bytes[0] = 7; *u8 view = &bytes[0]; *view };
enum copy Event { none; started; }
Event(u64) enum_indexed = fn(length) { Event[length] events; events[0] = Event::started; events[0] };
Event(u64) enum_copied = fn(length) { Event[length] events; events[0] = Event::started; Event result = events[0]; result };
%%end";
    let source = SourceIdentity::new(
        "project".into(),
        "package".into(),
        "src/main.w".into(),
        "r1".into(),
    );
    for layout in [ScalarTargetLayout::WASM32, ScalarTargetLayout::NATIVE64] {
        let parsed = parse_source(source.clone(), text.into(), &[]);
        assert!(parsed.diagnostics.is_empty(), "{:?}", parsed.diagnostics);
        let single = derive_scalar_program_with_layout(&parsed.result, layout);
        assert!(single.diagnostics.is_empty(), "{:?}", single.diagnostics);
        let project = validate_scalar_project(ScalarProject::new(
            vec![ScalarModule::from_program(single.program.clone(), Vec::new())],
            vec![source.clone()],
        ));
        assert!(project.diagnostics.is_empty(), "{:?}", project.diagnostics);
        for (project_route, llvm) in [
            (false, emit_scalar_llvm(&single).unwrap().to_text()),
            (true, emit_scalar_project_llvm(&project).unwrap().to_text()),
        ] {
            for (name, output) in [
                ("indexed", "i8"),
                ("copied", "i8"),
                ("indirect", "i8"),
                ("enum_indexed", "i32"),
                ("enum_copied", "i32"),
            ] {
                let symbol = if project_route {
                    project_function_name(&source, name)
                } else {
                    name.to_owned()
                };
                let (_, tail) = llvm
                    .split_once(&format!("@{symbol}("))
                    .unwrap_or_else(|| panic!("{name}: {llvm}"));
                let (body, _) = tail.split_once("\n}").expect("function end");
                let allocation = "call ptr @__wosy_core_alloc";
                let release = "call void @__wosy_core_free";
                assert_eq!(body.matches(allocation).count(), 1, "{name}: {body}");
                assert_eq!(body.matches(release).count(), 1, "{name}: {body}");
                let allocation = body.find(allocation).unwrap();
                let loaded = body
                    .rfind(&format!("load {output}, ptr"))
                    .expect("result load");
                let release = body.find(release).unwrap();
                let returned = body.find(&format!("ret {output} ")).expect("value return");
                assert!(
                    allocation < loaded && loaded < release && release < returned,
                    "{name}: {body}"
                );
            }
        }
    }
}

#[test]
fn publishes_runtime_array_storage_for_ordinary_module_calls_before_root_cleanup() {
    let text = "%%start\nu64 length = 2;\nu8[length] values;\nvalues[0] = 7;\nu8() read = fn { values[0] };\nunit(u8) write = fn(value) { values[0] = value; };\nu8 first = read();\nwrite(9);\nu8 second = read();\n%%end";
    for target_layout in [ScalarTargetLayout::NATIVE64, ScalarTargetLayout::WASM32] {
        let prepared = prepare_initializer_test(text, target_layout);
        let source = &prepared.project().modules[0].source;
        let parsed = parse_source(source.clone(), text.into(), &[]);
        let standalone = derive_scalar_program_with_layout(&parsed.result, target_layout);
        assert!(standalone.diagnostics.is_empty(), "{:?}", standalone.diagnostics);
        let build = analyze_scalar_build(&prepared);
        assert!(build.diagnostics().is_empty(), "{:?}", build.diagnostics());
        for (partition, global, read, write) in [
            (emit_scalar_llvm(&standalone), "values".to_owned(), "read".to_owned(), "write".to_owned()),
            (emit_scalar_build_single_module_llvm(&build), "values".to_owned(), "read".to_owned(), "write".to_owned()),
            (
                emit_scalar_build_project_llvm(&build),
                project_global_name(source, "values"),
                project_function_name(source, "read"),
                project_function_name(source, "write"),
            ),
        ] {
            let text = partition.unwrap().to_text();
            let body = |definition: &str| {
                text.split_once(definition).unwrap_or_else(|| panic!("{definition}: {text}"))
                    .1.split_once("\n}").unwrap().0
            };
            let entry = body("define i32 @main()");
            let allocation = entry.find("call ptr @__wosy_core_alloc").unwrap();
            let publication = entry.find(&format!("store ptr %allocation, ptr @{global}")).unwrap();
            let first_write = entry.find("store i8 7").unwrap();
            let first_read = entry.find(&format!("call i8 @{read}()")).unwrap();
            let write_call = entry.find(&format!("call void @{write}(i8 9)")).unwrap();
            let last_read = entry.rfind(&format!("call i8 @{read}()")).unwrap();
            let cleanup = entry.find("call void @__wosy_core_free").unwrap();
            assert!(allocation < publication && publication < first_write && first_write < first_read
                && first_read < write_call && write_call < last_read && last_read < cleanup, "{entry}");
            assert_eq!(text.matches("call ptr @__wosy_core_alloc").count(), 1, "{text}");
            assert_eq!(text.matches("call void @__wosy_core_free").count(), 1, "{text}");
            for name in [read, write] {
                let definition = text.lines().find(|line| line.starts_with("define ")
                    && line.contains(&format!("@{name}("))).unwrap();
                assert!(body(definition).contains(&format!("load ptr, ptr @{global}")), "{text}");
            }
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
fn primitive_return_lanes_do_not_release_backing_escaped_by_sibling() {
    let text = "%%start
struct Packet { *?u8 data; u64 length; }
(u8, *Packet)(u64) scalar_first = fn(length) { u8[length] bytes; bytes[0] = 7; *?u8 data = null; unsafe { data = &?bytes[0]; }; Packet result = { .data = data; .length = length; }; bytes[0], &result };
(*Packet, u8)(u64) pointer_first = fn(length) { u8[length] bytes; bytes[0] = 7; *?u8 data = null; unsafe { data = &?bytes[0]; }; Packet result = { .data = data; .length = length; }; &result, bytes[0] };
%%end";
    let source = SourceIdentity::new(
        "project".into(),
        "package".into(),
        "src/main.w".into(),
        "r1".into(),
    );
    for layout in [ScalarTargetLayout::WASM32, ScalarTargetLayout::NATIVE64] {
        let parsed = parse_source(source.clone(), text.into(), &[]);
        assert!(parsed.diagnostics.is_empty(), "{:?}", parsed.diagnostics);
        let single = derive_scalar_program_with_layout(&parsed.result, layout);
        assert!(single.diagnostics.is_empty(), "{:?}", single.diagnostics);
        let project = validate_scalar_project(ScalarProject::new(
            vec![ScalarModule::from_program(single.program.clone(), Vec::new())],
            vec![source.clone()],
        ));
        assert!(project.diagnostics.is_empty(), "{:?}", project.diagnostics);
        for (project_route, llvm) in [
            (false, emit_scalar_llvm(&single).unwrap().to_text()),
            (true, emit_scalar_project_llvm(&project).unwrap().to_text()),
        ] {
            let pointer = if layout == ScalarTargetLayout::WASM32 { "i32" } else { "i64" };
            for (name, shape) in [
                ("scalar_first", format!("{{ i8, {pointer} }}")),
                ("pointer_first", format!("{{ {pointer}, i8 }}")),
            ] {
                let symbol = if project_route {
                    project_function_name(&source, name)
                } else {
                    name.to_owned()
                };
                let (_, tail) = llvm
                    .split_once(&format!("@{symbol}("))
                    .unwrap_or_else(|| panic!("{name}: {llvm}"));
                let (body, _) = tail.split_once("\n}").expect("function end");
                assert!(body.contains("call ptr @__wosy_core_alloc"), "{name}: {body}");
                assert!(body.contains("load i8, ptr"), "{name}: {body}");
                assert!(body.contains(&format!("ret {shape} ")), "{name}: {body}");
                assert!(!body.contains("call void @__wosy_core_free"), "{name}: {body}");
            }
        }
    }
}

#[test]
fn runtime_array_group_captures_previous_releases_before_installs_on_both_routes() {
    let source = SourceIdentity::new(
        "project".into(),
        "package".into(),
        "src/main.w".into(),
        "r1".into(),
    );
    for (case, group, expected_previous, expected_total) in [
        ("ordinary", "left, right = third, fourth;", 2, 4),
        ("transfer", "left, right = right, third;", 1, 4),
        ("crossed", "left, right = right, left;", 0, 4),
        ("local_live", "left, right = third, fourth;", 2, 4),
    ] {
        let text = if case == "local_live" {
            format!("%%start\nunit() replace = fn {{ u64 length = 1; u8[length] left; u8[length] right; u8[length] third; u8[length] fourth; left; right; {group} left; right; }};\n%%end")
        } else {
            format!("%%start\nu64 length = 1;\nu8[length] first;\nu8[length] second;\nu8[length] third;\nu8[length] fourth;\nu8[] left = first;\nu8[] right = second;\n{group}\n%%end")
        };
        for layout in [ScalarTargetLayout::WASM32, ScalarTargetLayout::NATIVE64] {
            let parsed = parse_source(source.clone(), text.clone(), &[]);
            assert!(
                parsed.diagnostics.is_empty(),
                "{case}: {:?}",
                parsed.diagnostics
            );
            let single = crate::derive_scalar_program_with_layout(&parsed.result, layout);
            assert!(
                single.diagnostics.is_empty(),
                "{case}: {:?}",
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
                project.diagnostics.is_empty(),
                "{case}: {:?}",
                project.diagnostics
            );
            for (route, llvm) in [
                ("standalone", emit_scalar_llvm(&single).unwrap().to_text()),
                (
                    "project",
                    emit_scalar_project_llvm(&project).unwrap().to_text(),
                ),
            ] {
                let lines = llvm.lines().collect::<Vec<_>>();
                let slots = ["left", "right"].map(|name| {
                    if case == "local_live" {
                        format!("%{name}_backing")
                    } else if route == "standalone" {
                        format!("@{name}")
                    } else {
                        format!("@{}", project_global_name(&source, name))
                    }
                });
                let installs = slots.each_ref().map(|slot| {
                    lines
                        .iter()
                        .enumerate()
                        .filter(|(_, line)| {
                            line.contains("store ptr ") && line.contains(&format!(", ptr {slot},"))
                        })
                        .last()
                        .unwrap_or_else(|| {
                            panic!("missing {slot} install in {case} {route}: {llvm}")
                        })
                        .0
                });
                assert!(installs[0] < installs[1], "{case} {route}: {llvm}");
                let captures = lines
                    .iter()
                    .enumerate()
                    .filter(|(index, line)| {
                        *index < installs[0]
                            && line.contains("runtime_array_owner")
                            && line.contains(" = load ptr,")
                    })
                    .collect::<Vec<_>>();
                assert_eq!(captures.len(), expected_previous, "{case} {route}: {llvm}");
                let first_release = lines
                    .iter()
                    .position(|line| line.contains("call void @__wosy_core_free"))
                    .unwrap();
                if case == "local_live" {
                    let live = lines
                        .iter()
                        .enumerate()
                        .filter(|(index, line)| {
                            *index < installs[0]
                                && line.contains("allocation_live")
                                && line.contains(" = load i1,")
                        })
                        .collect::<Vec<_>>();
                    assert_eq!(live.len(), 2, "{case} {route}: {llvm}");
                    for (index, line) in live {
                        assert!(index < first_release, "{case} {route}: {llvm}");
                        let bit = line.split_whitespace().next().unwrap();
                        assert_eq!(
                            lines
                                .iter()
                                .filter(|line| line
                                    .contains(&format!("br i1 {bit}, label %release.live")))
                                .count(),
                            1,
                            "{case} {route}: {llvm}"
                        );
                    }
                }
                for (index, line) in captures {
                    assert!(index < first_release, "{case} {route}: {llvm}");
                    let pointer = line.split_whitespace().next().unwrap();
                    assert_eq!(
                        lines
                            .iter()
                            .filter(|line| line
                                .contains(&format!("call void @__wosy_core_free(ptr {pointer})")))
                            .count(),
                        1,
                        "{case} {route}: {llvm}"
                    );
                }
                assert_eq!(
                    llvm.matches("call void @__wosy_core_free").count(),
                    expected_total,
                    "{case} {route}: {llvm}"
                );
                for slot in &slots {
                    let remaining = lines
                        .iter()
                        .enumerate()
                        .filter(|(index, line)| {
                            *index > installs[1]
                                && line.contains("runtime_array_owner")
                                && line.contains(&format!("load ptr, ptr {slot},"))
                        })
                        .collect::<Vec<_>>();
                    assert_eq!(
                        remaining.len(),
                        1,
                        "scope owner {slot} in {case} {route}: {llvm}"
                    );
                    let pointer = remaining[0].1.split_whitespace().next().unwrap();
                    assert_eq!(
                        lines
                            .iter()
                            .filter(|line| line
                                .contains(&format!("call void @__wosy_core_free(ptr {pointer})")))
                            .count(),
                        1,
                        "scope release {slot} in {case} {route}: {llvm}"
                    );
                }
                if case == "transfer" || case == "crossed" {
                    assert!(
                        !lines[..installs[0]]
                            .iter()
                            .any(|line| line.contains("runtime_array_owner")
                                && line.contains(&format!("load ptr, ptr {},", slots[1]))),
                        "transferred right released: {llvm}"
                    );
                }
                println!("{case} {layout:?} {route}: captured previous releases={expected_previous}, total releases={expected_total}");
            }
        }
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
fn serializes_imports_in_metadata_order_without_changing_body_lines() {
    let attribute = |name: &str, group| LlvmFunction {
        name: name.into(),
        result: LlvmValueType::Void,
        parameters: Vec::new(),
        attributes: vec![LlvmFunctionAttributes {
            group,
            wasm_import_module: "imports".into(),
            wasm_import_name: format!("import{group}"),
        }],
        body: String::new(),
    };
    let source = "; ModuleID = 'test'\r\nsource_filename = \"test\"\r\n";
    for name in ["\"quoted\"", "\"quoted(@name)\""] {
        let raw = format!(
            "{source}\r\ndeclare void @first_extra()\r\ndeclare void @first()\r\ndeclare void @{name}()\r\ndeclare void @first() ; second instance\r\n\r\ndefine void @main() {{\r\n  ret void\r\n}}\r\n\r\n"
        );
        let partition = LlvmPartition {
            module_name: "test".into(),
            declarations: vec![attribute(name, 7), attribute("first", 3)],
            functions: Vec::new(),
            text: raw,
        };
        let expected = format!(
            "; ModuleID = 'test'\nsource_filename = \"test\"\n\ndeclare void @{name}() #7\ndeclare void @first() #3\n\nattributes #3 = {{ \"wasm-import-module\"=\"imports\" \"wasm-import-name\"=\"import3\" }}\nattributes #7 = {{ \"wasm-import-module\"=\"imports\" \"wasm-import-name\"=\"import7\" }}\n\ndeclare void @first_extra()\n\ndefine void @main() {{\n  ret void\n}}\n"
        );
        let mut streamed = Vec::new();
        partition.write_text(&mut streamed).expect("write extern declarations");
        assert_eq!(streamed, expected.as_bytes(), "{name}");
        assert_eq!(partition.to_text(), expected, "{name}");
    }
}

#[test]
fn serializes_without_externs_preserving_line_endings_and_separator() {
    let cases = [
        ("", ""),
        ("first\nsecond", "first\nsecond"),
        ("\r\nfirst\r\nlast\r", "\nfirst\nlast\r"),
        ("source_filename = \"m\"\n\n\nbody\n", "source_filename = \"m\"\n\nbody\n"),
        ("source_filename = \"m\"\r\n\r\n\r\n", "source_filename = \"m\"\n\n"),
    ];
    for (raw, expected) in cases {
        let partition = LlvmPartition {
            module_name: "test".into(),
            declarations: Vec::new(),
            functions: Vec::new(),
            text: raw.into(),
        };
        assert_eq!(partition.to_text(), expected);
    }
}

#[test]
fn preserves_callable_return_metadata_for_every_output_form() {
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

    assert_eq!(
        function_value_result(&sequence(Vec::new())).expect("zero-output function type"),
        LlvmValueType::Void
    );
    assert_eq!(
        function_value_result(&sequence(vec![ScalarType::I32])).expect("direct function type"),
        LlvmValueType::I32
    );
    assert_eq!(
        function_value_result(&sequence(vec![ScalarType::I32, ScalarType::Bool])).expect("aggregate function type"),
        LlvmValueType::Aggregate(vec![LlvmValueType::I32, LlvmValueType::I1])
    );
    assert_eq!(
        function_value_result(&sequence(vec![ScalarType::Unit, ScalarType::I32])).expect("unit aggregate function type"),
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
                "%%start\ncast = overload {\n    i32(i64) => fn(value) { 1 };\n    generic T;\n    T(T) => fn(value) { value };\n};\npair = overload {\n    generic T;\n    (T, T)(T) => fn(value) { value, value };\n};\ni64 wide = 1;\ni32 narrow = cast(wide);\nbool flag = true;\nbool copied = cast(flag);\ni64 first, i64 second = pair(wide);\n%%end"
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
fn emits_explicit_generic_overload_returns_in_both_routes_and_targets() {
    let text = "%%start
pair = overload {
    generic T;
    (bool, T)(T) => fn(input) { true, input };
};
i64 value = 17;
bool flag, i64 number = pair(value);
%%end";
    for layout in [ScalarTargetLayout::NATIVE64, ScalarTargetLayout::WASM32] {
        let prepared = prepare_initializer_test(text, layout);
        let build = analyze_scalar_build(&prepared);
        assert!(build.diagnostics().is_empty(), "{:?}", build.diagnostics());
        for (route, llvm) in [
            ("single", emit_scalar_build_single_module_llvm(&build)),
            ("project", emit_scalar_build_project_llvm(&build)),
        ] {
            let llvm = llvm
                .unwrap_or_else(|error| panic!("{route} {layout:?}: {error}"))
                .to_text();
            assert!(
                llvm.contains("define { i1, i64 } @wosy_generic__"),
                "{llvm}"
            );
            assert!(llvm.contains("call { i1, i64 } @wosy_generic__"), "{llvm}");
            assert!(llvm.contains("ret { i1, i64 }"), "{llvm}");
            for position in 0..2 {
                assert!(
                    llvm.contains(&format!("extractvalue {{ i1, i64 }} %call, {position}")),
                    "{llvm}"
                );
            }
        }
    }
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
fn rejects_source_invalid_utf8_rewrite_in_typed_call_arguments() {
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
    assert_eq!(
        emit_scalar_llvm(&program).expect_err("source-invalid typed rewrite must be rejected"),
        "cannot emit LLVM for an invalid scalar program",
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
fn lowers_runtime_float_negation_as_ieee_fneg() {
    let text = r#"%%start
f32(f32) narrow = fn(value) { -value };
f64(f64) wide = fn(value) { -(value + value) };
f64(f64) flip_wide = fn(value) { -value };
i32(i32) integer = fn(value) { ~(-value) };
f32 positive32 = 0.0;
f32 negative32 = -0.0;
f64 positive64 = 0.0;
f64 negative64 = -0.0;
u32 sign32 = core.bitcast<u32>(narrow(positive32));
u32 clear32 = core.bitcast<u32>(narrow(negative32));
u64 sign64 = core.bitcast<u64>(flip_wide(positive64));
u64 clear64 = core.bitcast<u64>(flip_wide(negative64));
wide(positive64);
integer(2);
%%end"#;
    for layout in [ScalarTargetLayout::NATIVE64, ScalarTargetLayout::WASM32] {
        let prepared = prepare_initializer_test(text, layout);
        let checked = analyze_scalar_build(&prepared);
        assert!(checked.diagnostics().is_empty(), "{:?}", checked.diagnostics());
        let source = SourceIdentity::new("project".into(), "package".into(), "src/main.w".into(), "r1".into());
        let standalone = derive_scalar_program_with_layout(
            &parse_source(source, text.into(), &[]).result,
            layout,
        );
        assert!(standalone.diagnostics.is_empty(), "{:?}", standalone.diagnostics);
        for partition in [
            emit_scalar_llvm(&standalone),
            emit_scalar_build_single_module_llvm(&checked),
            emit_scalar_build_project_llvm(&checked),
        ] {
            let ir = partition.unwrap().to_text();
            for expected in [
                "fneg float", "fneg double", "fadd double",
                "bitcast float", "bitcast double", "sub i32 0", "xor i32",
                "store float 0.000000e+00", "store float -0.000000e+00",
                "store double 0.000000e+00", "store double -0.000000e+00",
            ] {
                assert!(ir.contains(expected), "{layout:?}, missing {expected}: {ir}");
            }
            assert!(!ir.contains("fsub"), "{layout:?}: {ir}");
        }
    }
}

#[test]
fn emits_float_conversions_of_width_inferred_arithmetic() {
    let source = SourceIdentity::new(
        "project".into(),
        "package".into(),
        "src/main.w".into(),
        "r1".into(),
    );
    let text = "%%start
i32(f32, f32) narrow = fn(a, b) { core.float_to_sint_trunc<i32>(a / b) };
u64(f64, f64) wide = fn(a, b) { core.float_to_uint_trunc<u64>(a * b) };
narrow(1.5, 2.0);
wide(1.5, 2.0);
%%end";
    let program = derive_scalar_program(&parse_source(source.clone(), text.into(), &[]).result);
    assert!(program.diagnostics.is_empty(), "{:?}", program.diagnostics);
    let single = emit_scalar_llvm(&program).unwrap().to_text();
    let validation = validate_scalar_project(ScalarProject::new(
        vec![ScalarModule::from_program(program.program, Vec::new())],
        vec![source],
    ));
    assert!(validation.diagnostics.is_empty(), "{:?}", validation.diagnostics);
    let project = emit_scalar_project_llvm(&validation).unwrap().to_text();
    for ir in [single, project] {
        for operation in ["fdiv float", "fptosi float", "fmul double", "fptoui double"] {
            assert!(ir.contains(operation), "{operation}: {ir}");
        }
        for operation in ["bitcast", "fptrunc", "fpext"] {
            assert!(!ir.contains(operation), "{operation}: {ir}");
        }
    }
}

#[test]
fn infers_float_width_for_literal_arithmetic_in_conversion_operands() {
    let text = r#"%%start
i32(f32) cell = fn(x) {
    core.float_to_sint_trunc<i32>((x + 344.0) / 56.0)
};
i32(f32) opposite = fn(x) {
    core.float_to_sint_trunc<i32>((2.0 + 3.0) - x)
};
u64(f64) wide = fn(x) {
    core.float_to_uint_trunc<u64>(x * 2.0 % 7.0)
};
u64(f64) wide_opposite = fn(x) {
    core.float_to_uint_trunc<u64>(10.0 / x)
};
i32 a = cell(7.0);
i32 b = opposite(2.0);
u64 c = wide(3.0);
u64 d = wide_opposite(2.0);
%%end"#;
    for layout in [ScalarTargetLayout::NATIVE64, ScalarTargetLayout::WASM32] {
        let source = SourceIdentity::new("project".into(), "package".into(), "src/main.w".into(), "r1".into());
        let standalone = derive_scalar_program_with_layout(
            &parse_source(source, text.into(), &[]).result,
            layout,
        );
        assert!(standalone.diagnostics.is_empty(), "{:?}", standalone.diagnostics);
        let prepared = prepare_initializer_test(text, layout);
        let checked = analyze_scalar_build(&prepared);
        assert!(checked.diagnostics().is_empty(), "{:?}", checked.diagnostics());
        for partition in [
            emit_scalar_llvm(&standalone),
            emit_scalar_build_single_module_llvm(&checked),
            emit_scalar_build_project_llvm(&checked),
        ] {
            let ir = partition.unwrap().to_text();
            for operation in [
                "fadd float", "fdiv float", "fsub float", "fptosi float",
                "fmul double", "frem double", "fdiv double", "fptoui double",
            ] {
                assert!(ir.contains(operation), "{layout:?}, missing {operation}: {ir}");
            }
            for forbidden in ["fpext", "fptrunc", "fptosi double", "fptoui float"] {
                assert!(!ir.contains(forbidden), "{layout:?}, unexpected {forbidden}: {ir}");
            }
        }
    }
}

#[test]
fn reports_ambiguous_or_mixed_float_literal_arithmetic_before_emission() {
    for argument in ["1.0 + (2.0 * 3.0)", "x + 1.0", "1.0 + x"] {
        let source = SourceIdentity::new("project".into(), "package".into(), "src/main.w".into(), "r1".into());
        let text = format!(
            "%%start\ni32(i32) convert = fn(x) {{ core.float_to_sint_trunc<i32>({argument}) }};\nconvert(2);\n%%end",
        );
        let parsed = parse_source(source.clone(), text, &[]);
        assert!(parsed.diagnostics.is_empty(), "{:?}", parsed.diagnostics);
        let standalone = derive_scalar_program(&parsed.result);
        let project = prepare_scalar_project(
            vec![(derive_scalar_declarations_from_cst(parsed.result.canonical_cst()), Vec::new())],
            vec![source],
            ScalarPreparationInputs { target_layout: ScalarTargetLayout::WASM32 },
        );
        let project_diagnostics = match project {
            Ok(prepared) => analyze_scalar_build(&prepared).diagnostics().to_vec(),
            Err(error) => error.diagnostics.clone(),
        };
        for diagnostics in [standalone.diagnostics, project_diagnostics] {
            assert!(diagnostics.iter().any(|diagnostic|
                diagnostic.code == "B0003"
                    && diagnostic.message == "floating-point arithmetic requires a matching f32 or f64 context"
            ), "{argument}: {diagnostics:?}");
        }
    }
}

#[test]
fn emits_integer_conversions_with_checked_indexed_operand_types() {
    let text = r#"%%start
i32[2] stage = [-17, 0];
u32[2] unsigned_stage = [4000000000, 0];
u8() word = fn { core.int_trunc<u8>(stage[0] % 256) };
i64() signed_word = fn { core.int_extend<i64>(stage[0]) };
u64() unsigned_word = fn { core.int_extend<u64>(unsigned_stage[0]) };
u8 body_trunc = word();
i64 body_signed = signed_word();
u64 body_unsigned = unsigned_word();
u8 initializer_trunc = core.int_trunc<u8>(stage[0] % 256);
i64 initializer_signed = core.int_extend<i64>(stage[0]);
u64 initializer_unsigned = core.int_extend<u64>(unsigned_stage[0]);
f64 initializer_float = core.uint_to_float<f64>(unsigned_stage[0]);
%%end"#;
    for layout in [ScalarTargetLayout::NATIVE64, ScalarTargetLayout::WASM32] {
        let prepared = prepare_initializer_test(text, layout);
        let checked = analyze_scalar_build(&prepared);
        assert!(checked.diagnostics().is_empty(), "{:?}", checked.diagnostics());
        for partition in [
            emit_scalar_build_single_module_llvm(&checked),
            emit_scalar_build_project_llvm(&checked),
        ] {
            let ir = partition.unwrap().to_text();
            assert_eq!(ir.matches("srem i32").count(), 2, "{layout:?}: {ir}");
            for (op, destination) in [("trunc", "i8"), ("sext", "i64"), ("zext", "i64")] {
                assert_eq!(ir.lines().filter(|line| line.contains(" = ")
                    && line.contains("int_")
                    && line.contains(&format!(" = {op} i32 "))
                    && line.ends_with(&format!(" to {destination}"))).count(), 2, "{layout:?}: {ir}");
            }
            assert!(ir.lines().any(|line| line.contains(" = uitofp i32 ")
                    && line.ends_with(" to double")), "{layout:?}: {ir}");
        }
    }
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
fn extern_import_module_roundtrips_escaped_llvm_attribute_bytes() {
    use inkwell::{attributes::AttributeLoc, memory_buffer::MemoryBuffer};

    let source = SourceIdentity::new(
        "project".into(),
        "package".into(),
        "src/main.w".into(),
        "r1".into(),
    );
    let validation = derive_scalar_program(
        &parse_source(
            source,
            r#"%%start
env = extern wasm "café\" tools\\path\n\t" { i32() read; };
i32 value = env.read();
%%end"#
                .into(),
            &[],
        )
        .result,
    );
    assert!(validation.diagnostics.is_empty(), "{:?}", validation.diagnostics);
    let partition = emit_scalar_llvm(&validation).expect("valid Wosy import");
    let [declaration] = partition.declarations.as_slice() else {
        panic!("expected one extern declaration")
    };
    let ir = partition.to_text();
    assert!(ir.contains(r#"wasm-import-module"="caf\C3\A9\22 tools\5Cpath\0A\09""#));
    let buffer = MemoryBuffer::create_from_memory_range_copy(ir.as_bytes(), "extern-attribute");
    let context = Context::create();
    let module = context.create_module_from_ir(buffer).expect("parse emitted LLVM");
    module.verify().expect("verify emitted LLVM");
    let function = module.get_function(&declaration.name).expect("extern symbol");
    for (key, expected) in [
        ("wasm-import-module", "café\" tools\\path\n\t"),
        ("wasm-import-name", "read"),
    ] {
        let attribute = function
            .get_string_attribute(AttributeLoc::Function, key)
            .expect("extern string attribute");
        assert_eq!(attribute.get_string_value().to_bytes(), expected.as_bytes());
    }
}

#[test]
fn extern_import_groups_do_not_replace_llvm_function_attributes() {
    use inkwell::{attributes::AttributeLoc, memory_buffer::MemoryBuffer};

    let source = SourceIdentity::new(
        "project".into(),
        "package".into(),
        "src/main.w".into(),
        "r1".into(),
    );
    let text = "%%start
scene = extern wasm \"showcase\" { unit() mesh; };
events = extern wasm \"events\" { unit() frame; };
u8[2] row = [1, 2];
u8[2][2] grid = [row, row];
scene.mesh();
events.frame();
%%end";
    let parsed = parse_source(source.clone(), text.into(), &[]);
    let single = derive_scalar_program_with_layout(&parsed.result, ScalarTargetLayout::WASM32);
    assert!(single.diagnostics.is_empty(), "{:?}", single.diagnostics);
    let single_partition = emit_scalar_llvm(&single).unwrap();
    let project = validate_scalar_project(ScalarProject::new(
        vec![ScalarModule::from_program(single.program, Vec::new())],
        vec![source],
    ));
    assert!(project.diagnostics.is_empty(), "{:?}", project.diagnostics);
    let project_partition = emit_scalar_project_llvm(&project).unwrap();

    for partition in [single_partition, project_partition] {
        let ir = partition.to_text();
        let buffer = MemoryBuffer::create_from_memory_range_copy(ir.as_bytes(), "extern-groups");
        let context = Context::create();
        let module = context.create_module_from_ir(buffer).expect("parse emitted LLVM");
        module.verify().expect("verify emitted LLVM");
        assert_eq!(partition.declarations.len(), 2, "{ir}");
        for (declaration, (module_name, import_name)) in partition
            .declarations
            .iter()
            .zip([("showcase", "mesh"), ("events", "frame")])
        {
            let attributes = &declaration.attributes[0];
            assert_eq!(attributes.wasm_import_module, module_name);
            assert_eq!(attributes.wasm_import_name, import_name);
            assert!(
                ir.contains(&format!("declare void @{}() #{}", declaration.name, attributes.group)),
                "{ir}"
            );
            let function = module.get_function(&declaration.name).unwrap();
            for (key, expected) in [
                ("wasm-import-module", module_name),
                ("wasm-import-name", import_name),
            ] {
                let attribute = function.get_string_attribute(AttributeLoc::Function, key).unwrap();
                assert_eq!(attribute.get_string_value().to_bytes(), expected.as_bytes());
            }
        }
        assert!(module.get_functions().any(|function| {
            !function.is_null()
                && function.count_basic_blocks() > 0
                && function
                    .get_string_attribute(AttributeLoc::Function, "target-features")
                    .is_some_and(|attribute| attribute.get_string_value().to_bytes() == b"+bulk-memory")
        }), "{ir}");
        assert!(ir.contains("attributes #") && ir.contains("memory(argmem: readwrite)"), "{ir}");
    }
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
fn emits_canonical_tail_conditionals_in_both_routes_and_targets() {
    let source = SourceIdentity::new(
        "project".into(),
        "package".into(),
        "src/main.w".into(),
        "r1".into(),
    );
    let parse = |outputs: &str| {
        parse_source(
            source.clone(),
            format!(
                "%%start\nbool() choose = fn {{ true }};\n{outputs:11}() pair = fn {{ if (choose()) {{ if (true) {{ 17, true }} else {{ 42, false }} }} else {{ 8, true }} }};\n%%end"
            ),
            &[],
        )
    };
    let parsed = parse("(i32, bool)");
    // The one-output declaration preserves the raw body during extraction; the
    // output field's fixed width keeps the original source spans.
    let raw = parse("i32");
    assert!(raw.diagnostics.is_empty(), "{:?}", raw.diagnostics);
    let raw = derive_scalar_program(&raw.result);
    let [_, ScalarItem::Function(pair)] = raw.program.items.as_slice() else {
        panic!("expected choose and pair");
    };
    assert_eq!(pair.body.final_expressions().count(), 1);

    for layout in [ScalarTargetLayout::NATIVE64, ScalarTargetLayout::WASM32] {
        let mut single = derive_scalar_program_with_layout(&parsed.result, layout);
        assert!(single.diagnostics.is_empty(), "{:?}", single.diagnostics);
        let mut project = validate_scalar_project(ScalarProject::new(
            vec![ScalarModule::from_program(
                single.program.clone(),
                Vec::new(),
            )],
            vec![source.clone()],
        ));
        assert!(project.diagnostics.is_empty(), "{:?}", project.diagnostics);
        for items in [
            &mut single.program.items,
            &mut project.project.modules[0].items,
        ] {
            let [_, ScalarItem::Function(function)] = items.as_mut_slice() else {
                panic!("expected choose and pair");
            };
            assert_eq!(function.body.span, pair.body.span);
            function.body = pair.body.clone();
        }

        for llvm in [
            emit_scalar_llvm(&single)
                .expect("canonical single LLVM")
                .to_text(),
            emit_scalar_project_llvm(&project)
                .expect("canonical project LLVM")
                .to_text(),
        ] {
            assert_eq!(llvm.matches("call i1").count(), 1, "{llvm}");
            assert_eq!(llvm.matches("phi { i32, i1 }").count(), 2, "{llvm}");
            assert!(llvm.contains("{ i32 17, i1 true }"), "{llvm}");
            assert!(llvm.contains("{ i32 42, i1 false }"), "{llvm}");
            assert!(llvm.contains("{ i32 8, i1 true }"), "{llvm}");
            assert!(llvm.contains("ret { i32, i1 }"), "{llvm}");
        }
    }
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
fn assignment_destinations_are_retained_before_any_write_on_both_targets() {
    let source = SourceIdentity::new(
        "project".into(),
        "package".into(),
        "src/main.w".into(),
        "r1".into(),
    );
    let text = "%%start\nunit() check = fn { u8 first = 1; u8 second = 2; *!u8 selected = &!first; selected, *selected = &!second, 9; u8 observed = *selected; u8 first_expected = 9; u8 observed_expected = 2; if (first != first_expected || observed != observed_expected) { core.system_panic(); }; };\ncheck();\n%%end";
    for (layout, pointer_width) in [
        (ScalarTargetLayout::WASM32, "i32"),
        (ScalarTargetLayout::NATIVE64, "i64"),
    ] {
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
        for llvm in [
            emit_scalar_llvm(&single).unwrap().to_text(),
            emit_scalar_project_llvm(&project).unwrap().to_text(),
        ] {
            let lines = llvm.lines().collect::<Vec<_>>();
            let selected_stores = lines
                .iter()
                .enumerate()
                .filter_map(|(index, line)| {
                    (line.contains(&format!("store {pointer_width} "))
                        && line.contains(", ptr %selected,"))
                    .then_some(index)
                })
                .collect::<Vec<_>>();
            assert_eq!(selected_stores.len(), 2, "{llvm}");
            let byte_write = lines
                .iter()
                .position(|line| line.contains("store i8 ") && line.contains(", ptr %deref,"))
                .unwrap_or_else(|| panic!("retained byte destination: {llvm}"));
            let selector_loads = lines
                .iter()
                .enumerate()
                .filter(|(index, line)| {
                    *index > selected_stores[0]
                        && *index < byte_write
                        && line.contains(&format!("load {pointer_width}, ptr %selected,"))
                })
                .collect::<Vec<_>>();
            assert_eq!(selector_loads.len(), 1, "{llvm}");
            let (selector_index, selector_line) = selector_loads[0];
            assert!(selected_stores[0] < selector_index, "{llvm}");
            assert!(selector_index < selected_stores[1], "{llvm}");
            assert!(selected_stores[1] < byte_write, "{llvm}");
            let selector_value = selector_line.split_whitespace().next().unwrap();
            let conversion = lines
                .iter()
                .enumerate()
                .find(|(_, line)| {
                    line.contains(&format!("inttoptr {pointer_width} {selector_value} to ptr"))
                })
                .expect("retained selector pointer");
            assert!(selector_index < conversion.0, "{llvm}");
            assert!(conversion.0 < selected_stores[1], "{llvm}");
            let retained_pointer = conversion.1.split_whitespace().next().unwrap();
            let staged_nine = lines
                .iter()
                .position(|line| line.contains("store i8 9, ptr %assignment_value"))
                .expect("staged byte replacement");
            let replacement_slot = lines[staged_nine]
                .split_once(", ptr ")
                .unwrap()
                .1
                .split(',')
                .next()
                .unwrap();
            let replacement_load = lines
                .iter()
                .enumerate()
                .find(|(_, line)| line.contains(&format!("load i8, ptr {replacement_slot},")))
                .expect("materialized byte replacement");
            assert!(staged_nine < replacement_load.0, "{llvm}");
            assert!(replacement_load.0 < selector_index, "{llvm}");
            let replacement = replacement_load.1.split_whitespace().next().unwrap();
            assert!(
                lines[byte_write]
                    .contains(&format!("store i8 {replacement}, ptr {retained_pointer},")),
                "{llvm}"
            );
        }
    }
}

#[test]
fn fixed_bool_array_places_use_target_width_addresses() {
    let text = "%%start\nbool[2] ready = [false, false];\nu64(u64) select_index = fn(index) { index };\nbool(u64) set_ready = fn(index) { ready[select_index(index)] = true; ready[index] };\n%%end";
    for target_layout in [ScalarTargetLayout::NATIVE64, ScalarTargetLayout::WASM32] {
        let source = SourceIdentity::new(
            "project".into(), "package".into(), "src/main.w".into(), "r1".into(),
        );
        let parsed = parse_source(source.clone(), text.into(), &[]);
        assert!(parsed.diagnostics.is_empty(), "{:?}", parsed.diagnostics);
        let prepared = prepare_scalar_project(
            vec![(derive_scalar_declarations_from_cst(parsed.result.canonical_cst()), Vec::new())],
            vec![source],
            ScalarPreparationInputs { target_layout },
        ).unwrap_or_else(|invalid| panic!("{:?}", invalid.diagnostics));
        let build = analyze_scalar_build(&prepared);
        assert!(build.diagnostics().is_empty(), "{:?}", build.diagnostics());
        let llvm = emit_scalar_build_single_module_llvm(&build).expect("fixed bool array LLVM").to_text();
        let body = llvm.split_once("define i1 @set_ready(").expect("bool function").1
            .split_once("\n}").expect("bool body").0;
        assert_eq!(body.matches("call i64 @select_index(").count(), 1, "{body}");
        let addresses = body.lines().filter(|line| {
            line.contains("getelementptr inbounds [2 x i1]")
        }).collect::<Vec<_>>();
        assert_eq!(addresses.len(), 2, "{body}");
        if target_layout == ScalarTargetLayout::WASM32 {
            assert_eq!(body.lines().filter(|line| {
                line.contains("trunc i64") && line.contains(" to i32")
            }).count(), 2, "{body}");
            assert!(addresses.iter().all(|line| line.contains(", i32 %array_address_index")), "{body}");
        } else {
            assert!(!body.contains("trunc i64"), "{body}");
            assert!(addresses.iter().all(|line| line.contains(", i64 %")), "{body}");
        }
    }
}

#[test]
fn indexed_copy_aggregate_assignment_guards_the_single_saved_selector() {
    let text = "%%start\nstruct copy Entry { u8 value; }\nEntry[2] entries = [{ .value = 1; }, { .value = 2; }];\nu64(u64) select_index = fn(index) { index };\nu8(u8) replacement = fn(value) { value };\nunit(u64, u8) update = fn(index, value) { entries[select_index(index)] = { .value = replacement(value); }; };\n%%end";
    for target_layout in [ScalarTargetLayout::NATIVE64, ScalarTargetLayout::WASM32] {
        let source = SourceIdentity::new("project".into(), "package".into(), "src/main.w".into(), "r1".into());
        let parsed = parse_source(source.clone(), text.into(), &[]);
        assert!(parsed.diagnostics.is_empty(), "{:?}", parsed.diagnostics);
        let prepared = prepare_scalar_project(
            vec![(derive_scalar_declarations_from_cst(parsed.result.canonical_cst()), Vec::new())],
            vec![source],
            ScalarPreparationInputs { target_layout },
        ).unwrap_or_else(|invalid| panic!("{:?}", invalid.diagnostics));
        let build = analyze_scalar_build(&prepared);
        assert!(build.diagnostics().is_empty(), "{:?}", build.diagnostics());
        let llvm = emit_scalar_build_single_module_llvm(&build).expect("checked assignment LLVM").to_text();
        let body = llvm.split_once("define void @update(").expect("update function").1
            .split_once("\n}").expect("update body").0;
        let selector = "call i64 @select_index(";
        let rhs = body.find("call i8 @replacement(").expect("replacement");
        let selector_at = body.find(selector).expect("selector");
        assert_eq!(body.matches(selector).count(), 1);
        let check = body.find("icmp uge i64").expect("full-width checked selector");
        let branch = body.find("br i1 %assignment_index_out_of_bounds").expect("guard branch");
        let continuation = body.find("\nassignment_index_continue:").expect("valid continuation");
        let address = body[continuation..].find("getelementptr inbounds").expect("valid destination") + continuation;
        assert!(rhs < selector_at && selector_at < check && check < branch && branch < continuation && continuation < address, "{body}");
        assert!(body[..continuation].contains("call void @__wosy_core_system_panic()"), "{body}");
        assert!(body[address..].contains("@llvm.memmove"), "{body}");
    }
}

#[test]
fn assignment_selector_effects_execute_once_after_rhs_on_both_targets() {
    for (target_layout, pointer_type, text) in [
        (
            ScalarTargetLayout::NATIVE64,
            "i64",
            include_str!("../../../../tests/fixtures/build/static_assignment_destinations_native/project/src/main.w"),
        ),
        (
            ScalarTargetLayout::WASM32,
            "i32",
            include_str!("../../../../tests/fixtures/build/static_assignment_destinations_wasm/project/src/main.w"),
        ),
    ] {
        let source = SourceIdentity::new(
            "project".into(),
            "package".into(),
            "src/main.w".into(),
            "r1".into(),
        );
        let parsed = parse_source(source.clone(), text.into(), &[]);
        assert!(parsed.diagnostics.is_empty(), "{:?}", parsed.diagnostics);
        let prepared = prepare_scalar_project(
            vec![(
                derive_scalar_declarations_from_cst(parsed.result.canonical_cst()),
                Vec::new(),
            )],
            vec![source],
            ScalarPreparationInputs { target_layout },
        )
        .unwrap_or_else(|invalid| panic!("{:?}", invalid.diagnostics));
        let build = analyze_scalar_build(&prepared);
        assert!(build.diagnostics().is_empty(), "{:?}", build.diagnostics());
        let llvm = emit_scalar_build_single_module_llvm(&build)
            .expect("selector witness LLVM")
            .to_text();
        assert_eq!(llvm.matches("call void @check_selector_effects()").count(), 1);
        let (body, _) = llvm
            .split_once("define void @check_selector_effects()")
            .expect("selector witness function")
            .1
            .split_once("\n}")
            .expect("end of selector witness function");
        let lines = body.lines().collect::<Vec<_>>();
        let once = |instruction: &str| {
            let mut matches = lines
                .iter()
                .enumerate()
                .filter(|(_, line)| line.contains(instruction));
            let matched = matches
                .next()
                .unwrap_or_else(|| panic!("{instruction}: {body}"));
            assert!(matches.next().is_none(), "{instruction}: {body}");
            matched
        };
        let value = |line: &str| line.split_whitespace().next().unwrap().to_owned();
        let (rhs, _) = once("call i8 @replacement()");
        let (selector, selector_call) = once("call i64 @select_index(");
        let selector_input = selector_call
            .split_once("@select_index(i64 ")
            .unwrap()
            .1
            .strip_suffix(')')
            .unwrap();
        let (index_read, _) = once(&format!("{selector_input} = load i64, ptr %index,"));
        let selector_value = value(selector_call);
        let narrowing = format!("trunc i64 {selector_value} to i32");
        let (address_index, narrowed_at) = if target_layout == ScalarTargetLayout::WASM32 {
            let (index, instruction) = once(&narrowing);
            (value(instruction), Some(index))
        } else {
            assert!(!body.contains(&narrowing), "{body}");
            (selector_value, None)
        };
        let gep_operand = format!("i32 0, {pointer_type} {address_index}");
        let (element, element_address) = once(&gep_operand);
        assert!(element_address.contains("getelementptr inbounds [2 x i8]"), "{body}");
        assert!(element_address.trim_end().ends_with(&gep_operand), "{body}");
        if let Some(narrowed) = narrowed_at {
            assert!(selector < narrowed && narrowed < element, "{body}");
        }
        let (address, checked_address) = once(&format!(
            "ptrtoint ptr {} to {pointer_type}",
            value(element_address)
        ));
        let (retain, retained_item) = once(&format!("call {pointer_type} @retain_item("));
        assert!(
            retained_item.contains(&format!("{pointer_type} {}", value(checked_address))),
            "{body}"
        );
        let (freeze, destination) = once(&format!(
            "inttoptr {pointer_type} {} to ptr",
            value(retained_item)
        ));
        let index_stores = lines
            .iter()
            .enumerate()
            .filter(|(_, line)| line.contains("store i64 ") && line.contains(", ptr %index,"))
            .map(|(index, _)| index)
            .collect::<Vec<_>>();
        let [index_init, index_write] = index_stores.as_slice() else {
            panic!("expected initialized index and one assignment write: {body}");
        };
        let (item_write, store) = once(&format!(", ptr {},", value(destination)));
        assert!(store.contains("store i8 "), "{body}");
        let sequence = [
            *index_init,
            rhs,
            index_read,
            selector,
            element,
            address,
            retain,
            freeze,
            *index_write,
            item_write,
        ];
        assert!(sequence.windows(2).all(|pair| pair[0] < pair[1]), "{body}");
    }
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
fn emits_local_extern_and_public_import_with_the_same_symbol_on_both_targets() {
    let source = |path: &str| {
        SourceIdentity::new("project".into(), "package".into(), path.into(), "r1".into())
    };
    let root = source("src/main.w");
    let library = source("src/library.w");
    let declarations = |source: &SourceIdentity, text: &str| {
        let parsed = parse_source(source.clone(), text.into(), &[]);
        assert!(parsed.diagnostics.is_empty(), "{:?}", parsed.diagnostics);
        derive_scalar_declarations_from_cst(parsed.result.canonical_cst())
    };
    for target_layout in [ScalarTargetLayout::NATIVE64, ScalarTargetLayout::WASM32] {
        let main = declarations(
            &root,
            "%%start
disk = namespace package \"src/library.w\";
peer = namespace package \"src/library.w\";
i32 first = disk.read(7);
i32 second = peer.read(8);
%%end",
        );
        let namespaces = main
            .namespaces()
            .map(|namespace| ScalarNamespaceBinding {
                binding: namespace.binding.clone(),
                target: library.clone(),
                span: namespace.span,
            })
            .collect();
        let dependency = declarations(
            &library,
            "%%start
i32(i32) read = fn(value) { disk.read() + value };
disk = extern wasm \"disk_abi\" { i32() read; };
%%end",
        );
        let prepared = prepare_scalar_project(
            vec![(main, namespaces), (dependency, Vec::new())],
            vec![root.clone(), library.clone()],
            ScalarPreparationInputs { target_layout },
        )
        .unwrap_or_else(|invalid| panic!("{target_layout:?}: {:?}", invalid.diagnostics));
        let build = analyze_scalar_build(&prepared);
        assert!(build.diagnostics().is_empty(), "{:?}", build.diagnostics());
        let partition = emit_scalar_build_project_llvm(&build).expect("project LLVM");
        let text = partition.to_text();
        let public = project_function_name(&library, "read");
        for value in [7, 8] {
            assert!(
                text.contains(&format!("call i32 @{public}(i32 {value})")),
                "{text}"
            );
        }
        let [external] = partition.declarations.as_slice() else {
            panic!("one extern declaration: {text}");
        };
        assert_eq!(external.attributes[0].wasm_import_module, "disk_abi");
        assert_eq!(external.attributes[0].wasm_import_name, "read");
        let call = format!("call i32 @{}()", external.name);
        let public_body = text
            .split_once(&format!("define i32 @{public}("))
            .unwrap()
            .1
            .split_once("\n}")
            .unwrap()
            .0;
        assert!(public_body.contains(&call), "{text}");
        assert_eq!(text.matches(&call).count(), 1, "{text}");
    }
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
fn detached_project_llvm_initializes_interleaved_namespace_diamond_once() {
    let source = |name| {
        SourceIdentity::new(
            "project".into(),
            "package".into(),
            format!("src/{name}.w"),
            "r1".into(),
        )
    };
    let (root, left, right, shared) = (
        source("main"),
        source("left"),
        source("right"),
        source("shared"),
    );
    let declarations = |source: &SourceIdentity, text: &str, targets: &[&SourceIdentity]| {
        let parsed = parse_source(source.clone(), text.into(), &[]);
        assert!(parsed.diagnostics.is_empty(), "{:?}", parsed.diagnostics);
        let declarations = derive_scalar_declarations_from_cst(parsed.result.canonical_cst());
        assert_eq!(declarations.namespaces().count(), targets.len());
        let bindings = declarations
            .namespaces()
            .zip(targets)
            .map(|(namespace, &target)| ScalarNamespaceBinding {
                binding: namespace.binding.clone(),
                span: namespace.span,
                target: target.clone(),
            })
            .collect();
        (declarations, bindings)
    };

    for target_layout in [ScalarTargetLayout::WASM32, ScalarTargetLayout::NATIVE64] {
        let prepared = prepare_scalar_project(
            vec![
                declarations(&shared, "%%start\ni32 value = 30;\n%%end", &[]),
                declarations(
                    &right,
                    "%%start\ncommon = namespace package \"src/shared.w\";\ni32 value = 40;\n%%end",
                    &[&shared],
                ),
                declarations(
                    &root,
                    "%%start\ni32 value = 10;\nleft = namespace package \"src/left.w\";\nvalue = 11;\nright = namespace package \"src/right.w\";\ni32 after = 12;\n%%end",
                    &[&left, &right],
                ),
                declarations(
                    &left,
                    "%%start\ni32() read = fn { 20 };\ni32 value = 20;\ncommon = namespace package \"src/shared.w\";\ni32 after = 21;\n%%end",
                    &[&shared],
                ),
            ],
            vec![root.clone(), left.clone(), shared.clone(), right.clone()],
            ScalarPreparationInputs { target_layout },
        )
        .unwrap_or_else(|invalid| panic!("{:?}", invalid.diagnostics));
        let build = analyze_scalar_build(&prepared);
        assert!(build.diagnostics().is_empty(), "{:?}", build.diagnostics());
        let detached_ir = emit_scalar_build_project_llvm(&build).unwrap().to_text();
        let legacy = validate_scalar_project(prepared.project().clone());
        assert_eq!(
            detached_ir,
            emit_scalar_project_llvm(&legacy).unwrap().to_text(),
        );

        let main = detached_ir.split("define i32 @main").nth(1).expect("main");
        let global_store = |source, name, value| {
            format!(
                "store i32 {value}, ptr @{},",
                project_global_name(source, name),
            )
        };
        let mut next = 0;
        for store in [
            global_store(&root, "value", 10),
            global_store(&left, "value", 20),
            global_store(&shared, "value", 30),
            global_store(&left, "after", 21),
            "store i32 11, ptr %assignment_value,".to_owned(),
            global_store(&right, "value", 40),
            global_store(&root, "after", 12),
        ] {
            assert_eq!(main.matches(&store).count(), 1, "{store}: {main}");
            next += main[next..]
                .find(&store)
                .unwrap_or_else(|| panic!("{store}: {main}"))
                + store.len();
        }

        let missing = super::emit_project_llvm(
            prepared.project(),
            &|module| {
                if module == 0 {
                    prepared.initialization_region(prepared.project().modules.len())
                } else {
                    prepared.initialization_region(module)
                }
            },
            super::GenericInitializerInput::PreparedRows(&prepared),
            &|reservations| build.aggregate(reservations),
            &|module, item, arm, _| build.reference_cfg(module, item, arm),
            None,
            super::PartitionMetadata::Full,
            &[],
        )
        .unwrap_err();
        assert_eq!(
            missing,
            format!(
                "internal error: missing module initialization region for {shared:?} (module 0)"
            ),
        );
    }
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
fn aligns_byte_backed_aggregate_storage_at_each_typed_producer() {
    let text = "%%start
struct copy Packet { u8 tag; *?u8 data; }
Packet first = { .tag = 7; .data = null; };
Packet[2] collection = [first, first];
*?u8(Packet) read_packet = fn(input) { *Packet view = &input; (*view).data };
u8() read_local = fn { Packet local = { .tag = 9; .data = null; }; local.tag };
%%end";
    let source = SourceIdentity::new(
        "project".into(), "package".into(), "src/main.w".into(), "r1".into(),
    );
    for (layout, alignment, size) in [
        (ScalarTargetLayout::WASM32, 4, 8),
        (ScalarTargetLayout::NATIVE64, 8, 16),
    ] {
        let parsed = parse_source(source.clone(), text.into(), &[]);
        assert!(parsed.diagnostics.is_empty(), "{:?}", parsed.diagnostics);
        let single = derive_scalar_program_with_layout(&parsed.result, layout);
        assert!(single.diagnostics.is_empty(), "{:?}", single.diagnostics);
        let project = validate_scalar_project(ScalarProject::new(
            vec![ScalarModule::from_program(single.program.clone(), Vec::new())],
            vec![source.clone()],
        ));
        assert!(project.diagnostics.is_empty(), "{:?}", project.diagnostics);
        for (project_route, llvm) in [
            (false, emit_scalar_llvm(&single).unwrap().to_text()),
            (true, emit_scalar_project_llvm(&project).unwrap().to_text()),
        ] {
            for (name, ty) in [
                ("first", format!("[{size} x i8]")),
                ("collection", format!("[2 x [{size} x i8]]")),
            ] {
                let name = if project_route {
                    project_global_name(&source, name)
                } else {
                    name.to_owned()
                };
                assert!(
                    llvm.contains(&format!(
                        "@{name} = internal global {ty} zeroinitializer, align {alignment}"
                    )),
                    "{llvm}"
                );
            }
            for (name, ty) in [
                ("struct_literal", format!("[{size} x i8]")),
                ("local", format!("[{size} x i8]")),
                ("array_literal", format!("[2 x [{size} x i8]]")),
            ] {
                assert!(
                    llvm.contains(&format!("%{name} = alloca {ty}, align {alignment}")),
                    "{llvm}"
                );
            }
            assert!(
                llvm.lines().any(|line| line.trim_start().starts_with("%input")
                    && line.contains(&format!(" = alloca [{size} x i8], align {alignment}"))),
                "{llvm}"
            );
            assert!(
                llvm.lines().any(|line| line.contains("getelementptr inbounds i8, ptr %deref")
                    && line.ends_with(&format!(", i8 {alignment}"))),
                "{llvm}"
            );
        }
    }
}

#[test]
fn preserves_large_aggregate_byte_offsets_in_literals_and_resolved_accesses() {
    let padding = vec!["0"; 127].join(", ");
    let middle = vec!["0"; 126].join(", ");
    let text = format!(
        "%%start\nstruct Wide {{ u8[127] padding; u8 a; u8 b; u8[126] middle; u8 c; u8 d; }}\nu8() probe = fn {{ Wide item = {{ .padding = [{padding}]; .a = 11; .b = 17; .middle = [{middle}]; .c = 23; .d = 31; }}; *Wide view = &item; (*view).a + (*view).b + (*view).c + (*view).d }};\n%%end"
    );
    let source = SourceIdentity::new(
        "project".into(),
        "package".into(),
        "src/main.w".into(),
        "r1".into(),
    );
    for (layout, width) in [
        (ScalarTargetLayout::WASM32, "i32"),
        (ScalarTargetLayout::NATIVE64, "i64"),
    ] {
        let parsed = parse_source(source.clone(), text.clone(), &[]);
        assert!(parsed.diagnostics.is_empty(), "{:?}", parsed.diagnostics);
        let single = derive_scalar_program_with_layout(&parsed.result, layout);
        assert!(single.diagnostics.is_empty(), "{:?}", single.diagnostics);
        let project = validate_scalar_project(ScalarProject::new(
            vec![ScalarModule::from_program(single.program.clone(), Vec::new())],
            vec![source.clone()],
        ));
        assert!(project.diagnostics.is_empty(), "{:?}", project.diagnostics);
        for llvm in [
            emit_scalar_llvm(&single).unwrap().to_text(),
            emit_scalar_project_llvm(&project).unwrap().to_text(),
        ] {
            for (offset, index_width) in [(127, "i8"), (128, width), (255, width), (256, width)] {
                let position = format!(", {index_width} {offset}");
                for base in ["%struct_literal", "%deref"] {
                    assert!(
                        llvm.lines().any(|line| {
                            line.contains(&format!("getelementptr inbounds i8, ptr {base}"))
                                && line.ends_with(&position)
                        }),
                        "byte offset {offset} from {base}: {llvm}"
                    );
                }
            }
        }
    }
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
    let first = text.find("store i8 1").expect("first field store");
    let second = text.find("store i32 2").expect("second field store");
    assert!(first < second, "{text}");
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
    assert!(forward.contains("%local = alloca [8 x i8]"), "{forward}");
    assert!(
        forward.contains("ptr align 1 %local, ptr align 1 %struct_literal"),
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
    let leaf = validation.program.layouts.aggregate(&leaf.id).unwrap();
    let grid = validation.program.layouts.aggregate(&grid.id).unwrap();

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
fn raw_final_output_ignores_redundant_expression_and_uses_actual_suffix() {
    let text = "%%start
i32(i32) first = fn(n) { n };
i32(i32) second = fn(n) { n + 1 };
i32(i32) relay = fn(n) { first(n) };
%%end";
    let source = SourceIdentity::new(
        "project".into(),
        "package".into(),
        "src/main.w".into(),
        "r1".into(),
    );
    for layout in [ScalarTargetLayout::NATIVE64, ScalarTargetLayout::WASM32] {
        let parsed = parse_source(source.clone(), text.into(), &[]);
        assert!(parsed.diagnostics.is_empty(), "{:?}", parsed.diagnostics);
        let valid = derive_scalar_program_with_layout(&parsed.result, layout);
        assert!(valid.diagnostics.is_empty(), "{:?}", valid.diagnostics);
        let project = |program: &crate::ScalarProgram| crate::ScalarProjectValidation {
            project: ScalarProject::new(
                vec![ScalarModule::from_program(program.clone(), Vec::new())],
                vec![source.clone()],
            ),
            diagnostics: Vec::new(),
        };
        let standalone = emit_scalar_llvm(&valid).unwrap();
        let project_partition = emit_scalar_project_llvm(&project(&valid.program)).unwrap();
        for (actual_suffix, selected) in [(false, "first"), (true, "second")] {
            let mut changed = crate::ScalarValidation {
                program: valid.program.clone(),
                diagnostics: Vec::new(),
            };
            let ScalarItem::Function(relay) = &mut changed.program.items[2] else {
                panic!("relay");
            };
            let body = &mut relay.body;
            let value = if actual_suffix {
                let crate::ScalarBlockItem::Expression(expression) = body.items.last_mut().unwrap()
                else {
                    panic!("canonical suffix");
                };
                expression
            } else {
                &mut body.final_output_values[0].value
            };
            let crate::ScalarExpression::Call { name, .. } = value else {
                panic!("final expression call");
            };
            assert_eq!(name, "first");
            *name = "second".into();

            let project_input = project(&changed.program);
            let raw_decoded =
                serde_json::from_slice(&serde_json::to_vec(&changed).unwrap()).unwrap();
            let project_decoded =
                serde_json::from_slice(&serde_json::to_vec(&project_input).unwrap()).unwrap();
            for (raw, project) in [(&changed, &project_input), (&raw_decoded, &project_decoded)] {
                let raw_output = emit_scalar_llvm(raw).unwrap();
                let project_output = emit_scalar_project_llvm(project).unwrap();
                assert_eq!(raw_output == standalone, !actual_suffix);
                assert_eq!(project_output == project_partition, !actual_suffix);
                let standalone_body =
                    self_tail_test_body(&raw_output.to_text(), "i32", "relay").to_owned();
                let project_body = self_tail_test_body(
                    &project_output.to_text(),
                    "i32",
                    &project_function_name(&source, "relay"),
                )
                .to_owned();
                assert_eq!(standalone_body.matches("call i32").count(), 1);
                assert_eq!(project_body.matches("call i32").count(), 1);
                assert!(standalone_body.contains(&format!("call i32 @{selected}(")));
                assert!(project_body.contains(&format!(
                    "call i32 @{}(",
                    project_function_name(&source, selected)
                )));
            }
        }
    }
}

#[test]
fn raw_call_selection_uses_checked_occurrences_instead_of_ast_mirrors() {
    let text = "%%start
pick = overload {
    (i32, bool)(i64) => fn(n) { n; 1, true };
    (bool, i32)(i64) => fn(n) { n; true, 2 };
};
i64(i64) pass = fn(n) { n };
i64 input = pass(9);
i32 value, bool flag = pick(input);
%%end";
    let source = SourceIdentity::new(
        "project".into(),
        "package".into(),
        "src/main.w".into(),
        "r1".into(),
    );
    for layout in [ScalarTargetLayout::NATIVE64, ScalarTargetLayout::WASM32] {
        let parsed = parse_source(source.clone(), text.into(), &[]);
        assert!(parsed.diagnostics.is_empty(), "{:?}", parsed.diagnostics);
        let typed = derive_scalar_program_with_layout(&parsed.result, layout);
        assert!(typed.diagnostics.is_empty(), "{:?}", typed.diagnostics);
        let project = |program: &crate::ScalarProgram| crate::ScalarProjectValidation {
            project: ScalarProject::new(
                vec![ScalarModule::from_program(program.clone(), Vec::new())],
                vec![source.clone()],
            ),
            diagnostics: Vec::new(),
        };
        let standalone = emit_scalar_llvm(&typed).unwrap();
        let project_result = emit_scalar_project_llvm(&project(&typed.program)).unwrap();
        for replacement in [
            None,
            Some(crate::scalar::ScalarOverloadSelection {
                arm_index: 1,
                substitutions: BTreeMap::new(),
            }),
            Some(crate::scalar::ScalarOverloadSelection {
                arm_index: 0,
                substitutions: BTreeMap::from([("T".into(), crate::ScalarType::U8)]),
            }),
        ] {
            let mut changed = crate::ScalarValidation {
                program: typed.program.clone(),
                diagnostics: Vec::new(),
            };
            let mut changed_calls = 0;
            for item in &mut changed.program.items {
                if let ScalarItem::Binding(binding) = item {
                    if let crate::ScalarExpression::Call {
                        overload_selection, ..
                    } = &mut binding.value
                    {
                        *overload_selection = replacement.clone();
                        changed_calls += 1;
                    }
                }
            }
            assert_eq!(changed_calls, 2);
            let altered_project = project(&changed.program);
            let decoded_single =
                serde_json::from_slice(&serde_json::to_vec(&changed).unwrap()).unwrap();
            let decoded_project =
                serde_json::from_slice(&serde_json::to_vec(&altered_project).unwrap()).unwrap();
            assert_eq!(emit_scalar_llvm(&changed).unwrap(), standalone);
            assert_eq!(emit_scalar_llvm(&decoded_single).unwrap(), standalone);
            assert_eq!(
                emit_scalar_project_llvm(&altered_project).unwrap(),
                project_result
            );
            assert_eq!(
                emit_scalar_project_llvm(&decoded_project).unwrap(),
                project_result
            );
        }
    }
}

#[test]
fn raw_imported_overload_result_uses_bound_declaration_for_nested_conversion() {
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
    let root_text = "%%start
library = namespace package \"src/child.w\";
i8 initial = 1;
i16 converted = core.int_extend<i16>(library.pick(initial));
%%end";
    let child_text = "%%start
pick = overload {
    i8(i8) => fn(value) { value };
    i64(i64) => fn(value) { value };
};
%%end";
    let declarations = |source: SourceIdentity, text: &str| {
        let parsed = parse_source(source, text.into(), &[]);
        assert!(parsed.diagnostics.is_empty(), "{:?}", parsed.diagnostics);
        derive_scalar_declarations_from_cst(parsed.result.canonical_cst())
    };
    for layout in [ScalarTargetLayout::NATIVE64, ScalarTargetLayout::WASM32] {
        let root_declarations = declarations(root.clone(), root_text);
        let namespace = root_declarations.namespaces().next().unwrap();
        let bindings = vec![crate::ScalarNamespaceBinding {
            binding: namespace.binding.clone(),
            target: child.clone(),
            span: namespace.span,
        }];
        let prepared = prepare_scalar_project(
            vec![
                (root_declarations, bindings),
                (declarations(child.clone(), child_text), vec![]),
            ],
            vec![root.clone(), child.clone()],
            ScalarPreparationInputs {
                target_layout: layout,
            },
        )
        .unwrap();
        let build = analyze_scalar_build(&prepared);
        assert!(build.diagnostics().is_empty(), "{:?}", build.diagnostics());
        let raw = validate_scalar_project(prepared.project().clone());
        let raw = emit_scalar_project_llvm(&raw).unwrap();
        let prepared_partition = emit_scalar_build_project_llvm(&build).unwrap();
        let base = project_function_name(&child, "pick");
        let indexed_arm = super::encoded_llvm_name("wosy_overload", [base.as_str(), "0"]);
        let expected = super::generic_specialization_name(&indexed_arm, &[]);
        for partition in [raw, prepared_partition] {
            let text = partition.to_text();
            assert_eq!(
                text.matches(&format!("call i8 @{expected}(")).count(),
                1,
                "{text}"
            );
            assert!(text.contains("sext i8"), "{text}");
            assert!(!text.contains("sext i64"), "{text}");
        }
    }
}

#[test]
fn closed_ordinary_struct_result_writes_caller_storage_before_returning() {
    let text = "%%start
struct Pair { u8 tag; u64 total; }
Pair(bool) _make = fn(flag) { if (flag) { Pair item = { .tag = 7; .total = 19; }; item } else { Pair item = { .tag = 5; .total = 23; }; item } };
u64(bool) verify = fn(flag) { Pair value = _make(flag); value.total };
%%end";
    for layout in [ScalarTargetLayout::WASM32, ScalarTargetLayout::NATIVE64] {
        let prepared = prepare_initializer_test(text, layout);
        let checked = analyze_scalar_build(&prepared);
        assert!(checked.diagnostics().is_empty(), "{:?}", checked.diagnostics());
        let source = &prepared.project().modules[0].source;
        for (producer, caller, llvm) in [
            ("_make".to_owned(), "verify".to_owned(),
                emit_scalar_build_single_module_llvm(&checked).unwrap().to_text()),
            (project_function_name(source, "_make"), project_function_name(source, "verify"),
                emit_scalar_build_project_llvm(&checked).unwrap().to_text()),
        ] {
            let definition = format!("define internal void @{producer}(");
            let (_, tail) = llvm.split_once(&definition).expect("private ordinary definition");
            let (body, _) = tail.split_once("\n}").expect("producer end");
            let (_, hidden) = body.split_once("ptr ").expect("caller destination parameter");
            let destination = hidden.split(')').next().unwrap();
            assert!(body.contains("phi ptr "), "{body}");
            assert!(body.lines().any(|line| {
                line.contains("@llvm.memmove.")
                    && line.contains(&format!("ptr align 1 {destination}, ptr align 1 %if,"))
                    && (line.contains("i32 16, i1 false") || line.contains("i64 16, i1 false"))
            }), "{body}");
            assert!(body.contains("ret void"), "{body}");
            assert!(!body.contains("ret ptr"), "{body}");

            let (_, tail) = llvm.split_once(&format!("define i64 @{caller}(")).expect("caller");
            let (body, _) = tail.split_once("\n}").expect("caller end");
            let slot = body.lines().find_map(|line| {
                let (name, rest) = line.trim_start().split_once(" = alloca ")?;
                (name.starts_with("%ordinary_result_destination") && rest.starts_with("[16 x i8], align 8"))
                    .then_some(name)
            }).expect("caller-owned aligned storage");
            assert!(body.lines().any(|line| line.contains(&format!("call void @{producer}("))
                && line.ends_with(&format!(", ptr {slot})"))), "{body}");
            assert_eq!(fixed_aggregate_move_count(body, slot, 16), 1, "{body}");
        }
    }
}

#[test]
fn ordinary_result_transport_declines_legacy_source_and_aggregate_forwarding() {
    let pair = "struct Pair { u8 tag; u64 total; }";
    for body in [
        "Pair() old_make = fn { Pair item = { .tag = 7; .total = 19; }; item }; Pair() _make = fn { old_make() }; u64() verify = fn { Pair value = _make(); value.total };",
        "Pair() _make = fn { Pair item = { .tag = 7; .total = 19; }; item }; Pair() forward = fn { _make() };",
        "Pair() _make = fn { Pair item = { .tag = 7; .total = 19; }; item }; Pair() forward = fn { Pair value = _make(); value };",
    ] {
        let text = format!("%%start\n{pair}\n{body}\n%%end");
        for layout in [ScalarTargetLayout::WASM32, ScalarTargetLayout::NATIVE64] {
            let prepared = prepare_initializer_test(&text, layout);
            let checked = analyze_scalar_build(&prepared);
            assert!(checked.diagnostics().is_empty(), "{:?}", checked.diagnostics());
            let source = &prepared.project().modules[0].source;
            for (producer, llvm) in [
                ("_make".to_owned(), emit_scalar_build_single_module_llvm(&checked).unwrap().to_text()),
                (project_function_name(source, "_make"), emit_scalar_build_project_llvm(&checked).unwrap().to_text()),
            ] {
                let legacy = format!("define ptr @{producer}(");
                assert!(llvm.contains(&legacy), "{llvm}");
                assert!(!llvm.contains("ordinary_result_destination"), "{llvm}");
            }
        }
    }
}

#[test]
fn projected_named_checked_result_retains_the_exact_acquired_owner() {
    let text = "%%start
struct copy Cell { u8 value; }
(*Cell, u8)(bool) new_or_null = fn(present) {
    Cell item = { .value = 7; };
    if (present) { &item, 1 } else { null, 2 }
};
(*Cell, u8)(*Cell, bool) new_or_borrowed = fn(input, own) {
    Cell item = { .value = 8; };
    if (own) { &item, 1 } else { input, 3 }
};
(*Cell, u8)(bool, bool) nullable = fn(local, present) {
    if (local) { Cell item = { .value = 9; }; &item, 4 } else {
        *Cell result, u8 status = new_or_null(present);
        result, status
    }
};
(*Cell, u8)(*Cell, bool, bool) mixed = fn(input, local, own) {
    if (local) { Cell item = { .value = 10; }; &item, 4 } else {
        *Cell result, u8 status = new_or_borrowed(input, own);
        result, status
    }
};
%%end";
    let source = SourceIdentity::new(
        "project".into(), "package".into(), "src/main.w".into(), "r1".into(),
    );
    for (layout, pointer) in [
        (ScalarTargetLayout::WASM32, "i32"),
        (ScalarTargetLayout::NATIVE64, "i64"),
    ] {
        let parsed = parse_source(source.clone(), text.into(), &[]);
        let single = derive_scalar_program_with_layout(&parsed.result, layout);
        assert!(single.diagnostics.is_empty(), "{:?}", single.diagnostics);
        let project = validate_scalar_project(ScalarProject::new(
            vec![ScalarModule::from_program(single.program.clone(), Vec::new())],
            vec![source.clone()],
        ));
        assert!(project.diagnostics.is_empty(), "{:?}", project.diagnostics);
        for (project_route, llvm) in [
            (false, emit_scalar_llvm(&single).unwrap().to_text()),
            (true, emit_scalar_project_llvm(&project).unwrap().to_text()),
        ] {
            let body = |name: &str| {
                let symbol = if project_route {
                    project_function_name(&source, name)
                } else {
                    name.to_owned()
                };
                llvm.split_once(&format!("define {{ {pointer}, i8 }} @{symbol}("))
                    .and_then(|(_, rest)| rest.split_once("\n}"))
                    .map(|(body, _)| body)
                    .expect("wrapper definition")
            };
            let nullable = body("nullable");
            let phi = nullable.lines()
                .find(|line| line.contains("return_owner_0 = phi i1"))
                .expect("nullable owner merge");
            assert_eq!(phi.matches("[ true,").count(), 2, "{nullable}");
            let mixed = body("mixed");
            let flag = mixed.lines().find_map(|line| {
                let (name, _) = line.trim_start().split_once(" = load i1, ")?;
                name.starts_with("%checked_return_owned_0").then_some(name)
            }).expect("exact owner flag from callee");
            let phi = mixed.lines()
                .find(|line| line.contains("return_owner_0 = phi i1"))
                .expect("mixed owner merge");
            assert!(phi.contains("[ true,") && phi.contains(&format!("[ {flag},")), "{mixed}");
            assert!(!nullable.contains("forwarded_return_present"), "{nullable}");
        }
    }
}


#[test]
fn emits_contextual_float_literals_in_left_operands_and_conditional_arms() {
    let source = "
%%start
f32(f32) f32_ratio = fn(v) { 1.0 / v };
f64(f64) f64_ratio = fn(v) { 1.0 / v };
f32(f32) f32_branch = fn(v) { if (v < 0.0) { 1.0 } else { v } };
f64(f64) f64_branch = fn(v) { if (v < 0.0) { v } else { 1.0 } };
%%end";
    for layout in [ScalarTargetLayout::WASM32, ScalarTargetLayout::NATIVE64] {
        let prepared = prepare_initializer_test(source, layout);
        let build = analyze_scalar_build(&prepared);
        assert!(build.diagnostics().is_empty(), "{:?}", build.diagnostics());
        for partition in [
            emit_scalar_build_single_module_llvm(&build),
            emit_scalar_build_project_llvm(&build),
        ] {
            let ir = partition.expect("contextual float LLVM").to_text();
            let context = Context::create();
            let buffer = inkwell::memory_buffer::MemoryBuffer::create_from_memory_range_copy(
                ir.as_bytes(), "contextual-float",
            );
            let module = context.create_module_from_ir(buffer).expect("parse contextual LLVM");
            module.verify().expect("verify contextual LLVM");
            for ty in ["float", "double"] {
                assert!(ir.contains(&format!("fdiv {ty} 1.000000e+00,")), "{ir}");
                assert!(ir.contains(&format!("phi {ty} ")), "{ir}");
            }
        }
    }
}

#[test]
fn emits_fixed_width_integer_results_for_both_conditional_arms() {
    let source = "
%%start
u8(bool, u8) mixed = fn(b, n) { if (b) { n } else { 0 } };
u8(bool) literals = fn(b) { if (b) { 1 } else { 0 } };
%%end";
    for layout in [ScalarTargetLayout::WASM32, ScalarTargetLayout::NATIVE64] {
        let prepared = prepare_initializer_test(source, layout);
        let build = analyze_scalar_build(&prepared);
        assert!(build.diagnostics().is_empty(), "{:?}", build.diagnostics());
        for partition in [
            emit_scalar_build_single_module_llvm(&build),
            emit_scalar_build_project_llvm(&build),
        ] {
            let ir = partition.expect("conditional result LLVM").to_text();
            let context = Context::create();
            let buffer = inkwell::memory_buffer::MemoryBuffer::create_from_memory_range_copy(
                ir.as_bytes(), "conditional-integer-width",
            );
            let module = context.create_module_from_ir(buffer).expect("parse conditional LLVM");
            module.verify().expect("verify conditional LLVM");
            assert_eq!(ir.matches("phi i8 ").count(), 2, "{ir}");
            assert!(!ir.contains("phi i32"), "{ir}");
        }
    }
}

#[test]
fn emits_deferred_shared_arm_reborrow_and_post_join_store() {
    let source = r#"%%start
u8(bool) update = fn(choose) {
    u8 owner = 7;
    *!u8 parent = &!owner;
    if (choose) { u8 marker = 9; marker; };
    if (choose) {
        *u8 leaf = &(*parent);
        u8 prior = *parent;
        u8 seen = *leaf;
        prior; seen;
    };
    *parent = 11;
    *parent
};
%%end"#;
    for layout in [ScalarTargetLayout::WASM32, ScalarTargetLayout::NATIVE64] {
        let prepared = prepare_initializer_test(source, layout);
        let build = analyze_scalar_build(&prepared);
        assert!(build.diagnostics().is_empty(), "{:?}", build.diagnostics());
        for partition in [
            emit_scalar_build_single_module_llvm(&build),
            emit_scalar_build_project_llvm(&build),
        ] {
            let ir = partition.expect("checked branch reborrow LLVM").to_text();
            let context = Context::create();
            let buffer = inkwell::memory_buffer::MemoryBuffer::create_from_memory_range_copy(
                ir.as_bytes(), "checked-branch-reborrow",
            );
            let module = context.create_module_from_ir(buffer).expect("parse checked branch LLVM");
            module.verify().expect("verify checked branch LLVM");
            assert!(ir.contains("br i1 "), "{ir}");
            assert!(ir.contains("store i8 11"), "{ir}");
            assert!(ir.contains("load i8, ptr "), "{ir}");
        }
    }
}

#[test]
fn emits_deferred_shared_copy_only_when_its_use_precedes_the_ancestor_store() {
    let source = r#"%%start
u8(bool) inspect = fn(choose) {
    u8 owner = 7;
    *!u8 parent = &!owner;
    if (choose) { bool marker = true; marker; };
    *u8 leaf = &(*parent);
    *u8 alias = leaf;
    u8 observed = *alias;
    *parent = 11;
    observed + *parent
};
%%end"#;
    let invalid = source.replace(
        "u8 observed = *alias;\n    *parent = 11;",
        "*parent = 11;\n    u8 observed = *alias;",
    );
    let borrow = invalid.find("&(*parent)").unwrap() as u32;
    let write = invalid.find("parent = 11").unwrap() as u32;
    for layout in [ScalarTargetLayout::WASM32, ScalarTargetLayout::NATIVE64] {
        let prepared = prepare_initializer_test(source, layout);
        let build = analyze_scalar_build(&prepared);
        assert!(build.diagnostics().is_empty(), "{:?}", build.diagnostics());
        for partition in [
            emit_scalar_build_single_module_llvm(&build),
            emit_scalar_build_project_llvm(&build),
        ] {
            let ir = partition.expect("shared copy LLVM").to_text();
            let context = Context::create();
            let buffer = inkwell::memory_buffer::MemoryBuffer::create_from_memory_range_copy(
                ir.as_bytes(), "checked-shared-copy",
            );
            let module = context.create_module_from_ir(buffer).expect("parse shared copy LLVM");
            module.verify().expect("verify shared copy LLVM");
            let lines = ir.lines().map(str::trim).collect::<Vec<_>>();
            let (add, (before, after)) = lines.iter().enumerate().find_map(|(index, line)| {
                let (_, operands) = line.split_once(" = add i8 ")?;
                Some((index, operands.split_once(", ")?))
            }).expect("combine the before and after reads");
            let operand_load = |operand| lines.iter().enumerate().find_map(|(index, line)| {
                let (value, pointer) = line.split_once(" = load i8, ptr ")?;
                (value == operand).then_some((index, pointer.split(',').next()?))
            });
            let (reload, spill) = operand_load(before).expect("saved result of shared read");
            let (save, saved) = lines.iter().enumerate().find_map(|(index, line)| {
                let (value, pointer) = line.strip_prefix("store i8 ")?.split_once(", ptr ")?;
                (pointer.split(',').next()? == spill).then_some((index, value))
            }).expect("copy shared read into the ordinary local");
            let (before, _) = operand_load(saved).expect("read through shared alias");
            let (after, _) = operand_load(after).expect("read through resumed parent");
            let (stage, slot) = lines.iter().enumerate().find_map(|(index, line)| {
                let slot = line.strip_prefix("store i8 11, ptr ")?.split(',').next()?;
                Some((index, slot))
            }).expect("evaluate RHS of the resumed parent store");
            let (assignment_load, assignment_value) = lines.iter().enumerate().find_map(|(index, line)| {
                let (value, pointer) = line.split_once(" = load i8, ptr ")?;
                (index > stage && pointer.split(',').next()? == slot).then_some((index, value))
            }).expect("load the prepared assignment RHS");
            let write_prefix = format!("store i8 {assignment_value}, ptr ");
            let write = lines.iter().position(|line| line.starts_with(&write_prefix))
                .expect("write through resumed parent");
            assert!(before < save && save < stage && stage < assignment_load
                && assignment_load < write && write < reload && reload < after && after < add, "{ir}");
        }

        let prepared = prepare_initializer_test(&invalid, layout);
        let denied = analyze_scalar_build(&prepared);
        assert!(denied.diagnostics().iter().any(|diagnostic| {
            diagnostic.code == "B0003" && diagnostic.message == "read or move of unavailable place"
                && diagnostic.labels.first().is_some_and(|label| label.span.range.start == write)
                && diagnostic.labels.iter().skip(1).any(|label| label.span.range.start == borrow)
        }), "{:?}", denied.diagnostics());
    }
}

#[test]
fn emits_deferred_whole_owner_store_only_after_all_borrower_uses() {
    let source = r#"%%start
u8(bool) update = fn(choose) {
    u8 owner = 7;
    *!u8 parent = &!owner;
    if (choose) { bool marker = true; marker; };
    *u8 leaf = &(*parent);
    *u8 alias = leaf;
    u8 observed = *alias;
    owner = 11;
    observed; owner
};
%%end"#;
    let invalid = source.replace(
        "u8 observed = *alias;\n    owner = 11;",
        "owner = 11;\n    u8 observed = *alias;",
    );
    let borrow = invalid.find("&(*parent)").unwrap() as u32;
    let write = invalid.find("owner = 11").unwrap() as u32;
    for layout in [ScalarTargetLayout::WASM32, ScalarTargetLayout::NATIVE64] {
        let prepared = prepare_initializer_test(source, layout);
        let build = analyze_scalar_build(&prepared);
        assert!(build.diagnostics().is_empty(), "{:?}", build.diagnostics());
        for partition in [
            emit_scalar_build_single_module_llvm(&build),
            emit_scalar_build_project_llvm(&build),
        ] {
            let ir = partition.expect("whole-owner assignment LLVM").to_text();
            let context = Context::create();
            let buffer = inkwell::memory_buffer::MemoryBuffer::create_from_memory_range_copy(
                ir.as_bytes(), "checked-whole-owner-assignment",
            );
            let module = context.create_module_from_ir(buffer).expect("parse whole-owner assignment LLVM");
            module.verify().expect("verify whole-owner assignment LLVM");
            let lines = ir.lines().map(str::trim).collect::<Vec<_>>();
            let returned = lines.iter().find_map(|line| line.strip_prefix("ret i8 "))
                .expect("return owner scalar");
            let (load, owner_slot) = lines.iter().enumerate().find_map(|(index, line)| {
                let (value, address) = line.split_once(" = load i8, ptr ")?;
                (value == returned).then_some((index, address.split(',').next()?))
            }).expect("result loads the owner place");
            let (store, new_value) = lines[..load].iter().enumerate().rev().find_map(|(index, line)| {
                let (value, address) = line.strip_prefix("store i8 ")?.split_once(", ptr ")?;
                (address.split(',').next()? == owner_slot).then_some((index, value))
            }).expect("direct write to the returned owner place");
            if new_value != "11" {
                let (rhs, slot) = lines[..store].iter().enumerate().find_map(|(index, line)| {
                    let (value, address) = line.split_once(" = load i8, ptr ")?;
                    (value == new_value).then_some((index, address.split(',').next()?))
                }).expect("prepared RHS for direct owner store");
                assert!(lines[..rhs].iter().any(|line| {
                    line.strip_prefix("store i8 11, ptr ").and_then(|address| address.split(',').next()) == Some(slot)
                }), "{ir}");
            }
        }

        let prepared = prepare_initializer_test(&invalid, layout);
        let denied = analyze_scalar_build(&prepared);
        assert!(denied.diagnostics().iter().any(|diagnostic| {
            diagnostic.code == "B0003" && diagnostic.message == "read or move of unavailable place"
                && diagnostic.labels.first().is_some_and(|label| label.span.range.start == write)
                && diagnostic.labels.iter().skip(1).any(|label| label.span.range.start == borrow)
        }), "{:?}", denied.diagnostics());
    }
}

#[test]
fn emits_deferred_mutable_arm_reborrow_without_extending_it_into_the_other_arm() {
    use inkwell::basic_block::BasicBlock;
    use inkwell::values::{BasicValue, BasicValueEnum, InstructionOpcode, InstructionValue};

    let source = r#"%%start
u8(bool) update = fn(choose) {
    u8 owner = 7;
    *!u8 parent = &!owner;
    if (choose) { bool marker = true; marker; };
    if (choose) {
        *!u8 leaf = &!(*parent);
        *leaf = 8;
    } else {
        *parent = 9;
    };
    *parent
};
%%end"#;
    let invalid = source.replace("*leaf = 8;", "*parent = 8; *leaf = 8;");
    let borrow = invalid.find("&!(*parent)").unwrap() as u32;
    let write = invalid.find("parent = 8").unwrap() as u32;
    for layout in [ScalarTargetLayout::WASM32, ScalarTargetLayout::NATIVE64] {
        let prepared = prepare_initializer_test(source, layout);
        let build = analyze_scalar_build(&prepared);
        assert!(build.diagnostics().is_empty(), "{:?}", build.diagnostics());
        for partition in [
            emit_scalar_build_single_module_llvm(&build),
            emit_scalar_build_project_llvm(&build),
        ] {
            let ir = partition.expect("scoped mutable LLVM").to_text();
            let context = Context::create();
            let buffer = inkwell::memory_buffer::MemoryBuffer::create_from_memory_range_copy(
                ir.as_bytes(), "checked-scoped-mutable",
            );
            let module = context.create_module_from_ir(buffer).expect("parse scoped mutable LLVM");
            module.verify().expect("verify scoped mutable LLVM");
            let function = module.get_functions().find(|function| {
                matches!(function.get_type().get_return_type(), Some(inkwell::types::BasicTypeEnum::IntType(ty))
                    if ty.get_bit_width() == 8)
            }).expect("ordinary scoped function");
            let blocks = function.get_basic_blocks();
            let instructions = blocks.iter().flat_map(|block| {
                std::iter::successors(block.get_first_instruction(), |instruction| instruction.get_next_instruction())
            }).collect::<Vec<_>>();
            fn operand<'ctx>(instruction: InstructionValue<'ctx>, index: u32) -> Option<BasicValueEnum<'ctx>> {
                instruction.get_operand(index).and_then(|operand| operand.value())
            }
            let checked_pointer = |pointer: BasicValueEnum<'_>| {
                pointer.as_instruction_value().is_some_and(|destination| {
                    destination.get_opcode() == InstructionOpcode::IntToPtr
                        && operand(destination, 0).and_then(|address| address.as_instruction_value())
                            .is_some_and(|load| load.get_opcode() == InstructionOpcode::Load)
                })
            };
            let actual_store = |literal| {
                let stage = instructions.iter().copied().find(|instruction| {
                    instruction.get_opcode() == InstructionOpcode::Store
                        && matches!(operand(*instruction, 0), Some(BasicValueEnum::IntValue(value))
                            if value.get_type().get_bit_width() == 8 && value.get_zero_extended_constant() == Some(literal))
                })?;
                let slot = operand(stage, 1)?;
                if checked_pointer(slot) {
                    return Some(stage);
                }
                instructions.iter().copied().find(|instruction| {
                    instruction.get_opcode() == InstructionOpcode::Store
                        && operand(*instruction, 1).is_some_and(checked_pointer)
                        && operand(*instruction, 0).and_then(|value| value.as_instruction_value())
                            .is_some_and(|load| load.get_opcode() == InstructionOpcode::Load
                                && operand(load, 0) == Some(slot))
                })
            };
            let first = actual_store(8).and_then(|store| store.get_parent())
                .expect("taken arm writes through scoped child");
            let other = actual_store(9).and_then(|store| store.get_parent())
                .expect("other arm writes through original parent");
            let result = instructions.iter().copied().find(|instruction| {
                instruction.get_opcode() == InstructionOpcode::Return
            }).expect("post-join return");
            let output = operand(result, 0).and_then(|value| value.as_instruction_value())
                .expect("returned value is a load through resumed parent");
            assert_eq!(output.get_opcode(), InstructionOpcode::Load);
            assert!(operand(output, 0).is_some_and(checked_pointer));
            let join = output.get_parent().expect("joined output block");
            let reaches = |start: BasicBlock<'_>, end: BasicBlock<'_>| {
                let mut pending = vec![start];
                let mut seen = std::collections::HashSet::new();
                while let Some(block) = pending.pop() {
                    if block == end {
                        return true;
                    }
                    if seen.insert(block) {
                        if let Some(terminal) = block.get_terminator() {
                            pending.extend(terminal.get_operands().filter_map(|operand| operand?.block()));
                        }
                    }
                }
                false
            };
            assert!(first != other && reaches(first, join) && reaches(other, join), "{ir}");
            assert!(!reaches(first, other) && !reaches(other, first), "{ir}");
            assert!(instructions.iter().any(|instruction| {
                if instruction.get_opcode() != InstructionOpcode::Br || instruction.is_conditional() != Ok(true) {
                    return false;
                }
                let targets = instruction.get_operands().filter_map(|operand| operand?.block()).collect::<Vec<_>>();
                let [left, right] = targets.as_slice() else { return false };
                let separates = |left, right| reaches(left, first) && !reaches(left, other)
                    && reaches(right, other) && !reaches(right, first);
                separates(*left, *right) || separates(*right, *left)
            }), "one conditional must separate both stores before they rejoin: {ir}");
        }
        let prepared = prepare_initializer_test(&invalid, layout);
        let denied = analyze_scalar_build(&prepared);
        assert!(denied.diagnostics().iter().any(|diagnostic| {
            diagnostic.code == "B0003" && diagnostic.message == "read or move of unavailable place"
                && diagnostic.labels.first().is_some_and(|label| label.span.range.start == write)
                && diagnostic.labels.iter().skip(1).any(|label| label.span.range.start == borrow)
        }), "{:?}", denied.diagnostics());
    }
}

fn known_field_operand<'ctx>(instruction: InstructionValue<'ctx>, index: u32) -> Option<BasicValueEnum<'ctx>> {
    instruction.get_operand(index).and_then(|operand| operand.value())
}

fn known_field_stored_value<'ctx>(
    instructions: &[InstructionValue<'ctx>], slot: BasicValueEnum<'ctx>,
) -> Option<BasicValueEnum<'ctx>> {
    let mut stores = instructions.iter().copied().filter(|instruction| {
        instruction.get_opcode() == InstructionOpcode::Store && known_field_operand(*instruction, 1) == Some(slot)
    });
    let store = stores.next()?;
    if stores.next().is_some() { return None; }
    known_field_operand(store, 0)
}

fn known_checked_address<'ctx>(
    instructions: &[InstructionValue<'ctx>], address: BasicValueEnum<'ctx>,
) -> Option<BasicValueEnum<'ctx>> {
    let dereference = address.as_instruction_value()?;
    if dereference.get_opcode() != InstructionOpcode::IntToPtr { return None; }
    let handle_load = known_field_operand(dereference, 0)?.as_instruction_value()?;
    if handle_load.get_opcode() != InstructionOpcode::Load { return None; }
    let handle = known_field_stored_value(instructions, known_field_operand(handle_load, 0)?)?.as_instruction_value()?;
    if handle.get_opcode() != InstructionOpcode::PtrToInt { return None; }
    known_field_operand(handle, 0)
}

fn known_checked_field_path<'ctx>(
    instructions: &[InstructionValue<'ctx>], address: BasicValueEnum<'ctx>,
) -> Option<(BasicValueEnum<'ctx>, u64)> {
    known_direct_field_path(known_checked_address(instructions, address)?)
}

fn known_direct_field_path(address: BasicValueEnum<'_>) -> Option<(BasicValueEnum<'_>, u64)> {
    let field = address.as_instruction_value()?;
    if field.get_opcode() != InstructionOpcode::GetElementPtr { return None; }
    let BasicValueEnum::IntValue(offset) = known_field_operand(field, 1)? else { return None };
    Some((known_field_operand(field, 0)?, offset.get_zero_extended_constant()?))
}

fn known_stored_byte<'ctx>(
    instructions: &[InstructionValue<'ctx>], store: InstructionValue<'ctx>,
) -> Option<u64> {
    fn constant_byte(value: BasicValueEnum<'_>) -> Option<u64> {
        let BasicValueEnum::IntValue(value) = value else { return None };
        if value.get_type().get_bit_width() != 8 { return None; }
        value.get_zero_extended_constant()
    }

    let value = known_field_operand(store, 0)?;
    constant_byte(value).or_else(|| {
        let reload = value.as_instruction_value()?;
        if reload.get_opcode() != InstructionOpcode::Load { return None; }
        constant_byte(known_field_stored_value(instructions, known_field_operand(reload, 0)?)?)
    })
}

#[test]
fn emits_known_disjoint_checked_field_writes_through_the_ordinary_driver() {
    let source = r#"%%start
struct copy Pair { u8 left; u8 right; }
u8(Pair) update = fn(owner) {
    *!u8 left = &!owner.left;
    *!u8 right = &!owner.right;
    *left = 8;
    *right = 9;
    *left + *right
};
%%end"#;
    let invalid = source.replace("*left = 8;", "owner = { .left = 3; .right = 4; }; *left = 8;");
    let borrow = invalid.find("&!owner.left").unwrap() as u32;
    let write = invalid.find("owner =").unwrap() as u32;
    for layout in [ScalarTargetLayout::WASM32, ScalarTargetLayout::NATIVE64] {
        let prepared = prepare_initializer_test(source, layout);
        let build = analyze_scalar_build(&prepared);
        assert!(build.diagnostics().is_empty(), "{:?}", build.diagnostics());
        for partition in [
            emit_scalar_build_single_module_llvm(&build),
            emit_scalar_build_project_llvm(&build),
        ] {
            let ir = partition.expect("known checked fields LLVM").to_text();
            let context = Context::create();
            let buffer = inkwell::memory_buffer::MemoryBuffer::create_from_memory_range_copy(
                ir.as_bytes(), "known-checked-disjoint-fields",
            );
            let module = context.create_module_from_ir(buffer).expect("parse known checked fields LLVM");
            module.verify().expect("verify checked field LLVM");
            let routine = module.get_functions().find(|routine| {
                matches!(routine.get_type().get_return_type(), Some(inkwell::types::BasicTypeEnum::IntType(ty))
                    if ty.get_bit_width() == 8)
            }).expect("checked callable");
            let instructions = routine.get_basic_blocks().iter().flat_map(|block| {
                std::iter::successors(block.get_first_instruction(), |instruction| instruction.get_next_instruction())
            }).collect::<Vec<_>>();
            let mut writes = instructions.iter().copied().filter_map(|instruction| {
                if instruction.get_opcode() != InstructionOpcode::Store { return None; }
                let path = known_checked_field_path(&instructions, known_field_operand(instruction, 1)?)?;
                let literal = known_stored_byte(&instructions, instruction)?;
                Some((path, literal))
            }).collect::<Vec<_>>();
            writes.sort_by_key(|((_, offset), _)| *offset);
            let [(left @ (_, 0), 8), (right @ (_, 1), 9)] = writes.as_slice() else {
                panic!("two actual writes to distinct checked field offsets: {ir}");
            };
            assert_eq!(left.0, right.0, "both fields belong to the same owner");
            let result = instructions.iter().copied().find(|instruction| {
                instruction.get_opcode() == InstructionOpcode::Return
            }).expect("field result");
            let result = known_field_operand(result, 0).and_then(|value| value.as_instruction_value()).expect("computed field sum");
            assert_eq!(result.get_opcode(), InstructionOpcode::Add);
            let mut reads = [0, 1].map(|index| {
                let read = known_field_operand(result, index).and_then(|value| value.as_instruction_value())
                    .expect("add consumes the checked field read");
                assert_eq!(read.get_opcode(), InstructionOpcode::Load);
                known_checked_field_path(&instructions, known_field_operand(read, 0).expect("read address"))
                    .expect("read resolves to an actual checked field path")
            });
            reads.sort_by_key(|(_, offset)| *offset);
            assert_eq!(reads, [*left, *right], "the returned add reads the same fields that were written");
        }
        let prepared = prepare_initializer_test(&invalid, layout);
        let denied = analyze_scalar_build(&prepared);
        assert!(denied.diagnostics().iter().any(|diagnostic| {
            diagnostic.code == "B0003" && diagnostic.message == "read or move of unavailable place"
                && diagnostic.labels.first().is_some_and(|label| label.span.range.start == write)
                && diagnostic.labels.iter().skip(1).any(|label| label.span.range.start == borrow)
        }), "{:?}", denied.diagnostics());
    }
}

#[test]
fn emits_known_copy_read_disjoint_from_live_checked_field_through_the_ordinary_driver() {
    let source = r#"%%start
struct copy Pair { u8 left; u8 right; }
u8(Pair) inspect = fn(owner) {
    *!u8 leaf = &!owner.left;
    owner.right + *leaf
};
%%end"#;
    let invalid = source.replace("owner.right +", "owner.left +");
    let source_read = invalid.find("owner.left +").unwrap() as u32;
    let loan = invalid.find("&!owner.left").unwrap() as u32;
    for layout in [ScalarTargetLayout::WASM32, ScalarTargetLayout::NATIVE64] {
        let prepared = prepare_initializer_test(source, layout);
        let build = analyze_scalar_build(&prepared);
        assert!(build.diagnostics().is_empty(), "{:?}", build.diagnostics());
        for partition in [
            emit_scalar_build_single_module_llvm(&build),
            emit_scalar_build_project_llvm(&build),
        ] {
            let ir = partition.expect("ordinary disjoint Copy read LLVM").to_text();
            let context = Context::create();
            let buffer = inkwell::memory_buffer::MemoryBuffer::create_from_memory_range_copy(
                ir.as_bytes(), "known-disjoint-copy-read",
            );
            let module = context.create_module_from_ir(buffer).expect("parse disjoint Copy read LLVM");
            module.verify().expect("verify disjoint Copy read LLVM");
            let routine = module.get_functions().find(|routine| {
                matches!(routine.get_type().get_return_type(), Some(inkwell::types::BasicTypeEnum::IntType(ty))
                    if ty.get_bit_width() == 8)
            }).expect("typed byte result");
            let instructions = routine.get_basic_blocks().iter().flat_map(|block| {
                std::iter::successors(block.get_first_instruction(), |instruction| instruction.get_next_instruction())
            }).collect::<Vec<_>>();
            let result = instructions.iter().copied().find(|instruction| {
                instruction.get_opcode() == InstructionOpcode::Return
            }).expect("returned byte sum");
            let result = known_field_operand(result, 0).and_then(|value| value.as_instruction_value())
                .expect("returned add value");
            assert_eq!(result.get_opcode(), InstructionOpcode::Add);
            let [direct, checked] = [0, 1].map(|index| {
                let consumed = known_field_operand(result, index).and_then(|value| value.as_instruction_value())
                    .expect("add consumes field read");
                assert_eq!(consumed.get_opcode(), InstructionOpcode::Load);
                known_field_operand(consumed, 0).expect("actual field address")
            });
            let direct = known_direct_field_path(direct).expect("direct owner field");
            let checked = known_checked_field_path(&instructions, checked).expect("checked owner field");
            assert_eq!((direct.1, checked.1), (1, 0));
            assert_eq!(direct.0, checked.0, "both returned reads use the same owner");

        }

        let prepared = prepare_initializer_test(&invalid, layout);
        let denied = analyze_scalar_build(&prepared);
        assert!(denied.diagnostics().iter().any(|diagnostic| {
            diagnostic.code == "B0003" && diagnostic.message == "read or move of unavailable place"
                && diagnostic.labels.first().is_some_and(|label| label.span.range.start == source_read)
                && diagnostic.labels.iter().skip(1).any(|label| label.span.range.start == loan)
        }), "{:?}", denied.diagnostics());
    }
}

fn known_block_reaches<'ctx>(
    start: inkwell::basic_block::BasicBlock<'ctx>,
    end: inkwell::basic_block::BasicBlock<'ctx>,
    excluded: Option<inkwell::basic_block::BasicBlock<'ctx>>,
) -> bool {
    let mut pending = vec![start];
    let mut seen = std::collections::HashSet::new();
    while let Some(block) = pending.pop() {
        if excluded == Some(block) || !seen.insert(block) {
            continue;
        }
        if block == end {
            return true;
        }
        if let Some(terminal) = block.get_terminator() {
            pending.extend(terminal.get_operands().filter_map(|operand| operand?.block()));
        }
    }
    false
}

fn known_instruction_precedes_on_all_paths<'ctx>(
    before: InstructionValue<'ctx>, after: InstructionValue<'ctx>, entry: inkwell::basic_block::BasicBlock<'ctx>,
) -> bool {
    let before_block = before.get_parent().expect("first instruction block");
    let after_block = after.get_parent().expect("second instruction block");
    if before_block == after_block {
        std::iter::successors(before.get_next_instruction(), |instruction| instruction.get_next_instruction())
            .any(|instruction| instruction == after)
    } else {
        known_block_reaches(entry, after_block, None) && !known_block_reaches(entry, after_block, Some(before_block))
    }
}

#[test]
fn emits_known_projected_leaf_from_the_original_owner_through_the_ordinary_driver() {
    for (layout, parent_mode, child_mode, left_holder_mode, parent_store) in [ScalarTargetLayout::WASM32, ScalarTargetLayout::NATIVE64]
        .into_iter().flat_map(|layout| [
            (layout, "", "", None, false),
            (layout, "!", "", None, false),
            (layout, "!", "", None, true),
            (layout, "!", "!", None, false),
            (layout, "!", "!", None, true),
            (layout, "!", "!", Some("!"), false),
            (layout, "!", "!", Some(""), false),
        ])
    {
        let left_declaration = left_holder_mode.map(|mode| {
            format!("*{mode}u8 left_leaf = &{mode}(*parent).left;")
        }).unwrap_or_default();
        let left = if left_holder_mode.is_some() {
            "*left_leaf"
        } else if parent_mode == "!" {
            "(*parent).left"
        } else {
            "owner.left"
        };
        let stores = if parent_store {
            "(*parent).left = 7;"
        } else if left_holder_mode == Some("!") {
            "*left_leaf = 7; *leaf = 9;"
        } else if child_mode == "!" {
            "*leaf = 9;"
        } else {
            ""
        };
        let source = format!(r#"%%start
struct copy Pair {{ u8 left; u8 right; }}
u8(Pair) inspect = fn(owner) {{
    *{parent_mode}Pair parent = &{parent_mode}owner;
    {left_declaration}
    *{child_mode}u8 leaf = &{child_mode}(*parent).right;
    {stores}
    {left} + *leaf
}};
%%end"#);
        let prepared = prepare_initializer_test(&source, layout);
        let build = analyze_scalar_build(&prepared);
        assert!(build.diagnostics().is_empty(), "{:?}", build.diagnostics());
        for partition in [
            emit_scalar_build_single_module_llvm(&build),
            emit_scalar_build_project_llvm(&build),
        ] {
            let ir = partition.expect("ordinary projected leaf LLVM").to_text();
            let context = Context::create();
            let buffer = inkwell::memory_buffer::MemoryBuffer::create_from_memory_range_copy(
                ir.as_bytes(), "known-projected-leaf",
            );
            let module = context.create_module_from_ir(buffer).expect("parse projected leaf LLVM");
            module.verify().expect("verify projected leaf LLVM");
            let routine = module.get_functions().find(|routine| {
                matches!(routine.get_type().get_return_type(), Some(inkwell::types::BasicTypeEnum::IntType(ty))
                    if ty.get_bit_width() == 8)
            }).expect("typed byte result");
            let instructions = routine.get_basic_blocks().iter().flat_map(|block| {
                std::iter::successors(block.get_first_instruction(), |instruction| instruction.get_next_instruction())
            }).collect::<Vec<_>>();
            let result = instructions.iter().copied().find(|instruction| {
                instruction.get_opcode() == InstructionOpcode::Return
            }).expect("returned byte sum");
            let result = known_field_operand(result, 0).and_then(|value| value.as_instruction_value())
                .expect("returned add value");
            assert_eq!(result.get_opcode(), InstructionOpcode::Add);
            let [(direct, returned_left_load), (checked, returned_leaf_load)] = [0, 1].map(|index| {
                let read = known_field_operand(result, index).and_then(|value| value.as_instruction_value())
                    .expect("add consumes field read");
                assert_eq!(read.get_opcode(), InstructionOpcode::Load);
                (known_field_operand(read, 0).expect("actual field address"), read)
            });
            let (direct_base, direct_offset) = if left_holder_mode.is_some() {
                known_checked_field_path(&instructions, direct).expect("left checked sibling field")
            } else {
                known_direct_field_path(direct).expect("left owner field")
            };
            let direct_owner = if parent_mode.is_empty() {
                direct_base
            } else {
                known_checked_address(&instructions, direct_base).expect("mutable checked parent owner")
            };
            let (through_parent, leaf_offset) = known_checked_field_path(&instructions, checked)
                .expect("read through the projected checked leaf");
            let loan_owner = known_checked_address(&instructions, through_parent).expect("whole checked parent");
            assert_eq!((direct_offset, leaf_offset), (0, 1));
            assert_eq!(direct_owner, loan_owner, "both returned reads derive from the actual owner");
            if parent_store || !child_mode.is_empty() {
                let writes = instructions.iter().copied().filter_map(|instruction| {
                    if instruction.get_opcode() != InstructionOpcode::Store { return None; }
                    let address = known_field_operand(instruction, 1)?;
                    let (parent, offset) = if parent_store {
                        known_direct_field_path(address)?
                    } else {
                        known_checked_field_path(&instructions, address)?
                    };
                    let owner = known_checked_address(&instructions, parent)?;
                    (owner == loan_owner).then(|| (offset, instruction, known_stored_byte(&instructions, instruction)))
                }).collect::<Vec<_>>();
                let expected = [
                    (!parent_store && !child_mode.is_empty()).then_some((leaf_offset, 9, returned_leaf_load)),
                    (parent_store || left_holder_mode == Some("!")).then_some((direct_offset, 7, returned_left_load)),
                ];
                assert_eq!(writes.len(), expected.iter().flatten().count(), "{ir}");
                let entry = routine.get_first_basic_block().expect("entry block");
                for (offset, byte, read) in expected.into_iter().flatten() {
                    let matches = writes.iter().filter(|(stored, _, _)| *stored == offset).collect::<Vec<_>>();
                    let [(_, write, Some(actual))] = matches.as_slice() else {
                        panic!("one checked byte store to each exact returned field: {ir}");
                    };
                    assert_eq!(*actual, byte, "{ir}");
                    assert!(known_instruction_precedes_on_all_paths(*write, read, entry),
                        "each exact store precedes its returned field load: {ir}");
                    if parent_store {
                        assert!(known_instruction_precedes_on_all_paths(*write, returned_leaf_load, entry),
                            "the right child remains usable after the parent's left store: {ir}");
                    }
                }
            }
        }
    }
}

fn known_unspill_once<'ctx>(
    instructions: &[InstructionValue<'ctx>], value: BasicValueEnum<'ctx>,
) -> BasicValueEnum<'ctx> {
    value.as_instruction_value()
        .filter(|inst| inst.get_opcode() == InstructionOpcode::Load)
        .and_then(|load| known_field_operand(load, 0))
        .and_then(|slot| known_field_stored_value(instructions, slot))
        .unwrap_or(value)
}

#[test]
fn emits_known_recursive_frame_release_after_child_return_and_local_reads() {
    use inkwell::values::CallSiteValue;

    let source = r#"%%start
u64(*u8, u64) descend = fn(backing, depth) {
    u64 length = 4;
    u8[length] scratch;
    scratch[0] = core.int_trunc<u8>(depth);
    u64 subtotal = if (depth == 0) { 0 } else { descend(backing, depth - 1) };
    subtotal + core.int_extend<u64>(scratch[0]) + core.int_extend<u64>(*backing)
};
%%end"#;
    for layout in [ScalarTargetLayout::WASM32, ScalarTargetLayout::NATIVE64] {
        let prepared = prepare_initializer_test(source, layout);
        let build = analyze_scalar_build(&prepared);
        assert!(build.diagnostics().is_empty(), "{:?}", build.diagnostics());
        for (project, partition) in [
            (false, emit_scalar_build_single_module_llvm(&build)),
            (true, emit_scalar_build_project_llvm(&build)),
        ] {
            let ir = partition.expect("ordinary recursive resource LLVM").to_text();
            let context = Context::create();
            let buffer = inkwell::memory_buffer::MemoryBuffer::create_from_memory_range_copy(
                ir.as_bytes(), "known-recursive-owned-frame",
            );
            let module = context.create_module_from_ir(buffer).expect("parse recursive frame LLVM");
            module.verify().expect("verify recursive frame LLVM");
            let symbol = self_tail_test_symbol(&prepared.project().modules[0].source, project, "descend");
            let routine = module.get_function(&symbol).expect("recursive callable");
            let alloc_symbol = module.get_function("__wosy_core_alloc").expect("ordinary allocator");
            let release_symbol = module.get_function("__wosy_core_free").expect("ordinary release");
            let entry = routine.get_first_basic_block().expect("recursive entry");
            let instructions = routine.get_basic_blocks().iter().flat_map(|block| {
                std::iter::successors(block.get_first_instruction(), |instruction| instruction.get_next_instruction())
            }).collect::<Vec<_>>();
            let unique_call = |target| {
                let calls = instructions.iter().copied().filter_map(|instruction| {
                    let call = CallSiteValue::try_from(instruction).ok()?;
                    (call.get_called_fn_value() == Some(target)).then_some((instruction, call))
                }).collect::<Vec<_>>();
                let [only] = calls.as_slice() else { panic!("one actual call to each target: {ir}") };
                *only
            };
            let (allocation, allocation_call) = unique_call(alloc_symbol);
            let allocated = allocation_call.try_as_basic_value().basic().expect("frame allocation result");
            let (recursive, _) = unique_call(routine);
            let (release, _) = unique_call(release_symbol);
            let incoming = routine.get_nth_param(0).expect("incoming checked backing");

            let unspill = |value| known_unspill_once(&instructions, value);
            let recurse_backing = known_field_operand(recursive, 0).expect("recursive checked backing operand");
            assert_eq!(unspill(recurse_backing), incoming, "{ir}");
            assert_eq!(unspill(known_field_operand(release, 0).expect("released allocation")), allocated, "{ir}");

            let mut scratch = Vec::new();
            let mut backing = Vec::new();
            for instruction in instructions.iter().copied().filter(|instruction| {
                instruction.get_opcode() == InstructionOpcode::Load
                    && matches!(instruction.get_type(), inkwell::types::AnyTypeEnum::IntType(ty) if ty.get_bit_width() == 8)
            }) {
                let Some(address) = known_field_operand(instruction, 0).and_then(|value| value.as_instruction_value())
                    else { continue };
                match address.get_opcode() {
                    InstructionOpcode::GetElementPtr if known_field_operand(address, 0)
                        .is_some_and(|base| unspill(base) == allocated) => scratch.push(instruction),
                    InstructionOpcode::IntToPtr if known_field_operand(address, 0)
                        .is_some_and(|handle| unspill(handle) == incoming) => backing.push(instruction),
                    _ => {}
                }
            }
            let [scratch] = scratch.as_slice() else { panic!("one real read from this frame's owned scratch: {ir}") };
            let [backing] = backing.as_slice() else { panic!("one real read from original borrowed backing: {ir}") };
            let returned = instructions.iter().copied().find(|instruction| {
                instruction.get_opcode() == InstructionOpcode::Return
            }).expect("normal frame return");

            let return_value = known_field_operand(returned, 0).expect("actual returned frame result");
            let contributes = |needle| {
                let mut pending = vec![return_value];
                let mut seen = Vec::new();
                while let Some(value) = pending.pop() {
                    if seen.contains(&value) { continue; }
                    seen.push(value);
                    let Some(instruction) = value.as_instruction_value() else { continue };
                    if instruction == needle { return true; }
                    if instruction.get_opcode() == InstructionOpcode::Load {
                        let source = unspill(value);
                        if source != value { pending.push(source); }
                    } else if matches!(instruction.get_opcode(), InstructionOpcode::Add | InstructionOpcode::ZExt
                        | InstructionOpcode::Phi | InstructionOpcode::Select)
                    {
                        pending.extend(instruction.get_operands().filter_map(|operand| operand?.value()));
                    }
                }
                false
            };
            assert!(contributes(recursive) && contributes(*scratch) && contributes(*backing), "{ir}");

            let call_block = recursive.get_parent().expect("recursive path");
            let return_block = returned.get_parent().expect("normal return block");
            assert_ne!(call_block, entry, "the base path avoids recursive descent: {ir}");
            assert!(known_block_reaches(entry, return_block, Some(call_block)), "{ir}");
            assert!(known_instruction_precedes_on_all_paths(allocation, recursive, entry), "{ir}");
            assert!(known_instruction_precedes_on_all_paths(recursive, *scratch, call_block), "{ir}");
            assert!(known_instruction_precedes_on_all_paths(recursive, *backing, call_block), "{ir}");
            assert!(known_instruction_precedes_on_all_paths(*scratch, release, entry), "{ir}");
            assert!(known_instruction_precedes_on_all_paths(*backing, release, entry), "{ir}");
            assert!(known_instruction_precedes_on_all_paths(release, returned, entry), "{ir}");
        }
    }
}

#[test]
fn guarded_fallible_scalar_uses_shared_lexical_ownership_on_both_targets() {
    let source = SourceIdentity::new(
        "project".into(), "package".into(), "src/main.w".into(), "r1".into(),
    );
    let text = "%%start\ni32() observed = fn {\n\ti32 result =? 42;\n\tif (&result == null) { 7 } else { result }\n};\ni32() flipped = fn {\n\ti32 result =? 19;\n\tif (null != &result) { result } else { 9 }\n};\ni32(bool) nested = fn(gate) {\n\tif (gate) {\n\t\ti32 result =? 25;\n\t\tif (&result == null) { 11 } else { result }\n\t} else { 13 }\n};\n%%end";
    for layout in [ScalarTargetLayout::NATIVE64, ScalarTargetLayout::WASM32] {
        let parsed = parse_source(source.clone(), text.into(), &[]);
        assert!(parsed.diagnostics.is_empty(), "{:?}", parsed.diagnostics);
        let single = crate::derive_scalar_program_with_layout(&parsed.result, layout);
        assert!(single.diagnostics.is_empty(), "{:?}", single.diagnostics);
        for item in &single.program.items {
            let ScalarItem::Function(function) = item else { continue };
            assert!(
                function.reference_cfg.guarded_scalar_allocation().is_some(),
                "missing checked allocation proof for {}", function.name,
            );
        }
        let rebuilt = crate::scalar::PreparedProgram::rebuild_raw(&single.program).expect("raw rebuild");
        for item in &rebuilt.program().items {
            let ScalarItem::Function(function) = item else { continue };
            let proof = function.reference_cfg.guarded_scalar_allocation()
                .unwrap_or_else(|| panic!("raw rebuild dropped proof for {}", function.name));
            let declaration = proof.binding().declaration_span;
            assert_eq!(&text[declaration.start as usize..declaration.end as usize], "result");
            assert!(matches!(function.reference_cfg.points[proof.branch().0].kind, crate::scalar::CfgPointKind::Branch { .. }), "raw rebuild changed branch for {}", function.name);
        }
        let project = validate_scalar_project(ScalarProject::new(
            vec![ScalarModule::from_program(single.program.clone(), Vec::new())],
            vec![source.clone()],
        ));
        assert!(project.diagnostics.is_empty(), "{:?}", project.diagnostics);
        for (project_route, llvm) in [
            (false, emit_scalar_llvm(&single).expect("single LLVM").to_text()),
            (true, emit_scalar_project_llvm(&project).expect("project LLVM").to_text()),
        ] {
            if let Some(output_dir) = std::env::var_os("WOSY_FALLIBLE_TEST_IR_DIR") {
                let target = if layout == ScalarTargetLayout::WASM32 { "wasm32" } else { "native64" };
                let file = format!("{target}-{}.ll", if project_route { "project" } else { "standalone" });
                std::fs::write(std::path::PathBuf::from(output_dir).join(file), &llvm).expect("guarded test IR");
            }
            for (name, initial) in [("observed", 42), ("flipped", 19), ("nested", 25)] {
                let symbol = self_tail_test_symbol(&source, project_route, name);
                let body = self_tail_test_body(&llvm, "i32", &symbol);
                assert_eq!(body.matches("call ptr @__wosy_core_alloc(i64 4, i64 4)").count(), 1, "{body}");
                assert_eq!(body.matches("icmp ne ptr %allocation, null").count(), 1, "{body}");
                assert_eq!(body.matches("call void @__wosy_core_free(ptr %allocation)").count(), 1, "{body}");
                assert_eq!(body.matches(&format!("store i32 {initial}, ptr %allocation")).count(), 1, "{body}");
                assert!(body.contains("br i1 %fallible.present, label %fallible.initialize, label %fallible.continue"), "{body}");
                assert!(body.contains("br i1 %fallible.present, label %fallible.release, label %fallible.released"), "{body}");
                if name == "flipped" {
                    assert!(body.contains("br i1 %fallible.present, label %if.then, label %if.else"), "{body}");
                } else {
                    assert!(body.lines().any(|line| line.contains("br i1 %fallible.absent, label %if.then") && line.contains("label %if.else")), "{body}");
                }
                let (_, initialize) = body.split_once("\nfallible.initialize:").expect("initialize block");
                let (initialize, _) = initialize.split_once("\nfallible.continue:").expect("continue block");
                assert!(initialize.contains(&format!("store i32 {initial}, ptr %allocation")), "{body}");
                let (_, cleanup) = body.split_once("\nfallible.release:").expect("release block");
                let (cleanup, _) = cleanup.split_once("\nfallible.released:").expect("release continuation");
                assert!(cleanup.contains("call void @__wosy_core_free(ptr %allocation)"), "{body}");
            }
        }
    }
}

#[test]
fn guarded_integer_literal_uses_exact_declared_layout() {
    let source = SourceIdentity::new(
        "project".into(), "package".into(), "src/main.w".into(), "r1".into(),
    );
    for (ty, ir_ty, width, literal, stored) in [
        ("u8", "i8", 1, "241", "-15"),
        ("i16", "i16", 2, "30001", "30001"),
        ("u64", "i64", 8, "4294967301", "4294967301"),
        ("u128", "i128", 16, "18446744073709551619", "18446744073709551619"),
        ("u128", "i128", 16, "340282366920938463463374607431768211455", "-1"),
    ] {
        let text = format!("%%start\n{ty}() checked = fn {{\n\t{ty} result =? {literal};\n\tif (&result == null) {{ 3 }} else {{ result }}\n}};\n%%end");
        let parsed = parse_source(source.clone(), text, &[]);
        assert!(parsed.diagnostics.is_empty(), "{:?}", parsed.diagnostics);
        for layout in [ScalarTargetLayout::NATIVE64, ScalarTargetLayout::WASM32] {
            let single = crate::derive_scalar_program_with_layout(&parsed.result, layout);
            assert!(single.diagnostics.is_empty(), "{:?}", single.diagnostics);
            let llvm = emit_scalar_llvm(&single).expect("guarded integer LLVM").to_text();
            let body = self_tail_test_body(&llvm, ir_ty, "checked");
            assert_eq!(body.matches(&format!("call ptr @__wosy_core_alloc(i64 {width}, i64 {width})")).count(), 1, "{body}");
            assert_eq!(body.matches(&format!("store {ir_ty} {stored}, ptr %allocation")).count(), 1, "{body}");
            assert!(body.contains("call void @__wosy_core_free(ptr %allocation)"), "{body}");
        }
    }
}

#[test]
fn guarded_fallible_named_integer_initializer_stays_single_evaluation() {
    let source = SourceIdentity::new(
        "project".into(), "package".into(), "src/main.w".into(), "r1".into(),
    );
    let text = "%%start\ni64(i64) dynamic = fn(input) {\n\ti64 result =? input;\n\tif (null != &result) { result } else { 3 }\n};\n%%end";
    for layout in [ScalarTargetLayout::NATIVE64, ScalarTargetLayout::WASM32] {
        let parsed = parse_source(source.clone(), text.into(), &[]);
        assert!(parsed.diagnostics.is_empty(), "{:?}", parsed.diagnostics);
        let single = crate::derive_scalar_program_with_layout(&parsed.result, layout);
        assert!(single.diagnostics.is_empty(), "{:?}", single.diagnostics);
        let project = validate_scalar_project(ScalarProject::new(
            vec![ScalarModule::from_program(single.program.clone(), Vec::new())],
            vec![source.clone()],
        ));
        assert!(project.diagnostics.is_empty(), "{:?}", project.diagnostics);
        for (project_route, llvm) in [
            (false, emit_scalar_llvm(&single).expect("single LLVM").to_text()),
            (true, emit_scalar_project_llvm(&project).expect("project LLVM").to_text()),
        ] {
            let symbol = self_tail_test_symbol(&source, project_route, "dynamic");
            let body = self_tail_test_body(&llvm, "i64", &symbol);
            assert_eq!(body.matches("call ptr @__wosy_core_alloc(i64 8, i64 8)").count(), 1, "{body}");
            let input_slot = body.lines().find_map(|line| {
                line.trim_start().strip_prefix("store i64 %input, ptr ")
                    .and_then(|operands| operands.split(',').next())
            }).expect("input parameter spill");
            let mut loads = body.lines().filter_map(|line| {
                let (value, operands) = line.trim_start().split_once(" = load i64, ptr ")?;
                (operands.split(',').next() == Some(input_slot)).then_some(value)
            });
            let input_value = loads.next().expect("named initializer load");
            assert!(loads.next().is_none(), "{body}");
            assert_eq!(body.matches(&format!("store i64 {input_value}, ptr %allocation")).count(), 1, "{body}");
            assert_eq!(body.matches("call void @__wosy_core_free(ptr %allocation)").count(), 1, "{body}");
        }
    }
}

#[test]
fn guarded_integer_sibling_arms_use_only_their_own_allocation() {
    let source = SourceIdentity::new("project".into(), "package".into(), "src/main.w".into(), "r1".into());
    let text = r#"%%start
i32(bool) siblings = fn(take_left) {
	if (take_left) {
		u8 small =? 241;
		if (&small == null) { 7 } else { core.int_extend<i32>(small) }
	} else {
		i32 wide =? 30001;
		if (null != &wide) { wide } else { 9 }
	}
};
if (siblings(true) != 241 || siblings(false) != 30001) { core.system_panic(); };
%%end"#;
    let overlapping = "%%start
i32() overlap = fn {
	i32 first =? 11;
	i32 second =? 22;
	i32 one = if (&first != null) { first } else { 1 };
	i32 two = if (&second != null) { second } else { 2 };
	one + two
};
%%end";
    for layout in [ScalarTargetLayout::NATIVE64, ScalarTargetLayout::WASM32] {
        let parsed = parse_source(source.clone(), text.into(), &[]);
        assert!(parsed.diagnostics.is_empty(), "{:?}", parsed.diagnostics);
        let single = crate::derive_scalar_program_with_layout(&parsed.result, layout);
        assert!(single.diagnostics.is_empty(), "{:?}", single.diagnostics);
        let function = single.program.items.iter().find_map(|item| match item {
            ScalarItem::Function(function) if function.name == "siblings" => Some(function),
            _ => None,
        }).expect("sibling allocation callable");
        let [left, right] = function.reference_cfg.guarded_scalar_allocations() else { panic!("two exact sites required") };
        assert_eq!([left.ty(), right.ty()], [&ScalarType::U8, &ScalarType::I32]);
        assert_ne!(left.binding().block_span, right.binding().block_span);
        assert_ne!(left.condition(), right.condition());
        assert_eq!([left.present_on_then(), right.present_on_then()], [false, true]);
        let project = validate_scalar_project(ScalarProject::new(
            vec![ScalarModule::from_program(single.program.clone(), Vec::new())], vec![source.clone()],
        ));
        assert!(project.diagnostics.is_empty(), "{:?}", project.diagnostics);
        for (project_route, ir) in [
            (false, emit_scalar_llvm(&single).unwrap().to_text()),
            (true, emit_scalar_project_llvm(&project).unwrap().to_text()),
        ] {
            let symbol = self_tail_test_symbol(&source, project_route, "siblings");
            let body = self_tail_test_body(&ir, "i32", &symbol);
            assert_eq!(body.matches("call ptr @__wosy_core_alloc(").count(), 2, "{body}");
            assert_eq!(body.matches("call void @__wosy_core_free(").count(), 2, "{body}");
            assert!(body.contains("call ptr @__wosy_core_alloc(i64 1, i64 1)"), "{body}");
            assert!(body.contains("call ptr @__wosy_core_alloc(i64 4, i64 4)"), "{body}");
            if let Some(output_dir) = std::env::var_os("WOSY_FALLIBLE_TEST_IR_DIR") {
                let target = if layout == ScalarTargetLayout::WASM32 { "wasm32" } else { "native64" };
                let path = format!("{target}-{}-siblings.ll", if project_route { "project" } else { "standalone" });
                std::fs::write(std::path::PathBuf::from(output_dir).join(path), &ir).unwrap();
            }
        }
        let parsed = parse_source(source.clone(), overlapping.into(), &[]);
        assert!(parsed.diagnostics.is_empty(), "{:?}", parsed.diagnostics);
        let overlap = crate::derive_scalar_program_with_layout(&parsed.result, layout);
        assert!(overlap.diagnostics.is_empty(), "{:?}", overlap.diagnostics);
        let ir = emit_scalar_llvm(&overlap).unwrap().to_text();
        let body = self_tail_test_body(&ir, "i32", &self_tail_test_symbol(&source, false, "overlap"));
        assert_eq!(body.matches("call ptr @__wosy_core_alloc(").count(), 2, "{body}");
        assert_eq!(body.matches("call void @__wosy_core_free(").count(), 2, "{body}");
    }
}

#[test]
fn guarded_integer_sequential_scopes_do_not_overlap() {
    let source = SourceIdentity::new("project".into(), "package".into(), "src/main.w".into(), "r1".into());
    let text = r#"%%start
i32(bool, bool) sequential = fn(take_first, take_second) {
	i32 left = 1;
	i32 right = 2;
	if (take_first) {
		u8 small =? 241;
		left = if (&small == null) { 7 } else { core.int_extend<i32>(small) };
	};
	if (take_second) {
		i32 wide =? 30001;
		right = if (null != &wide) { wide } else { 9 };
	};
	left + right
};
if (sequential(true, true) != 30242 || sequential(false, false) != 3) { core.system_panic(); };
%%end"#;
    let nested = r#"%%start
i32() nested = fn {
	i32 outer =? 11;
	if (&outer == null) { 1 } else {
		i32 inner =? 22;
		if (&inner == null) { 2 } else { outer + inner }
	}
};
%%end"#;
    for layout in [ScalarTargetLayout::NATIVE64, ScalarTargetLayout::WASM32] {
        let parsed = parse_source(source.clone(), text.into(), &[]);
        assert!(parsed.diagnostics.is_empty(), "{:?}", parsed.diagnostics);
        let single = crate::derive_scalar_program_with_layout(&parsed.result, layout);
        assert!(single.diagnostics.is_empty(), "{:?}", single.diagnostics);
        let function = single.program.items.iter().find_map(|item| match item {
            ScalarItem::Function(function) if function.name == "sequential" => Some(function),
            _ => None,
        }).expect("sequential allocation callable");
        let [left, right] = function.reference_cfg.guarded_scalar_allocations() else { panic!("two exact sites required") };
        assert_eq!([left.ty(), right.ty()], [&ScalarType::U8, &ScalarType::I32]);
        assert!(left.binding().block_span.end < right.binding().block_span.start);
        assert_eq!([left.present_on_then(), right.present_on_then()], [false, true]);
        let project = validate_scalar_project(ScalarProject::new(
            vec![ScalarModule::from_program(single.program.clone(), Vec::new())], vec![source.clone()],
        ));
        assert!(project.diagnostics.is_empty(), "{:?}", project.diagnostics);
        for (project_route, ir) in [
            (false, emit_scalar_llvm(&single).unwrap().to_text()),
            (true, emit_scalar_project_llvm(&project).unwrap().to_text()),
        ] {
            let symbol = self_tail_test_symbol(&source, project_route, "sequential");
            let body = self_tail_test_body(&ir, "i32", &symbol);
            assert_eq!(body.matches("call ptr @__wosy_core_alloc(").count(), 2, "{body}");
            assert_eq!(body.matches("call void @__wosy_core_free(").count(), 2, "{body}");
            assert!(body.contains("call ptr @__wosy_core_alloc(i64 1, i64 1)"), "{body}");
            assert!(body.contains("call ptr @__wosy_core_alloc(i64 4, i64 4)"), "{body}");
            if let Some(output_dir) = std::env::var_os("WOSY_FALLIBLE_TEST_IR_DIR") {
                let target = if layout == ScalarTargetLayout::WASM32 { "wasm32" } else { "native64" };
                let path = format!("{target}-{}-sequential.ll", if project_route { "project" } else { "standalone" });
                std::fs::write(std::path::PathBuf::from(output_dir).join(path), &ir).unwrap();
            }
        }
        let parsed = parse_source(source.clone(), nested.into(), &[]);
        assert!(parsed.diagnostics.is_empty(), "{:?}", parsed.diagnostics);
        let nested = crate::derive_scalar_program_with_layout(&parsed.result, layout);
        assert!(nested.diagnostics.is_empty(), "{:?}", nested.diagnostics);
        let ir = emit_scalar_llvm(&nested).unwrap().to_text();
        let body = self_tail_test_body(&ir, "i32", &self_tail_test_symbol(&source, false, "nested"));
        assert_eq!(body.matches("call ptr @__wosy_core_alloc(").count(), 2, "{body}");
        assert_eq!(body.matches("call void @__wosy_core_free(").count(), 2, "{body}");
    }
}

#[test]
fn guarded_fallible_struct_initializes_the_exact_backing_fields() {
    let source = SourceIdentity::new("project".into(), "package".into(), "src/main.w".into(), "r1".into());
    let text = r#"%%start
struct Record { u8 tag; i32 amount; }
i32() observed = fn {
	Record held =? { .tag = 241; .amount = 30001; };
	if (&held == null) { 7 } else { core.int_extend<i32>(held.tag) + held.amount }
};
if (observed() != 30242) { core.system_panic(); };
%%end"#;
    for layout in [ScalarTargetLayout::NATIVE64, ScalarTargetLayout::WASM32] {
        let parsed = parse_source(source.clone(), text.into(), &[]);
        assert!(parsed.diagnostics.is_empty(), "{:?}", parsed.diagnostics);
        let single = crate::derive_scalar_program_with_layout(&parsed.result, layout);
        assert!(single.diagnostics.is_empty(), "{:?}", single.diagnostics);
        let project = validate_scalar_project(ScalarProject::new(
            vec![ScalarModule::from_program(single.program.clone(), Vec::new())],
            vec![source.clone()],
        ));
        assert!(project.diagnostics.is_empty(), "{:?}", project.diagnostics);
        for (project_route, ir) in [
            (false, emit_scalar_llvm(&single).unwrap().to_text()),
            (true, emit_scalar_project_llvm(&project).unwrap().to_text()),
        ] {
            let symbol = self_tail_test_symbol(&source, project_route, "observed");
            let body = self_tail_test_body(&ir, "i32", &symbol);
            assert_eq!(body.matches("call ptr @__wosy_core_alloc(i64 8, i64 4)").count(), 1, "{body}");
            assert_eq!(body.matches("call void @__wosy_core_free(ptr %allocation)").count(), 1, "{body}");
            assert!(!body.contains("struct_literal") && !body.contains("memmove") && !body.contains("memcpy"), "{body}");
            let (_, initialize) = body.split_once("\nfallible.initialize:").expect("initialize block");
            let (initialize, _) = initialize.split_once("\nfallible.continue:").expect("initialize continuation");
            assert!(initialize.contains("store i8 -15") && initialize.contains("store i32 30001"), "{body}");
            assert!(initialize.contains("getelementptr inbounds i8, ptr %allocation, i8 0")
                && initialize.contains("getelementptr inbounds i8, ptr %allocation, i8 4"), "{body}");
            if let Some(output_dir) = std::env::var_os("WOSY_FALLIBLE_TEST_IR_DIR") {
                let target = if layout == ScalarTargetLayout::WASM32 { "wasm32" } else { "native64" };
                let path = format!("{target}-{}-struct.ll", if project_route { "project" } else { "standalone" });
                std::fs::write(std::path::PathBuf::from(output_dir).join(path), &ir).unwrap();
            }
        }
        for (replacement, diagnostic) in [
            ("held.amount", "fallible allocation is not known to be available"),
            ("if (&held == null) { 7 } else { Record copied = held; copied.amount }", "whole fallible struct read requires a proven Copy type"),
            ("if (&held == null) { 7 } else { *i32 alias = &held.amount; *alias }", "fallible allocation access is not a supported whole Copy overwrite or direct guarded read"),
            ("if (&held == null) { 7 } else { held.amount = 1; held.amount }", "fallible allocation access is not a supported whole Copy overwrite or direct guarded read"),
        ] {
            let rejected = text.replace(
                "if (&held == null) { 7 } else { core.int_extend<i32>(held.tag) + held.amount }",
                replacement,
            );
            let parsed = parse_source(source.clone(), rejected, &[]);
            assert!(parsed.diagnostics.is_empty(), "{:?}", parsed.diagnostics);
            let rejected = crate::derive_scalar_program_with_layout(&parsed.result, layout);
            assert!(rejected.diagnostics.iter().any(|item| item.message == diagnostic), "{:?}", rejected.diagnostics);
        }
    }
}

#[test]
fn guarded_fallible_struct_reads_named_fields_once_before_allocation() {
    let source = SourceIdentity::new("project".into(), "package".into(), "src/main.w".into(), "r1".into());
    let text = r#"%%start
struct Record { u8 tag; i32 amount; }
i32(u8,i32) observed = fn(input_tag,input_wide) {
	Record held =? { .amount = input_wide; .tag = input_tag; };
	if (&held == null) { 7 } else { core.int_extend<i32>(held.tag) + held.amount }
};
if (observed(241,30001) != 30242) { core.system_panic(); };
%%end"#;
    for layout in [ScalarTargetLayout::NATIVE64, ScalarTargetLayout::WASM32] {
        let parsed = parse_source(source.clone(), text.into(), &[]);
        assert!(parsed.diagnostics.is_empty(), "{:?}", parsed.diagnostics);
        let single = crate::derive_scalar_program_with_layout(&parsed.result, layout);
        assert!(single.diagnostics.is_empty(), "{:?}", single.diagnostics);
        let project = validate_scalar_project(ScalarProject::new(
            vec![ScalarModule::from_program(single.program.clone(), Vec::new())],
            vec![source.clone()],
        ));
        assert!(project.diagnostics.is_empty(), "{:?}", project.diagnostics);
        for (project_route, ir) in [
            (false, emit_scalar_llvm(&single).unwrap().to_text()),
            (true, emit_scalar_project_llvm(&project).unwrap().to_text()),
        ] {
            let symbol = self_tail_test_symbol(&source, project_route, "observed");
            let body = self_tail_test_body(&ir, "i32", &symbol);
            let (before, after) = body
                .split_once("call ptr @__wosy_core_alloc(i64 8, i64 4)")
                .expect("exact allocation layout");
            assert!(!after.contains("call ptr @__wosy_core_alloc("), "{body}");
            let load_for = |ty: &str, name: &str| {
                let spill = format!("store {ty} %{name}, ptr ");
                let slot = body.lines().find_map(|line| {
                    line.trim_start().strip_prefix(spill.as_str())
                        .and_then(|operands| operands.split(',').next())
                }).expect("parameter spill");
                let instruction = format!(" = load {ty}, ptr {slot},");
                assert_eq!(body.matches(&instruction).count(), 1, "{body}");
                let load = before.find(&instruction).expect("load before allocator call");
                let value = before[..load].lines().last().unwrap().trim_start();
                (load, value)
            };
            let (wide_at, wide) = load_for("i32", "input_wide");
            let (tag_at, tag) = load_for("i8", "input_tag");
            assert!(wide_at < tag_at, "{body}");
            let (_, initialize) = after.split_once("\nfallible.initialize:").expect("present block");
            let (initialize, _) = initialize.split_once("\nfallible.continue:").expect("join block");
            assert!(initialize.contains(&format!("store i8 {tag}"))
                && initialize.contains(&format!("store i32 {wide}")), "{body}");
            assert_eq!(body.matches("call void @__wosy_core_free(ptr %allocation)").count(), 1, "{body}");
            assert!(!body.contains("struct_literal") && !body.contains("memmove") && !body.contains("memcpy"), "{body}");
            if let Some(output_dir) = std::env::var_os("WOSY_FALLIBLE_TEST_IR_DIR") {
                let target = if layout == ScalarTargetLayout::WASM32 { "wasm32" } else { "native64" };
                let path = format!("{target}-{}-named-struct.ll", if project_route { "project" } else { "standalone" });
                std::fs::write(std::path::PathBuf::from(output_dir).join(path), &ir).unwrap();
            }
        }
        let local = text.replace(
            "Record held =?",
            "i32 local_amount = input_wide; Record held =?",
        ).replace(".amount = input_wide;", ".amount = local_amount;");
        let parsed = parse_source(source.clone(), local, &[]);
        assert!(parsed.diagnostics.is_empty(), "{:?}", parsed.diagnostics);
        let result = crate::derive_scalar_program_with_layout(&parsed.result, layout);
        assert!(result.diagnostics.is_empty(), "{:?}", result.diagnostics);
        let project = validate_scalar_project(ScalarProject::new(
            vec![ScalarModule::from_program(result.program, Vec::new())],
            vec![source.clone()],
        ));
        assert!(project.diagnostics.is_empty(), "{:?}", project.diagnostics);
        let global = text.replace("i32(u8,i32)", "i32 module_amount = 30001;\ni32(u8,i32)")
            .replace(".amount = input_wide;", ".amount = module_amount;")
            .replace("{ 7 } else", "{ input_wide } else");
        let position = global.find(".amount = module_amount").unwrap() + ".amount = ".len();
        let span = wosy_syntax::ByteSpan::new(position as u32, (position + "module_amount".len()) as u32);
        let parsed = parse_source(source.clone(), global, &[]);
        assert!(parsed.diagnostics.is_empty(), "{:?}", parsed.diagnostics);
        let result = crate::derive_scalar_program_with_layout(&parsed.result, layout);
        let project = validate_scalar_project(ScalarProject::new(
            vec![ScalarModule::from_program(result.program, Vec::new())],
            vec![source.clone()],
        ));
        for diagnostics in [&result.diagnostics, &project.diagnostics] {
            assert!(diagnostics.iter().any(|item| {
                item.code == "B0003"
                    && item.message == "fallible struct initializer requires a literal or a lexical binding of the field type"
                    && item.labels.iter().any(|label| label.span.range == span)
            }), "{diagnostics:?}");
        }
        for (replacement, message) in [
            ("input_wide + 1", "fallible allocation requires a local integer"),
            ("input_tag", "expression type does not match expected type"),
        ] {
            let rejected = text.replace(".amount = input_wide;", &format!(".amount = {replacement};"))
                .replace("{ 7 } else", "{ input_wide } else");
            let parsed = parse_source(source.clone(), rejected, &[]);
            assert!(parsed.diagnostics.is_empty(), "{:?}", parsed.diagnostics);
            let rejected = crate::derive_scalar_program_with_layout(&parsed.result, layout);
            assert!(rejected.diagnostics.iter().any(|item| item.message.contains(message)), "{:?}", rejected.diagnostics);
        }
    }
}

#[test]
fn guarded_fallible_struct_snapshots_bool_fields() {
    let source = SourceIdentity::new("project".into(), "package".into(), "src/main.w".into(), "r1".into());
    let text = r#"%%start
struct Record { bool flag; i32 amount; bool marker; }
i32(bool,i32) observed = fn(input_flag,input_amount) {
	Record held =? { .marker = true; .amount = input_amount; .flag = input_flag; };
	if (&held == null) { 7 } else {
		if (!held.marker) { 11 } else { if (held.flag) { held.amount } else { 13 } }
	}
};
if (observed(true,30001) != 30001 || observed(false,30001) != 13) { core.system_panic(); };
%%end"#;
    for layout in [ScalarTargetLayout::NATIVE64, ScalarTargetLayout::WASM32] {
        let parsed = parse_source(source.clone(), text.into(), &[]);
        assert!(parsed.diagnostics.is_empty(), "{:?}", parsed.diagnostics);
        let single = crate::derive_scalar_program_with_layout(&parsed.result, layout);
        assert!(single.diagnostics.is_empty(), "{:?}", single.diagnostics);
        let project = validate_scalar_project(ScalarProject::new(
            vec![ScalarModule::from_program(single.program.clone(), Vec::new())],
            vec![source.clone()],
        ));
        assert!(project.diagnostics.is_empty(), "{:?}", project.diagnostics);
        for (project_route, ir) in [
            (false, emit_scalar_llvm(&single).unwrap().to_text()),
            (true, emit_scalar_project_llvm(&project).unwrap().to_text()),
        ] {
            let symbol = self_tail_test_symbol(&source, project_route, "observed");
            let body = self_tail_test_body(&ir, "i32", &symbol);
            let (before, after) = body
                .split_once("call ptr @__wosy_core_alloc(i64 12, i64 4)")
                .expect("mixed scalar allocation layout");
            assert!(!after.contains("call ptr @__wosy_core_alloc("), "{body}");
            let slot = before.lines().find_map(|line| {
                line.trim_start().strip_prefix("store i1 %input_flag, ptr ")
                    .and_then(|operands| operands.split(',').next())
            }).expect("Boolean parameter spill");
            let instruction = format!(" = load i1, ptr {slot},");
            assert_eq!(body.matches(&instruction).count(), 1, "{body}");
            let load = before.find(&instruction).expect("Boolean snapshot before allocation");
            let flag = before[..load].lines().last().unwrap().trim_start();
            let (_, initialize) = after.split_once("\nfallible.initialize:").expect("present block");
            let (initialize, _) = initialize.split_once("\nfallible.continue:").expect("join block");
            assert!(initialize.contains(&format!("store i1 {flag},")) && initialize.contains("store i1 true"), "{body}");
            for offset in [0, 4, 8] {
                assert!(initialize.contains(&format!("getelementptr inbounds i8, ptr %allocation, i8 {offset}")), "{body}");
            }
            assert_eq!(body.matches("call void @__wosy_core_free(ptr %allocation)").count(), 1, "{body}");
            assert!(!body.contains("struct_literal") && !body.contains("memmove") && !body.contains("memcpy"), "{body}");
            if let Some(output_dir) = std::env::var_os("WOSY_FALLIBLE_TEST_IR_DIR") {
                let target = if layout == ScalarTargetLayout::WASM32 { "wasm32" } else { "native64" };
                let path = format!("{target}-{}-bool-struct.ll", if project_route { "project" } else { "standalone" });
                std::fs::write(std::path::PathBuf::from(output_dir).join(path), &ir).unwrap();
            }
        }
        for (field, substitute, diagnostic) in [
            ("input_flag", "input_amount", "expression type does not match expected type"),
            ("input_flag", "!input_flag", "fallible allocation requires a local integer"),
            ("true", "global_flag", "fallible struct initializer requires a literal or a lexical binding of the field type"),
        ] {
            let initializer = if field == "true" {
                (".marker = true", format!(".marker = {substitute}"))
            } else {
                (".flag = input_flag", format!(".flag = {substitute}"))
            };
            let rejected = text.replace("%%start\n", "%%start\nbool global_flag = true;\n")
                .replace(initializer.0, &initializer.1)
                .replace("{ 7 } else", "{ input_amount } else");
            let parsed = parse_source(source.clone(), rejected, &[]);
            assert!(parsed.diagnostics.is_empty(), "{:?}", parsed.diagnostics);
            let single = crate::derive_scalar_program_with_layout(&parsed.result, layout);
            let project = validate_scalar_project(ScalarProject::new(
                vec![ScalarModule::from_program(single.program.clone(), Vec::new())],
                vec![source.clone()],
            ));
            for diagnostics in [&single.diagnostics, &project.diagnostics] {
                assert!(diagnostics.iter().any(|item| item.message.contains(diagnostic)), "{diagnostics:?}");
            }
        }
    }
}

#[test]
fn guarded_fallible_struct_snapshots_char_fields() {
    let source = SourceIdentity::new("project".into(), "package".into(), "src/main.w".into(), "r1".into());
    let text = r#"%%start
struct Record { bool active; char observed; char marker; }
u32(char) inspect = fn(input_char) {
	Record held =? { .marker = 'Q'; .observed = input_char; .active = true; };
	if (&held == null) { 7 } else {
		if (!held.active || held.marker != 'Q') { 11 } else { core.bitcast<u32>(held.observed) }
	}
};
if (inspect('G') != 71) { core.system_panic(); };
%%end"#;
    for layout in [ScalarTargetLayout::NATIVE64, ScalarTargetLayout::WASM32] {
        let parsed = parse_source(source.clone(), text.into(), &[]);
        assert!(parsed.diagnostics.is_empty(), "{:?}", parsed.diagnostics);
        let single = crate::derive_scalar_program_with_layout(&parsed.result, layout);
        assert!(single.diagnostics.is_empty(), "{:?}", single.diagnostics);
        let project = validate_scalar_project(ScalarProject::new(
            vec![ScalarModule::from_program(single.program.clone(), Vec::new())],
            vec![source.clone()],
        ));
        assert!(project.diagnostics.is_empty(), "{:?}", project.diagnostics);
        for (project_route, ir) in [
            (false, emit_scalar_llvm(&single).unwrap().to_text()),
            (true, emit_scalar_project_llvm(&project).unwrap().to_text()),
        ] {
            let symbol = self_tail_test_symbol(&source, project_route, "inspect");
            let body = self_tail_test_body(&ir, "i32", &symbol);
            let (before, after) = body
                .split_once("call ptr @__wosy_core_alloc(i64 12, i64 4)")
                .expect("mixed scalar allocation layout");
            assert!(!after.contains("call ptr @__wosy_core_alloc("), "{body}");
            let slot = before.lines().find_map(|line| {
                line.trim_start().strip_prefix("store i32 %input_char, ptr ")
                    .and_then(|operands| operands.split(',').next())
            }).expect("character parameter spill");
            let instruction = format!(" = load i32, ptr {slot},");
            assert_eq!(body.matches(&instruction).count(), 1, "{body}");
            let load = before.find(&instruction).expect("character snapshot before allocation");
            let character = before[..load].lines().last().unwrap().trim_start();
            let (_, initialize) = after.split_once("\nfallible.initialize:").expect("present block");
            let (initialize, _) = initialize.split_once("\nfallible.continue:").expect("join block");
            assert!(initialize.contains(&format!("store i32 {character},")), "{body}");
            assert!(initialize.contains("store i32 81,") && initialize.contains("store i1 true,"), "{body}");
            for offset in [0, 4, 8] {
                assert!(initialize.contains(&format!("getelementptr inbounds i8, ptr %allocation, i8 {offset}")), "{body}");
            }
            assert_eq!(body.matches("call void @__wosy_core_free(ptr %allocation)").count(), 1, "{body}");
            assert!(!body.contains("struct_literal") && !body.contains("memmove") && !body.contains("memcpy"), "{body}");
            if let Some(output_dir) = std::env::var_os("WOSY_FALLIBLE_TEST_IR_DIR") {
                let target = if layout == ScalarTargetLayout::WASM32 { "wasm32" } else { "native64" };
                let path = format!("{target}-{}-char-struct.ll", if project_route { "project" } else { "standalone" });
                std::fs::write(std::path::PathBuf::from(output_dir).join(path), &ir).unwrap();
            }
        }
        for (from, to, diagnostic) in [
            (".marker = 'Q'", ".marker = 81", "expression type does not match expected type"),
            (".marker = 'Q'", ".marker = global_marker", "fallible struct initializer requires a literal or a lexical binding of the field type"),
            (".observed = input_char", ".observed = identity(input_char)", "fallible allocation requires a local integer"),
        ] {
            let rejected = text.replace("%%start\n", "%%start\nchar global_marker = 'Q';\nchar(char) identity = fn(value) { value };\n")
                .replace(from, to);
            let parsed = parse_source(source.clone(), rejected, &[]);
            assert!(parsed.diagnostics.is_empty(), "{:?}", parsed.diagnostics);
            let single = crate::derive_scalar_program_with_layout(&parsed.result, layout);
            let project = validate_scalar_project(ScalarProject::new(
                vec![ScalarModule::from_program(single.program.clone(), Vec::new())],
                vec![source.clone()],
            ));
            for diagnostics in [&single.diagnostics, &project.diagnostics] {
                assert!(diagnostics.iter().any(|item| item.message.contains(diagnostic)), "{diagnostics:?}");
            }
        }
    }
}

#[test]
fn guarded_fallible_struct_preserves_float_field_bits() {
    let source = SourceIdentity::new("project".into(), "package".into(), "src/main.w".into(), "r1".into());
    let text = r#"%%start
struct Record { u8 tag; f32 observed; f64 marker; }
u32(f32) inspect = fn(input_float) {
	Record held =? { .marker = -0.0; .observed = input_float; .tag = 3; };
	if (&held == null) { 7 } else {
		if (held.tag != 3 || core.bitcast<u64>(held.marker) != 9223372036854775808) {
			11
		} else {
			core.bitcast<u32>(held.observed)
		}
	}
};
if (inspect(1.25) != 1067450368) { core.system_panic(); };
%%end"#;
    for layout in [ScalarTargetLayout::NATIVE64, ScalarTargetLayout::WASM32] {
        let parsed = parse_source(source.clone(), text.into(), &[]);
        assert!(parsed.diagnostics.is_empty(), "{:?}", parsed.diagnostics);
        let single = crate::derive_scalar_program_with_layout(&parsed.result, layout);
        assert!(single.diagnostics.is_empty(), "{:?}", single.diagnostics);
        let project = validate_scalar_project(ScalarProject::new(
            vec![ScalarModule::from_program(single.program.clone(), Vec::new())],
            vec![source.clone()],
        ));
        assert!(project.diagnostics.is_empty(), "{:?}", project.diagnostics);
        for (project_route, ir) in [
            (false, emit_scalar_llvm(&single).unwrap().to_text()),
            (true, emit_scalar_project_llvm(&project).unwrap().to_text()),
        ] {
            let symbol = self_tail_test_symbol(&source, project_route, "inspect");
            let body = self_tail_test_body(&ir, "i32", &symbol);
            let (before, after) = body
                .split_once("call ptr @__wosy_core_alloc(i64 16, i64 8)")
                .expect("mixed scalar allocation layout");
            assert!(!after.contains("call ptr @__wosy_core_alloc("), "{body}");
            let slot = before.lines().find_map(|line| {
                line.trim_start().strip_prefix("store float %input_float, ptr ")
                    .and_then(|operands| operands.split(',').next())
            }).expect("float parameter spill");
            let instruction = format!(" = load float, ptr {slot},");
            assert_eq!(body.matches(&instruction).count(), 1, "{body}");
            let load = before.find(&instruction).expect("float snapshot before allocation");
            let value = before[..load].lines().last().unwrap().trim_start();
            let (_, initialize) = after.split_once("\nfallible.initialize:").expect("present block");
            let (initialize, _) = initialize.split_once("\nfallible.continue:").expect("join block");
            assert!(initialize.contains(&format!("store float {value},")), "{body}");
            assert!(initialize.contains("store double -0.000000e+00,") && initialize.contains("store i8 3,"), "{body}");
            for offset in [0, 4, 8] {
                assert!(initialize.contains(&format!("getelementptr inbounds i8, ptr %allocation, i8 {offset}")), "{body}");
            }
            assert_eq!(body.matches("call void @__wosy_core_free(ptr %allocation)").count(), 1, "{body}");
            assert!(!body.contains("struct_literal") && !body.contains("memmove") && !body.contains("memcpy"), "{body}");
            if let Some(output_dir) = std::env::var_os("WOSY_FALLIBLE_TEST_IR_DIR") {
                let target = if layout == ScalarTargetLayout::WASM32 { "wasm32" } else { "native64" };
                let path = format!("{target}-{}-float-struct.ll", if project_route { "project" } else { "standalone" });
                std::fs::write(std::path::PathBuf::from(output_dir).join(path), &ir).unwrap();
            }
        }
        for (from, to, diagnostic) in [
            (".marker = -0.0", ".marker = input_float", "expression type does not match expected type"),
            (".observed = input_float", ".observed = 1.0e39", "floating-point literal is outside the f32 range"),
            (".marker = -0.0", ".marker = global_marker", "fallible struct initializer requires a literal or a lexical binding of the field type"),
            (".observed = input_float", ".observed = -input_float", "fallible allocation requires a local integer"),
        ] {
            let rejected = text.replace("%%start\n", "%%start\nf64 global_marker = 0.0;\n")
                .replace(from, to);
            let parsed = parse_source(source.clone(), rejected, &[]);
            assert!(parsed.diagnostics.is_empty(), "{:?}", parsed.diagnostics);
            let single = crate::derive_scalar_program_with_layout(&parsed.result, layout);
            let project = validate_scalar_project(ScalarProject::new(
                vec![ScalarModule::from_program(single.program.clone(), Vec::new())],
                vec![source.clone()],
            ));
            for diagnostics in [&single.diagnostics, &project.diagnostics] {
                assert!(diagnostics.iter().any(|item| item.message.contains(diagnostic)), "{diagnostics:?}");
            }
        }
    }
}

#[test]
fn guarded_fallible_whole_copy_overwrites_one_original_backing() {
    let source = SourceIdentity::new("project".into(), "package".into(), "src/main.w".into(), "r1".into());
    let text = include_str!("../../tests/fixtures/guarded-fallible-whole-copy.w");
    for layout in [ScalarTargetLayout::NATIVE64, ScalarTargetLayout::WASM32] {
        let parsed = parse_source(source.clone(), text.into(), &[]);
        assert!(parsed.diagnostics.is_empty(), "{:?}", parsed.diagnostics);
        let single = crate::derive_scalar_program_with_layout(&parsed.result, layout);
        assert!(single.diagnostics.is_empty(), "{:?}", single.diagnostics);
        let project = validate_scalar_project(ScalarProject::new(
            vec![ScalarModule::from_program(single.program.clone(), Vec::new())],
            vec![source.clone()],
        ));
        assert!(project.diagnostics.is_empty(), "{:?}", project.diagnostics);
        for (project_route, ir) in [
            (false, emit_scalar_llvm(&single).unwrap().to_text()),
            (true, emit_scalar_project_llvm(&project).unwrap().to_text()),
        ] {
            let symbol = self_tail_test_symbol(&source, project_route, "observed");
            let body = self_tail_test_body(&ir, "i32", &symbol);
            assert_eq!(body.matches("call ptr @__wosy_core_alloc(i64 8, i64 4)").count(), 1, "{body}");
            assert_eq!(body.matches("call void @__wosy_core_free(ptr %allocation)").count(), 1, "{body}");
            assert!(body.contains("br i1 %fallible.present, label %if.then, label %if.else"), "{body}");
            assert!(body.contains("br i1 %fallible.present, label %fallible.release, label %fallible.released"), "{body}");
            let (_, present) = body.split_once("\nif.then:").expect("present edge");
            let (present, _) = present.split_once("\nif.else:").expect("absent edge");
            let moves = present.lines().filter(|line| line.contains("call void @llvm.memmove")).collect::<Vec<_>>();
            assert_eq!(moves.len(), body.matches("call void @llvm.memmove").count(), "{body}");
            let reads = moves.iter().enumerate().filter(|(_, row)| row.contains(", ptr align 1 %allocation,"))
                .map(|(index, _)| index).collect::<Vec<_>>();
            let writes = moves.iter().enumerate().filter(|(_, row)| row.contains("(ptr align 1 %allocation,"))
                .map(|(index, _)| index).collect::<Vec<_>>();
            let [read_before, read_after] = reads.as_slice() else { panic!("two snapshots from original backing: {body}") };
            let [overwrite] = writes.as_slice() else { panic!("one overwrite of original backing: {body}") };
            assert!(read_before < overwrite && overwrite < read_after, "{body}");
            if let Some(directory) = std::env::var_os("WOSY_FALLIBLE_WHOLE_COPY_IR_DIR") {
                let target = if layout == ScalarTargetLayout::WASM32 { "wasm32" } else { "native64" };
                let route = if project_route { "project" } else { "standalone" };
                std::fs::write(std::path::PathBuf::from(directory).join(format!("{target}-{route}.ll")), &ir).unwrap();
            }
        }
    }
}

#[test]
fn guarded_fallible_named_aliases_keep_one_backing_and_one_presence_guard() {
    let source = SourceIdentity::new("project".into(), "package".into(), "src/main.w".into(), "r1".into());
    let text = include_str!("../../tests/fixtures/guarded-fallible-named-alias.w");
    for layout in [ScalarTargetLayout::NATIVE64, ScalarTargetLayout::WASM32] {
        let parsed = parse_source(source.clone(), text.into(), &[]);
        assert!(parsed.diagnostics.is_empty(), "{:?}", parsed.diagnostics);
        let single = crate::derive_scalar_program_with_layout(&parsed.result, layout);
        assert!(single.diagnostics.is_empty(), "{:?}", single.diagnostics);
        for item in &single.program.items {
            let ScalarItem::Function(function) = item else { continue };
            let proof = function.reference_cfg.guarded_scalar_allocation().expect("one exact guard");
            assert_eq!(proof.alias_accesses().len(), if function.name == "shared" { 1 } else { 3 });
        }
        let project = validate_scalar_project(ScalarProject::new(
            vec![ScalarModule::from_program(single.program.clone(), Vec::new())],
            vec![source.clone()],
        ));
        assert!(project.diagnostics.is_empty(), "{:?}", project.diagnostics);
        for (project_route, llvm) in [
            (false, emit_scalar_llvm(&single).expect("single LLVM").to_text()),
            (true, emit_scalar_project_llvm(&project).expect("project LLVM").to_text()),
        ] {
            for (name, condition, positive, negative, reads, writes) in [
                ("shared", "absent", "if.else", "if.then", 1, 0),
                ("exclusive", "present", "if.then", "if.else", 2, 1),
            ] {
                let body = self_tail_test_body(&llvm, "i64", &self_tail_test_symbol(&source, project_route, name));
                assert_eq!(body.matches("call ptr @__wosy_core_alloc(i64 8, i64 8)").count(), 1, "{body}");
                assert_eq!(body.matches("call void @__wosy_core_free(ptr %allocation)").count(), 1, "{body}");
                assert_eq!(body.matches("icmp ne ptr %allocation, null").count(), 1, "{body}");
                assert!(body.contains(&format!("br i1 %fallible.{condition}, label %if.then, label %if.else")), "{body}");
                assert!(body.contains("br i1 %fallible.present, label %fallible.release, label %fallible.released"), "{body}");
                assert_eq!(body.matches("ptrtoint ptr %allocation to ").count(), 1, "{body}");
                assert!(!body.lines().any(|row|
                    row.trim_start().starts_with("%deref_is_null")
                        || row.starts_with("deref_null_panic")
                        || row.starts_with("deref_null_continue")), "{body}");
                let release_prefix = "wosy_fn__776f73795f746f6f6c636861696e5f7374646c6962__7372632f636865636b65645f6163636573732e77__";
                let null_helper_suffix = "__636865636b5f6e6f6e5f6e756c6c5f61646472657373";
                assert!(!body.lines().any(|row| row.split_once("call void @")
                    .and_then(|(_, callee)| callee.split_once('('))
                    .is_some_and(|(callee, _)| callee.starts_with(release_prefix)
                        && callee.ends_with(null_helper_suffix))), "{body}");
                let basic_block = |label: &str| {
                    let suffix = body.split_once(&format!("\n{label}:")).unwrap().1;
                    suffix.split_once("\n\n").unwrap().0
                };
                let good = basic_block(positive);
                assert_eq!(good.lines().filter(|line| line.contains("load i64, ptr %deref")).count(), reads, "{body}");
                assert_eq!(good.lines().filter(|line| line.contains("store i64 ") && line.contains(", ptr %deref")).count(), writes, "{body}");
                assert!(!basic_block(negative).contains("ptr %deref"), "{body}");
            }
            if let Some(directory) = std::env::var_os("WOSY_FALLIBLE_ALIAS_IR_DIR") {
                let target = if layout == ScalarTargetLayout::WASM32 { "wasm32" } else { "native64" };
                let route = if project_route { "project" } else { "standalone" };
                let directory = std::path::PathBuf::from(directory);
                std::fs::write(directory.join(format!("{target}-{route}.ll")), &llvm).unwrap();
                let mut controlled = llvm.replace("define i32 @main(", "define i32 @wosy_authored_main(");
                for name in ["shared", "exclusive"] {
                    let callee = self_tail_test_symbol(&source, project_route, name);
                    controlled.push_str(&format!(
                        "\ndefine i64 @wosy_test_{name}(i64 %input) {{\nentry:\n  %result = call i64 @{callee}(i64 %input)\n  ret i64 %result\n}}\n"
                    ));
                }
                std::fs::write(directory.join(format!("{target}-{route}-control.ll")), controlled).unwrap();
            }
        }
    }
}

#[test]
fn guarded_fallible_composed_conditions_keep_one_presence_and_one_cleanup() {
    let source = SourceIdentity::new("project".into(), "package".into(), "src/main.w".into(), "r1".into());
    let text = include_str!("../../tests/fixtures/guarded-fallible-composed.w");
    for layout in [ScalarTargetLayout::NATIVE64, ScalarTargetLayout::WASM32] {
        let parsed = parse_source(source.clone(), text.into(), &[]);
        assert!(parsed.diagnostics.is_empty(), "{:?}", parsed.diagnostics);
        let single = crate::derive_scalar_program_with_layout(&parsed.result, layout);
        assert!(single.diagnostics.is_empty(), "{:?}", single.diagnostics);
        for function in single.program.items.iter().filter_map(|item| match item {
            ScalarItem::Function(function) => Some(function),
            _ => None,
        }) {
            let proof = function.reference_cfg.guarded_scalar_allocation().expect("one exact allocation");
            let [first, second, nested] = proof.guards() else { panic!("three guarded occurrences"); };
            assert_eq!([first.present_on_then, second.present_on_then, nested.present_on_then], [true, false, true]);
            let points = &function.reference_cfg.points;
            assert_eq!(points[first.branch.0].scope, points[second.branch.0].scope);
            assert_ne!(points[second.branch.0].scope, points[nested.branch.0].scope);
            assert_eq!(proof.alias_accesses().len(), if function.name == "shared" { 1 } else { 3 });
        }
        let project = validate_scalar_project(ScalarProject::new(
            vec![ScalarModule::from_program(single.program.clone(), Vec::new())],
            vec![source.clone()],
        ));
        assert!(project.diagnostics.is_empty(), "{:?}", project.diagnostics);
        for (project_route, llvm) in [
            (false, emit_scalar_llvm(&single).expect("single LLVM").to_text()),
            (true, emit_scalar_project_llvm(&project).expect("project LLVM").to_text()),
        ] {
            for name in ["shared", "exclusive"] {
                let body = self_tail_test_body(&llvm, "i64", &self_tail_test_symbol(&source, project_route, name));
                assert_eq!(body.matches("call ptr @__wosy_core_alloc(i64 8, i64 8)").count(), 1, "{body}");
                assert_eq!(body.matches("call void @__wosy_core_free(ptr %allocation)").count(), 1, "{body}");
                assert_eq!(body.matches("icmp ne ptr %allocation, null").count(), 1, "{body}");
                assert_eq!(body.matches("br i1 %fallible.present, label %if.then").count(), 2, "{body}");
                assert_eq!(body.matches("br i1 %fallible.absent, label %if.then").count(), 1, "{body}");
                assert!(body.contains("br i1 %fallible.present, label %fallible.release, label %fallible.released"), "{body}");
                assert!(!body.contains("deref_is_null") && !body.contains("deref_null_panic")
                    && !body.contains("__636865636b5f6e6f6e5f6e756c6c5f61646472657373("), "{body}");
            }
            if let Some(directory) = std::env::var_os("WOSY_FALLIBLE_COMPOSED_IR_DIR") {
                let target = if layout == ScalarTargetLayout::WASM32 { "wasm32" } else { "native64" };
                let route = if project_route { "project" } else { "standalone" };
                let mut controlled = llvm.replace("define i32 @main(", "define i32 @wosy_authored_main(");
                for name in ["shared", "exclusive"] {
                    let callee = self_tail_test_symbol(&source, project_route, name);
                    controlled.push_str(&format!(
                        "\ndefine i64 @wosy_test_{name}(i64 %input) {{\nentry:\n  %result = call i64 @{callee}(i64 %input)\n  ret i64 %result\n}}\n"
                    ));
                }
                std::fs::write(std::path::PathBuf::from(directory).join(format!("{target}-{route}-control.ll")), controlled).unwrap();
            }
        }
    }
}

#[test]
fn closed_nested_copy_local_result_uses_caller_storage_for_scalar_projection() {
    let text = "%%start
struct copy Part { u8 tag; u64 score; }
struct copy Bundle { Part selected; u64 other; }
Bundle() _make_bundle = fn {
    Bundle prepared_value = { .selected = { .tag = 7; .score = 41; }; .other = 17; };
    prepared_value
};
u64() observe = fn { Bundle received = _make_bundle(); received.selected.score };
%%end";
    for layout in [ScalarTargetLayout::WASM32, ScalarTargetLayout::NATIVE64] {
        let checked_project = prepare_initializer_test(text, layout);
        let checked = analyze_scalar_build(&checked_project);
        assert!(checked.diagnostics().is_empty(), "{:?}", checked.diagnostics());
        let source = &checked_project.project().modules[0].source;
        for (producer, caller, partition) in [
            ("_make_bundle".to_owned(), "observe".to_owned(),
                emit_scalar_build_single_module_llvm(&checked).unwrap().to_text()),
            (project_function_name(source, "_make_bundle"), project_function_name(source, "observe"),
                emit_scalar_build_project_llvm(&checked).unwrap().to_text()),
        ] {
            let producer_body = partition.split_once(&format!("define internal void @{producer}("))
                .expect("producer caller-owned boundary").1.split_once("\n}").unwrap().0;
            let (producer_args, producer_ir) = producer_body.split_once(')').unwrap();
            let destination = producer_args.strip_prefix("ptr ").expect("caller storage parameter");
            let transfer = producer_ir.lines().find(|line| {
                line.contains("@llvm.memmove.")
                    && line.contains(&format!("ptr align 1 {destination},"))
                    && (line.contains("i32 24, i1 false") || line.contains("i64 24, i1 false"))
            }).expect("layout-sized pre-return transfer");
            assert!(producer_ir.find(transfer).unwrap() < producer_ir.find("ret void").unwrap());
            assert!(!producer_ir.contains("ret ptr"), "{producer_ir}");
            let caller_ir = partition.split_once(&format!("define i64 @{caller}("))
                .expect("ordinary scalar caller").1.split_once("\n}").unwrap().0;
            assert_eq!(caller_ir.matches(&format!("call void @{producer}(")).count(), 1);
            assert!(caller_ir.contains("alloca [24 x i8], align 8"), "{caller_ir}");
            assert!(caller_ir.contains("ordinary_result_destination"), "{caller_ir}");
            assert!(caller_ir.contains("ret i64"), "{caller_ir}");
        }
    }
}

#[test]
fn value_field_projection_loads_only_from_proven_caller_storage() {
    for expression in ["_make(seed).value", "(_make(seed)).value"] {
        let text = format!("%%start
struct copy Pair {{ u8 tag; u64 value; }}
Pair(u64) _make = fn(seed) {{ {{ .tag = 7; .value = seed; }} }};
u64(u64) verify = fn(seed) {{ {expression} }};
%%end");
        for layout in [ScalarTargetLayout::WASM32, ScalarTargetLayout::NATIVE64] {
            let prepared = prepare_initializer_test(&text, layout);
            let checked = analyze_scalar_build(&prepared);
            assert!(checked.diagnostics().is_empty(), "{:?}", checked.diagnostics());
            let source = &prepared.project().modules[0].source;
            for (make, verify, partition) in [
                ("_make".to_owned(), "verify".to_owned(),
                    emit_scalar_build_single_module_llvm(&checked).unwrap()),
                (project_function_name(source, "_make"), project_function_name(source, "verify"),
                    emit_scalar_build_project_llvm(&checked).unwrap()),
            ] {
                let llvm = partition.to_text();
                assert!(llvm.contains(&format!("define internal void @{make}(")), "{llvm}");
                let verify = llvm.split_once(&format!("define i64 @{verify}(")).unwrap()
                    .1.split_once("\n}").unwrap().0;
                assert_eq!(verify.matches(&format!("call void @{make}(")).count(), 1);
                assert!(verify.contains("%ordinary_result_destination = alloca [16 x i8], align 8"));
                assert!(verify.contains("load i64, ptr %value"), "{verify}");
                assert!(!verify.contains("call ptr "), "{verify}");
            }
        }
    }
}

#[test]
fn value_field_projection_rejects_legacy_result_without_storage_proof() {
    let text = "%%start
struct copy Pair { i32 value; }
Pair(i32) make = fn(seed) { { .value = seed; } };
i32(i32) verify = fn(seed) { make(seed).value };
%%end";
    let prepared = prepare_initializer_test(text, ScalarTargetLayout::WASM32);
    let checked = analyze_scalar_build(&prepared);
    assert!(checked.diagnostics().is_empty(), "{:?}", checked.diagnostics());
    for result in [emit_scalar_build_single_module_llvm(&checked), emit_scalar_build_project_llvm(&checked)] {
        assert!(matches!(result, Err(message) if message.contains("no proven caller-owned storage")));
    }
}

#[test]
fn value_field_projection_reports_the_selected_field_for_typing_errors() {
    for (structure, field, message) in [
        ("struct Pair { i32 value; }", "value", "field value receiver must be a copy struct"),
        ("struct copy Pair { i32 value; }", "missing", "unknown struct field"),
    ] {
        let text = format!("%%start
{structure}
Pair(i32) _make = fn(seed) {{ {{ .value = seed; }} }};
i32(i32) verify = fn(seed) {{ _make(seed).{field} }};
%%end");
        let source = SourceIdentity::new("project".into(), "package".into(), "src/main.w".into(), "r1".into());
        let start = text.rfind(field).unwrap() as u32;
        let parsed = parse_source(source.clone(), text, &[]);
        assert!(parsed.diagnostics.is_empty(), "{:?}", parsed.diagnostics);
        let rejected = prepare_scalar_project(
            vec![(derive_scalar_declarations_from_cst(parsed.result.canonical_cst()), Vec::new())],
            vec![source],
            ScalarPreparationInputs { target_layout: ScalarTargetLayout::WASM32 },
        );
        let error = rejected.err().expect("unsupported field type");
        let diagnostic = error.diagnostics.iter().find(|diagnostic| diagnostic.message == message)
            .expect("field diagnostic");
        assert_eq!(diagnostic.labels[0].span.range, ByteSpan::new(start, start + field.len() as u32));
    }
}

#[test]
fn closed_copy_parameter_result_writes_caller_storage_before_returning() {
    let text = "%%start
struct copy Pair { u8 tag; u64 total; }
Pair(u64) _make = fn(seed) { { .tag = 7; .total = seed; } };
u64(u64) verify = fn(seed) { Pair item = _make(seed); item.total };
u64(u64) verify_other = fn(seed) { Pair item = _make(seed); item.total };
%%end";
    for (layout, size_ty) in [
        (ScalarTargetLayout::WASM32, "i32"),
        (ScalarTargetLayout::NATIVE64, "i64"),
    ] {
        let prepared = prepare_initializer_test(text, layout);
        let checked = analyze_scalar_build(&prepared);
        assert!(checked.diagnostics().is_empty(), "{:?}", checked.diagnostics());
        let source = &prepared.project().modules[0].source;
        for (producer, caller, partition) in [
            ("_make".to_owned(), "verify".to_owned(),
                emit_scalar_build_single_module_llvm(&checked).unwrap()),
            (project_function_name(source, "_make"), project_function_name(source, "verify"),
                emit_scalar_build_project_llvm(&checked).unwrap()),
        ] {
            let llvm = partition.to_text();
            let (_, tail) = llvm.split_once(&format!("define internal void @{producer}(i64 %seed, ptr "))
                .expect("checked caller destination parameter");
            let (destination, body) = tail.split_once(')').expect("producer parameters");
            let (body, _) = body.split_once("\n}").expect("producer end");
            let copy = body.lines().find(|line| line.contains("@llvm.memmove.")
                && line.contains(&format!("ptr align 1 {destination},"))
                && line.contains(&format!("{size_ty} 16, i1 false")))
                .expect("aligned result bytes copied into caller destination");
            assert!(body.contains("store i64 %seed"), "{body}");
            assert!(body.find(copy).unwrap() < body.find("ret void").unwrap(), "{body}");
            assert!(!body.contains("ret ptr"), "{body}");

            let caller = llvm.split_once(&format!("define i64 @{caller}(")).expect("caller")
                .1.split_once("\n}").expect("caller end").0;
            assert!(caller.contains("alloca [16 x i8], align 8"), "{caller}");
            assert_eq!(caller.matches(&format!("call void @{producer}(")).count(), 1);
            assert!(caller.lines().any(|line| line.contains(&format!("call void @{producer}("))
                && line.ends_with(", ptr %ordinary_result_destination)")), "{caller}");
            assert!(caller.contains("ret i64 "), "{caller}");
            let other = if producer == "_make" {
                "verify_other".to_owned()
            } else {
                project_function_name(source, "verify_other")
            };
            let other = llvm.split_once(&format!("define i64 @{other}(")).unwrap()
                .1.split_once("\n}").unwrap().0;
            assert_eq!(other.matches(&format!("call void @{producer}(")).count(), 1);
        }
    }
}

#[test]
fn namespace_copy_snapshots_use_distinct_caller_slots_in_startup_and_value_arms() {
    let root = "%%start
shapes = namespace package \"shapes.w\";
seeds = namespace package \"seeds.w\";
shapes.Bundle stored = seeds.first();
struct copy SideNote { u64 value; }
SideNote(u64) _note = fn(seed) { { .value = seed; } };
u64(bool) choose = fn(flag) {
    u64 prior = stored.unit_item.total + _note(2).value;
    stored = if (flag) { seeds.first() } else { seeds.second() };
    prior * 100 + stored.unit_item.total
};
%%end";
    let shapes = "%%start
struct copy Sample { u64 total; }
struct copy Bundle { Sample unit_item; u64 guard; }
%%end";
    let seeds = "%%start
shapes = namespace package \"shapes.w\";
shapes.Sample yellow = { .total = 13; };
shapes.Sample blue = { .total = 29; };
shapes.Bundle() first = fn {
    shapes.Bundle snapshot = { .unit_item = yellow; .guard = 7; };
    snapshot
};
shapes.Bundle() second = fn {
    shapes.Bundle snapshot = { .unit_item = blue; .guard = 19; };
    snapshot
};
%%end";
    let source = |path: &str| SourceIdentity::new("project".into(), "package".into(), path.into(), "r1".into());
    let main = source("main.w");
    let seed_source = source("seeds.w");
    let shape_source = source("shapes.w");
    let declarations = |source, text: &str| {
        let parsed = parse_source(source, text.into(), &[]);
        assert!(parsed.diagnostics.is_empty(), "{:?}", parsed.diagnostics);
        derive_scalar_declarations_from_cst(parsed.result.canonical_cst())
    };
    let bindings = |input: &crate::ScalarSourceDeclarations, targets: &[&SourceIdentity]| {
        input.namespaces().zip(targets.iter().copied()).map(|(namespace, target)| ScalarNamespaceBinding {
            binding: namespace.binding.clone(), span: namespace.span, target: target.clone(),
        }).collect::<Vec<_>>()
    };
    let prepare = |target_layout, root_text: &str, seed_text: &str| {
        let root = declarations(main.clone(), root_text);
        let seeds = declarations(seed_source.clone(), seed_text);
        let root_bindings = bindings(&root, &[&shape_source, &seed_source]);
        let seed_bindings = bindings(&seeds, &[&shape_source]);
        prepare_scalar_project(
            vec![(root, root_bindings), (seeds, seed_bindings),
                 (declarations(shape_source.clone(), shapes), Vec::new())],
            vec![main.clone(), seed_source.clone(), shape_source.clone()],
            ScalarPreparationInputs { target_layout },
        ).unwrap()
    };
    for target_layout in [ScalarTargetLayout::WASM32, ScalarTargetLayout::NATIVE64] {
        let prepared = prepare(target_layout, root, seeds);
        let build = analyze_scalar_build(&prepared);
        assert!(build.diagnostics().is_empty(), "{:?}", build.diagnostics());
        let llvm = emit_scalar_build_project_llvm(&build).unwrap().to_text();
        let first = project_function_name(&seed_source, "first");
        let second = project_function_name(&seed_source, "second");
        for producer in [&first, &second] {
            let body = llvm.split_once(&format!("define internal void @{producer}("))
                .expect("ordinary compiler-owned producer ABI").1.split_once("\n}").unwrap().0;
            let (arguments, body) = body.split_once(')').unwrap();
            let destination = arguments.strip_prefix("ptr ").expect("fixed destination");
            assert!(body.lines().any(|line| line.contains("@llvm.memmove.")
                && line.contains(&format!("ptr align 1 {destination},"))
                && (line.contains("i32 16, i1 false") || line.contains("i64 16, i1 false"))), "{body}");
            assert!(body.contains("ret void"), "{body}");
        }
        let global = format!("@{}", project_global_name(&main, "stored"));
        let startup = llvm.lines().skip_while(|line| !line.starts_with("define ") || !line.contains("@main("))
            .skip(1).take_while(|line| *line != "}").collect::<Vec<_>>();
        let startup_call = startup.iter().position(|line| line.contains(&format!("call void @{first}(")))
            .expect("ordinary startup call");
        let startup_destination = startup[startup_call].split_once("(ptr ").unwrap().1.trim_end_matches(')');
        let destination_alloca = startup.iter().position(|line| line.contains(
            &format!("{startup_destination} = alloca [16 x i8], align 8")
        )).expect("caller startup result storage");
        let startup_store = startup.iter().position(|line| line.contains("@llvm.memmove.")
            && line.contains(&format!("ptr align 1 {global},"))
            && line.contains(startup_destination)).expect("post-return startup copy");
        assert!(destination_alloca < startup_call && startup_call < startup_store);
        let choose = project_function_name(&main, "choose");
        let choose = llvm.split_once(&format!("define i64 @{choose}("))
            .expect("original scalar consumer ABI").1.split_once("\n}").unwrap().0;
        assert_eq!(llvm.matches(&format!("call void @{first}(")).count(), 2);
        assert_eq!(llvm.matches(&format!("call void @{second}(")).count(), 1);
        let note = project_function_name(&main, "_note");
        assert_eq!(choose.matches(&format!("call void @{note}(")).count(), 1, "{choose}");
        let destination_of = |producer| choose.lines().find_map(|line| {
            line.trim().strip_prefix(&format!("call void @{producer}(ptr "))
                .and_then(|argument| argument.strip_suffix(')')).map(str::to_owned)
        }).expect("one caller destination in selected arm");
        let left = destination_of(&first);
        let right = destination_of(&second);
        assert_ne!(left, right);
        let final_store = choose.lines().find(|line| line.contains("@llvm.memmove.")
            && line.contains(&format!("ptr align 1 {global},"))).expect("canonical final assignment store");
        let stage = choose.lines().find(|line| line.contains("@llvm.memmove.")
            && line.contains("assignment_value")).expect("assignment snapshot staging");
        assert!(choose.find(&format!("call void @{first}(")).unwrap() < choose.find(stage).unwrap());
        assert!(choose.find(&format!("call void @{second}(")).unwrap() < choose.find(stage).unwrap());
        assert!(choose.find(stage).unwrap() < choose.find(final_store).unwrap());
        for root in [left, right] {
            assert!(choose.contains(&format!("{root} = alloca [16 x i8], align 8")), "{choose}");
        }

        let extra_use = root.replace(
            "u64 prior = stored.unit_item.total + _note(2).value;",
            "shapes.Bundle extra_value = seeds.second(); u64 prior = extra_value.unit_item.total;",
        );
        let no_candidate = seeds.replace("shapes.Bundle snapshot = { .unit_item = yellow; .guard = 7; };
    snapshot",
                "{ .unit_item = yellow; .guard = 7; }")
            .replace("shapes.Bundle snapshot = { .unit_item = blue; .guard = 19; };
    snapshot",
                "{ .unit_item = blue; .guard = 19; }");
        // The extra-use control removes the private projection. The invalid
        // namespace constructor control deliberately leaves it in place:
        // refusing the namespace cohort must not revoke an unrelated proof.
        for (root_variant, seeds_variant, keeps_private) in [
            (extra_use.as_str(), seeds, false),
            (root, no_candidate.as_str(), true),
        ] {
            let prepared = prepare(target_layout, root_variant, seeds_variant);
            let build = analyze_scalar_build(&prepared);
            assert!(build.diagnostics().is_empty(), "{:?}", build.diagnostics());
            let project = prepared.project();
            let snapshot_struct = |id: &crate::ScalarStructId| {
                let module = project.modules.iter().find(|module| module.source == id.source)?;
                let structure = module.structs.get(id.index)?;
                (&structure.id == id && module.layouts.aggregate(id).is_some()).then_some(structure)
            };
            let mut reservations = crate::scalar::AggregateReservations::default();
            reservations.snapshot_struct = Some(&snapshot_struct);
            for (module, input) in project.modules.iter().enumerate() {
                reservations.modules.push(module);
                reservations.bodies.extend(input.items.iter().enumerate().filter_map(|(item, declaration)|
                    matches!(declaration, ScalarItem::Function(_)).then_some(((module, item, None), true))));
            }
            let plans = build.aggregate(&reservations).unwrap();
            assert!(plans.initializer(&main).is_none_or(|plan| plan.ordinary_calls.is_empty()));
            for &(module, item, arm) in reservations.bodies.keys() {
                let ScalarItem::Function(function) = &project.modules[module].items[item] else {
                    unreachable!("reserved callable body")
                };
                let plan = plans.body(module, item, arm);
                let private_producer = keeps_private
                    && project.modules[module].source == main && function.name == "_note";
                let private_consumer = keeps_private
                    && project.modules[module].source == main && function.name == "choose";
                assert_eq!(plan.is_some_and(|plan| plan.ordinary_result), private_producer,
                    "unexpected caller-owned producer: {}", function.name);
                let calls = plan.map(|plan| &plan.ordinary_calls);
                if private_consumer {
                    let calls = calls.expect("independent private projection plan survives");
                    assert_eq!(calls.len(), 1);
                    let declaration = &project.modules[module].items.iter().find_map(|item| match item {
                        ScalarItem::Function(note) if note.name == "_note" => Some(note.name_span),
                        _ => None,
                    }).expect("private declaration");
                    assert!(calls.values().all(|target| target.source == main && target.range == *declaration));
                } else {
                    assert!(calls.is_none_or(|calls| calls.is_empty()),
                        "unexpected ordinary call: {}", function.name);
                }
            }
        }
    }
}

#[test]
fn private_projection_remains_at_exact_binding_assignment_and_branch_sites() {
    let text = "%%start
struct copy Pair { u8 tag; u64 value; }
u64 trace = 0;
u64(u64) record = fn(digit) { trace = trace * 10 + digit; digit };
Pair(u64) _make_pair = fn(seed) { { .tag = 7; .value = seed; } };
u64(bool) read_slots = fn(selected) {
    u64 before = record(1);
    u64 value = _make_pair(record(2)).value;
    value = value + (if (selected) { (_make_pair(record(3))).value } else { 11 });
    u64 after = record(4);
    before + value + after
};
%%end";
    for layout in [ScalarTargetLayout::WASM32, ScalarTargetLayout::NATIVE64] {
        let prepared = prepare_initializer_test(text, layout);
        let checked = analyze_scalar_build(&prepared);
        assert!(checked.diagnostics().is_empty(), "{:?}", checked.diagnostics());
        let source = &prepared.project().modules[0].source;
        for (project, partition) in [
            (false, emit_scalar_build_single_module_llvm(&checked).unwrap()),
            (true, emit_scalar_build_project_llvm(&checked).unwrap()),
        ] {
            let symbol = |name: &str| if project { project_function_name(source, name) } else { name.to_owned() };
            let producer = symbol("_make_pair");
            let record = symbol("record");
            let reader = symbol("read_slots");
            let llvm = partition.to_text();
            assert!(llvm.contains(&format!("define internal void @{producer}(")), "{llvm}");
            let reader = llvm.split_once(&format!("define i64 @{reader}(")).unwrap()
                .1.split_once("\n}").unwrap().0;
            let constructor_call = format!("call void @{producer}(");
            let trace_call = format!("call i64 @{record}(");
            assert_eq!(reader.matches(&constructor_call).count(), 2, "{reader}");
            assert_eq!(reader.matches(&trace_call).count(), 4, "{reader}");
            assert_eq!(reader.lines().filter(|line| line.trim_start().starts_with("%ordinary_result_destination")
                && line.contains("alloca [16 x i8], align 8")).count(), 2, "{reader}");
            assert!(!reader.contains("call ptr "), "{reader}");
        }
    }
}

#[test]
fn private_projection_refuses_same_producer_when_an_aggregate_assignment_is_unproved() {
    let text = "%%start
struct copy Pair { u64 value; }
Pair(u64) _make_pair = fn(seed) { { .value = seed; } };
u64(u64) read_value = fn(seed) { u64 word = _make_pair(seed).value; word };
u64(u64) replace_value = fn(seed) {
    Pair item = { .value = 13; };
    u64 original = item.value;
    item = _make_pair(seed);
    original + item.value
};
%%end";
    let prepared = prepare_initializer_test(text, ScalarTargetLayout::WASM32);
    let checked = analyze_scalar_build(&prepared);
    assert!(checked.diagnostics().is_empty(), "{:?}", checked.diagnostics());
    for result in [emit_scalar_build_single_module_llvm(&checked), emit_scalar_build_project_llvm(&checked)] {
        assert!(matches!(result, Err(message) if message.contains("no proven caller-owned storage")));
    }
}

#[test]
fn private_terminal_global_copy_snapshot_checks_each_arm_and_copies_before_return() {
    let text = "%%start
struct copy Pair { u8 tag; u64 value; }
u32 side = 0;
Pair[2] left = [{ .tag = 1; .value = 40; }, { .tag = 2; .value = 42; }];
Pair[2] right = [{ .tag = 3; .value = 70; }, { .tag = 4; .value = 71; }];
Pair(u64) _pick = fn(index) { if (side == 0) { left[index] } else { right[index] } };
u64(u64) read_direct = fn(index) { _pick(index).value };
%%end";
    for layout in [ScalarTargetLayout::WASM32, ScalarTargetLayout::NATIVE64] {
        let prepared = prepare_initializer_test(text, layout);
        let checked = analyze_scalar_build(&prepared);
        assert!(checked.diagnostics().is_empty(), "{:?}", checked.diagnostics());
        let source = &prepared.project().modules[0].source;
        for (partition, project) in [
            (emit_scalar_build_single_module_llvm(&checked).unwrap(), false),
            (emit_scalar_build_project_llvm(&checked).unwrap(), true),
        ] {
            let symbol = |name: &str| if project { project_function_name(source, name) } else { name.to_owned() };
            let producer = symbol("_pick");
            let reader = symbol("read_direct");
            let llvm = partition.to_text();
            let producer_body = llvm.split_once(&format!("define internal void @{producer}(")).unwrap()
                .1.split_once("\n}").unwrap().0;
            assert_eq!(producer_body.matches("icmp uge i64").count(), 2, "{producer_body}");
            assert_eq!(producer_body.matches("@__wosy_core_system_panic()").count(), 2, "{producer_body}");
            assert!(producer_body.contains("phi ptr "), "{producer_body}");
            let copy = producer_body.find("@llvm.memmove.").expect("copy before returning");
            assert!(copy < producer_body.find("ret void").unwrap(), "{producer_body}");
            assert!(!producer_body.contains("ret ptr"), "{producer_body}");
            let reader_body = llvm.split_once(&format!("define i64 @{reader}(")).unwrap()
                .1.split_once("\n}").unwrap().0;
            assert_eq!(reader_body.matches(&format!("call void @{producer}(")).count(), 1);
            assert!(reader_body.contains("ordinary_result_destination"));
            assert!(reader_body.contains("load i64, ptr %value"));
        }
    }
}

#[test]
fn private_terminal_global_copy_snapshot_refuses_calling_conditions_and_computed_indexes() {
    for (condition, index) in [("choose()", "index"), ("side == 0", "index + 1")] {
        let text = format!("%%start
struct copy Pair {{ u64 value; }}
u32 side = 0;
bool() choose = fn {{ side == 0 }};
Pair[2] left = [{{ .value = 40; }}, {{ .value = 42; }}];
Pair[2] right = [{{ .value = 70; }}, {{ .value = 71; }}];
Pair(u64) _pick = fn(index) {{ if ({condition}) {{ left[{index}] }} else {{ right[index] }} }};
u64(u64) read_direct = fn(index) {{ _pick(index).value }};
%%end");
        let prepared = prepare_initializer_test(&text, ScalarTargetLayout::WASM32);
        let checked = analyze_scalar_build(&prepared);
        assert!(checked.diagnostics().is_empty(), "{:?}", checked.diagnostics());
        for result in [emit_scalar_build_single_module_llvm(&checked), emit_scalar_build_project_llvm(&checked)] {
            assert!(matches!(result, Err(message) if message.contains("no proven caller-owned storage")));
        }
    }
}

#[test]
fn namespace_indexed_copy_snapshots_guard_before_copy_and_use_paired_local_binding() {
    let root = "%%start
shapes = namespace package \"shapes.w\";
seeds = namespace package \"seeds.w\";
u64(bool,u64) choose = fn(flag,selector) {
    shapes.Measure sample_value = if (flag) { seeds.first(selector) } else { seeds.second(selector) };
    sample_value.total + sample_value.guard
};
%%end";
    let shapes = "%%start
struct copy Measure { u64 total; u64 guard; }
%%end";
    let seeds = "%%start
shapes = namespace package \"shapes.w\";
shapes.Measure yellow = { .total = 13; .guard = 7; };
shapes.Measure blue = { .total = 29; .guard = 19; };
shapes.Measure[2] first_values = [yellow,blue];
shapes.Measure[2] second_values = [blue,yellow];
shapes.Measure(u64) first = fn(position) { first_values[position] };
shapes.Measure(u64) second = fn(position) { second_values[position] };
%%end";
    let source = |path: &str| SourceIdentity::new("project".into(), "package".into(), path.into(), "r1".into());
    let main = source("main.w");
    let seed_source = source("seeds.w");
    let shape_source = source("shapes.w");
    let declarations = |source, text: &str| {
        let parsed = parse_source(source, text.into(), &[]);
        assert!(parsed.diagnostics.is_empty(), "{:?}", parsed.diagnostics);
        derive_scalar_declarations_from_cst(parsed.result.canonical_cst())
    };
    let bindings = |input: &crate::ScalarSourceDeclarations, targets: &[&SourceIdentity]| {
        input.namespaces().zip(targets.iter().copied()).map(|(namespace, target)| ScalarNamespaceBinding {
            binding: namespace.binding.clone(), span: namespace.span, target: target.clone(),
        }).collect::<Vec<_>>()
    };
    let prepare = |target_layout, root_text: &str| {
        let root = declarations(main.clone(), root_text);
        let seeds = declarations(seed_source.clone(), seeds);
        let root_bindings = bindings(&root, &[&shape_source, &seed_source]);
        let seed_bindings = bindings(&seeds, &[&shape_source]);
        prepare_scalar_project(
            vec![(root, root_bindings), (seeds, seed_bindings),
                 (declarations(shape_source.clone(), shapes), Vec::new())],
            vec![main.clone(), seed_source.clone(), shape_source.clone()],
            ScalarPreparationInputs { target_layout },
        ).unwrap()
    };
    for target_layout in [ScalarTargetLayout::WASM32, ScalarTargetLayout::NATIVE64] {
        let prepared = prepare(target_layout, root);
        let build = analyze_scalar_build(&prepared);
        assert!(build.diagnostics().is_empty(), "{:?}", build.diagnostics());
        let llvm = emit_scalar_build_project_llvm(&build).unwrap().to_text();
        let first = project_function_name(&seed_source, "first");
        let second = project_function_name(&seed_source, "second");
        for producer in [&first, &second] {
            let function = llvm.split_once(&format!("define internal void @{producer}("))
                .expect("checked internal snapshot producer").1.split_once("\n}").unwrap().0;
            let (arguments, body) = function.split_once(')').unwrap();
            let destination = arguments.split_once(", ptr ").expect("fixed destination").1;
            let guard = body.lines().find(|line| line.contains("icmp uge i64") && line.ends_with(", 2"))
                .expect("one full-width bounded check");
            assert_eq!(body.matches("icmp uge i64").count(), 1);
            let continue_at = body.lines().find(|line| line.starts_with("snapshot_index_continue:"))
                .expect("checked continuation");
            let element = body.lines().find(|line| line.contains(" = getelementptr inbounds")
                && line.contains("array_element")).expect("checked fixed array address");
            let transfer = body.lines().find(|line| line.contains("@llvm.memmove.")
                && line.contains(&format!("ptr align 1 {destination},"))
                && (line.contains("i32 16, i1 false") || line.contains("i64 16, i1 false")))
                .expect("fixed transfer into caller storage");
            assert!(body.find(guard) < body.find(continue_at));
            assert!(body.find(continue_at) < body.find(element));
            assert!(body.find(element) < body.find(transfer));
            assert!(body.find(transfer) < body.find("ret void"));
            if target_layout == ScalarTargetLayout::WASM32 {
                let narrow = body.lines().find(|line| line.contains("trunc i64") && line.ends_with(" to i32"))
                    .expect("wasm32 address-width normalization");
                assert!(body.find(continue_at) < body.find(narrow) && body.find(narrow) < body.find(element));
            }
        }
        let choose = project_function_name(&main, "choose");
        let caller = llvm.split_once(&format!("define i64 @{choose}("))
            .expect("original scalar caller ABI").1.split_once("\n}").unwrap().0;
        let call_slot = |producer| caller.lines().find_map(|line| {
            line.trim().strip_prefix(&format!("call void @{producer}("))
                .and_then(|arguments| arguments.split_once(", ptr "))
                .and_then(|(_, argument)| argument.strip_suffix(')')).map(str::to_owned)
        }).expect("one caller destination per selected arm");
        let left = call_slot(&first);
        let right = call_slot(&second);
        assert_ne!(left, right);
        for slot in [&left, &right] {
            assert!(caller.contains(&format!("{slot} = alloca [16 x i8], align 8")), "{caller}");
        }
        let local_copy = caller.lines().find(|line| line.contains("@llvm.memmove.")
            && line.contains("ptr align 1 %sample_value,")).expect("post-branch canonical local snapshot");
        assert!(caller.find(&format!("call void @{first}(")) < caller.find(local_copy));
        assert!(caller.find(&format!("call void @{second}(")) < caller.find(local_copy));
        assert_eq!(caller.matches(&format!("call void @{first}(")).count(), 1);
        assert_eq!(caller.matches(&format!("call void @{second}(")).count(), 1);

        let extra = root.replace("    sample_value.total + sample_value.guard",
            "    shapes.Measure another_value = seeds.first(selector);\n    sample_value.total + another_value.guard");
        let prepared = prepare(target_layout, &extra);
        let build = analyze_scalar_build(&prepared);
        assert!(build.diagnostics().is_empty(), "{:?}", build.diagnostics());
        let project = prepared.project();
        let snapshot_struct = |id: &crate::ScalarStructId| {
            let module = project.modules.iter().find(|module| module.source == id.source)?;
            let structure = module.structs.get(id.index)?;
            (&structure.id == id && module.layouts.aggregate(id).is_some()).then_some(structure)
        };
        let mut reservations = crate::scalar::AggregateReservations::default();
        reservations.snapshot_struct = Some(&snapshot_struct);
        for (module, input) in project.modules.iter().enumerate() {
            reservations.modules.push(module);
            reservations.bodies.extend(input.items.iter().enumerate().filter_map(|(item, declaration)|
                matches!(declaration, ScalarItem::Function(_)).then_some(((module, item, None), true))));
        }
        let plans = build.aggregate(&reservations).unwrap();
        assert!(reservations.bodies.keys().all(|&(module, item, arm)|
            plans.body(module, item, arm).is_none_or(|plan| plan.ordinary_indexed.is_none()
                && !plan.ordinary_result && plan.ordinary_calls.is_empty())));
    }
}

#[test]
fn distinct_private_copy_producers_share_exact_caller_without_overwriting_sites() {
    let text = "%%start
struct copy Left { u64 value; }
struct copy Right { u64 value; }
Left(u64) _make_left = fn(seed) { { .value = seed; } };
Right(u64) _make_right = fn(seed) { { .value = seed; } };
u64(u64) both = fn(seed) { _make_left(seed).value + _make_right(seed).value };
%%end";
    for layout in [ScalarTargetLayout::WASM32, ScalarTargetLayout::NATIVE64] {
        let prepared = prepare_initializer_test(text, layout);
        let checked = analyze_scalar_build(&prepared);
        assert!(checked.diagnostics().is_empty(), "{:?}", checked.diagnostics());
        let source = &prepared.project().modules[0].source;
        for (project, partition) in [
            (false, emit_scalar_build_single_module_llvm(&checked).unwrap()),
            (true, emit_scalar_build_project_llvm(&checked).unwrap()),
        ] {
            let symbol = |name: &str| if project { project_function_name(source, name) } else { name.to_owned() };
            let llvm = partition.to_text();
            let body = llvm.split_once(&format!("define i64 @{}(", symbol("both"))).unwrap()
                .1.split_once("\n}").unwrap().0;
            let [left, right] = ["_make_left", "_make_right"].map(|producer| {
                let label = format!("call void @{}(", symbol(producer));
                let mut calls = body.lines().filter(|line| line.contains(&label));
                let call = calls.next().expect("exact prepared producer call");
                assert!(calls.next().is_none(), "{body}");
                call.split_once(", ptr ").unwrap().1.trim_end_matches(')').to_owned()
            });
            assert_ne!(left, right, "{body}");
            assert!(body.contains(&format!("{left} = alloca [8 x i8]")), "{body}");
            assert!(body.contains(&format!("{right} = alloca [8 x i8]")), "{body}");
            assert!(!body.contains("call ptr "), "{body}");
        }
    }
}

#[test]
fn local_array_len_observes_the_original_construction_count() {
    let text = "%%start
u64(u64) construction = fn(seed) { seed + 1 };
u64(u64) observed = fn(seed) {
    u8[construction(seed)] bytes;
    core.array_len(&bytes)
};
u64() fixed = fn { u8[2] bytes = [7, 9]; core.array_len(&bytes) };
u64(u64) observed_before_release = fn(seed) {
    u8[construction(seed)] bytes;
    u64 extent = core.array_len(&bytes);
    unsafe { core.free(&bytes); }
    extent
};
if (observed(4) != 5 || observed(8) != 9 || fixed() != 2 || observed_before_release(6) != 7) { core.system_panic(); };
%%end";
    for layout in [ScalarTargetLayout::NATIVE64, ScalarTargetLayout::WASM32] {
        let prepared = prepare_initializer_test(text, layout);
        let checked = analyze_scalar_build(&prepared);
        assert!(checked.diagnostics().is_empty(), "{:?}", checked.diagnostics());
        let source = &prepared.project().modules[0].source;
        for (project, llvm) in [
            (false, emit_scalar_build_single_module_llvm(&checked).unwrap().to_text()),
            (true, emit_scalar_build_project_llvm(&checked).unwrap().to_text()),
        ] {
            let body = self_tail_test_body(&llvm, "i64", &self_tail_test_symbol(source, project, "observed"));
            let symbol = self_tail_test_symbol(source, project, "construction");
            let call_match = format!("call i64 @{symbol}(");
            let [evaluation] = body.lines().filter(|line| line.contains(&call_match)).collect::<Vec<_>>()[..]
                else { panic!("one construction count evaluation: {body}") };
            let count = evaluation.trim().split_once(" = ").unwrap().0;
            assert!(body.contains(&format!("call ptr @__wosy_core_alloc(i64 {count}, i64 1)")), "{body}");
            assert!(body.contains(&format!("ret i64 {count}")), "{body}");
            assert_eq!(body.matches("call void @__wosy_core_free(").count(), 1, "{body}");
            let released = self_tail_test_body(&llvm, "i64",
                &self_tail_test_symbol(source, project, "observed_before_release"));
            assert_eq!(released.matches(&call_match).count(), 1, "{released}");
            // Same live-bit cleanup contract as the baseline checked-free path:
            // the lexical fallback exists in IR, but cannot execute on this edge.
            let observation = released.find("load i64, ptr %bytes_length").expect("live extent observation");
            let release = "call void @__wosy_core_free(ptr %checked_release_backing)";
            let fallback = "call void @__wosy_core_free(ptr %runtime_array_owner)";
            assert_eq!(released.matches(release).count(), 1, "{released}");
            assert_eq!(released.matches(fallback).count(), 1, "{released}");
            assert_eq!(released.matches("call void @__wosy_core_free(").count(), 2, "{released}");
            let release = released.find(release).unwrap();
            let cleared = released.find("store i1 false, ptr %bytes_live").unwrap();
            let read = released.find("%allocation_live = load i1, ptr %bytes_live").unwrap();
            let branch = released.find("br i1 %allocation_live, label %release.live, label %release.next").unwrap();
            let (live_block, after_cleanup) = released.split_once("\nrelease.live:").unwrap().1
                .split_once("\nrelease.next:").unwrap();
            assert!(live_block.contains(fallback) && !after_cleanup.contains("@__wosy_core_free"), "{released}");
            assert!(observation < release && release < cleared && cleared < read && read < branch, "{released}");
            let fixed = self_tail_test_body(&llvm, "i64", &self_tail_test_symbol(source, project, "fixed"));
            assert!(fixed.contains("ret i64 2"), "{fixed}");
            if let Some(directory) = std::env::var_os("WOSY_LOCAL_ARRAY_LEN_IR_DIR") {
                let target = if layout == ScalarTargetLayout::WASM32 { "wasm32" } else { "native64" };
                let route = if project { "project" } else { "standalone" };
                let output = std::path::PathBuf::from(directory);
                let controlled = llvm.replacen("define i32 @main(", "define i32 @wosy_authored_main(", 1);
                assert_ne!(controlled, llvm, "authored main entry");
                std::fs::write(output.join(format!("{target}-{route}.ll")), llvm).unwrap();
                std::fs::write(output.join(format!("{target}-{route}-control.ll")), controlled).unwrap();
            }
        }
    }
}

#[test]
fn array_len_does_not_invent_missing_extent_or_accept_nonarrays() {
    for (source, message) in [
        ("u64(u8[]) observed = fn(bytes) { core.array_len(&bytes) };",
            "core.array_len requires an exact local array extent"),
        ("u64(u8) observed = fn(value) { core.array_len(&value) };",
            "core.array_len requires the shared address of a named array place"),
        ("u64() observed = fn { core.array_len(null) };",
            "core.array_len requires the shared address of a named array place"),
        ("u64(u64) observed = fn(count) { u8[count] bytes; u8[] carried = bytes; carried; core.array_len(&bytes) };",
            "core.array_len requires an exact local array extent"),
    ] {
        let text = format!("%%start\n{source}\n%%end");
        for layout in [ScalarTargetLayout::NATIVE64, ScalarTargetLayout::WASM32] {
            let parsed = parse_source(SourceIdentity::new("project".into(), "package".into(), "src/main.w".into(), "r1".into()),
                text.clone(), &[]);
            assert!(parsed.diagnostics.is_empty(), "{:?}", parsed.diagnostics);
            let checked = crate::derive_scalar_program_with_layout(&parsed.result, layout);
            assert!(checked.diagnostics.iter().any(|d| d.message == message), "{:?}", checked.diagnostics);
        }
    }
}

#[test]
fn grounded_array_descriptor_uses_returned_extent_and_preserves_unclosed_callers() {
    let source_text = "%%start
u8[](u64) fabricate = fn(count) { u8[count] fresh; fresh };
unit(u64) replace = fn(count) { u8[2] bytes; bytes = fabricate(count); };
replace(7);
%%end";
    let ordinary_use = source_text.replace("replace(7);",
        "u8[] ordinary = fabricate(5);\nordinary;\nreplace(7);");
    for layout in [ScalarTargetLayout::NATIVE64, ScalarTargetLayout::WASM32] {
        for (closed, text) in [(true, source_text), (false, ordinary_use.as_str())] {
            let prepared = prepare_initializer_test(text, layout);
            let checked = analyze_scalar_build(&prepared);
            assert!(checked.diagnostics().is_empty(), "{:?}", checked.diagnostics());
            let source = &prepared.project().modules[0].source;
            for (project, llvm) in [
                (false, emit_scalar_build_single_module_llvm(&checked).unwrap().to_text()),
                (true, emit_scalar_build_project_llvm(&checked).unwrap().to_text()),
            ] {
                let producer = self_tail_test_symbol(source, project, "fabricate");
                let caller = self_tail_test_symbol(source, project, "replace");
                let caller = self_tail_test_body(&llvm, "void", &caller);
                if closed {
                    let body = llvm.split_once(&format!("define internal {{ ptr, i64 }} @{producer}("))
                        .expect("sealed producer ABI").1.split_once("\n}").unwrap().0;
                    assert!(body.contains("insertvalue { ptr, i64 } undef, ptr %allocation, 0"), "{body}");
                    assert!(body.contains("insertvalue { ptr, i64 } %array_return_backing, i64 %count, 1"), "{body}");
                    assert!(body.contains("ret { ptr, i64 } %array_return_extent"), "{body}");
                    assert_eq!(caller.matches(&format!("call {{ ptr, i64 }} @{producer}(")).count(), 1, "{caller}");
                    assert!(caller.contains("extractvalue { ptr, i64 } %array_descriptor, 0"), "{caller}");
                    assert!(caller.contains("extractvalue { ptr, i64 } %array_descriptor, 1"), "{caller}");
                    assert!(caller.contains("store ptr %array_descriptor_backing, ptr %bytes_backing"), "{caller}");
                    assert!(caller.contains("store i64 %array_descriptor_extent, ptr %bytes_length"), "{caller}");
                    assert_eq!(caller.matches("call void @__wosy_core_free(").count(), 2, "{caller}");
                    if let Some(out) = std::env::var_os("WOSY_ARRAY_DESCRIPTOR_IR_DIR") {
                        let target = if layout == ScalarTargetLayout::WASM32 { "wasm32" } else { "native64" };
                        let route = if project { "project" } else { "standalone" };
                        let controlled = llvm.replacen("define i32 @main(", "define i32 @wosy_authored_main(", 1);
                        assert_ne!(controlled, llvm);
                        std::fs::write(std::path::PathBuf::from(out).join(format!("{target}-{route}-control.ll")),
                            controlled).unwrap();
                    }
                } else {
                    assert!(llvm.contains(&format!("define ptr @{producer}(")), "{llvm}");
                    assert!(caller.contains(&format!("call ptr @{producer}(")), "{caller}");
                    assert!(!caller.contains("%array_descriptor_extent"), "{caller}");
                }
            }
        }
    }
}

#[test]
fn returned_array_extent_is_observed_only_after_closed_grounded_replacement() {
    let text = "%%start
u8[](u64) fabricate = fn(count) { u8[count] fresh; fresh };
u64(u64) replace = fn(count) {
    u8[2] bytes;
    bytes = fabricate(count);
    core.array_len(&bytes)
};
if (replace(7) != 7) { core.system_panic(); };
%%end";
    let exposed = text.replace("if (replace(7)", "u8[] ordinary = fabricate(5); ordinary;
if (replace(7)");
    for layout in [ScalarTargetLayout::NATIVE64, ScalarTargetLayout::WASM32] {
        for (closed, text) in [(true, text), (false, exposed.as_str())] {
            let prepared = prepare_initializer_test(text, layout);
            let checked = analyze_scalar_build(&prepared);
            assert!(checked.diagnostics().is_empty(), "{:?}", checked.diagnostics());
            let source = &prepared.project().modules[0].source;
            for (project, llvm) in [
                (false, emit_scalar_build_single_module_llvm(&checked)),
                (true, emit_scalar_build_project_llvm(&checked)),
            ] {
                if !closed {
                    assert!(matches!(llvm, Err(error) if error.contains("core.array_len requires a closed exact returned array extent")));
                    continue;
                }
                let llvm = llvm.unwrap().to_text();
                let caller = self_tail_test_body(&llvm, "i64", &self_tail_test_symbol(source, project, "replace"));
                let paired = caller.find("%array_descriptor_extent = extractvalue { ptr, i64 } %array_descriptor, 1")
                    .expect("exact returned extent");
                let install = caller.find("store i64 %array_descriptor_extent, ptr %bytes_length")
                    .expect("install returned extent");
                let observe = caller.find("%runtime_extent = load i64, ptr %bytes_length")
                    .expect("observe exact installed extent");
                let release_returned = caller.find("call void @__wosy_core_free(ptr %array_descriptor_backing)")
                    .expect("release returned original backing");
                assert!(paired < install && install < observe && observe < release_returned, "{caller}");
                assert!(caller.contains("ret i64 %runtime_extent"), "{caller}");
                if let Some(out) = std::env::var_os("WOSY_RETURNED_ARRAY_LEN_IR_DIR") {
                    let target = if layout == ScalarTargetLayout::WASM32 { "wasm32" } else { "native64" };
                    let route = if project { "project" } else { "standalone" };
                    let controlled = llvm.replacen("define i32 @main(", "define i32 @wosy_authored_main(", 1);
                    assert_ne!(controlled, llvm);
                    std::fs::write(std::path::PathBuf::from(out).join(format!("{target}-{route}-control.ll")), controlled).unwrap();
                }
            }
        }
    }
}

#[test]
fn returned_array_index_checks_installed_extent_before_addressing() {
    let text = "%%start
u8[](u64) fabricate = fn(count) { u8[count] fresh; fresh };
u8(u64) replace = fn(count) {
    u8[2] bytes;
    bytes = fabricate(count);
    bytes[3]
};
if (replace(7) != 37) { core.system_panic(); };
%%end";
    let mixed = text.replace("if (replace(7) != 37)", "u64(u64) measure = fn(count) { u8[2] bytes; bytes = fabricate(count); core.array_len(&bytes) };\nif (replace(7) != 37 || measure(7) != 7)");
    let other_use = text.replace("if (replace(7)", "u8[] ordinary = fabricate(5); ordinary;
if (replace(7)");
    for layout in [ScalarTargetLayout::NATIVE64, ScalarTargetLayout::WASM32] {
        let source = SourceIdentity::new("project".into(), "package".into(), "src/main.w".into(), "r1".into());
        for (access, closed, mixed_uses, text) in [(true, true, false, text),
            (true, true, true, mixed.as_str()), (true, false, false, other_use.as_str()), (false, true, false, text)]
        {
            let parsed = parse_source(source.clone(), text.into(), &[]);
            assert!(parsed.diagnostics.is_empty(), "{:?}", parsed.diagnostics);
            let prepared = crate::prepare_scalar_project_with_access(
                vec![(crate::derive_scalar_declarations_from_cst(parsed.result.canonical_cst()), vec![])],
                vec![source.clone()],
                ScalarPreparationInputs { target_layout: layout },
                access.then(wosy_stdlib_source::CheckedAccessComponent::linked),
            ).unwrap();
            let checked = analyze_scalar_build(&prepared);
            assert!(checked.diagnostics().is_empty(), "{:?}", checked.diagnostics());
            let result = emit_scalar_build_project_llvm(&checked);
            if !access {
                assert!(matches!(result, Err(error) if error.contains("authenticated checked-access helper")));
                continue;
            }
            if !closed {
                assert!(matches!(result, Err(error) if error.contains("closed returned extent")));
                continue;
            }
            let llvm = result.unwrap().to_text();
            let body = self_tail_test_body(&llvm, "i8", &project_function_name(&source, "replace"));
            let extent = body.find("%runtime_extent = load i64, ptr %bytes_length").expect("paired extent");
            let transported = body.find("store i64 %array_descriptor_extent, ptr %bytes_length").expect("transported extent");
            let guards = body.lines().filter(|line| line.contains("call void @wosy_fn__776f73795f746f6f6c636861696e5f7374646c6962__")
                && line.contains("__636865636b5f696e6465785f696e5f657874656e74(")).collect::<Vec<_>>();
            let [guard] = guards.as_slice() else { panic!("one canonical bounds helper call: {body}") };
            assert!(guard.contains("(i64 3, i64 %runtime_extent)"), "{body}");
            let guard = body.find(guard).unwrap();
            let indexed = body.find("getelementptr inbounds i8, ptr %bytes, i64 3").expect("index paired backing");
            let release = body.find("call void @__wosy_core_free(ptr %array_descriptor_backing)").expect("returned backing free");
            assert!(transported < extent && extent < guard && guard < indexed && indexed < release, "{body}");
            let producer = project_function_name(&source, "fabricate");
            let factory = llvm.split_once(&format!("define internal {{ ptr, i64 }} @{producer}(")).unwrap().1
                .split_once("
}").unwrap().0;
            assert!(!factory.contains("allocation_size_overflow"), "shared factory must keep its existing count domain: {factory}");
            if mixed_uses {
                let measure = self_tail_test_body(&llvm, "i64", &project_function_name(&source, "measure"));
                assert!(measure.contains(&format!("call {{ ptr, i64 }} @{producer}(")), "{measure}");
                assert!(measure.contains("ret i64 %runtime_extent"), "{measure}");
                assert!(!measure.contains("__636865636b5f696e6465785f696e5f657874656e74("), "{measure}");
            }
            if let Some(out) = std::env::var_os("WOSY_RETURNED_ARRAY_INDEX_IR_DIR").filter(|_| !mixed_uses) {
                let target = if layout == ScalarTargetLayout::WASM32 { "wasm32" } else { "native64" };
                let controlled = llvm.replacen("define i32 @main(", "define i32 @wosy_authored_main(", 1);
                assert_ne!(controlled, llvm);
                std::fs::write(std::path::PathBuf::from(out).join(format!("{target}-project-control.ll")), controlled).unwrap();
            }
        }
    }
}

#[test]
fn masked_returned_array_index_reuses_the_same_dynamic_i64_for_check_and_access() {
    let text = "%%start
u8[](u64) fabricate = fn(count) { u8[count] fresh; fresh };
u8(u64) replace = fn(count) {
    u8[2] bytes;
    bytes = fabricate(count);
    bytes[count & 3]
};
if (replace(7) != 37) { core.system_panic(); };
%%end";
    let mask_first = text.replace("count & 3", "3 & count");
    let computed = text.replace("count & 3", "(count * 2 + 5) & 3");
    let computed_mask_first = text.replace("count & 3", "3 & (count * 2 + 5)");
    for layout in [ScalarTargetLayout::NATIVE64, ScalarTargetLayout::WASM32] {
        for (form, text) in [("value-first", text), ("mask-first", mask_first.as_str()),
            ("computed-value-first", computed.as_str()), ("computed-mask-first", computed_mask_first.as_str())] {
            let source = SourceIdentity::new("project".into(), "package".into(), "src/main.w".into(), "r1".into());
            let parsed = parse_source(source.clone(), text.into(), &[]);
            assert!(parsed.diagnostics.is_empty(), "{:?}", parsed.diagnostics);
            let prepared = crate::prepare_scalar_project_with_access(
                vec![(crate::derive_scalar_declarations_from_cst(parsed.result.canonical_cst()), vec![])],
                vec![source.clone()],
                ScalarPreparationInputs { target_layout: layout },
                Some(wosy_stdlib_source::CheckedAccessComponent::linked()),
            ).unwrap();
            let checked = analyze_scalar_build(&prepared);
            assert!(checked.diagnostics().is_empty(), "{:?}", checked.diagnostics());
            let llvm = emit_scalar_build_project_llvm(&checked).unwrap().to_text();
            let body = self_tail_test_body(&llvm, "i8", &project_function_name(&source, "replace"));
            let extent = body.find("%runtime_extent = load i64, ptr %bytes_length").expect("paired extent");
            let selectors = body.lines().filter(|line| line.contains(" = and i64 ") && line.contains('3')).collect::<Vec<_>>();
            let [selector] = selectors.as_slice() else { panic!("one computed mask: {body}") };
            if form.starts_with("computed-") {
                let operations = body.lines().filter(|line|
                    line.contains(" = mul i64 ") || line.contains(" = add i64 ")).collect::<Vec<_>>();
                let [multiply, add] = operations.as_slice() else { panic!("one input computation: {body}") };
                assert!(multiply.contains(" = mul i64 ") && add.contains(" = add i64 "), "{body}");
                assert!(body.find(add).unwrap() < body.find(selector).unwrap(), "{body}");
            }
            let value = selector.trim().split_once(" = ").unwrap().0;
            let guards = body.lines().filter(|line| line.contains("call void @wosy_fn__776f73795f746f6f6c636861696e5f7374646c6962__")
                && line.contains("__636865636b5f696e6465785f696e5f657874656e74(")).collect::<Vec<_>>();
            let [guard] = guards.as_slice() else { panic!("one canonical bounds check: {body}") };
            assert!(guard.ends_with(&format!("(i64 {value}, i64 %runtime_extent)")), "{body}");
            let address = body.find(&format!("getelementptr inbounds i8, ptr %bytes, i64 {value}"))
                .expect("checked same dynamic i64");
            let release = body.find("call void @__wosy_core_free(ptr %array_descriptor_backing)").expect("returned backing free");
            assert!(extent < body.find(selector).unwrap() && body.find(selector) < body.find(guard)
                && body.find(guard).unwrap() < address && address < release, "{body}");
            if let Some(out) = std::env::var_os("WOSY_MASKED_RETURNED_ARRAY_INDEX_IR_DIR") {
                let target = if layout == ScalarTargetLayout::WASM32 { "wasm32" } else { "native64" };
                let controlled = llvm.replacen("define i32 @main(", "define i32 @wosy_authored_main(", 1);
                assert_ne!(controlled, llvm);
                std::fs::write(std::path::PathBuf::from(out).join(format!("{target}-{form}-control.ll")), controlled).unwrap();
            }
        }
    }
}

#[test]
fn owning_array_identity_rebind_preserves_the_full_extent() {
    let text = "%%start
u8[](u8[]) relay = fn(values) { values };
u64(u64) roundtrip = fn(count) {
    u8[count] bytes;
    bytes = relay(bytes);
    core.array_len(&bytes)
};
if (roundtrip(2) != 2) { core.system_panic(); };
if (roundtrip(7) != 7) { core.system_panic(); };
%%end";
    let exposed = text.replace("if (roundtrip(2)", "unit(u64) legacy = fn(count) { u8[count] other; other = relay(other); }; legacy(1);
if (roundtrip(2)");
    for layout in [ScalarTargetLayout::NATIVE64, ScalarTargetLayout::WASM32] {
        for (closed, text) in [(true, text), (false, exposed.as_str())] {
            let prepared = prepare_initializer_test(text, layout);
            let checked = analyze_scalar_build(&prepared);
            assert!(checked.diagnostics().is_empty(), "{:?}", checked.diagnostics());
            let source = &prepared.project().modules[0].source;
            for (project, llvm) in [
                (false, emit_scalar_build_single_module_llvm(&checked)),
                (true, emit_scalar_build_project_llvm(&checked)),
            ] {
                if !closed {
                    assert!(matches!(llvm, Err(error) if error.contains("core.array_len requires a closed exact returned array extent")));
                    continue;
                }
                let llvm = llvm.unwrap().to_text();
                let relay = self_tail_test_symbol(source, project, "relay");
                let caller = self_tail_test_body(&llvm, "i64", &self_tail_test_symbol(source, project, "roundtrip"));
                let definition = llvm.split_once(&format!("define internal {{ ptr, i64 }} @{relay}({{ ptr, i64 }} %values)"))
                    .unwrap_or_else(|| panic!("missing sealed paired formal and return ABI for {relay}: {llvm}")).1.split_once("\n}").unwrap().0;
                assert!(definition.contains("ret { ptr, i64 } %values"), "{definition}");
                assert!(!definition.contains("@__wosy_core_free"), "{definition}");
                let taking = caller.find(&format!("call {{ ptr, i64 }} @{relay}({{ ptr, i64 }} %array_argument)"))
                    .expect("one exact paired invocation");
                let storing = caller.find("store i64 %array_descriptor_extent, ptr %bytes_length")
                    .expect("install the returned extent");
                let observing = caller.find("%runtime_extent = load i64, ptr %bytes_length")
                    .expect("observe the installed extent");
                let releasing = caller.find("call void @__wosy_core_free(").expect("lexical cleanup");
                assert!(taking < storing && storing < observing && observing < releasing, "{caller}");
                assert_eq!(caller.matches("call void @__wosy_core_free(").count(), 1, "{caller}");
                if let Some(out) = std::env::var_os("WOSY_ARRAY_IDENTITY_IR_DIR") {
                    let target = if layout == ScalarTargetLayout::WASM32 { "wasm32" } else { "native64" };
                    let route = if project { "project" } else { "standalone" };
                    let controlled = llvm.replacen("define i32 @main(", "define i32 @wosy_authored_main(", 1);
                    assert_ne!(controlled, llvm);
                    std::fs::write(std::path::PathBuf::from(out).join(format!("{target}-{route}-control.ll")),
                        controlled).unwrap();
                }
            }
        }
    }
}

#[test]
fn callee_observes_the_exact_full_width_owning_array_formal() {
    let text = "%%start
u8[](u8[]) inspect = fn(values) {
    if (core.array_len(&values) < 2) { core.system_panic(); };
    values
};
u64(u64) roundtrip = fn(count) {
    u8[count] bytes;
    bytes = inspect(bytes);
    core.array_len(&bytes)
};
if (roundtrip(2) != 2) { core.system_panic(); };
if (roundtrip(7) != 7) { core.system_panic(); };
%%end";
    let exposed = text.replace("if (roundtrip(2)", "unit(u64) legacy = fn(count) { u8[count] other; other = inspect(other); }; legacy(1);
if (roundtrip(2)");
    for layout in [ScalarTargetLayout::NATIVE64, ScalarTargetLayout::WASM32] {
        for (closed, text) in [(true, text), (false, exposed.as_str())] {
            let prepared = prepare_initializer_test(text, layout);
            let checked = analyze_scalar_build(&prepared);
            assert!(checked.diagnostics().is_empty(), "{:?}", checked.diagnostics());
            let source = &prepared.project().modules[0].source;
            for (project, llvm) in [
                (false, emit_scalar_build_single_module_llvm(&checked)),
                (true, emit_scalar_build_project_llvm(&checked)),
            ] {
                if !closed {
                    assert!(matches!(llvm, Err(error) if error.contains("core.array_len requires a closed exact owning formal extent")));
                    continue;
                }
                let llvm = llvm.unwrap().to_text();
                let inspect = self_tail_test_symbol(source, project, "inspect");
                let callee = llvm.split_once(&format!("define internal {{ ptr, i64 }} @{inspect}({{ ptr, i64 }} %values)"))
                    .unwrap_or_else(|| panic!("missing sealed owning formal ABI for {inspect}: {llvm}"))
                    .1.split_once("\n}").unwrap().0;
                let extent = callee.find("%array_formal_extent = extractvalue { ptr, i64 } %values, 1")
                    .expect("unpack incoming full-width extent");
                let observe = callee.find("icmp ult i64 %array_formal_extent, 2")
                    .expect("authored test of that exact extent");
                let return_original = callee.find("ret { ptr, i64 } %values").expect("return the unchanged pair");
                assert!(extent < observe && observe < return_original, "{callee}");
                assert_eq!(callee.matches("extractvalue { ptr, i64 } %values, 1").count(), 1, "{callee}");
                assert!(!callee.contains("@__wosy_core_free"), "{callee}");
                let caller = self_tail_test_body(&llvm, "i64", &self_tail_test_symbol(source, project, "roundtrip"));
                assert_eq!(caller.matches(&format!("call {{ ptr, i64 }} @{inspect}({{ ptr, i64 }} %array_argument)")).count(), 1, "{caller}");
                assert_eq!(caller.matches("call void @__wosy_core_free(").count(), 1, "{caller}");
                if let Some(out) = std::env::var_os("WOSY_ARRAY_FORMAL_OBSERVE_IR_DIR") {
                    let target = if layout == ScalarTargetLayout::WASM32 { "wasm32" } else { "native64" };
                    let route = if project { "project" } else { "standalone" };
                    let controlled = llvm.replacen("define i32 @main(", "define i32 @wosy_authored_main(", 1);
                    assert_ne!(controlled, llvm);
                    std::fs::write(std::path::PathBuf::from(out).join(format!("{target}-{route}-control.ll")),
                        controlled).unwrap();
                }
            }
        }
    }
}

#[test]
fn checked_ordered_call_comparisons_use_the_exact_result_signedness() {
    let text = "%%start
u64(u64) unsigned_value = fn(value) { value };
i64(i64) signed_value = fn(value) { value };
bool(u64) unsigned_call_lt = fn(value) { unsigned_value(value) < 2 };
bool(i64) signed_call_lt = fn(value) { signed_value(value) < 2 };
bool(u64) unsigned_name_lt = fn(value) { value < 2 };
if (unsigned_call_lt(9223372036854775808)) { core.system_panic(); };
if (!signed_call_lt(-3)) { core.system_panic(); };
if (unsigned_name_lt(9223372036854775808)) { core.system_panic(); };
%%end";
    for layout in [ScalarTargetLayout::NATIVE64, ScalarTargetLayout::WASM32] {
        let prepared = prepare_initializer_test(text, layout);
        let checked = analyze_scalar_build(&prepared);
        assert!(checked.diagnostics().is_empty(), "{:?}", checked.diagnostics());
        let source = &prepared.project().modules[0].source;
        for (project, llvm) in [
            (false, emit_scalar_build_single_module_llvm(&checked).unwrap().to_text()),
            (true, emit_scalar_build_project_llvm(&checked).unwrap().to_text()),
        ] {
            for (name, predicate) in [("unsigned_call_lt", "ult"), ("signed_call_lt", "slt"), ("unsigned_name_lt", "ult")] {
                let body = self_tail_test_body(&llvm, "i1", &self_tail_test_symbol(source, project, name));
                assert_eq!(body.matches(&format!("icmp {predicate} i64")).count(), 1, "{body}");
            }
        }
    }
}

#[test]
fn closed_owning_array_forwarders_keep_the_same_full_width_pair_to_cleanup() {
    let text = "%%start
u8[](u8[]) inspect = fn(values) {
    if (core.array_len(&values) < 2) { core.system_panic(); };
    values
};
u8[](u8[]) bridge_one = fn(values) { inspect(values) };
u8[](u8[]) bridge_two = fn(values) { bridge_one(values) };
u64(u64) roundtrip = fn(count) {
    u8[count] bytes;
    bytes = bridge_two(bytes);
    core.array_len(&bytes)
};
if (roundtrip(2) != 2) { core.system_panic(); };
if (roundtrip(7) != 7) { core.system_panic(); };
%%end";
    let exposed = text.replace("if (roundtrip(2)", "unit(u64) legacy = fn(count) { u8[count] other; other = bridge_one(other); }; legacy(3);
if (roundtrip(2)");
    for layout in [ScalarTargetLayout::NATIVE64, ScalarTargetLayout::WASM32] {
        for (closed, text) in [(true, text), (false, exposed.as_str())] {
            let prepared = prepare_initializer_test(text, layout);
            let checked = analyze_scalar_build(&prepared);
            assert!(checked.diagnostics().is_empty(), "{:?}", checked.diagnostics());
            let source = &prepared.project().modules[0].source;
            for (project, llvm) in [
                (false, emit_scalar_build_single_module_llvm(&checked)),
                (true, emit_scalar_build_project_llvm(&checked)),
            ] {
                if !closed {
                    assert!(matches!(llvm, Err(error) if error.contains("core.array_len requires a closed exact owning formal extent")));
                    continue;
                }
                let llvm = llvm.unwrap().to_text();
                let symbol = |name| self_tail_test_symbol(source, project, name);
                for (wrapper, target) in [("bridge_two", "bridge_one"), ("bridge_one", "inspect")] {
                    let (wrapper, target) = (symbol(wrapper), symbol(target));
                    let wrapper_body = llvm.split_once(&format!("define internal {{ ptr, i64 }} @{wrapper}({{ ptr, i64 }} %values)"))
                        .unwrap_or_else(|| panic!("missing sealed wrapper ABI for {wrapper}: {llvm}"))
                        .1.split_once("\n}").unwrap().0;
                    assert_eq!(wrapper_body.matches(&format!("%array_forward = call {{ ptr, i64 }} @{target}({{ ptr, i64 }} %values)")).count(), 1, "{wrapper_body}");
                    assert!(wrapper_body.contains("ret { ptr, i64 } %array_forward"), "{wrapper_body}");
                    assert!(!wrapper_body.contains("extractvalue") && !wrapper_body.contains("@__wosy_core_free"), "{wrapper_body}");
                }
                let inspect = symbol("inspect");
                let leaf = llvm.split_once(&format!("define internal {{ ptr, i64 }} @{inspect}({{ ptr, i64 }} %values)"))
                    .expect("sealed observing leaf").1.split_once("\n}").unwrap().0;
                assert!(leaf.contains("icmp ult i64 %array_formal_extent, 2"), "{leaf}");
                let caller = self_tail_test_body(&llvm, "i64", &symbol("roundtrip"));
                assert_eq!(caller.matches("call void @__wosy_core_free(").count(), 1, "{caller}");
                if let Some(out) = std::env::var_os("WOSY_ARRAY_FORMAL_FORWARD_IR_DIR") {
                    let target = if layout == ScalarTargetLayout::WASM32 { "wasm32" } else { "native64" };
                    let route = if project { "project" } else { "standalone" };
                    let controlled = llvm.replacen("define i32 @main(", "define i32 @wosy_authored_main(", 1);
                    assert_ne!(controlled, llvm);
                    std::fs::write(std::path::PathBuf::from(out).join(format!("{target}-{route}-control.ll")),
                        controlled).unwrap();
                }
            }
        }
    }
}

#[test]
fn borrowed_fixed_array_extent_uses_the_canonical_checked_null_policy() {
    let source = SourceIdentity::new("project".into(), "package".into(), "src/borrowed_fixed_extent.w".into(), "r1".into());
    let text = "%%start
u64(*(u8[3])) extent = fn(view) { core.array_len(&(*view)) };
u64() observe = fn {
    u8[3] bytes = [1, 2, 3];
    *(u8[3]) view = &bytes;
    extent(view)
};
if (observe() != 3) { core.system_panic(); };
%%end";
    for layout in [ScalarTargetLayout::NATIVE64, ScalarTargetLayout::WASM32] {
        let parsed = parse_source(source.clone(), text.into(), &[]);
        assert!(parsed.diagnostics.is_empty(), "{:?}", parsed.diagnostics);
        let prepared = crate::prepare_scalar_project_with_access(
            vec![(crate::derive_scalar_declarations_from_cst(parsed.result.canonical_cst()), vec![])],
            vec![source.clone()],
            ScalarPreparationInputs { target_layout: layout },
            Some(wosy_stdlib_source::CheckedAccessComponent::linked()),
        ).unwrap();
        let checked = analyze_scalar_build(&prepared);
        assert!(checked.diagnostics().is_empty(), "{:?}", checked.diagnostics());
        let llvm = emit_scalar_build_project_llvm(&checked).unwrap().to_text();
        let body = self_tail_test_body(&llvm, "i64", &project_function_name(&source, "extent"));
        let [guard] = body.lines().filter(|line| line.contains("call void @wosy_fn__776f73795f746f6f6c636861696e5f7374646c6962__")
            && line.contains("__636865636b5f6e6f6e5f6e756c6c5f61646472657373(")).collect::<Vec<_>>()[..]
        else { panic!("exact authenticated non-null guard: {body}") };
        let address_width = if layout == ScalarTargetLayout::WASM32 { "i32" } else { "i64" };
        assert!(guard.contains(&format!("({address_width} %")), "{body}");
        assert!(body.find(guard).unwrap() < body.find("ret i64 3").expect("type-known full-width extent"), "{body}");
        assert!(!body.contains("deref_is_null") && !body.contains("inttoptr") && !body.contains("getelementptr"), "{body}");
        if let Some(out) = std::env::var_os("WOSY_FIXED_BORROWED_EXTENT_IR_DIR") {
            let target = if layout == ScalarTargetLayout::WASM32 { "wasm32" } else { "native64" };
            let controlled = llvm.replacen("define i32 @main(", "define i32 @wosy_authored_main(", 1);
            assert_ne!(controlled, llvm);
            std::fs::write(std::path::PathBuf::from(out).join(format!("{target}-project-control.ll")),
                controlled).unwrap();
        }
        for rejected in [
            "u64(*?u8) extent = fn(view) { unsafe { core.array_len(&(*view)) } };",
        ] {
            let text = format!("%%start\n{rejected}\n%%end");
            let parsed = parse_source(source.clone(), text, &[]);
            assert!(parsed.diagnostics.is_empty(), "{:?}", parsed.diagnostics);
            let checked = crate::derive_scalar_program_with_layout(&parsed.result, layout);
            assert!(checked.diagnostics.iter().any(|diagnostic|
                diagnostic.message == "core.array_len requires the shared address of a named array place"),
                "{:?}", checked.diagnostics);
        }
    }
}

#[test]
fn direct_shared_runtime_array_borrow_transports_one_exact_extent() {
    let source = SourceIdentity::new("project".into(), "package".into(), "src/borrowed_runtime_extent.w".into(), "r1".into());
    let text = "%%start
u64(*(u8[])) extent = fn(view) { core.array_len(&(*view)) };
u64(*(u8[])) relay_one = fn(view) { extent(view) };
u64(*(u8[])) relay_two = fn(view) { relay_one(view) };
u64(u64) observe = fn(count) {
    u8[count] bytes;
    extent(&bytes)
};
u64(u64) via_aliases = fn(count) {
    u8[count] bytes;
    *(u8[]) first = &bytes;
    *(u8[]) second = first;
    relay_two(second)
};
if (observe(2) != 2) { core.system_panic(); };
if (via_aliases(7) != 7) { core.system_panic(); };
%%end";
    let unpaired = text.replace("if (observe(2)", "u64(u64) not_exact = fn(count) {
    u8[count] bytes;
    *(u8[]) alias = &bytes;
    count;
    relay_one(alias)
};
if (not_exact(3) != 3) { core.system_panic(); };
if (observe(2)");
    for layout in [ScalarTargetLayout::NATIVE64, ScalarTargetLayout::WASM32] {
        for (access, closed, text) in [(true, true, text), (true, false, unpaired.as_str()), (false, false, text)] {
            let parsed = parse_source(source.clone(), text.into(), &[]);
            assert!(parsed.diagnostics.is_empty(), "{:?}", parsed.diagnostics);
            let prepared = crate::prepare_scalar_project_with_access(
                vec![(crate::derive_scalar_declarations_from_cst(parsed.result.canonical_cst()), vec![])],
                vec![source.clone()],
                ScalarPreparationInputs { target_layout: layout },
                access.then(wosy_stdlib_source::CheckedAccessComponent::linked),
            ).unwrap();
            let checked = analyze_scalar_build(&prepared);
            assert!(checked.diagnostics().is_empty(), "{:?}", checked.diagnostics());
            let emitted = emit_scalar_build_project_llvm(&checked);
            if !closed {
                assert!(matches!(emitted, Err(error) if error.contains("requires a closed checked borrowed runtime extent")));
                continue;
            }
            let llvm = emitted.unwrap().to_text();
            let target = project_function_name(&source, "extent");
            let word = if layout == ScalarTargetLayout::WASM32 { "i32" } else { "i64" };
            let paired = format!("{{ {word}, i64 }}");
            let body = llvm.split_once(&format!("define internal i64 @{target}({paired} %view)"))
                .unwrap_or_else(|| panic!("missing sealed borrowed extent signature {target}: {llvm}"))
                .1.split_once("\n}").unwrap().0;
            let [guard] = body.lines().filter(|line| line.contains("call void @wosy_fn__776f73795f746f6f6c636861696e5f7374646c6962__")
                && line.contains("__636865636b5f6e6f6e5f6e756c6c5f61646472657373(")).collect::<Vec<_>>()[..]
            else { panic!("one authenticated non-null guard: {body}") };
            assert!(guard.contains(&format!("({word} %")), "{body}");
            assert!(body.contains(&format!("extractvalue {paired} %view, 1")), "{body}");
            assert!(body.find(guard).unwrap() < body.find("ret i64 %borrowed_array_extent").expect("full incoming extent"), "{body}");
            assert!(!body.contains("deref_is_null") && !body.contains("inttoptr")
                && !body.contains("getelementptr") && !body.contains("@__wosy_core_free"), "{body}");
            for (relay, next) in [("relay_two", "relay_one"), ("relay_one", "extent")] {
                let relay = project_function_name(&source, relay);
                let next = project_function_name(&source, next);
                let relay = llvm.split_once(&format!("define internal i64 @{relay}({paired} %view)"))
                    .expect("same checked borrowed descriptor signature").1.split_once("\n}").unwrap().0;
                assert_eq!(relay.matches(&format!("%array_forward = call i64 @{next}({paired} %view)")).count(), 1, "{relay}");
                assert!(relay.contains("ret i64 %array_forward"), "{relay}");
                assert!(!relay.contains("extractvalue") && !relay.contains("insertvalue")
                    && !relay.contains("call void") && !relay.contains("getelementptr"), "{relay}");
            }
            for (named, next) in [("observe", "extent"), ("via_aliases", "relay_two")] {
                let next = project_function_name(&source, next);
                let call = format!("call i64 @{next}({paired} %borrowed_array_argument)");
                let caller = self_tail_test_body(&llvm, "i64", &project_function_name(&source, named));
                assert_eq!(caller.matches(&call).count(), 1, "{caller}");
                assert_eq!(caller.matches("call void @__wosy_core_free(").count(), 1, "{caller}");
                let allocation = caller.lines().find(|line| line.contains("call ptr @__wosy_core_alloc("))
                    .unwrap_or_else(|| panic!("original allocation: {caller}"));
                let count = allocation.split_once("(i64 ").unwrap().1.split_once(',').unwrap().0;
                assert!(caller.contains(&format!("insertvalue {paired} %borrowed_array_argument_address, i64 {count}, 1")), "{caller}");
                assert!(caller.find(&call).unwrap() < caller.find("call void @__wosy_core_free(").unwrap(), "{caller}");
            }
            if let Some(out) = std::env::var_os("WOSY_RUNTIME_BORROWED_EXTENT_IR_DIR") {
                let target = if layout == ScalarTargetLayout::WASM32 { "wasm32" } else { "native64" };
                let controlled = llvm.replacen("define i32 @main(", "define i32 @wosy_authored_main(", 1);
                assert_ne!(controlled, llvm);
                std::fs::write(std::path::PathBuf::from(out).join(format!("{target}-project-control.ll")),
                    controlled).unwrap();
            }
        }
    }
}

#[test]
fn widened_local_array_literal_keeps_one_runtime_backing_and_extent() {
    let source = SourceIdentity::new("project".into(), "package".into(), "src/widened_array.w".into(), "r1".into());
    let text = "%%start
u8(u8) seed = fn(value) { value };
u64(u8, u64) check = fn(value, index) {
    u8[] bytes = [3, seed(value), 7];
    if (bytes[index] != value) { core.system_panic(); };
    core.array_len(&bytes)
};
if (check(5, 1) != 3) { core.system_panic(); };
%%end";
    let moved = text.replace("u8[] bytes = [3, seed(value), 7];",
        "u8[] original = [3, seed(value), 7]; u8[] handoff = original; u8[] bytes = handoff;");
    let rebound = "%%start
u64(bool) refused = fn(take) {
    u8[] bytes = [1, 2];
    u8[] other = [3, 4];
    if (take) { bytes = other; };
    core.array_len(&bytes)
};
%%end";
    for layout in [ScalarTargetLayout::NATIVE64, ScalarTargetLayout::WASM32] {
        for (moved, text) in [(false, text), (true, moved.as_str())] {
            let parsed = parse_source(source.clone(), text.into(), &[]);
            assert!(parsed.diagnostics.is_empty(), "{:?}", parsed.diagnostics);
            let prepared = crate::prepare_scalar_project_with_access(
                vec![(crate::derive_scalar_declarations_from_cst(parsed.result.canonical_cst()), vec![])],
                vec![source.clone()],
                ScalarPreparationInputs { target_layout: layout },
                Some(wosy_stdlib_source::CheckedAccessComponent::linked()),
            ).unwrap_or_else(|error| panic!("{:?}", error.diagnostics));
            let checked = analyze_scalar_build(&prepared);
            assert!(checked.diagnostics().is_empty(), "{:?}", checked.diagnostics());
            let llvm = emit_scalar_build_project_llvm(&checked).unwrap().to_text();
            let body = self_tail_test_body(&llvm, "i64", &project_function_name(&source, "check"));
            let seed = format!("call i8 @{}(", project_function_name(&source, "seed"));
            assert_eq!(body.matches(&seed).count(), 1, "{body}");
            assert_eq!(body.matches("call ptr @__wosy_core_alloc(i64 3, i64 1)").count(), 1, "{body}");
            assert_eq!(body.lines().filter(|line| line.contains(" = getelementptr") && line.contains("%widened_array_cell")).count(), 3, "{body}");
            let [guard] = body.lines().filter(|line| line.contains("call void @wosy_fn__776f73795f746f6f6c636861696e5f7374646c6962__")
                && line.contains("__636865636b5f696e6465785f696e5f657874656e74(")).collect::<Vec<_>>()[..]
            else { panic!("one canonical checked bounds call: {body}") };
            assert!(guard.contains(", i64 3)"), "{body}");
            let projected = body.lines().find(|line| line.contains("%array_element") && line.contains(" = getelementptr"))
                .expect("checked element address");
            assert!(body.find(guard).unwrap() < body.find(projected).unwrap(), "{body}");
            assert_eq!(body.matches("call void @__wosy_core_free(ptr %allocation)").count(), 1, "{body}");
            assert!(body.contains("ret i64 3"), "{body}");
            if moved {
                assert!(!body.contains("alloca ptr"), "move must not duplicate runtime backing storage: {body}");
            }
            if let Some(out) = std::env::var_os("WOSY_WIDENED_LOCAL_ARRAY_IR_DIR").filter(|_| moved) {
                let target = if layout == ScalarTargetLayout::WASM32 { "wasm32" } else { "native64" };
                let controlled = llvm.replacen("define i32 @main(", "define i32 @wosy_authored_main(", 1);
                assert_ne!(controlled, llvm);
                std::fs::write(std::path::PathBuf::from(out).join(format!("{target}-project-control.ll")),
                    controlled).unwrap();
            }
        }
        let parsed = parse_source(source.clone(), rebound.into(), &[]);
        assert!(parsed.diagnostics.is_empty(), "{:?}", parsed.diagnostics);
        let checked = crate::derive_scalar_program_with_layout(&parsed.result, layout);
        assert!(checked.diagnostics.iter().any(|diagnostic|
            diagnostic.message == "widened array backing must remain in its original local binding"), "{:?}", checked.diagnostics);
    }
}

#[test]
fn widened_local_array_rebind_retires_the_predecessor_and_adopts_the_exact_extent() {
    let source = SourceIdentity::new("project".into(), "package".into(), "src/widened_array_rebind.w".into(), "r1".into());
    let text = "%%start
u64(u64) observe = fn(index) {
    u8[] target = [11, 13];
    if (core.array_len(&target) != 2) { core.system_panic(); };
    u8[] source = [3, 5, 7];
    target = source;
    if (target[index] != 5) { core.system_panic(); };
    core.array_len(&target)
};
if (observe(1) != 3) { core.system_panic(); };
%%end";
    for layout in [ScalarTargetLayout::NATIVE64, ScalarTargetLayout::WASM32] {
        let parsed = parse_source(source.clone(), text.into(), &[]);
        assert!(parsed.diagnostics.is_empty(), "{:?}", parsed.diagnostics);
        let prepared = crate::prepare_scalar_project_with_access(
            vec![(crate::derive_scalar_declarations_from_cst(parsed.result.canonical_cst()), vec![])],
            vec![source.clone()],
            ScalarPreparationInputs { target_layout: layout },
            Some(wosy_stdlib_source::CheckedAccessComponent::linked()),
        ).unwrap_or_else(|error| panic!("{:?}", error.diagnostics));
        let checked = analyze_scalar_build(&prepared);
        assert!(checked.diagnostics().is_empty(), "{:?}", checked.diagnostics());
        let llvm = emit_scalar_build_project_llvm(&checked).unwrap().to_text();
        let body = self_tail_test_body(&llvm, "i64", &project_function_name(&source, "observe"));
        let allocation = |bytes| {
            let label = format!("call ptr @__wosy_core_alloc(i64 {bytes}, i64 1)");
            let line = body.lines().find(|line| line.contains(&label)).unwrap_or_else(|| panic!("{body}"));
            (body.find(line).unwrap(), line.trim().split_once(" = ").unwrap().0)
        };
        let (previous, previous_ptr) = allocation(2);
        let (incoming, incoming_ptr) = allocation(3);
        assert_ne!(previous_ptr, incoming_ptr);
        assert_eq!(body.matches("call ptr @__wosy_core_alloc(").count(), 2, "{body}");
        let release = |pointer| body.find(&format!("call void @__wosy_core_free(ptr {pointer})"))
            .unwrap_or_else(|| panic!("{body}"));
        let [guard] = body.lines().filter(|line| line.contains("call void @wosy_fn__776f73795f746f6f6c636861696e5f7374646c6962__")
            && line.contains("__636865636b5f696e6465785f696e5f657874656e74(")).collect::<Vec<_>>()[..]
        else { panic!("one canonical checked bounds call: {body}") };
        let gep = body.lines().find(|line| line.contains("%array_element") && line.contains(" = getelementptr"))
            .unwrap_or_else(|| panic!("one checked indexed access: {body}"));
        assert!(guard.contains(", i64 3)") && gep.contains(&format!("ptr {incoming_ptr},")), "{body}");
        assert!(previous < incoming && incoming < release(previous_ptr)
            && release(previous_ptr) < body.find(guard).unwrap() && body.find(guard).unwrap() < body.find(gep).unwrap()
            && body.find(gep).unwrap() < release(incoming_ptr), "{body}");
        assert_eq!(body.matches("call void @__wosy_core_free(").count(), 2, "{body}");
        assert!(!body.contains("alloca ptr") && body.contains("ret i64 3"), "{body}");
        if let Some(out) = std::env::var_os("WOSY_WIDENED_REBIND_IR_DIR") {
            let target = if layout == ScalarTargetLayout::WASM32 { "wasm32" } else { "native64" };
            let controlled = llvm.replacen("define i32 @main(", "define i32 @wosy_authored_main(", 1);
            assert_ne!(controlled, llvm);
            std::fs::write(std::path::PathBuf::from(out).join(format!("{target}-project-control.ll")), controlled).unwrap();
        }
    }
}

#[test]
fn widened_array_join_transfers_one_backing_and_its_extent() {
    let source = SourceIdentity::new("project".into(), "package".into(), "src/widened_join.w".into(), "r1".into());
    let text = "%%start
u64(bool, u64, u8) observe = fn(pick, index, expected) {
    u8[] selected = if (pick) {
        u8[] left = [11, 13];
        left
    } else {
        u8[] right = [3, 5, 7];
        right
    };
    if (selected[index] != expected) { core.system_panic(); };
    core.array_len(&selected)
};
if (observe(true, 1, 13) != 2) { core.system_panic(); };
if (observe(false, 1, 5) != 3) { core.system_panic(); };
%%end";
    for layout in [ScalarTargetLayout::NATIVE64, ScalarTargetLayout::WASM32] {
        let parsed = parse_source(source.clone(), text.into(), &[]);
        assert!(parsed.diagnostics.is_empty(), "{:?}", parsed.diagnostics);
        let prepared = crate::prepare_scalar_project_with_access(
            vec![(crate::derive_scalar_declarations_from_cst(parsed.result.canonical_cst()), vec![])],
            vec![source.clone()], ScalarPreparationInputs { target_layout: layout },
            Some(wosy_stdlib_source::CheckedAccessComponent::linked()),
        ).unwrap_or_else(|error| panic!("{:?}", error.diagnostics));
        let checked = analyze_scalar_build(&prepared);
        assert!(checked.diagnostics().is_empty(), "{:?}", checked.diagnostics());
        let llvm = emit_scalar_build_project_llvm(&checked).unwrap().to_text();
        let body = self_tail_test_body(&llvm, "i64", &project_function_name(&source, "observe"));
        let [pair] = body.lines().filter(|line| line.contains("phi { ptr, i64 }")).collect::<Vec<_>>()[..]
            else { panic!("expected one joined descriptor: {body}") };
        let pair_value = pair.trim().split_once(" = ").unwrap().0;
        assert!(body.contains(&format!("%widened_join_backing = extractvalue {{ ptr, i64 }} {pair_value}, 0")), "{body}");
        assert!(body.contains(&format!("%widened_join_extent = extractvalue {{ ptr, i64 }} {pair_value}, 1")), "{body}");
        let [guard] = body.lines().filter(|line| line.contains("call void @wosy_fn__776f73795f746f6f6c636861696e5f7374646c6962__")
            && line.contains("__636865636b5f696e6465785f696e5f657874656e74(")).collect::<Vec<_>>()[..]
            else { panic!("expected one canonical bounds check: {body}") };
        assert!(guard.contains(", i64 %widened_join_extent)"), "{body}");
        assert!(body.lines().any(|line| line.contains("%array_element") && line.contains("getelementptr")
            && line.contains("ptr %widened_join_backing,")), "{body}");
        assert_eq!(body.matches("call ptr @__wosy_core_alloc(").count(), 2, "{body}");
        assert_eq!(body.matches("call void @__wosy_core_free(").count(), 1, "{body}");
        assert!(body.contains("call void @__wosy_core_free(ptr %widened_join_backing)")
            && body.contains("ret i64 %widened_join_extent") && !body.contains("alloca ptr"), "{body}");
        if let Some(out) = std::env::var_os("WOSY_WIDENED_JOIN_IR_DIR") {
            let target = if layout == ScalarTargetLayout::WASM32 { "wasm32" } else { "native64" };
            let controlled = llvm.replacen("define i32 @main(", "define i32 @wosy_authored_main(", 1);
            assert_ne!(controlled, llvm);
            std::fs::write(std::path::PathBuf::from(out).join(format!("{target}-project-control.ll")), controlled).unwrap();
        }
    }
}

#[test]
fn widened_array_return_transfers_original_initialized_backing_to_closed_receiver() {
    let text = "%%start
u8[]() fabricate = fn() { u8[] fresh = [3, 5, 7]; fresh };
u64() replace = fn() {
    u8[2] bytes;
    bytes = fabricate();
    core.array_len(&bytes)
};
if (replace() != 3) { core.system_panic(); };
%%end";
    let mixed = text.replace("if (replace()", "u8[] ordinary = fabricate(); ordinary;\nif (replace()");
    for layout in [ScalarTargetLayout::NATIVE64, ScalarTargetLayout::WASM32] {
        for (closed, text) in [(true, text), (false, mixed.as_str())] {
            let prepared = prepare_initializer_test(text, layout);
            let checked = analyze_scalar_build(&prepared);
            assert!(checked.diagnostics().is_empty(), "{:?}", checked.diagnostics());
            let source = &prepared.project().modules[0].source;
            for (project, emitted) in [
                (false, emit_scalar_build_single_module_llvm(&checked)),
                (true, emit_scalar_build_project_llvm(&checked)),
            ] {
                if !closed {
                    assert!(matches!(emitted, Err(error) if error.contains("widened array return requires a closed exact owning receiver")));
                    continue;
                }
                let llvm = emitted.unwrap().to_text();
                let producer = self_tail_test_symbol(source, project, "fabricate");
                let body = llvm.split_once(&format!("define internal {{ ptr, i64 }} @{producer}()"))
                    .unwrap_or_else(|| panic!("missing sealed array factory: {llvm}")).1.split_once("\n}").unwrap().0;
                assert_eq!(body.matches("call ptr @__wosy_core_alloc(").count(), 1, "{body}");
                assert!(!body.contains("call void @__wosy_core_free("), "{body}");
                assert!(body.contains("insertvalue { ptr, i64 } undef, ptr %allocation, 0")
                    && body.contains("insertvalue { ptr, i64 } %array_return_backing, i64 3, 1")
                    && body.contains("ret { ptr, i64 } %array_return_extent"), "{body}");
                let caller = self_tail_test_body(&llvm, "i64", &self_tail_test_symbol(source, project, "replace"));
                assert_eq!(caller.matches(&format!("call {{ ptr, i64 }} @{producer}()")).count(), 1, "{caller}");
                assert_eq!(caller.matches("call void @__wosy_core_free(").count(), 2, "{caller}");
                assert!(caller.contains("store ptr %array_descriptor_backing, ptr %bytes_backing")
                    && caller.contains("store i64 %array_descriptor_extent, ptr %bytes_length")
                    && caller.contains("call void @__wosy_core_free(ptr %array_descriptor_backing)")
                    && caller.contains("ret i64 %runtime_extent"), "{caller}");
                if let Some(out) = std::env::var_os("WOSY_WIDENED_RETURN_IR_DIR") {
                    let target = if layout == ScalarTargetLayout::WASM32 { "wasm32" } else { "native64" };
                    let route = if project { "project" } else { "standalone" };
                    let controlled = llvm.replacen("define i32 @main(", "define i32 @wosy_authored_main(", 1);
                    assert_ne!(controlled, llvm);
                    std::fs::write(std::path::PathBuf::from(out).join(format!("{target}-{route}-control.ll")), controlled).unwrap();
                }
            }
        }
    }
}

#[test]
fn widened_array_borrow_shares_exact_backing_and_existing_private_extent_call() {
    use inkwell::{memory_buffer::MemoryBuffer, values::{CallSiteValue, InstructionOpcode}};

    let source = SourceIdentity::new("project".into(), "package".into(), "src/widened_array_borrow.w".into(), "r1".into());
    let text = "%%start
u64(*(u8[])) extent = fn(view) { core.array_len(&(*view)) };
u64(*(u8[])) relay = fn(view) { extent(view) };
u64() two = fn() { u8[] bytes = [11, 13]; extent(&bytes) };
u64() three = fn() { u8[] bytes = [3, 5, 7]; relay(&bytes) };
if (two() != 2) { core.system_panic(); };
if (three() != 3) { core.system_panic(); };
u64(bool) joined = fn(pick) {
    u8[] bytes = if (pick) { u8[] left = [11, 13]; left }
        else { u8[] right = [3, 5, 7]; right };
    relay(&bytes)
};
u64() rebound = fn() {
    u8[] bytes = [11, 13];
    u64 before = relay(&bytes);
    u8[] incoming = [3, 5, 7];
    bytes = incoming;
    u64 after = relay(&bytes);
    before * 10 + after
};
if (joined(true) != 2) { core.system_panic(); };
if (joined(false) != 3) { core.system_panic(); };
if (rebound() != 23) { core.system_panic(); };
u64(bool) nested = fn(pick) {
    u8[] bytes = [3, 5, 7];
    u64 picked = if (pick) {
        u64 inner = if (pick) { relay(&bytes) } else { 0 };
        inner + 10
    } else {
        relay(&bytes) + 20
    };
    picked + extent(&bytes)
};
if (nested(true) != 16) { core.system_panic(); };
if (nested(false) != 26) { core.system_panic(); };
u64(bool) scoped = fn(pick) {
    u8[] outer = [3, 5, 7];
    u64 arm = if (pick) {
        u8[] bytes = [11, 13];
        relay(&bytes)
    } else {
        u8[] bytes = [3, 5, 7];
        relay(&bytes)
    };
    arm * 10 + extent(&outer)
};
if (scoped(true) != 23) { core.system_panic(); };
if (scoped(false) != 33) { core.system_panic(); };
%%end";
    for layout in [ScalarTargetLayout::NATIVE64, ScalarTargetLayout::WASM32] {
        for access in [true, false] {
            let parsed = parse_source(source.clone(), text.into(), &[]);
            assert!(parsed.diagnostics.is_empty(), "{:?}", parsed.diagnostics);
            let prepared = crate::prepare_scalar_project_with_access(
                vec![(crate::derive_scalar_declarations_from_cst(parsed.result.canonical_cst()), vec![])],
                vec![source.clone()], ScalarPreparationInputs { target_layout: layout },
                access.then(wosy_stdlib_source::CheckedAccessComponent::linked),
            ).unwrap_or_else(|error| panic!("{:?}", error.diagnostics));
            let checked = analyze_scalar_build(&prepared);
            assert!(checked.diagnostics().is_empty(), "{:?}", checked.diagnostics());
            let emitted = emit_scalar_build_project_llvm(&checked);
            if !access {
                assert!(matches!(emitted, Err(error) if error.contains("widened array borrow requires a closed exact extent reader")));
                continue;
            }
            let llvm = emitted.unwrap().to_text();
            let word = if layout == ScalarTargetLayout::WASM32 { "i32" } else { "i64" };
            let pair = format!("{{ {word}, i64 }}");
            let target = project_function_name(&source, "extent");
            let relay = project_function_name(&source, "relay");
            let observer = llvm.split_once(&format!("define internal i64 @{target}({pair} %view)")).unwrap().1.split_once("\n}").unwrap().0;
            assert_eq!(observer.lines().filter(|line| line.contains("call void @wosy_fn__776f73795f746f6f6c636861696e5f7374646c6962__")
                && line.contains("__636865636b5f6e6f6e5f6e756c6c5f61646472657373(")).count(), 1, "{observer}");
            assert!(observer.contains("ret i64 %borrowed_array_extent") && !observer.contains("@__wosy_core_free"), "{observer}");
            let wrapper = llvm.split_once(&format!("define internal i64 @{relay}({pair} %view)")).unwrap().1.split_once("\n}").unwrap().0;
            assert_eq!(wrapper.matches(&format!("call i64 @{target}({pair} %view)")).count(), 1, "{wrapper}");
            assert!(!wrapper.contains("extractvalue") && !wrapper.contains("insertvalue"), "{wrapper}");
            for (caller, target, count) in [("two", &target, 2), ("three", &relay, 3)] {
                let body = self_tail_test_body(&llvm, "i64", &project_function_name(&source, caller));
                assert_eq!(body.matches("call ptr @__wosy_core_alloc(").count(), 1, "{body}");
                assert_eq!(body.matches("call void @__wosy_core_free(").count(), 1, "{body}");
                let address = body.lines().find(|line| line.contains("ptrtoint ptr %allocation")).unwrap().trim().split_once(" = ").unwrap().0;
                assert!(body.contains(&format!("insertvalue {pair} undef, {word} {address}, 0")), "{body}");
                assert!(body.contains(&format!("insertvalue {pair} %borrowed_array_argument_address, i64 {count}, 1")), "{body}");
                let call = body.find(&format!("call i64 @{target}({pair} %borrowed_array_argument)")).unwrap();
                let free = body.find("call void @__wosy_core_free(ptr %allocation)").unwrap();
                assert!(call < free, "{body}");
            }
            let joined = self_tail_test_body(&llvm, "i64", &project_function_name(&source, "joined"));
            assert_eq!(joined.matches("phi { ptr, i64 }").count(), 1, "{joined}");
            assert!(joined.contains("ptrtoint ptr %widened_join_backing")
                && joined.contains(&format!("insertvalue {pair} %borrowed_array_argument_address, i64 %widened_join_extent, 1")), "{joined}");
            let call = format!("call i64 @{relay}({pair} %borrowed_array_argument)");
            assert_eq!(joined.matches("call void @__wosy_core_free(").count(), 1, "{joined}");
            assert!(joined.find(&call).unwrap() < joined.find("call void @__wosy_core_free(ptr %widened_join_backing)").unwrap(), "{joined}");

            let rebound = self_tail_test_body(&llvm, "i64", &project_function_name(&source, "rebound"));
            assert_eq!(rebound.matches("call void @__wosy_core_free(").count(), 2, "{rebound}");
            let context = Context::create();
            let buffer = MemoryBuffer::create_from_memory_range_copy(llvm.as_bytes(), "exact widened extent calls");
            let module = context.create_module_from_ir(buffer).expect("parse widened extent LLVM");
            module.verify().expect("verify widened extent LLVM");
            let function = module.get_function(&project_function_name(&source, "rebound")).unwrap();
            let relay = module.get_function(&relay).unwrap();
            let alloc = module.get_function("__wosy_core_alloc").unwrap();
            let free = module.get_function("__wosy_core_free").unwrap();
            let instructions = function.get_basic_blocks().into_iter().flat_map(|block|
                std::iter::successors(block.get_first_instruction(), |instruction| instruction.get_next_instruction()))
                .collect::<Vec<_>>();
            let calls_to = |callee| instructions.iter().enumerate().filter_map(|(position, &instruction)|
                CallSiteValue::try_from(instruction).ok().filter(|call| call.get_called_fn_value() == Some(callee))
                    .map(|_| (position, instruction))).collect::<Vec<_>>();
            let [(before, first_call), (after, second_call)] = calls_to(relay)[..]
                else { panic!("expected exactly two sealed calls: {rebound}") };
            let [(_, first_alloc), (_, second_alloc)] = calls_to(alloc)[..]
                else { panic!("expected exactly two original bases: {rebound}") };
            let [(free_old, first_free), (free_new, second_free)] = calls_to(free)[..]
                else { panic!("expected exactly two lexical frees: {rebound}") };
            for (call, backing, cleanup, count) in [(first_call, first_alloc, first_free, 2),
                (second_call, second_alloc, second_free, 3)]
            {
                let operand = |instruction, index| known_field_operand(instruction, index).unwrap();
                assert_eq!(operand(backing, 0).into_int_value().get_zero_extended_constant(), Some(count));
                let extent = operand(call, 0).as_instruction_value().unwrap();
                assert_eq!(extent.get_opcode(), InstructionOpcode::InsertValue);
                assert_eq!(extent.get_indices(), vec![1]);
                assert_eq!(operand(extent, 1).into_int_value().get_zero_extended_constant(), Some(count));
                let address_lane = operand(extent, 0).as_instruction_value().unwrap();
                assert_eq!(address_lane.get_opcode(), InstructionOpcode::InsertValue);
                assert_eq!(address_lane.get_indices(), vec![0]);
                let address = operand(address_lane, 1).as_instruction_value().unwrap();
                assert_eq!(address.get_opcode(), InstructionOpcode::PtrToInt);
                assert_eq!(operand(address, 0).as_instruction_value(), Some(backing));
                assert_eq!(operand(cleanup, 0).as_instruction_value(), Some(backing));
            }
            assert!(before < free_old && free_old < after && after < free_new, "{rebound}");
            let nested = self_tail_test_body(&llvm, "i64", &project_function_name(&source, "nested"));
            assert_eq!(nested.matches("call ptr @__wosy_core_alloc(").count(), 1, "{nested}");
            assert_eq!(nested.matches("call void @__wosy_core_free(").count(), 1, "{nested}");
            let function = module.get_function(&project_function_name(&source, "nested")).unwrap();
            let extent_function = module.get_function(&target).unwrap();
            let mut allocation = None;
            let mut cleanup = None;
            let mut readers = Vec::new();
            for block in function.get_basic_blocks() {
                for instruction in std::iter::successors(block.get_first_instruction(), |instruction| instruction.get_next_instruction()) {
                    let Some(callee) = CallSiteValue::try_from(instruction).ok().and_then(|call| call.get_called_fn_value())
                    else { continue };
                    if callee == alloc {
                        assert!(allocation.replace(instruction).is_none(), "{nested}");
                    } else if callee == free {
                        assert!(cleanup.replace((block, instruction)).is_none(), "{nested}");
                    } else if callee == relay || callee == extent_function {
                        readers.push((block, instruction, callee));
                    }
                }
            }
            let allocation = allocation.unwrap();
            let (exit, cleanup) = cleanup.unwrap();
            let [first, second, third] = readers.as_slice() else { panic!("{nested}") };
            assert_eq!([first, second, third].iter().filter(|item| item.2 == relay).count(), 2, "{nested}");
            assert_ne!(first.0, second.0, "{nested}");
            assert_ne!(first.0, third.0, "{nested}");
            assert_ne!(second.0, third.0, "{nested}");
            for (_, reader, callee) in readers {
                let extent = known_field_operand(reader, 0).unwrap().as_instruction_value().unwrap();
                assert_eq!(extent.get_indices(), vec![1]);
                assert_eq!(known_field_operand(extent, 1).unwrap().into_int_value().get_zero_extended_constant(), Some(3));
                let partial = known_field_operand(extent, 0).unwrap().as_instruction_value().unwrap();
                assert_eq!(partial.get_indices(), vec![0]);
                let address = known_field_operand(partial, 1).unwrap().as_instruction_value().unwrap();
                assert_eq!(address.get_opcode(), InstructionOpcode::PtrToInt);
                assert_eq!(known_field_operand(address, 0).unwrap().as_instruction_value(), Some(allocation));
                if callee == extent_function {
                    assert_eq!(reader.get_parent(), Some(exit));
                    assert!(std::iter::successors(reader.get_next_instruction(), |instruction| instruction.get_next_instruction())
                        .any(|instruction| instruction == cleanup), "{nested}");
                }
            }
            assert_eq!(known_field_operand(cleanup, 0).unwrap().as_instruction_value(), Some(allocation));
            let scoped = self_tail_test_body(&llvm, "i64", &project_function_name(&source, "scoped"));
            let function = module.get_function(&project_function_name(&source, "scoped")).unwrap();
            let mut scoped_bases = Vec::new();
            for block in function.get_basic_blocks() {
                let instructions = std::iter::successors(block.get_first_instruction(), |instruction| instruction.get_next_instruction())
                    .collect::<Vec<_>>();
                let call_to = |callee| instructions.iter().enumerate().find(|(_, instruction)|
                    CallSiteValue::try_from(**instruction).ok().is_some_and(|call| call.get_called_fn_value() == Some(callee)));
                let (expected_callee, reader) = match (call_to(relay), call_to(extent_function)) {
                    (Some(reader), None) => (relay, reader),
                    (None, Some(reader)) => (extent_function, reader),
                    (None, None) => continue,
                    _ => panic!("{scoped}"),
                };
                let (read, &reader) = reader;
                let (release, &release_instruction) = call_to(free).unwrap_or_else(|| panic!("{scoped}"));
                assert!(read < release, "{scoped}");
                let extent = known_field_operand(reader, 0).unwrap().as_instruction_value().unwrap();
                assert_eq!(extent.get_indices(), vec![1]);
                let count = known_field_operand(extent, 1).unwrap().into_int_value().get_zero_extended_constant().unwrap();
                let partial = known_field_operand(extent, 0).unwrap().as_instruction_value().unwrap();
                assert_eq!(partial.get_indices(), vec![0]);
                let address = known_field_operand(partial, 1).unwrap().as_instruction_value().unwrap();
                assert_eq!(address.get_opcode(), InstructionOpcode::PtrToInt);
                let original = known_field_operand(address, 0).unwrap().as_instruction_value().unwrap();
                assert_eq!(CallSiteValue::try_from(original).unwrap().get_called_fn_value(), Some(alloc));
                assert_eq!(known_field_operand(original, 0).unwrap().into_int_value().get_zero_extended_constant(), Some(count));
                assert_eq!(known_field_operand(release_instruction, 0).unwrap().as_instruction_value(), Some(original));
                assert!(!scoped_bases.iter().any(|(_, _, previous)| *previous == original), "{scoped}");
                scoped_bases.push((expected_callee == extent_function, count, original));
            }
            let mut types = scoped_bases.iter().map(|&(outer, count, _)| (outer, count)).collect::<Vec<_>>();
            types.sort_unstable();
            assert_eq!(types, [(false, 2), (false, 3), (true, 3)], "{scoped}");
            assert_eq!(scoped.matches("call void @__wosy_core_free(").count(), 3, "{scoped}");
            if let Some(out) = std::env::var_os("WOSY_BRANCH_OWNER_BORROW_IR_DIR") {
                let target = if layout == ScalarTargetLayout::WASM32 { "wasm32" } else { "native64" };
                let controlled = llvm.replacen("define i32 @main(", "define i32 @wosy_authored_main(", 1);
                assert_ne!(controlled, llvm);
                std::fs::write(std::path::PathBuf::from(out).join(format!("{target}-project-control.ll")), controlled).unwrap();
            }
        }
    }
}

#[test]
fn widened_owning_array_roundtrip_preserves_qualified_original_pair() {
    let text = "%%start
u8[](u8[]) identity = fn(values) { values };
u8[](u8[]) relay = fn(values) { identity(values) };
u64() two = fn() { u8[] bytes = [11, 13]; bytes = relay(bytes); core.array_len(&bytes) };
u64() three = fn() { u8[] bytes = [3, 5, 7]; bytes = relay(bytes); core.array_len(&bytes) };
if (two() != 2) { core.system_panic(); };
if (three() != 3) { core.system_panic(); };
%%end";
    let exposed = text.replace("if (two()", "unit(u64) ordinary = fn(count) { u8[count] array; array = relay(array); }; ordinary(1);\nif (two()");
    for layout in [ScalarTargetLayout::NATIVE64, ScalarTargetLayout::WASM32] {
        for (closed, text) in [(true, text), (false, exposed.as_str())] {
            let prepared = prepare_initializer_test(text, layout);
            let checked = analyze_scalar_build(&prepared);
            assert!(checked.diagnostics().is_empty(), "{:?}", checked.diagnostics());
            let source = &prepared.project().modules[0].source;
            for (project, emitted) in [(false, emit_scalar_build_single_module_llvm(&checked)),
                (true, emit_scalar_build_project_llvm(&checked))]
            {
                if !closed {
                    assert!(matches!(emitted, Err(error) if error.contains("widened array move requires a closed exact owning roundtrip")));
                    continue;
                }
                let llvm = emitted.unwrap().to_text();
                let relay = self_tail_test_symbol(source, project, "relay");
                let identity = self_tail_test_symbol(source, project, "identity");
                let callee = |symbol| llvm.split_once(&format!("define internal {{ ptr, i64 }} @{symbol}({{ ptr, i64 }} %values)"))
                    .unwrap_or_else(|| panic!("missing closed owning signature: {llvm}")).1.split_once("\n}").unwrap().0;
                assert!(callee(&identity).contains("ret { ptr, i64 } %values"));
                assert!(callee(&relay).contains(&format!("call {{ ptr, i64 }} @{identity}({{ ptr, i64 }} %values)")));
                for (name, length) in [("two", 2), ("three", 3)] {
                    let body = self_tail_test_body(&llvm, "i64", &self_tail_test_symbol(source, project, name));
                    assert_eq!(body.matches("call ptr @__wosy_core_alloc(").count(), 1, "{body}");
                    assert_eq!(body.matches("call void @__wosy_core_free(").count(), 1, "{body}");
                    assert!(body.contains("insertvalue { ptr, i64 } undef, ptr %allocation, 0")
                        && body.contains(&format!("insertvalue {{ ptr, i64 }} %array_argument_backing, i64 {length}, 1"))
                        && body.contains(&format!("call {{ ptr, i64 }} @{relay}({{ ptr, i64 }} %array_argument)"))
                        && body.contains("call void @__wosy_core_free(ptr %array_descriptor_backing)")
                        && body.contains("ret i64 %array_descriptor_extent") && !body.contains("alloca ptr"), "{body}");
                }
                if let Some(out) = std::env::var_os("WOSY_WIDENED_OWNING_ROUNDTRIP_IR_DIR") {
                    let target = if layout == ScalarTargetLayout::WASM32 { "wasm32" } else { "native64" };
                    let route = if project { "project" } else { "standalone" };
                    let controlled = llvm.replacen("define i32 @main(", "define i32 @wosy_authored_main(", 1);
                    assert_ne!(controlled, llvm);
                    std::fs::write(std::path::PathBuf::from(out).join(format!("{target}-{route}-control.ll")), controlled).unwrap();
                }
            }
        }
    }
}

#[test]
fn widened_owning_array_fresh_receivers_preserve_backing_and_scope() {
    use inkwell::{memory_buffer::MemoryBuffer, values::{CallSiteValue, InstructionOpcode}};
    let text = "%%start
u8[](u8[]) identity = fn(values) { values };
u8[](u8[]) relay = fn(values) { identity(values) };
u64() two = fn() {
    u8[] source = [11, 13];
    u8[] received = relay(source);
    u64 saved = core.array_len(&received);
    u8[] final_owner = relay(received);
    saved * 10 + core.array_len(&final_owner)
};
u64() three = fn() {
    u8[] source = [3, 5, 7];
    u8[] received = relay(source);
    core.array_len(&received)
};
if (two() != 22) { core.system_panic(); };
if (three() != 3) { core.system_panic(); };
%%end";
    let mixed = text.replace("if (two()", "unit(u64) ordinary = fn(count) { u8[count] array; array = relay(array); }; ordinary(1);\nif (two()");
    for layout in [ScalarTargetLayout::NATIVE64, ScalarTargetLayout::WASM32] {
        for (closed, text) in [(true, text), (false, mixed.as_str())] {
            let prepared = prepare_initializer_test(text, layout);
            let checked = analyze_scalar_build(&prepared);
            assert!(checked.diagnostics().is_empty(), "{:?}", checked.diagnostics());
            let source = &prepared.project().modules[0].source;
            for (project, emitted) in [(false, emit_scalar_build_single_module_llvm(&checked)),
                (true, emit_scalar_build_project_llvm(&checked))]
            {
                if !closed {
                    assert!(matches!(emitted, Err(error) if error.contains("widened array move requires a closed exact owning receiver")));
                    continue;
                }
                let llvm = emitted.unwrap().to_text();
                let context = Context::create();
                let buffer = MemoryBuffer::create_from_memory_range_copy(llvm.as_bytes(), "exact widened fresh owning receipts");
                let module = context.create_module_from_ir(buffer).unwrap();
                module.verify().unwrap();
                let relay = module.get_function(&self_tail_test_symbol(source, project, "relay")).unwrap();
                let alloc = module.get_function("__wosy_core_alloc").unwrap();
                let free = module.get_function("__wosy_core_free").unwrap();
                for (name, count, hops) in [("two", 2, 2), ("three", 3, 1)] {
                    let function = module.get_function(&self_tail_test_symbol(source, project, name)).unwrap();
                    let mut allocation = None;
                    let mut calls = Vec::new();
                    let mut cleanup = Vec::new();
                    for block in function.get_basic_blocks() {
                        for instruction in std::iter::successors(block.get_first_instruction(), |instruction| instruction.get_next_instruction()) {
                            let Some(callee) = CallSiteValue::try_from(instruction).ok().and_then(|call| call.get_called_fn_value())
                            else { continue };
                            if callee == alloc {
                                assert!(allocation.replace(instruction).is_none());
                            } else if callee == relay {
                                calls.push(instruction);
                            } else if callee == free {
                                cleanup.push(instruction);
                            }
                        }
                    }
                    let allocation = allocation.unwrap();
                    assert_eq!(calls.len(), hops);
                    let [cleanup] = cleanup.as_slice() else { panic!("one cleanup per returned owner: {llvm}") };
                    let operand = |instruction, index| known_field_operand(instruction, index).unwrap();
                    let mut previous = allocation;
                    for call in calls {
                        let length = operand(call, 0).as_instruction_value().unwrap();
                        assert_eq!(length.get_opcode(), InstructionOpcode::InsertValue);
                        assert_eq!(length.get_indices(), vec![1]);
                        let backing = operand(length, 0).as_instruction_value().unwrap();
                        assert_eq!(backing.get_opcode(), InstructionOpcode::InsertValue);
                        assert_eq!(backing.get_indices(), vec![0]);
                        let passed = operand(backing, 1).as_instruction_value().unwrap();
                        if previous == allocation {
                            assert_eq!(operand(length, 1).into_int_value().get_zero_extended_constant(), Some(count));
                            assert_eq!(passed, allocation);
                        } else {
                            assert_eq!(passed.get_indices(), vec![0]);
                            assert_eq!(operand(passed, 0).as_instruction_value(), Some(previous));
                            let length = operand(length, 1).as_instruction_value().unwrap();
                            assert_eq!(length.get_indices(), vec![1]);
                            assert_eq!(operand(length, 0).as_instruction_value(), Some(previous));
                        }
                        previous = call;
                    }
                    let released = operand(*cleanup, 0).as_instruction_value().unwrap();
                    assert_eq!(released.get_indices(), vec![0]);
                    assert_eq!(operand(released, 0).as_instruction_value(), Some(previous));
                }
                if let Some(out) = std::env::var_os("WOSY_WIDENED_OWNING_RECEIVE_IR_DIR") {
                    let target = if layout == ScalarTargetLayout::WASM32 { "wasm32" } else { "native64" };
                    let route = if project { "project" } else { "standalone" };
                    let controlled = llvm.replacen("define i32 @main(", "define i32 @wosy_authored_main(", 1);
                    assert_ne!(controlled, llvm);
                    std::fs::write(std::path::PathBuf::from(out).join(format!("{target}-{route}-control.ll")), controlled).unwrap();
                }
            }
        }
    }
}

#[test]
fn widened_owning_array_replaces_distinct_live_target_after_call() {
    use inkwell::{memory_buffer::MemoryBuffer, values::{CallSiteValue, InstructionOpcode}};
    let text = "%%start
u8[](u8[]) identity = fn(values) { values };
u8[](u8[]) relay = fn(values) { identity(values) };
u64() replace = fn() {
    u8[] target = [11, 13];
    u64 before = core.array_len(&target);
    u8[] source = [3, 5, 7];
    target = relay(source);
    before * 10 + core.array_len(&target)
};
if (replace() != 23) { core.system_panic(); };
%%end";
    for layout in [ScalarTargetLayout::NATIVE64, ScalarTargetLayout::WASM32] {
        let prepared = prepare_initializer_test(text, layout);
        let checked = analyze_scalar_build(&prepared);
        assert!(checked.diagnostics().is_empty(), "{:?}", checked.diagnostics());
        let source = &prepared.project().modules[0].source;
        for (project, emitted) in [(false, emit_scalar_build_single_module_llvm(&checked)),
            (true, emit_scalar_build_project_llvm(&checked))]
        {
            let llvm = emitted.unwrap().to_text();
            let context = Context::create();
            let buffer = MemoryBuffer::create_from_memory_range_copy(llvm.as_bytes(), "exact widened owning replacement");
            let module = context.create_module_from_ir(buffer).unwrap();
            module.verify().unwrap();
            let relay = module.get_function(&self_tail_test_symbol(source, project, "relay")).unwrap();
            let alloc = module.get_function("__wosy_core_alloc").unwrap();
            let free = module.get_function("__wosy_core_free").unwrap();
            let function = module.get_function(&self_tail_test_symbol(source, project, "replace")).unwrap();
            let instructions = function.get_basic_blocks().into_iter().flat_map(|block|
                std::iter::successors(block.get_first_instruction(), |instruction| instruction.get_next_instruction()))
                .collect::<Vec<_>>();
            let calls_to = |callee| instructions.iter().enumerate().filter_map(|(position, &instruction)|
                CallSiteValue::try_from(instruction).ok().filter(|call| call.get_called_fn_value() == Some(callee))
                    .map(|_| (position, instruction))).collect::<Vec<_>>();
            let [(previous_position, previous), (source_position, incoming)] = calls_to(alloc)[..]
                else { panic!("expected two exact original allocations: {llvm}") };
            let [(called, call)] = calls_to(relay)[..] else { panic!("one owning call: {llvm}") };
            let [(release_previous, predecessor_free), (release_result, result_free)] = calls_to(free)[..]
                else { panic!("one free per original backing: {llvm}") };
            let operand = |inst, index| known_field_operand(inst, index).unwrap();
            assert_eq!(operand(previous, 0).into_int_value().get_zero_extended_constant(), Some(2));
            assert_eq!(operand(incoming, 0).into_int_value().get_zero_extended_constant(), Some(3));
            let count_lane = operand(call, 0).as_instruction_value().unwrap();
            assert_eq!(count_lane.get_opcode(), InstructionOpcode::InsertValue);
            assert_eq!(count_lane.get_indices(), vec![1]);
            assert_eq!(operand(count_lane, 1).into_int_value().get_zero_extended_constant(), Some(3));
            let address_lane = operand(count_lane, 0).as_instruction_value().unwrap();
            assert_eq!(address_lane.get_indices(), vec![0]);
            assert_eq!(operand(address_lane, 1).as_instruction_value(), Some(incoming));
            assert_eq!(operand(predecessor_free, 0).as_instruction_value(), Some(previous));
            let returned = operand(result_free, 0).as_instruction_value().unwrap();
            assert_eq!(returned.get_opcode(), InstructionOpcode::ExtractValue);
            assert_eq!(returned.get_indices(), vec![0]);
            assert_eq!(operand(returned, 0).as_instruction_value(), Some(call));
            assert!(previous_position < source_position && source_position < called
                && called < release_previous && release_previous < release_result, "{llvm}");
            if let Some(out) = std::env::var_os("WOSY_WIDENED_OWNING_REPLACE_IR_DIR") {
                let target = if layout == ScalarTargetLayout::WASM32 { "wasm32" } else { "native64" };
                let route = if project { "project" } else { "standalone" };
                let controlled = llvm.replacen("define i32 @main(", "define i32 @wosy_authored_main(", 1);
                assert_ne!(controlled, llvm);
                std::fs::write(std::path::PathBuf::from(out).join(format!("{target}-{route}-control.ll")), controlled).unwrap();
            }
        }
    }
}

#[test]
fn named_fixed_copy_widens_into_independent_runtime_backing() {
    use inkwell::{memory_buffer::MemoryBuffer, values::{CallSiteValue, InstructionOpcode}};
    let source = SourceIdentity::new("project".into(), "package".into(), "src/fixed_copy_widen.w".into(), "r1".into());
    let text = "%%start
u64(u64) two = fn(index) {
    u8[2] fixed = [11, 13];
    u8[] bytes = fixed;
    fixed[1] = 19;
    if (fixed[1] != 19 || bytes[index] != 13) { core.system_panic(); };
    core.array_len(&bytes)
};
u64(u64) three = fn(index) {
    u8[3] fixed = [3, 5, 7];
    u8[] bytes = fixed;
    fixed[1] = 21;
    if (fixed[1] != 21 || bytes[index] != 5) { core.system_panic(); };
    core.array_len(&bytes)
};
if (two(1) != 2 || three(1) != 3) { core.system_panic(); };
%%end";
    for layout in [ScalarTargetLayout::NATIVE64, ScalarTargetLayout::WASM32] {
        let parsed = parse_source(source.clone(), text.into(), &[]);
        assert!(parsed.diagnostics.is_empty(), "{:?}", parsed.diagnostics);
        let prepared = crate::prepare_scalar_project_with_access(
            vec![(derive_scalar_declarations_from_cst(parsed.result.canonical_cst()), Vec::new())],
            vec![source.clone()], ScalarPreparationInputs { target_layout: layout },
            Some(wosy_stdlib_source::CheckedAccessComponent::linked()),
        ).unwrap_or_else(|error| panic!("{:?}", error.diagnostics));
        let checked = analyze_scalar_build(&prepared);
        assert!(checked.diagnostics().is_empty(), "{:?}", checked.diagnostics());
        let llvm = emit_scalar_build_project_llvm(&checked).unwrap().to_text();
        let context = Context::create();
        let buffer = MemoryBuffer::create_from_memory_range_copy(llvm.as_bytes(), "fixed named Copy widening");
        let module = context.create_module_from_ir(buffer).unwrap();
        module.verify().unwrap();
        let alloc = module.get_function("__wosy_core_alloc").unwrap();
        let free = module.get_function("__wosy_core_free").unwrap();
        for (name, count) in [("two", 2), ("three", 3)] {
            let function = module.get_function(&project_function_name(&source, name)).unwrap();
            let mut original = None;
            let mut snapshots = Vec::new();
            let mut releases = Vec::new();
            for block in function.get_basic_blocks() {
                for instruction in std::iter::successors(block.get_first_instruction(), |inst| inst.get_next_instruction()) {
                    let Some(callee) = CallSiteValue::try_from(instruction).ok().and_then(|call| call.get_called_fn_value())
                    else { continue };
                    if callee == alloc {
                        assert!(original.replace(instruction).is_none(), "{llvm}");
                    } else if callee == free {
                        releases.push(instruction);
                    } else if callee.get_name().to_bytes().starts_with(b"llvm.memmove.") {
                        snapshots.push(instruction);
                    }
                }
            }
            let original = original.unwrap();
            assert_eq!(known_field_operand(original, 0).unwrap().into_int_value().get_zero_extended_constant(), Some(count));
            let copies = snapshots.iter().filter(|&&snapshot|
                known_field_operand(snapshot, 0).unwrap().as_instruction_value() == Some(original)).copied().collect::<Vec<_>>();
            let [copy] = copies.as_slice() else { panic!("one independent runtime snapshot: {llvm}") };
            let fixed = known_field_operand(*copy, 1).unwrap().as_instruction_value().unwrap();
            assert_eq!(fixed.get_opcode(), InstructionOpcode::Alloca);
            assert_eq!(known_field_operand(*copy, 2).unwrap().into_int_value().get_zero_extended_constant(), Some(count));
            let [release] = releases.as_slice() else { panic!("one exact widened owner free: {llvm}") };
            assert_eq!(known_field_operand(*release, 0).unwrap().as_instruction_value(), Some(original));
        }
        if let Some(out) = std::env::var_os("WOSY_FIXED_COPY_WIDEN_IR_DIR") {
            let layout = if layout == ScalarTargetLayout::WASM32 { "wasm32" } else { "native64" };
            let controlled = llvm.replacen("define i32 @main(", "define i32 @wosy_authored_main(", 1);
            assert_ne!(controlled, llvm);
            std::fs::write(std::path::PathBuf::from(out).join(format!("{layout}-project-control.ll")), controlled).unwrap();
        }
    }
}

#[test]
fn fixed_copy_owning_argument_enters_sealed_runtime_formal_and_returns_one_owner() {
    use inkwell::{memory_buffer::MemoryBuffer, values::{CallSiteValue, InstructionOpcode}};
    let text = "%%start
u8[](u8[]) identity = fn(values) { values };
u8[](u8[]) relay = fn(values) { identity(values) };
u64() two = fn() {
    u8[2] fixed = [11, 13];
    u8[] output = relay(fixed);
    fixed[1] = 19;
    if (fixed[1] != 19) { core.system_panic(); };
    core.array_len(&output)
};
u64() three = fn() {
    u8[3] fixed = [3, 5, 7];
    u8[] output = relay(fixed);
    fixed[1] = 21;
    if (fixed[1] != 21) { core.system_panic(); };
    core.array_len(&output)
};
if (two() != 2 || three() != 3) { core.system_panic(); };
%%end";
    let mixed = text.replace("if (two()", "unit(u64) ordinary = fn(count) { u8[count] array; array = relay(array); }; ordinary(1);\nif (two()");
    for layout in [ScalarTargetLayout::NATIVE64, ScalarTargetLayout::WASM32] {
        for (closed, text) in [(true, text), (false, mixed.as_str())] {
            let prepared = prepare_initializer_test(text, layout);
            let checked = analyze_scalar_build(&prepared);
            assert!(checked.diagnostics().is_empty(), "{:?}", checked.diagnostics());
            let source = &prepared.project().modules[0].source;
            for (project, emitted) in [(false, emit_scalar_build_single_module_llvm(&checked)),
                (true, emit_scalar_build_project_llvm(&checked))]
            {
                if !closed {
                    assert!(matches!(emitted, Err(error) if error.contains("widened array move requires a closed exact owning receiver")));
                    continue;
                }
                let llvm = emitted.unwrap().to_text();
                let context = Context::create();
                let buffer = MemoryBuffer::create_from_memory_range_copy(llvm.as_bytes(), "fixed argument runtime formal");
                let module = context.create_module_from_ir(buffer).unwrap();
                module.verify().unwrap();
                let relay = module.get_function(&self_tail_test_symbol(source, project, "relay")).unwrap();
                let alloc = module.get_function("__wosy_core_alloc").unwrap();
                let free = module.get_function("__wosy_core_free").unwrap();
                for (name, count) in [("two", 2), ("three", 3)] {
                    let function = module.get_function(&self_tail_test_symbol(source, project, name)).unwrap();
                    let instructions = function.get_basic_blocks().into_iter().flat_map(|block|
                        std::iter::successors(block.get_first_instruction(), |instruction| instruction.get_next_instruction()))
                        .collect::<Vec<_>>();
                    let calls_to = |callee| instructions.iter().enumerate().filter_map(|(position, &instruction)|
                        CallSiteValue::try_from(instruction).ok().filter(|call| call.get_called_fn_value() == Some(callee))
                            .map(|_| (position, instruction))).collect::<Vec<_>>();
                    let [(born, allocation)] = calls_to(alloc)[..] else { panic!("{llvm}") };
                    let [(sent, send)] = calls_to(relay)[..] else { panic!("{llvm}") };
                    let [(released, release)] = calls_to(free)[..] else { panic!("{llvm}") };
                    let operand = |inst, index| known_field_operand(inst, index).unwrap();
                    let copies = instructions.iter().enumerate().filter_map(|(position, &inst)| {
                        let callee = CallSiteValue::try_from(inst).ok()?.get_called_fn_value()?;
                        (callee.get_name().to_bytes().starts_with(b"llvm.memmove.")
                            && operand(inst, 0).as_instruction_value() == Some(allocation)).then_some((position, inst))
                    }).collect::<Vec<_>>();
                    let [(copied, copy)] = copies[..] else { panic!("{llvm}") };
                    assert_eq!(operand(copy, 1).as_instruction_value().unwrap().get_opcode(), InstructionOpcode::Alloca);
                    assert_eq!(operand(copy, 2).into_int_value().get_zero_extended_constant(), Some(count));
                    let extent = operand(send, 0).as_instruction_value().unwrap();
                    assert_eq!(extent.get_indices(), vec![1]);
                    assert_eq!(operand(extent, 1).into_int_value().get_zero_extended_constant(), Some(count));
                    let backing = operand(extent, 0).as_instruction_value().unwrap();
                    assert_eq!(backing.get_indices(), vec![0]);
                    assert_eq!(operand(backing, 1).as_instruction_value(), Some(allocation));
                    let returned = operand(release, 0).as_instruction_value().unwrap();
                    assert_eq!(returned.get_indices(), vec![0]);
                    assert_eq!(operand(returned, 0).as_instruction_value(), Some(send));
                    assert!(born < copied && copied < sent && sent < released, "{llvm}");
                }
                if let Some(out) = std::env::var_os("WOSY_FIXED_OWNING_ARGUMENT_IR_DIR") {
                    let target = if layout == ScalarTargetLayout::WASM32 { "wasm32" } else { "native64" };
                    let route = if project { "project" } else { "standalone" };
                    let controlled = llvm.replacen("define i32 @main(", "define i32 @wosy_authored_main(", 1);
                    assert_ne!(controlled, llvm);
                    std::fs::write(std::path::PathBuf::from(out).join(format!("{target}-{route}-control.ll")), controlled).unwrap();
                }
            }
        }
    }
}

#[test]
fn widened_join_replaces_only_after_selected_pair_is_constructed() {
    use inkwell::{memory_buffer::MemoryBuffer, values::{CallSiteValue, InstructionOpcode, PhiValue}};
    let text = "%%start
u64(bool) replace = fn(pick) {
    u8[] target = [11, 13];
    u64 previous = core.array_len(&target);
    target = if (pick) {
        u8[] left = [3, 5, 7];
        left
    } else {
        u8[] right = [17, 19, 23, 29];
        right
    };
    previous * 10 + core.array_len(&target)
};
if (replace(true) != 23 || replace(false) != 24) { core.system_panic(); };
%%end";
    for layout in [ScalarTargetLayout::NATIVE64, ScalarTargetLayout::WASM32] {
        let prepared = prepare_initializer_test(text, layout);
        let checked = analyze_scalar_build(&prepared);
        assert!(checked.diagnostics().is_empty(), "{:?}", checked.diagnostics());
        let source = &prepared.project().modules[0].source;
        for (project, emitted) in [(false, emit_scalar_build_single_module_llvm(&checked)),
            (true, emit_scalar_build_project_llvm(&checked))]
        {
            let llvm = emitted.unwrap().to_text();
            let context = Context::create();
            let buffer = MemoryBuffer::create_from_memory_range_copy(llvm.as_bytes(), "exact widened conditional replacement");
            let module = context.create_module_from_ir(buffer).unwrap();
            module.verify().unwrap();
            let alloc = module.get_function("__wosy_core_alloc").unwrap();
            let free = module.get_function("__wosy_core_free").unwrap();
            let function = module.get_function(&self_tail_test_symbol(source, project, "replace")).unwrap();
            let instructions = function.get_basic_blocks().into_iter().flat_map(|block|
                std::iter::successors(block.get_first_instruction(), |instruction| instruction.get_next_instruction()))
                .collect::<Vec<_>>();
            let calls_to = |callee| instructions.iter().enumerate().filter_map(|(position, &instruction)|
                CallSiteValue::try_from(instruction).ok().filter(|call| call.get_called_fn_value() == Some(callee))
                    .map(|_| (position, instruction))).collect::<Vec<_>>();
            let allocs = calls_to(alloc);
            assert_eq!(allocs.len(), 3, "{llvm}");
            let [(release_old, predecessor), (_, selected)] = calls_to(free)[..]
                else { panic!("predecessor plus exact selected cleanup only: {llvm}") };
            let operand = |inst, index| known_field_operand(inst, index).unwrap();
            assert_eq!(operand(predecessor, 0).as_instruction_value(), Some(allocs[0].1));
            assert_eq!(operand(allocs[0].1, 0).into_int_value().get_zero_extended_constant(), Some(2));
            let joined_backing = operand(selected, 0).as_instruction_value().unwrap();
            assert_eq!(joined_backing.get_indices(), vec![0]);
            let joined = operand(joined_backing, 0).as_instruction_value().unwrap();
            let phi = PhiValue::try_from(joined).unwrap();
            assert_eq!(phi.count_incoming(), 2);
            assert_eq!(predecessor.get_parent(), joined.get_parent());
            assert!(instructions.iter().position(|&instruction| instruction == joined).unwrap() < release_old);
            let mut origins = Vec::new();
            for index in 0..2 {
                let (result, block) = phi.get_incoming(index).unwrap();
                let extent = result.as_instruction_value().unwrap();
                assert_eq!(extent.get_opcode(), InstructionOpcode::InsertValue);
                assert_eq!(extent.get_indices(), vec![1]);
                assert_eq!(extent.get_parent(), Some(block));
                let count = operand(extent, 1).into_int_value().get_zero_extended_constant().unwrap();
                let pointer = operand(extent, 0).as_instruction_value().unwrap();
                assert_eq!(pointer.get_indices(), vec![0]);
                let original = operand(pointer, 1).as_instruction_value().unwrap();
                assert!(allocs[1..].iter().any(|&(_, allocation)| allocation == original));
                assert_eq!(operand(original, 0).into_int_value().get_zero_extended_constant(), Some(count));
                origins.push(count);
            }
            origins.sort_unstable();
            assert_eq!(origins, [3, 4]);
            if let Some(out) = std::env::var_os("WOSY_WIDENED_JOIN_REPLACE_IR_DIR") {
                let target = if layout == ScalarTargetLayout::WASM32 { "wasm32" } else { "native64" };
                let route = if project { "project" } else { "standalone" };
                let controlled = llvm.replacen("define i32 @main(", "define i32 @wosy_authored_main(", 1);
                assert_ne!(controlled, llvm);
                std::fs::write(std::path::PathBuf::from(out).join(format!("{target}-{route}-control.ll")), controlled).unwrap();
            }
        }
    }
}

#[test]
fn guarded_fallible_coexisting_allocations_keep_independent_presence_and_cleanup() {
    use inkwell::{memory_buffer::MemoryBuffer, values::{CallSiteValue, InstructionOpcode}};
    let source = SourceIdentity::new("project".into(), "package".into(), "src/coexisting_fallible.w".into(), "r1".into());
    let text = "%%start
i32() observed = fn {
    i32 first =? 11;
    i32 second =? 22;
    i32 one = if (&first != null) { first } else { 1 };
    i32 other = if (&second != null) {
        i32 third =? 33;
        if (&third != null) { second + third } else { second + 3 }
    } else { 2 };
    one * 100 + other
};
if (observed() != 1155 || observed() != 155 || observed() != 1102
    || observed() != 1125 || observed() != 102) { core.system_panic(); };
%%end";
    for layout in [ScalarTargetLayout::NATIVE64, ScalarTargetLayout::WASM32] {
        let parsed = parse_source(source.clone(), text.into(), &[]);
        assert!(parsed.diagnostics.is_empty(), "{:?}", parsed.diagnostics);
        let single = crate::derive_scalar_program_with_layout(&parsed.result, layout);
        assert!(single.diagnostics.is_empty(), "{:?}", single.diagnostics);
        let project = validate_scalar_project(ScalarProject::new(
            vec![ScalarModule::from_program(single.program.clone(), Vec::new())], vec![source.clone()],
        ));
        assert!(project.diagnostics.is_empty(), "{:?}", project.diagnostics);
        for (in_project, llvm) in [(false, emit_scalar_llvm(&single).unwrap().to_text()),
            (true, emit_scalar_project_llvm(&project).unwrap().to_text())]
        {
            let context = Context::create();
            let buffer = MemoryBuffer::create_from_memory_range_copy(llvm.as_bytes(), "coexisting fallible owners");
            let module = context.create_module_from_ir(buffer).unwrap();
            module.verify().unwrap();
            let alloc = module.get_function("__wosy_core_alloc").unwrap();
            let free = module.get_function("__wosy_core_free").unwrap();
            let function = module.get_function(&self_tail_test_symbol(&source, in_project, "observed")).unwrap();
            let instructions = function.get_basic_blocks().into_iter().flat_map(|block|
                std::iter::successors(block.get_first_instruction(), |instruction| instruction.get_next_instruction()))
                .collect::<Vec<_>>();
            let calls_to = |callee| instructions.iter().copied().filter(|instruction|
                CallSiteValue::try_from(*instruction).ok().is_some_and(|call| call.get_called_fn_value() == Some(callee)))
                .collect::<Vec<_>>();
            let births = calls_to(alloc);
            let deaths = calls_to(free);
            assert_eq!(births.len(), 3, "{llvm}");
            assert_eq!(deaths.len(), 3, "{llvm}");
            let tests = instructions.iter().copied().filter(|instruction| instruction.get_opcode() == InstructionOpcode::ICmp)
                .collect::<Vec<_>>();
            assert_eq!(tests.len(), 3, "one presence test per allocation: {llvm}");
            for owner in births {
                let uses = |inst| known_field_operand(inst, 0).and_then(|value| value.as_instruction_value()) == Some(owner);
                assert_eq!(tests.iter().filter(|&&test| uses(test)).count(), 1, "{llvm}");
                assert_eq!(deaths.iter().filter(|&&free| uses(free)).count(), 1, "{llvm}");
            }
            if let Some(out) = std::env::var_os("WOSY_FALLIBLE_COEXISTING_IR_DIR") {
                let target = if layout == ScalarTargetLayout::WASM32 { "wasm32" } else { "native64" };
                let route = if in_project { "project" } else { "standalone" };
                let controlled = llvm.replacen("define i32 @main(", "define i32 @wosy_authored_main(", 1);
                assert_ne!(controlled, llvm);
                std::fs::write(std::path::PathBuf::from(out).join(format!("{target}-{route}-control.ll")), controlled).unwrap();
            }
        }
    }
}

#[test]
fn guarded_fallible_direct_call_initializer_evaluates_once_before_one_allocation() {
    use inkwell::{memory_buffer::MemoryBuffer, values::{CallSiteValue, InstructionOpcode}};
    let source = SourceIdentity::new("project".into(), "package".into(), "src/fallible_direct.w".into(), "r1".into());
    let text = "%%start
i32(i32) compute = fn(input) { input * 2 + 1 };
i32(i32) wrapper = fn(input) { compute(input) };
i32(i32) observed = fn(input) {
    i32 result =? wrapper(input);
    if (&result == null) { -1 } else { result }
};
if (observed(7) != 15 || observed(7) != -1) { core.system_panic(); };
%%end";
    let effectful = text.replace("input * 2 + 1", "u8[1] buffer; if (core.array_len(&buffer) == 1) { input } else { 0 }");
    for layout in [ScalarTargetLayout::NATIVE64, ScalarTargetLayout::WASM32] {
        for (closed, text) in [(true, text), (false, effectful.as_str())] {
            let parsed = parse_source(source.clone(), text.into(), &[]);
            assert!(parsed.diagnostics.is_empty(), "{:?}", parsed.diagnostics);
            let single = crate::derive_scalar_program_with_layout(&parsed.result, layout);
            assert!(single.diagnostics.is_empty(), "{:?}", single.diagnostics);
            let project = validate_scalar_project(ScalarProject::new(
                vec![ScalarModule::from_program(single.program.clone(), Vec::new())], vec![source.clone()],
            ));
            assert!(project.diagnostics.is_empty(), "{:?}", project.diagnostics);
            for (in_project, emitted) in [(false, emit_scalar_llvm(&single)), (true, emit_scalar_project_llvm(&project))] {
                if !closed {
                    assert!(matches!(emitted, Err(error) if error.contains("fallible call initializer requires a checked allocation-free scalar callable")));
                    continue;
                }
                let llvm = emitted.unwrap().to_text();
                let context = Context::create();
                let buffer = MemoryBuffer::create_from_memory_range_copy(llvm.as_bytes(), "fallible direct initializer");
                let module = context.create_module_from_ir(buffer).unwrap();
                module.verify().unwrap();
                let alloc = module.get_function("__wosy_core_alloc").unwrap();
                let free = module.get_function("__wosy_core_free").unwrap();
                let function = module.get_function(&self_tail_test_symbol(&source, in_project, "observed")).unwrap();
                let wrapper = module.get_function(&self_tail_test_symbol(&source, in_project, "wrapper")).unwrap();
                let instructions = function.get_basic_blocks().into_iter().flat_map(|block|
                    std::iter::successors(block.get_first_instruction(), |instruction| instruction.get_next_instruction()))
                    .collect::<Vec<_>>();
                let calls_to = |callee| instructions.iter().enumerate().filter_map(|(position, &instruction)|
                    CallSiteValue::try_from(instruction).ok().filter(|call| call.get_called_fn_value() == Some(callee))
                        .map(|_| (position, instruction))).collect::<Vec<_>>();
                let [(computed, value)] = calls_to(wrapper)[..] else { panic!("{llvm}") };
                let [(allocated, backing)] = calls_to(alloc)[..] else { panic!("{llvm}") };
                let [(released, cleanup)] = calls_to(free)[..] else { panic!("{llvm}") };
                let operand = |inst, index| known_field_operand(inst, index).unwrap();
                assert!(computed < allocated && allocated < released);
                assert_eq!(operand(cleanup, 0).as_instruction_value(), Some(backing));
                let tests = instructions.iter().filter(|&&instruction| instruction.get_opcode() == InstructionOpcode::ICmp
                    && operand(instruction, 0).as_instruction_value() == Some(backing)).count();
                assert_eq!(tests, 1, "one presence test: {llvm}");
                assert_eq!(instructions.iter().filter(|&&instruction| instruction.get_opcode() == InstructionOpcode::Store
                    && operand(instruction, 0).as_instruction_value() == Some(value)
                    && operand(instruction, 1).as_instruction_value() == Some(backing)).count(), 1, "{llvm}");
                if let Some(out) = std::env::var_os("WOSY_FALLIBLE_DIRECT_IR_DIR") {
                    let target = if layout == ScalarTargetLayout::WASM32 { "wasm32" } else { "native64" };
                    let route = if in_project { "project" } else { "standalone" };
                    let controlled = llvm.replacen("define i32 @main(", "define i32 @wosy_authored_main(", 1);
                    assert_ne!(controlled, llvm);
                    std::fs::write(std::path::PathBuf::from(out).join(format!("{target}-{route}-control.ll")), controlled).unwrap();
                }
            }
        }
    }
}

#[test]
fn guarded_fallible_allocating_call_skips_outer_materialization_on_absence() {
    use inkwell::{memory_buffer::MemoryBuffer, values::{CallSiteValue, InstructionOpcode}};
    let text = "%%start
u64(u64) _measure = fn(count) { u8[count] scratch; core.array_len(&scratch) };
u64(u64) _middle = fn(count) { u8[count] first; core.array_len(&first); u8[count] second; core.array_len(&second); u8[count] third; core.array_len(&third); _measure(count) };
u64(u64) _entry = fn(count) { _middle(count) };
u64(u64) observed = fn(count) {
    u64 result =? _entry(count);
    if (&result == null) { 0 } else { result + 1 }
};
if (observed(7) != 0 || observed(7) != 0 || observed(7) != 0 || observed(7) != 0 || observed(7) != 0 || observed(7) != 8 || observed(0) != 1) { core.system_panic(); };
%%end";
    let mixed = text.replace("u64 result =?",
        "u8[count] prior; core.array_len(&prior); _middle(count); u64 result =?");
    for layout in [ScalarTargetLayout::NATIVE64, ScalarTargetLayout::WASM32] {
        for (closed, text) in [(true, text), (false, mixed.as_str())] {
            let prepared = prepare_initializer_test(text, layout);
            let checked = analyze_scalar_build(&prepared);
            assert!(checked.diagnostics().is_empty(), "{:?}", checked.diagnostics());
            let source = &prepared.project().modules[0].source;
            for (in_project, emitted) in [(false, emit_scalar_build_single_module_llvm(&checked)),
                (true, emit_scalar_build_project_llvm(&checked))] {
                if !closed {
                    let Err(error) = emitted else { panic!("mixed callable demands its ordinary ABI") };
                    assert!(error.contains("requires exclusively checked fallible initializer uses"), "{error}");
                    continue;
                }
                let llvm = emitted.unwrap().to_text();
                let context = Context::create();
                let buffer = MemoryBuffer::create_from_memory_range_copy(llvm.as_bytes(), "fallible allocating initializer");
                let module = context.create_module_from_ir(buffer).unwrap();
                module.verify().unwrap();
                let alloc = module.get_function("__wosy_core_alloc").unwrap();
                let free = module.get_function("__wosy_core_free").unwrap();
                let leaf = module.get_function(&self_tail_test_symbol(source, in_project, "_measure")).unwrap();
                let caller = module.get_function(&self_tail_test_symbol(source, in_project, "observed")).unwrap();
                assert_eq!(leaf.get_linkage(), inkwell::module::Linkage::Internal);
                assert_eq!(caller.get_type().get_return_type(), Some(context.i64_type().into()));
                fn calls_to<'ctx>(function: inkwell::values::FunctionValue<'ctx>,
                    callee: inkwell::values::FunctionValue<'ctx>) -> Vec<InstructionValue<'ctx>> {
                    function.get_basic_blocks().into_iter()
                        .flat_map(|block| std::iter::successors(block.get_first_instruction(), |instruction| instruction.get_next_instruction()))
                        .filter(|&instruction| CallSiteValue::try_from(instruction).ok()
                            .is_some_and(|call| call.get_called_fn_value() == Some(callee))).collect()
                }
                let [scratch] = calls_to(leaf, alloc)[..] else { panic!("{llvm}") };
                let [release] = calls_to(leaf, free)[..] else { panic!("{llvm}") };
                assert_eq!(known_field_operand(release, 0).unwrap().as_instruction_value(), Some(scratch));
                let absence = leaf.get_basic_blocks().into_iter()
                    .find(|block| block.get_name().to_bytes() == b"allocation_absent").unwrap();
                assert!(absence.get_first_instruction().is_some_and(|instruction| instruction == absence.get_terminator().unwrap()
                    && instruction.get_opcode() == InstructionOpcode::Return));
                let mut previous = leaf;
                for name in ["_middle", "_entry"] {
                    let relay = module.get_function(&self_tail_test_symbol(source, in_project, name)).unwrap();
                    assert_eq!(relay.get_linkage(), inkwell::module::Linkage::Internal);
                    assert_eq!(relay.get_type(), previous.get_type());
                    let [forwarded] = calls_to(relay, previous)[..] else { panic!("{llvm}") };
                    let returned_forward = relay.get_basic_blocks().into_iter()
                        .filter_map(|block| block.get_terminator())
                        .find(|instruction| instruction.get_opcode() == InstructionOpcode::Return
                            && known_field_operand(*instruction, 0).and_then(|operand| operand.as_instruction_value()) == Some(forwarded))
                        .unwrap();
                    if name == "_middle" {
                        let [first, second, third] = calls_to(relay, alloc)[..] else { panic!("{llvm}") };
                        let releases = calls_to(relay, free);
                        assert_eq!(releases.len(), 5);
                        let pointer = |instruction| known_field_operand(instruction, 0).unwrap().as_instruction_value();
                        let mut cursor = forwarded;
                        for expected in [first, second, third] {
                            cursor = cursor.get_next_instruction().unwrap();
                            assert!(releases.contains(&cursor));
                            assert_eq!(pointer(cursor), Some(expected));
                        }
                        assert_eq!(cursor.get_next_instruction(), Some(returned_forward));
                        let blocks = relay.get_basic_blocks();
                        let absent = *blocks.iter().find(|block| block.get_name().to_bytes() == b"allocation_absent").unwrap();
                        let unwind = blocks.iter().copied().filter(|block|
                            block.get_name().to_bytes().starts_with(b"allocation_unwind")).collect::<Vec<_>>();
                        assert_eq!(unwind.len(), 2);
                        let mut prior = absent;
                        for expected in [first, second] {
                            let release = *releases.iter().find(|&&release| pointer(release) == Some(expected)
                                && release.get_parent().is_some_and(|block| unwind.contains(&block))).unwrap();
                            let block = release.get_parent().unwrap();
                            assert_eq!(block.get_first_instruction(), Some(release));
                            let jump = release.get_next_instruction().unwrap();
                            assert_eq!(jump.get_opcode(), InstructionOpcode::Br);
                            assert_eq!(jump.get_num_operands(), 1);
                            assert!(jump.get_operands().any(|operand| operand.and_then(|operand| operand.block()) == Some(prior)));
                            let next_allocation = if expected == first { second } else { third };
                            let branch = next_allocation.get_parent().unwrap().get_terminator().unwrap();
                            assert!(branch.get_operands().any(|operand| operand.and_then(|operand| operand.block()) == Some(block)));
                            prior = block;
                        }
                        let first_branch = first.get_parent().unwrap().get_terminator().unwrap();
                        assert!(first_branch.get_operands().any(|operand| operand.and_then(|operand| operand.block()) == Some(absent)));
                        assert_eq!(absent.get_first_instruction(), absent.get_terminator());
                        assert_eq!(absent.get_terminator().unwrap().get_opcode(), InstructionOpcode::Return);
                    } else {
                        assert!(calls_to(relay, alloc).is_empty() && calls_to(relay, free).is_empty());
                        assert!(relay.get_basic_blocks().into_iter().all(|block|
                            block.get_terminator().is_none_or(|term| term.get_opcode() != InstructionOpcode::Br
                                || term.get_num_operands() == 1)));
                    }
                    previous = relay;
                }
                let [rhs] = calls_to(caller, previous)[..] else { panic!("{llvm}") };
                let [dest] = calls_to(caller, alloc)[..] else { panic!("{llvm}") };
                assert_eq!(calls_to(caller, free).len(), 1);
                assert_eq!(dest.get_parent().unwrap().get_name().to_bytes(), b"fallible.destination");
                let flag = rhs.get_next_instruction().unwrap();
                assert_eq!(flag.get_opcode(), InstructionOpcode::ExtractValue);
                assert_eq!(flag.get_indices(), vec![0]);
                let branch = flag.get_next_instruction().unwrap();
                assert_eq!(branch.get_opcode(), InstructionOpcode::Br);
                assert_eq!(known_field_operand(branch, 0).unwrap().as_instruction_value(), Some(flag));
                if let Some(out) = std::env::var_os("WOSY_FALLIBLE_LINEAR_OWNING_RELAY_IR_DIR") {
                    let target = if layout == ScalarTargetLayout::WASM32 { "wasm32" } else { "native64" };
                    let route = if in_project { "project" } else { "standalone" };
                    let controlled = llvm.replacen("define i32 @main(", "define i32 @wosy_authored_main(", 1);
                    assert_ne!(controlled, llvm);
                    std::fs::write(std::path::PathBuf::from(out).join(format!("{target}-{route}-control.ll")), controlled).unwrap();
                }
            }
        }
    }
}

#[test]
fn guarded_fallible_conditional_call_selects_one_sealed_arm() {
    use inkwell::{memory_buffer::MemoryBuffer, values::CallSiteValue};
    let text = "%%start
u64(u64) _left = fn(count) { u8[count] scratch; core.array_len(&scratch) };
u64(u64) _right = fn(count) { u8[count] scratch; core.array_len(&scratch) };
u64(bool, u64) observed = fn(left, count) {
    u64 result =? if (left) { _left(count) } else { _right(count) };
    if (&result == null) { 0 } else { result + 1 }
};
if (observed(true, 7) != 0 || observed(false, 9) != 0 || observed(true, 7) != 0
    || observed(false, 9) != 10 || observed(true, 0) != 1) { core.system_panic(); };
%%end";
    let mixed = text.replace("u64(u64) _right = fn(count) { u8[count] scratch; core.array_len(&scratch) };",
        "u64(u64) _right = fn(count) { count };")
        .replace("observed(false, 9) != 10", "observed(false, 0) != 1");
    for layout in [ScalarTargetLayout::NATIVE64, ScalarTargetLayout::WASM32] {
        for (mixed, text) in [(false, text), (true, mixed.as_str())] {
            let prepared = prepare_initializer_test(text, layout);
            let checked = analyze_scalar_build(&prepared);
            assert!(checked.diagnostics().is_empty(), "{:?}", checked.diagnostics());
            let source = &prepared.project().modules[0].source;
            for (project, output) in [(false, emit_scalar_build_single_module_llvm(&checked)),
                (true, emit_scalar_build_project_llvm(&checked))]
            {
                let llvm = output.unwrap().to_text();
                let context = Context::create();
                let buffer = MemoryBuffer::create_from_memory_range_copy(llvm.as_bytes(), "sealed conditional initializer");
                let module = context.create_module_from_ir(buffer).unwrap();
                module.verify().unwrap();
                let caller = module.get_function(&self_tail_test_symbol(source, project, "observed")).unwrap();
                let blocks = caller.get_basic_blocks();
                let leaf = |name| module.get_function(&self_tail_test_symbol(source, project, name)).unwrap();
                let left = leaf("_left");
                let right = leaf("_right");
                let alloc = module.get_function("__wosy_core_alloc").unwrap();
                let named_calls = |callee| blocks.iter().flat_map(|block|
                    std::iter::successors(block.get_first_instruction(), |instruction| instruction.get_next_instruction()))
                    .filter(|&instruction| CallSiteValue::try_from(instruction).ok()
                        .is_some_and(|call| call.get_called_fn_value() == Some(callee))).collect::<Vec<_>>();
                for (leaf, arm) in [(left, b"if.then".as_slice()), (right, b"if.else".as_slice())] {
                    if leaf == right && mixed {
                        assert_eq!(leaf.get_type().get_return_type(), Some(context.i64_type().into()));
                    } else {
                        assert_eq!(leaf.get_linkage(), inkwell::module::Linkage::Internal);
                        assert!(leaf.get_type().get_return_type().unwrap().is_struct_type());
                    }
                    let [call] = named_calls(leaf)[..] else { panic!("{llvm}") };
                    assert_eq!(call.get_parent().unwrap().get_name().to_bytes(), arm);
                }
                let [destination] = named_calls(alloc)[..] else { panic!("{llvm}") };
                assert_eq!(destination.get_parent().unwrap().get_name().to_bytes(), b"fallible.destination");
                let merge = blocks.iter().find(|block| block.get_name().to_bytes() == b"fallible.binary_merge").unwrap();
                let phi = merge.get_first_instruction().unwrap();
                assert_eq!(phi.get_opcode(), InstructionOpcode::Phi);
                let present = phi.get_next_instruction().unwrap();
                assert_eq!(present.get_opcode(), InstructionOpcode::ExtractValue);
                assert_eq!(present.get_indices(), vec![0]);
                assert_eq!(known_field_operand(present, 0).unwrap().as_instruction_value(), Some(phi));
                let out_var = if mixed { "WOSY_FALLIBLE_MIXED_CONDITIONAL_IR_DIR" }
                    else { "WOSY_FALLIBLE_CONDITIONAL_IR_DIR" };
                if let Some(out) = std::env::var_os(out_var) {
                    let target = if layout == ScalarTargetLayout::WASM32 { "wasm32" } else { "native64" };
                    let route = if project { "project" } else { "standalone" };
                    let controlled = llvm.replacen("define i32 @main(", "define i32 @wosy_authored_main(", 1);
                    assert_ne!(controlled, llvm);
                    std::fs::write(std::path::PathBuf::from(out).join(format!("{target}-{route}-control.ll")), controlled).unwrap();
                }
            }
        }
    }
}

#[test]
fn guarded_fallible_and_ordinary_binding_share_one_private_body() {
    use inkwell::{memory_buffer::MemoryBuffer, values::CallSiteValue};
    let text = "%%start
u64(u64) _leaf = fn(count) { u8[count] scratch; core.array_len(&scratch) };
u64(u64) _relay = fn(count) { u8[count] local; core.array_len(&local); _leaf(count) };
u64(u64) ordinary = fn(count) { u64 result = _relay(count); result };
u64(u64) guarded = fn(count) {
    u64 result =? _relay(count);
    if (&result == null) { 0 } else { result + 1 }
};
if (ordinary(7) != 7 || ordinary(0) != 0 || guarded(9) != 0 || guarded(0) != 1) { core.system_panic(); };
ordinary(13);
%%end";
    let unsupported_use = text.replace("u64 result = _relay(count); result", "_relay(count)");
    for layout in [ScalarTargetLayout::NATIVE64, ScalarTargetLayout::WASM32] {
        let refused = prepare_initializer_test(&unsupported_use, layout);
        let refused = analyze_scalar_build(&refused);
        assert!(refused.diagnostics().is_empty(), "{:?}", refused.diagnostics());
        for output in [emit_scalar_build_single_module_llvm(&refused), emit_scalar_build_project_llvm(&refused)] {
            assert!(matches!(output, Err(error) if error.contains("requires exclusively checked fallible initializer uses")));
        }
        let prepared = prepare_initializer_test(text, layout);
        let checked = analyze_scalar_build(&prepared);
        assert!(checked.diagnostics().is_empty(), "{:?}", checked.diagnostics());
        let source = &prepared.project().modules[0].source;
        for (project, output) in [(false, emit_scalar_build_single_module_llvm(&checked)),
            (true, emit_scalar_build_project_llvm(&checked))]
        {
            let llvm = output.unwrap().to_text();
            let context = Context::create();
            let buffer = MemoryBuffer::create_from_memory_range_copy(llvm.as_bytes(), "sealed dual-use fallible callable");
            let module = context.create_module_from_ir(buffer).unwrap();
            module.verify().unwrap();
            let authored = |name| module.get_function(&self_tail_test_symbol(source, project, name)).unwrap();
            let ordinary = authored("ordinary");
            let guarded = authored("guarded");
            let relay = authored("_relay");
            let panic = module.get_function("__wosy_core_system_panic").unwrap();
            assert_eq!(relay.get_linkage(), inkwell::module::Linkage::Internal);
            fn calls_to<'ctx>(caller: inkwell::values::FunctionValue<'ctx>,
                callee: inkwell::values::FunctionValue<'ctx>) -> Vec<InstructionValue<'ctx>> {
                caller.get_basic_blocks().into_iter().flat_map(|block|
                    std::iter::successors(block.get_first_instruction(), |instruction| instruction.get_next_instruction()))
                    .filter(|&instruction| CallSiteValue::try_from(instruction).ok()
                        .is_some_and(|call| call.get_called_fn_value() == Some(callee))).collect()
            }
            for caller in [ordinary, guarded] {
                assert_eq!(caller.get_type().get_return_type(), Some(context.i64_type().into()));
                assert_eq!(calls_to(caller, relay).len(), 1);
            }
            assert!(calls_to(guarded, panic).is_empty());
            let [ordinary_panic] = calls_to(ordinary, panic)[..] else { panic!("{llvm}") };
            let absent = ordinary_panic.get_parent().unwrap();
            assert_eq!(absent.get_name().to_bytes(), b"ordinary.allocation_absent");
            assert_eq!(absent.get_first_instruction(), Some(ordinary_panic));
            assert_eq!(ordinary_panic.get_next_instruction().unwrap().get_opcode(), InstructionOpcode::Unreachable);
            let [ordinary_call] = calls_to(ordinary, relay)[..] else { panic!("{llvm}") };
            let status = ordinary_call.get_next_instruction().unwrap();
            assert_eq!(status.get_opcode(), InstructionOpcode::ExtractValue);
            assert_eq!(status.get_indices(), vec![0]);
            let branch = status.get_next_instruction().unwrap();
            assert_eq!(known_field_operand(branch, 0).unwrap().as_instruction_value(), Some(status));
            assert!(branch.get_operands().any(|operand| operand.and_then(|operand| operand.block()) == Some(absent)));
            assert!(calls_to(ordinary, module.get_function("__wosy_core_alloc").unwrap()).is_empty());
            assert!(calls_to(ordinary, module.get_function("__wosy_core_free").unwrap()).is_empty());
            if let Some(out) = std::env::var_os("WOSY_FALLIBLE_DUAL_USE_IR_DIR") {
                let target = if layout == ScalarTargetLayout::WASM32 { "wasm32" } else { "native64" };
                let route = if project { "project" } else { "standalone" };
                let controlled = llvm.replacen("define i32 @main(", "define i32 @wosy_authored_main(", 1);
                assert_ne!(controlled, llvm);
                std::fs::write(std::path::PathBuf::from(out).join(format!("{target}-{route}-control.ll")), controlled).unwrap();
            }
        }
    }
}

#[test]
fn guarded_fallible_signed_buffer_result_preserves_i32_and_presence() {
    use inkwell::{memory_buffer::MemoryBuffer, values::CallSiteValue};
    let text = "%%start
i32(u64) _calculate_from_new_buffer = fn(count) {
    u8[count] scratch; 7 - core.int_trunc<i32>(core.array_len(&scratch))
};
i32(u64) _relay = fn(count) { u8[count] local; core.array_len(&local); _calculate_from_new_buffer(count) };
i32(u64) _plain = fn(count) { if (count == 0) { 0 } else { 1 } };
i32(u64) ordinary = fn(count) { i32 result = _relay(count); result };
i32(u64) direct = fn(count) {
    i32 result =? _calculate_from_new_buffer(count);
    if (&result == null) { -99 } else { result }
};
i32(bool, u64) guarded = fn(use_buffer, count) {
    i32 result =? if (use_buffer) { _relay(count) } else { _plain(count) };
    if (&result == null) { -99 } else { result }
};
if (ordinary(0) != 7 || ordinary(7) != 0 || ordinary(9) != -2
    || guarded(true, 9) != -99 || guarded(true, 9) != -2
    || guarded(true, 7) != -99 || guarded(false, 0) != 0 || direct(0) != 7) { core.system_panic(); };
ordinary(13);
%%end";
    for layout in [ScalarTargetLayout::NATIVE64, ScalarTargetLayout::WASM32] {
        let prepared = prepare_initializer_test(text, layout);
        let checked = analyze_scalar_build(&prepared);
        assert!(checked.diagnostics().is_empty(), "{:?}", checked.diagnostics());
        let source = &prepared.project().modules[0].source;
        for (project, output) in [(false, emit_scalar_build_single_module_llvm(&checked)),
            (true, emit_scalar_build_project_llvm(&checked))]
        {
            let llvm = output.unwrap().to_text();
            let context = Context::create();
            let buffer = MemoryBuffer::create_from_memory_range_copy(llvm.as_bytes(), "sealed signed buffer result");
            let module = context.create_module_from_ir(buffer).unwrap();
            module.verify().unwrap();
            let authored = |name| module.get_function(&self_tail_test_symbol(source, project, name)).unwrap();
            let tagged = context.struct_type(&[context.bool_type().into(), context.i32_type().into()], false);
            for name in ["_calculate_from_new_buffer", "_relay"] {
                let callable = authored(name);
                assert_eq!(callable.get_linkage(), inkwell::module::Linkage::Internal);
                assert_eq!(callable.get_type().get_return_type(), Some(tagged.into()));
            }
            for name in ["ordinary", "_plain", "guarded", "direct"] {
                assert_eq!(authored(name).get_type().get_return_type(), Some(context.i32_type().into()));
            }
            let guarded = authored("guarded");
            let blocks = guarded.get_basic_blocks();
            let merge = blocks.iter().find(|block| block.get_name().to_bytes() == b"fallible.binary_merge").unwrap();
            let phi = merge.get_first_instruction().unwrap();
            assert_eq!(phi.get_opcode(), InstructionOpcode::Phi);
            assert_eq!(phi.get_type().into_struct_type(), tagged);
            let present = phi.get_next_instruction().unwrap();
            assert_eq!(present.get_opcode(), InstructionOpcode::ExtractValue);
            assert_eq!(present.get_indices(), vec![0]);
            let alloc = module.get_function("__wosy_core_alloc").unwrap();
            let destinations = blocks.iter().flat_map(|block|
                std::iter::successors(block.get_first_instruction(), |instruction| instruction.get_next_instruction()))
                .filter(|&instruction| CallSiteValue::try_from(instruction).ok()
                    .is_some_and(|call| call.get_called_fn_value() == Some(alloc))).collect::<Vec<_>>();
            let [destination] = destinations[..] else { panic!("{llvm}") };
            assert_eq!(destination.get_parent().unwrap().get_name().to_bytes(), b"fallible.destination");
            for i in [0, 1] {
                assert_eq!(known_field_operand(destination, i).unwrap().into_int_value().get_zero_extended_constant(), Some(4));
            }
            if let Some(out) = std::env::var_os("WOSY_FALLIBLE_SIGNED_BUFFER_IR_DIR") {
                let target = if layout == ScalarTargetLayout::WASM32 { "wasm32" } else { "native64" };
                let route = if project { "project" } else { "standalone" };
                let controlled = llvm.replacen("define i32 @main(", "define i32 @wosy_authored_main(", 1);
                assert_ne!(controlled, llvm);
                std::fs::write(std::path::PathBuf::from(out).join(format!("{target}-{route}-control.ll")), controlled).unwrap();
            }
        }
    }
}

#[test]
fn guarded_fallible_terminal_prefix_unwinds_only_installed_owners() {
    use inkwell::{memory_buffer::MemoryBuffer, values::CallSiteValue};
    let text = "%%start
i32(u64) _terminal = fn(count) {
    u8[count] first; core.array_len(&first);
    u8[count] second; core.array_len(&second);
    u8[count] third; 7 - core.int_trunc<i32>(core.array_len(&third))
};
i32(u64) observed = fn(count) {
    i32 result =? _terminal(count);
    if (&result == null) { -99 } else { result }
};
if (observed(7) != -99 || observed(7) != -99 || observed(7) != -99 || observed(7) != -99
    || observed(9) != -2 || observed(7) != 0 || observed(0) != 7) { core.system_panic(); };
%%end";
    for layout in [ScalarTargetLayout::NATIVE64, ScalarTargetLayout::WASM32] {
        let prepared = prepare_initializer_test(text, layout);
        let checked = analyze_scalar_build(&prepared);
        assert!(checked.diagnostics().is_empty(), "{:?}", checked.diagnostics());
        let source = &prepared.project().modules[0].source;
        for (project, output) in [(false, emit_scalar_build_single_module_llvm(&checked)),
            (true, emit_scalar_build_project_llvm(&checked))]
        {
            let llvm = output.unwrap().to_text();
            let context = Context::create();
            let buffer = MemoryBuffer::create_from_memory_range_copy(llvm.as_bytes(), "fallible terminal prefix");
            let module = context.create_module_from_ir(buffer).unwrap();
            module.verify().unwrap();
            let terminal = module.get_function(&self_tail_test_symbol(source, project, "_terminal")).unwrap();
            let observed = module.get_function(&self_tail_test_symbol(source, project, "observed")).unwrap();
            let allocate = module.get_function("__wosy_core_alloc").unwrap();
            let release = module.get_function("__wosy_core_free").unwrap();
            assert_eq!(terminal.get_linkage(), inkwell::module::Linkage::Internal);
            assert_eq!(terminal.get_type().get_return_type(), Some(
                context.struct_type(&[context.bool_type().into(), context.i32_type().into()], false).into()));
            assert_eq!(observed.get_type().get_return_type(), Some(context.i32_type().into()));
            let blocks = terminal.get_basic_blocks();
            let calls = |callee| blocks.iter().flat_map(|block|
                std::iter::successors(block.get_first_instruction(), |instruction| instruction.get_next_instruction()))
                .filter(|&instruction| CallSiteValue::try_from(instruction).ok()
                    .is_some_and(|call| call.get_called_fn_value() == Some(callee))).collect::<Vec<_>>();
            let [first, second, third] = calls(allocate)[..] else { panic!("{llvm}") };
            let releases = calls(release);
            assert_eq!(releases.len(), 5);
            let absent = *blocks.iter().find(|block| block.get_name().to_bytes() == b"allocation_absent").unwrap();
            assert_eq!(absent.get_first_instruction(), absent.get_terminator());
            assert_eq!(absent.get_terminator().unwrap().get_opcode(), InstructionOpcode::Return);
            let mut prior = absent;
            for (owner, next) in [(first, second), (second, third)] {
                let released = *releases.iter().find(|release| release.get_parent().unwrap().get_name().to_bytes()
                    .starts_with(b"allocation_unwind")
                    && known_field_operand(**release, 0).unwrap().as_instruction_value() == Some(owner)).unwrap();
                let cold = released.get_parent().unwrap();
                assert_eq!(cold.get_first_instruction(), Some(released));
                let jump = released.get_next_instruction().unwrap();
                assert_eq!(jump.get_num_operands(), 1);
                assert!(jump.get_operands().any(|operand| operand.and_then(|operand| operand.block()) == Some(prior)));
                let null_branch = next.get_parent().unwrap().get_terminator().unwrap();
                assert!(null_branch.get_operands().any(|operand| operand.and_then(|operand| operand.block()) == Some(cold)));
                prior = cold;
            }
            if let Some(out) = std::env::var_os("WOSY_FALLIBLE_TERMINAL_PREFIX_IR_DIR") {
                let target = if layout == ScalarTargetLayout::WASM32 { "wasm32" } else { "native64" };
                let route = if project { "project" } else { "standalone" };
                let controlled = llvm.replacen("define i32 @main(", "define i32 @wosy_authored_main(", 1);
                assert_ne!(controlled, llvm);
                std::fs::write(std::path::PathBuf::from(out).join(format!("{target}-{route}-control.ll")), controlled).unwrap();
            }
        }
    }
}

#[test]
fn guarded_fallible_typed_binary_propagates_before_later_work() {
    use inkwell::{memory_buffer::MemoryBuffer, values::CallSiteValue};
    let text = "%%start
u64(u64) _wide = fn(count) { u16[count] scratch; core.array_len(&scratch) };
i32(u64) _signed = fn(count) { u16[count] scratch; 7 - core.int_trunc<i32>(core.array_len(&scratch)) };
u64(u64) plus = fn(count) {
    u64 result =? _wide(count) + 2;
    if (&result == null) { 0 } else { result }
};
i32(u64, i32) masked = fn(count, mask) {
    i32 result =? _signed(count) ^ mask;
    if (&result == null) { -99 } else { result }
};
if (plus(7) != 0 || plus(7) != 9 || plus(0) != 0
    || masked(9, 0) != -2 || masked(7, 0) != 0 || masked(7, 0) != -99) { core.system_panic(); };
%%end";
    for layout in [ScalarTargetLayout::NATIVE64, ScalarTargetLayout::WASM32] {
        let prepared = prepare_initializer_test(text, layout);
        let checked = analyze_scalar_build(&prepared);
        assert!(checked.diagnostics().is_empty(), "{:?}", checked.diagnostics());
        let source = &prepared.project().modules[0].source;
        for (project, output) in [(false, emit_scalar_build_single_module_llvm(&checked)),
            (true, emit_scalar_build_project_llvm(&checked))]
        {
            let llvm = output.unwrap().to_text();
            let context = Context::create();
            let buffer = MemoryBuffer::create_from_memory_range_copy(llvm.as_bytes(), "sealed typed binary initializer");
            let module = context.create_module_from_ir(buffer).unwrap();
            module.verify().unwrap();
            let function = |name: &str| module.get_function(&self_tail_test_symbol(source, project, name)).unwrap();
            let allocate = module.get_function("__wosy_core_alloc").unwrap();
            for (caller, leaf, op, integer) in [
                ("plus", "_wide", InstructionOpcode::Add, context.i64_type()),
                ("masked", "_signed", InstructionOpcode::Xor, context.i32_type()),
            ] {
                let (caller, leaf) = (function(caller), function(leaf));
                assert_eq!(caller.get_type().get_return_type(), Some(integer.into()));
                let pair = context.struct_type(&[context.bool_type().into(), integer.into()], false);
                assert_eq!(leaf.get_type().get_return_type(), Some(pair.into()));
                let blocks = caller.get_basic_blocks();
                let calls = |callee| blocks.iter().flat_map(|block|
                    std::iter::successors(block.get_first_instruction(), |instruction| instruction.get_next_instruction()))
                    .filter(|&instruction| CallSiteValue::try_from(instruction).ok()
                        .is_some_and(|call| call.get_called_fn_value() == Some(callee))).collect::<Vec<_>>();
                let [left] = calls(leaf)[..] else { panic!("{llvm}") };
                let left_status = left.get_next_instruction().unwrap();
                let dispatch = left_status.get_next_instruction().unwrap();
                assert_eq!(left_status.get_indices(), vec![0]);
                assert_eq!(known_field_operand(dispatch, 0).unwrap().as_instruction_value(), Some(left_status));
                let present = *blocks.iter().find(|block| block.get_name().to_bytes() == b"fallible.binary_present").unwrap();
                let merge = *blocks.iter().find(|block| block.get_name().to_bytes() == b"fallible.binary_merge").unwrap();
                for destination in [present, merge] {
                    assert!(dispatch.get_operands().any(|operand| operand.and_then(|operand| operand.block()) == Some(destination)));
                }
                assert!(std::iter::successors(present.get_first_instruction(), |instruction| instruction.get_next_instruction())
                    .any(|instruction| instruction.get_opcode() == op));
                let [destination] = calls(allocate)[..] else { panic!("{llvm}") };
                assert_eq!(destination.get_parent().unwrap().get_name().to_bytes(), b"fallible.destination");
                let first = merge.get_first_instruction().unwrap();
                assert_eq!(first.get_opcode(), InstructionOpcode::Phi);
                assert_eq!(first.get_type().into_struct_type(), pair);
            }
            if let Some(out) = std::env::var_os("WOSY_FALLIBLE_TYPED_BINARY_IR_DIR") {
                let target = if layout == ScalarTargetLayout::WASM32 { "wasm32" } else { "native64" };
                let route = if project { "project" } else { "standalone" };
                let controlled = llvm.replacen("define i32 @main(", "define i32 @wosy_authored_main(", 1);
                assert_ne!(controlled, llvm);
                std::fs::write(std::path::PathBuf::from(out).join(format!("{target}-{route}-control.ll")), controlled).unwrap();
            }
        }
    }
}

#[test]
fn guarded_fallible_expression_walk_stops_before_the_next_allocating_operand() {
    use inkwell::{memory_buffer::MemoryBuffer, values::CallSiteValue};
    let text = "%%start
u64(u64) _first = fn(count) { u16[count] scratch; core.array_len(&scratch) };
u64(u64) _second = fn(count) { u8[count] scratch; core.array_len(&scratch) };
u64(u64, u64) right = fn(left, count) {
    u64 result =? left ^ _second(count);
    if (&result == null) { 99 } else { result }
};
u64(u64, u64) both = fn(first, second) {
    u64 result =? (_first(first) ^ 1) + _second(second);
    if (&result == null) { 99 } else { result }
};
if (right(10, 9) != 99 || right(10, 9) != 3 || both(7, 9) != 99 || both(7, 9) != 99
    || both(7, 9) != 99 || both(7, 9) != 15 || both(1, 0) != 0) { core.system_panic(); };
%%end";
    let unbounded = text.replace("left ^ _second(count)", "left + _second(count)");
    for layout in [ScalarTargetLayout::NATIVE64, ScalarTargetLayout::WASM32] {
        let checked = prepare_initializer_test(&unbounded, layout);
        let checked = analyze_scalar_build(&checked);
        for output in [emit_scalar_build_single_module_llvm(&checked), emit_scalar_build_project_llvm(&checked)] {
            assert!(matches!(output, Err(error) if error.contains("fallible checked addition has no proved in-range result")));
        }

        let prepared = prepare_initializer_test(text, layout);
        let checked = analyze_scalar_build(&prepared);
        assert!(checked.diagnostics().is_empty(), "{:?}", checked.diagnostics());
        let source = &prepared.project().modules[0].source;
        for (project, output) in [(false, emit_scalar_build_single_module_llvm(&checked)),
            (true, emit_scalar_build_project_llvm(&checked))]
        {
            let llvm = output.unwrap().to_text();
            let context = Context::create();
            let buffer = MemoryBuffer::create_from_memory_range_copy(llvm.as_bytes(), "fallible expression walk");
            let module = context.create_module_from_ir(buffer).unwrap();
            module.verify().unwrap();
            let authored = |name: &str| module.get_function(&self_tail_test_symbol(source, project, name)).unwrap();
            let alloc = module.get_function("__wosy_core_alloc").unwrap();
            let first = authored("_first");
            let second = authored("_second");
            for (name, has_first) in [("right", false), ("both", true)] {
                let caller = authored(name);
                assert_eq!(caller.get_type().get_return_type(), Some(context.i64_type().into()));
                let blocks = caller.get_basic_blocks();
                let calls = |callee| blocks.iter().flat_map(|block|
                    std::iter::successors(block.get_first_instruction(), |instruction| instruction.get_next_instruction()))
                    .filter(|&instruction| CallSiteValue::try_from(instruction).ok()
                        .is_some_and(|call| call.get_called_fn_value() == Some(callee))).collect::<Vec<_>>();
                let [second_call] = calls(second)[..] else { panic!("{llvm}") };
                let [destination] = calls(alloc)[..] else { panic!("{llvm}") };
                assert_eq!(destination.get_parent().unwrap().get_name().to_bytes(), b"fallible.destination");
                let merge = *blocks.iter().find(|block| block.get_name().to_bytes() == b"fallible.binary_merge").unwrap();
                let phi = merge.get_first_instruction().unwrap();
                assert_eq!(phi.get_opcode(), InstructionOpcode::Phi);
                assert_eq!(inkwell::values::PhiValue::try_from(phi).unwrap().count_incoming(), if has_first { 3 } else { 2 });
                let branch_after = |call: InstructionValue<'_>| {
                    let status = call.get_next_instruction().unwrap();
                    assert_eq!(status.get_indices(), vec![0]);
                    let branch = status.get_next_instruction().unwrap();
                    assert_eq!(known_field_operand(branch, 0).unwrap().as_instruction_value(), Some(status));
                    assert!(branch.get_operands().any(|operand| operand.and_then(|operand| operand.block()) == Some(merge)));
                };
                branch_after(second_call);
                if has_first {
                    let [first_call] = calls(first)[..] else { panic!("{llvm}") };
                    branch_after(first_call);
                    let second_block = second_call.get_parent().unwrap();
                    assert!(first_call.get_parent().unwrap().get_terminator().unwrap().get_operands()
                        .any(|operand| operand.and_then(|operand| operand.block()) == Some(second_block)));
                    assert!(std::iter::successors(second_block.get_first_instruction(), |instruction| instruction.get_next_instruction())
                        .take_while(|instruction| *instruction != second_call)
                        .any(|instruction| instruction.get_opcode() == InstructionOpcode::Xor));
                } else {
                    assert!(calls(first).is_empty());
                }
            }
            if let Some(out) = std::env::var_os("WOSY_FALLIBLE_EXPRESSION_WALK_IR_DIR") {
                let target = if layout == ScalarTargetLayout::WASM32 { "wasm32" } else { "native64" };
                let route = if project { "project" } else { "standalone" };
                let controlled = llvm.replacen("define i32 @main(", "define i32 @wosy_authored_main(", 1);
                assert_ne!(controlled, llvm);
                std::fs::write(std::path::PathBuf::from(out).join(format!("{target}-{route}-control.ll")), controlled).unwrap();
            }
        }
    }
}

#[test]
fn guarded_fallible_numeric_walk_selects_nested_arms_before_allocating() {
    use inkwell::{memory_buffer::MemoryBuffer, values::CallSiteValue};
    let text = "%%start
u64(u64) _first = fn(count) { u16[count] scratch; core.array_len(&scratch) };
u64(u64) _second = fn(count) { u8[count] scratch; core.array_len(&scratch) };
u64(bool, bool, bool, u64) observed = fn(choose, ready, fallback, count) {
    u64 result =? (if (choose && ready) { _first(count) ^ 1 }
        else { if (fallback) { _second(count) } else { 0 } }) + 2;
    if (&result == null) { 99 } else { result }
};
if (observed(false, true, false, 7) != 2 || observed(true, false, false, 7) != 99
    || observed(true, true, true, 7) != 99 || observed(true, true, true, 7) != 8
    || observed(true, false, true, 9) != 99 || observed(false, true, true, 9) != 11) { core.system_panic(); };
%%end";
    for layout in [ScalarTargetLayout::NATIVE64, ScalarTargetLayout::WASM32] {
        let prepared = prepare_initializer_test(text, layout);
        let checked = analyze_scalar_build(&prepared);
        assert!(checked.diagnostics().is_empty(), "{:?}", checked.diagnostics());
        let source = &prepared.project().modules[0].source;
        for (project, output) in [(false, emit_scalar_build_single_module_llvm(&checked)),
            (true, emit_scalar_build_project_llvm(&checked))]
        {
            let llvm = output.unwrap().to_text();
            let context = Context::create();
            let buffer = MemoryBuffer::create_from_memory_range_copy(llvm.as_bytes(), "fallible selected numeric arms");
            let module = context.create_module_from_ir(buffer).unwrap();
            module.verify().unwrap();
            let authored = |name: &str| module.get_function(&self_tail_test_symbol(source, project, name)).unwrap();
            let observed = authored("observed");
            assert_eq!(observed.get_type().get_return_type(), Some(context.i64_type().into()));
            let blocks = observed.get_basic_blocks();
            assert_eq!(blocks.iter().filter(|block| block.get_name().to_bytes() == b"short_circuit.rhs").count(), 1);
            let merge = *blocks.iter().find(|block| block.get_name().to_bytes() == b"fallible.binary_merge").unwrap();
            let phi = inkwell::values::PhiValue::try_from(merge.get_first_instruction().unwrap()).unwrap();
            assert_eq!(phi.count_incoming(), 3);
            let calls_to = |callee| blocks.iter().flat_map(|block|
                std::iter::successors(block.get_first_instruction(), |instruction| instruction.get_next_instruction()))
                .filter(|&instruction| CallSiteValue::try_from(instruction).ok()
                    .is_some_and(|call| call.get_called_fn_value() == Some(callee))).collect::<Vec<_>>();
            for target in [authored("_first"), authored("_second")] {
                let [call] = calls_to(target)[..] else { panic!("{llvm}") };
                let block = call.get_parent().unwrap();
                assert!(block.get_name().to_bytes().starts_with(b"if.then"));
                let status = call.get_next_instruction().unwrap();
                assert_eq!(status.get_indices(), vec![0]);
                assert!(block.get_terminator().unwrap().get_operands()
                    .any(|operand| operand.and_then(|operand| operand.block()) == Some(merge)));
            }
            let [destination] = calls_to(module.get_function("__wosy_core_alloc").unwrap())[..] else { panic!("{llvm}") };
            assert_eq!(destination.get_parent().unwrap().get_name().to_bytes(), b"fallible.destination");
            if let Some(out) = std::env::var_os("WOSY_FALLIBLE_CONDITIONAL_WALK_IR_DIR") {
                let target = if layout == ScalarTargetLayout::WASM32 { "wasm32" } else { "native64" };
                let route = if project { "project" } else { "standalone" };
                let controlled = llvm.replacen("define i32 @main(", "define i32 @wosy_authored_main(", 1);
                assert_ne!(controlled, llvm);
                std::fs::write(std::path::PathBuf::from(out).join(format!("{target}-{route}-control.ll")), controlled).unwrap();
            }
        }
    }
}

#[test]
fn guarded_fallible_call_arguments_stop_before_later_arguments_and_outer_call() {
    use inkwell::{memory_buffer::MemoryBuffer, values::CallSiteValue};
    let text = "%%start
u64(u64) _inner = fn(count) { u16[count] scratch; core.array_len(&scratch) };
u64(u64) _outer = fn(count) { u8[count] scratch; core.array_len(&scratch) };
u64(u64, u64) _plain = fn(first, second) { first ^ second };
u64(u64) allocating = fn(count) {
    u64 result =? _outer(_inner(count));
    if (&result == null) { 99 } else { result }
};
u64(u64, u64) ordinary = fn(first, second) {
    u64 result =? _plain(_inner(first), _outer(second));
    if (&result == null) { 99 } else { result }
};
if (allocating(7) != 99 || allocating(7) != 99 || allocating(7) != 99 || allocating(0) != 0
    || ordinary(7, 9) != 99 || ordinary(7, 9) != 99 || ordinary(7, 9) != 14) { core.system_panic(); };
%%end";
    for layout in [ScalarTargetLayout::NATIVE64, ScalarTargetLayout::WASM32] {
        let prepared = prepare_initializer_test(text, layout);
        let checked = analyze_scalar_build(&prepared);
        assert!(checked.diagnostics().is_empty(), "{:?}", checked.diagnostics());
        let source = &prepared.project().modules[0].source;
        for (project, output) in [(false, emit_scalar_build_single_module_llvm(&checked)),
            (true, emit_scalar_build_project_llvm(&checked))]
        {
            let llvm = output.unwrap().to_text();
            let context = Context::create();
            let buffer = MemoryBuffer::create_from_memory_range_copy(llvm.as_bytes(), "fallible argument evaluation");
            let module = context.create_module_from_ir(buffer).unwrap();
            module.verify().unwrap();
            let authored = |name: &str| module.get_function(&self_tail_test_symbol(source, project, name)).unwrap();
            let plain = authored("_plain");
            assert_eq!(plain.get_type(), context.i64_type().fn_type(&[context.i64_type().into(), context.i64_type().into()], false));
            for (caller, extra) in [(authored("allocating"), false), (authored("ordinary"), true)] {
                assert_eq!(caller.get_type().get_return_type(), Some(context.i64_type().into()));
                let blocks = caller.get_basic_blocks();
                let calls_to = |callee| blocks.iter().flat_map(|block| std::iter::successors(
                    block.get_first_instruction(), |instruction| instruction.get_next_instruction()))
                    .filter(|&instruction| CallSiteValue::try_from(instruction).ok()
                        .is_some_and(|call| call.get_called_fn_value() == Some(callee))).collect::<Vec<_>>();
                let [inner] = calls_to(authored("_inner"))[..] else { panic!("{llvm}") };
                let [outer] = calls_to(authored("_outer"))[..] else { panic!("{llvm}") };
                let merge = *blocks.iter().find(|block| block.get_name().to_bytes() == b"fallible.binary_merge").unwrap();
                fn branch<'ctx>(call: InstructionValue<'ctx>) -> InstructionValue<'ctx> {
                    call.get_parent().unwrap().get_terminator().unwrap()
                }
                let enters = |instruction: InstructionValue<'_>, block| instruction.get_operands()
                    .any(|operand| operand.and_then(|operand| operand.block()) == Some(block));
                assert!(enters(branch(inner), merge) && enters(branch(inner), outer.get_parent().unwrap()));
                assert!(enters(branch(outer), merge));
                assert_eq!(inkwell::values::PhiValue::try_from(merge.get_first_instruction().unwrap())
                    .unwrap().count_incoming(), 3);
                let calls = calls_to(plain);
                if extra {
                    let [plain] = calls[..] else { panic!("{llvm}") };
                    assert!(enters(branch(outer), plain.get_parent().unwrap()));
                } else {
                    assert!(calls.is_empty());
                    let payload = outer.get_parent().unwrap().get_first_instruction().unwrap();
                    assert_eq!(known_field_operand(outer, 0).unwrap().as_instruction_value(), Some(payload));
                    assert_eq!(payload.get_indices(), vec![1]);
                }
                let [destination] = calls_to(module.get_function("__wosy_core_alloc").unwrap())[..] else { panic!("{llvm}") };
                assert_eq!(destination.get_parent().unwrap().get_name().to_bytes(), b"fallible.destination");
            }
            if let Some(out) = std::env::var_os("WOSY_FALLIBLE_CALL_ARGUMENTS_IR_DIR") {
                let target = if layout == ScalarTargetLayout::WASM32 { "wasm32" } else { "native64" };
                let route = if project { "project" } else { "standalone" };
                let controlled = llvm.replacen("define i32 @main(", "define i32 @wosy_authored_main(", 1);
                assert_ne!(controlled, llvm);
                std::fs::write(std::path::PathBuf::from(out).join(format!("{target}-{route}-control.ll")), controlled).unwrap();
            }
        }
    }
}

#[test]
fn ordinary_fallible_expression_keeps_live_owners_on_the_success_path() {
    use inkwell::{memory_buffer::MemoryBuffer, values::CallSiteValue};
    let template = "%%start
u64(u64) _inner = fn(count) { u16[count] scratch; core.array_len(&scratch) };
u64(u64) _outer = fn(count) { u8[count] scratch; core.array_len(&scratch) };
u64(u64) guarded = fn(count) {
    u64 result =? _outer(_inner(count));
    if (&result == null) { 99 } else { result }
};
u64(bool, u64) ordinary = fn(select, count) {
    u8[count] prior;
    u64 result = if (select) { _outer(_inner(count)) ^ 0 } else { 0 };
    core.array_len(&prior) ^ result ^ count
};
if (guarded(0) != 0 || ordinary(false, 7) != 0 || ordinary(true, 0) != 0
    || ordinary(true, 7) != 7 || ordinary(true, COUNT) != 0) { core.system_panic(); };
%%end";
    for (witness, count) in [("inner", "11"), ("outer", "13")] {
        let text = template.replace("COUNT", count);
        for layout in [ScalarTargetLayout::NATIVE64, ScalarTargetLayout::WASM32] {
            let prepared = prepare_initializer_test(&text, layout);
            let checked = analyze_scalar_build(&prepared);
            assert!(checked.diagnostics().is_empty(), "{:?}", checked.diagnostics());
            let source = &prepared.project().modules[0].source;
            for (project, output) in [(false, emit_scalar_build_single_module_llvm(&checked)),
                (true, emit_scalar_build_project_llvm(&checked))]
            {
                let llvm = output.unwrap().to_text();
                let context = Context::create();
                let buffer = MemoryBuffer::create_from_memory_range_copy(llvm.as_bytes(), "ordinary fallible expression");
                let module = context.create_module_from_ir(buffer).unwrap();
                module.verify().unwrap();
                let authored = |name: &str| module.get_function(&self_tail_test_symbol(source, project, name)).unwrap();
                let ordinary = authored("ordinary");
                assert_eq!(ordinary.get_type().get_return_type(), Some(context.i64_type().into()));
                let blocks = ordinary.get_basic_blocks();
                let calls_to = |callee| blocks.iter().flat_map(|block| std::iter::successors(
                    block.get_first_instruction(), |instruction| instruction.get_next_instruction()))
                    .filter(|&instruction| CallSiteValue::try_from(instruction).ok()
                        .is_some_and(|call| call.get_called_fn_value() == Some(callee))).collect::<Vec<_>>();
                let panics = calls_to(module.get_function("__wosy_core_system_panic").unwrap());
                assert_eq!(panics.len(), 2);
                let [panic] = panics.into_iter().filter(|call|
                    call.get_parent().unwrap().get_name().to_bytes() == b"ordinary.allocation_absent")
                    .collect::<Vec<_>>()[..] else { panic!("{llvm}") };
                let failure = panic.get_parent().unwrap();
                assert_eq!(failure.get_first_instruction(), Some(panic));
                assert_eq!(failure.get_terminator().unwrap().get_opcode(), InstructionOpcode::Unreachable);
                let [prior] = calls_to(module.get_function("__wosy_core_alloc").unwrap())[..] else { panic!("{llvm}") };
                let [release] = calls_to(module.get_function("__wosy_core_free").unwrap())[..] else { panic!("{llvm}") };
                assert_eq!(known_field_operand(release, 0).unwrap().as_instruction_value(), Some(prior));
                assert_ne!(release.get_parent(), Some(failure));
                let [inner] = calls_to(authored("_inner"))[..] else { panic!("{llvm}") };
                let [outer] = calls_to(authored("_outer"))[..] else { panic!("{llvm}") };
                for instruction in [inner, outer] {
                    let branch = instruction.get_parent().unwrap().get_terminator().unwrap();
                    assert!(branch.get_operands().any(|operand| operand.and_then(|operand| operand.block()) == Some(failure)));
                }
                assert!(inner.get_parent().unwrap().get_terminator().unwrap().get_operands()
                    .any(|operand| operand.and_then(|operand| operand.block()) == outer.get_parent()));
                if let Some(out) = std::env::var_os("WOSY_ORDINARY_LIVE_OWNER_IR_DIR") {
                    let target = if layout == ScalarTargetLayout::WASM32 { "wasm32" } else { "native64" };
                    let route = if project { "project" } else { "standalone" };
                    let controlled = llvm.replacen("define i32 @main(", "define i32 @wosy_authored_main(", 1);
                    assert_ne!(controlled, llvm);
                    std::fs::write(std::path::PathBuf::from(out).join(witness).join(format!("{target}-{route}-control.ll")), controlled).unwrap();
                }
            }
        }
    }
}

#[test]
fn guarded_fallible_struct_fields_share_source_order_and_one_absence() {
    use inkwell::{memory_buffer::MemoryBuffer, values::CallSiteValue};
    let text = "%%start
struct Pair { u64 first; u64 second; u64 tag; }
u64(u64) _read_from_buffer = fn(count) { u16[count] scratch; core.array_len(&scratch) };
u64(u64) _read_next = fn(count) { u8[count] scratch; core.array_len(&scratch) };
u64(u64, u64) observed = fn(first, second) {
    Pair result =? {
        .second = _read_from_buffer(first);
        .first = _read_next(second) ^ 0;
        .tag = 7;
    };
    if (&result == null) { 99 } else { result.first ^ result.second }
};
if (observed(7, 9) != 99 || observed(7, 9) != 99 || observed(7, 9) != 99
    || observed(0, 0) != 0 || observed(7, 9) != 14) { core.system_panic(); };
%%end";
    for layout in [ScalarTargetLayout::NATIVE64, ScalarTargetLayout::WASM32] {
        let prepared = prepare_initializer_test(text, layout);
        let checked = analyze_scalar_build(&prepared);
        assert!(checked.diagnostics().is_empty(), "{:?}", checked.diagnostics());
        let source = &prepared.project().modules[0].source;
        for (project, output) in [(false, emit_scalar_build_single_module_llvm(&checked)),
            (true, emit_scalar_build_project_llvm(&checked))]
        {
            let llvm = output.unwrap().to_text();
            let context = Context::create();
            let buffer = MemoryBuffer::create_from_memory_range_copy(llvm.as_bytes(), "fallible struct field order");
            let module = context.create_module_from_ir(buffer).unwrap();
            module.verify().unwrap();
            let authored = |name: &str| module.get_function(&self_tail_test_symbol(source, project, name)).unwrap();
            let observed = authored("observed");
            assert_eq!(observed.get_type().get_return_type(), Some(context.i64_type().into()));
            let blocks = observed.get_basic_blocks();
            let calls_to = |callee| blocks.iter().flat_map(|block| std::iter::successors(
                block.get_first_instruction(), |instruction| instruction.get_next_instruction()))
                .filter(|&instruction| CallSiteValue::try_from(instruction).ok()
                    .is_some_and(|call| call.get_called_fn_value() == Some(callee))).collect::<Vec<_>>();
            let [first] = calls_to(authored("_read_from_buffer"))[..] else { panic!("{llvm}") };
            let [second] = calls_to(authored("_read_next"))[..] else { panic!("{llvm}") };
            let [destination] = calls_to(module.get_function("__wosy_core_alloc").unwrap())[..] else { panic!("{llvm}") };
            assert_eq!(calls_to(module.get_function("__wosy_core_free").unwrap()).len(), 1);
            let absence = *blocks.iter().find(|block| block.get_name().to_bytes() == b"fallible.field_absence").unwrap();
            let enters = |instruction: InstructionValue<'_>, block| instruction.get_operands()
                .any(|operand| operand.and_then(|operand| operand.block()) == Some(block));
            let first_branch = first.get_parent().unwrap().get_terminator().unwrap();
            let second_branch = second.get_parent().unwrap().get_terminator().unwrap();
            assert!(enters(first_branch, second.get_parent().unwrap()) && enters(first_branch, absence));
            assert!(enters(second_branch, destination.get_parent().unwrap()) && enters(second_branch, absence));
            let continuation = *blocks.iter().find(|block| block.get_name().to_bytes() == b"fallible.continue").unwrap();
            assert_eq!(absence.get_first_instruction(), absence.get_terminator());
            assert!(enters(absence.get_terminator().unwrap(), continuation));
            let initialization = *blocks.iter().find(|block| block.get_name().to_bytes() == b"fallible.initialize").unwrap();
            assert_eq!(std::iter::successors(initialization.get_first_instruction(), |i| i.get_next_instruction())
                .filter(|i| i.get_opcode() == InstructionOpcode::Store).count(), 3);
            let phi = inkwell::values::PhiValue::try_from(continuation.get_first_instruction().unwrap()).unwrap();
            assert_eq!(phi.count_incoming(), 3);
            assert!((0..3).filter_map(|index| phi.get_incoming(index)).any(|(value, block)|
                block == absence && value.into_pointer_value().is_null()));
            if let Some(out) = std::env::var_os("WOSY_FALLIBLE_STRUCT_FIELDS_IR_DIR") {
                let target = if layout == ScalarTargetLayout::WASM32 { "wasm32" } else { "native64" };
                let route = if project { "project" } else { "standalone" };
                let controlled = llvm.replacen("define i32 @main(", "define i32 @wosy_authored_main(", 1);
                assert_ne!(controlled, llvm);
                std::fs::write(std::path::PathBuf::from(out).join(format!("{target}-{route}-control.ll")), controlled).unwrap();
            }
        }
    }
}

#[test]
fn guarded_fallible_bool_distinguishes_false_and_absence_in_selected_paths() {
    use inkwell::{memory_buffer::MemoryBuffer, values::CallSiteValue};
    let text = "%%start
bool(u64) _empty = fn(count) { u8[count] scratch; core.array_len(&scratch) == 0 };
u64(u64) _then_value = fn(count) { u16[count] scratch; core.array_len(&scratch) };
u64(u64) _else_value = fn(count) { u32[count] scratch; core.array_len(&scratch) };
u64(u64, u64) both_empty = fn(first, second) {
    bool result =? _empty(first) && _empty(second);
    if (&result == null) { 99 } else { if (result) { 1 } else { 0 } }
};
u64(u64, u64, u64) selected = fn(first, second, count) {
    u64 result =? if (_empty(first) || _empty(second)) { _then_value(count) } else { _else_value(count) };
    if (&result == null) { 99 } else { result }
};
if (both_empty(7, 9) != 0 || both_empty(0, 9) != 0 || both_empty(0, 0) != 1
    || both_empty(11, 9) != 99 || both_empty(0, 13) != 99
    || selected(0, 9, 7) != 7 || selected(7, 0, 7) != 7 || selected(7, 9, 7) != 7
    || selected(11, 9, 7) != 99 || selected(7, 13, 7) != 99 || selected(0, 9, 15) != 99) {
    core.system_panic();
};
%%end";
    for layout in [ScalarTargetLayout::NATIVE64, ScalarTargetLayout::WASM32] {
        let prepared = prepare_initializer_test(text, layout);
        let checked = analyze_scalar_build(&prepared);
        assert!(checked.diagnostics().is_empty(), "{:?}", checked.diagnostics());
        let source = &prepared.project().modules[0].source;
        for (project, output) in [(false, emit_scalar_build_single_module_llvm(&checked)),
            (true, emit_scalar_build_project_llvm(&checked))]
        {
            let llvm = output.unwrap().to_text();
            let context = Context::create();
            let buffer = MemoryBuffer::create_from_memory_range_copy(llvm.as_bytes(), "fallible Bool short circuit");
            let module = context.create_module_from_ir(buffer).unwrap();
            module.verify().unwrap();
            let authored = |name: &str| module.get_function(&self_tail_test_symbol(source, project, name)).unwrap();
            let empty = authored("_empty");
            let pair = context.struct_type(&[context.bool_type().into(), context.bool_type().into()], false);
            assert_eq!(empty.get_linkage(), inkwell::module::Linkage::Internal);
            assert_eq!(empty.get_type(), pair.fn_type(&[context.i64_type().into()], false));
            for (caller, arms) in [(authored("both_empty"), false), (authored("selected"), true)] {
                assert_eq!(caller.get_type().get_return_type(), Some(context.i64_type().into()));
                let blocks = caller.get_basic_blocks();
                let calls_to = |callee| blocks.iter().flat_map(|block| std::iter::successors(
                    block.get_first_instruction(), |instruction| instruction.get_next_instruction()))
                    .filter(|&instruction| CallSiteValue::try_from(instruction).ok()
                        .is_some_and(|call| call.get_called_fn_value() == Some(callee))).collect::<Vec<_>>();
                let absence = *blocks.iter().find(|block| block.get_name().to_bytes() == b"fallible.binary_merge").unwrap();
                let continuation = *blocks.iter().find(|block| block.get_name().to_bytes() == b"short_circuit.merge").unwrap();
                let [first, second] = calls_to(empty)[..] else { panic!("{llvm}") };
                assert_eq!(second.get_parent().unwrap().get_name().to_bytes(), b"short_circuit.rhs");
                let phi = inkwell::values::PhiValue::try_from(continuation.get_first_instruction().unwrap()).unwrap();
                assert_eq!(phi.count_incoming(), 2);
                assert!((0..2).filter_map(|index| phi.get_incoming(index)).any(|(value, _)|
                    value.into_int_value().get_zero_extended_constant() == Some(u64::from(arms))));
                let reaches_absence = |instruction: InstructionValue<'_>| instruction.get_parent()
                    .and_then(|block| block.get_terminator()).unwrap().get_operands()
                    .any(|operand| operand.and_then(|operand| operand.block()) == Some(absence));
                assert!(reaches_absence(first) && reaches_absence(second));
                let [destination] = calls_to(module.get_function("__wosy_core_alloc").unwrap())[..] else { panic!("{llvm}") };
                assert_eq!(destination.get_parent().unwrap().get_name().to_bytes(), b"fallible.destination");
                assert_eq!(calls_to(module.get_function("__wosy_core_free").unwrap()).len(), 1);
                if arms {
                    for (name, arm) in [("_then_value", b"if.then".as_slice()), ("_else_value", b"if.else".as_slice())] {
                        let [call] = calls_to(authored(name))[..] else { panic!("{llvm}") };
                        assert_eq!(call.get_parent().unwrap().get_name().to_bytes(), arm);
                        assert!(reaches_absence(call));
                    }
                }
            }
            if let Some(out) = std::env::var_os("WOSY_FALLIBLE_BOOL_IR_DIR") {
                let target = if layout == ScalarTargetLayout::WASM32 { "wasm32" } else { "native64" };
                let route = if project { "project" } else { "standalone" };
                let controlled = llvm.replacen("define i32 @main(", "define i32 @wosy_authored_main(", 1);
                assert_ne!(controlled, llvm);
                std::fs::write(std::path::PathBuf::from(out).join(format!("{target}-{route}-control.ll")), controlled).unwrap();
            }
        }
    }
}

#[test]
fn ordinary_fallible_struct_fields_preserve_existing_storage_until_success() {
    use inkwell::{memory_buffer::MemoryBuffer, values::CallSiteValue};
    let template = "%%start
struct copy Pair { u64 first; u64 second; u64 tag; }
u64(u64) _read_from_buffer = fn(count) { u16[count] scratch; core.array_len(&scratch) };
u64(u64) _read_next = fn(count) { u8[count] scratch; core.array_len(&scratch) };
u64(u64, u64) guarded = fn(first, second) {
    Pair result =? { .second = _read_from_buffer(first); .first = _read_next(second); .tag = 7; };
    if (&result == null) { 99 } else { result.first ^ result.second }
};
u64(u64, u64) construct = fn(first, second) {
    Pair result = { .second = _read_from_buffer(first); .first = _read_next(second) ^ 0; .tag = 7; };
    result.first ^ result.second
};
u64(u64, u64) replace = fn(first, second) {
    Pair result =? { .second = 42; .first = 41; .tag = 43; };
    if (&result != null) {
        result = { .second = _read_from_buffer(first); .first = _read_next(second) ^ 0; .tag = 7; };
        result.first ^ result.second
    } else { 99 }
};
if (guarded(7, 9) != 14 || construct(0, 0) != 0 || replace(7, 9) != 14 || replace(FIRST, SECOND) != 0) {
    core.system_panic();
};
%%end";
    for (witness, first, second) in [("first", "11", "9"), ("middle", "13", "13")] {
        let text = template.replace("FIRST", first).replace("SECOND", second);
        for layout in [ScalarTargetLayout::NATIVE64, ScalarTargetLayout::WASM32] {
            let prepared = prepare_initializer_test(&text, layout);
            let checked = analyze_scalar_build(&prepared);
            assert!(checked.diagnostics().is_empty(), "{:?}", checked.diagnostics());
            let source = &prepared.project().modules[0].source;
            for (project, output) in [(false, emit_scalar_build_single_module_llvm(&checked)),
                (true, emit_scalar_build_project_llvm(&checked))]
            {
                let llvm = output.unwrap().to_text();
                let context = Context::create();
                let buffer = MemoryBuffer::create_from_memory_range_copy(llvm.as_bytes(), "ordinary fallible struct fields");
                let module = context.create_module_from_ir(buffer).unwrap();
                module.verify().unwrap();
                let authored = |name: &str| module.get_function(&self_tail_test_symbol(source, project, name)).unwrap();
                for (caller, assignment) in [(authored("construct"), false), (authored("replace"), true)] {
                    assert_eq!(caller.get_type().get_return_type(), Some(context.i64_type().into()));
                    let blocks = caller.get_basic_blocks();
                    let calls_to = |callee| blocks.iter().flat_map(|block| std::iter::successors(
                        block.get_first_instruction(), |instruction| instruction.get_next_instruction()))
                        .filter(|&instruction| CallSiteValue::try_from(instruction).ok()
                            .is_some_and(|call| call.get_called_fn_value() == Some(callee))).collect::<Vec<_>>();
                    let [panic] = calls_to(module.get_function("__wosy_core_system_panic").unwrap())[..] else { panic!("{llvm}") };
                    let failure = panic.get_parent().unwrap();
                    assert_eq!(failure.get_name().to_bytes(), b"ordinary.struct_allocation_absent");
                    assert_eq!(failure.get_first_instruction(), Some(panic));
                    assert_eq!(failure.get_terminator().unwrap().get_opcode(), InstructionOpcode::Unreachable);
                    let [first] = calls_to(authored("_read_from_buffer"))[..] else { panic!("{llvm}") };
                    let [second] = calls_to(authored("_read_next"))[..] else { panic!("{llvm}") };
                    let enters = |instruction: InstructionValue<'_>, block| instruction.get_operands()
                        .any(|operand| operand.and_then(|operand| operand.block()) == Some(block));
                    let first_branch = first.get_parent().unwrap().get_terminator().unwrap();
                    let second_branch = second.get_parent().unwrap().get_terminator().unwrap();
                    assert!(enters(first_branch, second.get_parent().unwrap()));
                    assert!(enters(first_branch, failure) && enters(second_branch, failure));
                    assert_eq!(calls_to(module.get_function("__wosy_core_alloc").unwrap()).len(), usize::from(assignment));
                    assert_eq!(calls_to(module.get_function("__wosy_core_free").unwrap()).len(), usize::from(assignment));
                }
                if let Some(out) = std::env::var_os("WOSY_ORDINARY_STRUCT_FIELDS_IR_DIR") {
                    let target = if layout == ScalarTargetLayout::WASM32 { "wasm32" } else { "native64" };
                    let route = if project { "project" } else { "standalone" };
                    let controlled = llvm.replacen("define i32 @main(", "define i32 @wosy_authored_main(", 1);
                    assert_ne!(controlled, llvm);
                    std::fs::write(std::path::PathBuf::from(out).join(witness).join(format!("{target}-{route}-control.ll")), controlled).unwrap();
                }
            }
        }
    }
}

#[test]
fn ordinary_fallible_array_elements_stage_before_replacing_prior_storage() {
    use inkwell::{memory_buffer::MemoryBuffer, values::CallSiteValue};
    let template = "%%start
struct Pair { u64 first; u64 second; u64 tag; }
u64(u64) _read_from_buffer = fn(count) { u16[count] scratch; core.array_len(&scratch) };
u64(u64) _read_next = fn(count) { u8[count] scratch; core.array_len(&scratch) };
u64(u64, u64) guarded = fn(first, second) {
    Pair result =? { .second = _read_from_buffer(first); .first = _read_next(second); .tag = 7; };
    if (&result == null) { 99 } else { result.first ^ result.second }
};
u64(u64, u64) construct = fn(first, second) {
    u64[2] result = [_read_from_buffer(first), _read_next(second) ^ 0];
    result[0] ^ result[1]
};
u64(u64, u64) replace = fn(first, second) {
    u8[first] prior;
    u64[2] result = [41, 42];
    u64 before = result[0] ^ result[1];
    result = [_read_from_buffer(first), _read_next(second) ^ 0];
    core.array_len(&prior) ^ first ^ before ^ 3 ^ result[0] ^ result[1]
};
if (guarded(7, 9) != 14 || construct(0, 0) != 0 || replace(7, 9) != 14 || replace(FIRST, SECOND) != 0) {
    core.system_panic();
};
%%end";
    for (witness, first, second) in [("first", "11", "9"), ("middle", "13", "13")] {
        let text = template.replace("FIRST", first).replace("SECOND", second);
        for layout in [ScalarTargetLayout::NATIVE64, ScalarTargetLayout::WASM32] {
            let prepared = prepare_initializer_test(&text, layout);
            let checked = analyze_scalar_build(&prepared);
            assert!(checked.diagnostics().is_empty(), "{:?}", checked.diagnostics());
            let source = &prepared.project().modules[0].source;
            for (project, output) in [(false, emit_scalar_build_single_module_llvm(&checked)),
                (true, emit_scalar_build_project_llvm(&checked))]
            {
                let llvm = output.unwrap().to_text();
                let context = Context::create();
                let buffer = MemoryBuffer::create_from_memory_range_copy(llvm.as_bytes(), "ordinary fallible array elements");
                let module = context.create_module_from_ir(buffer).unwrap();
                module.verify().unwrap();
                let authored = |name: &str| module.get_function(&self_tail_test_symbol(source, project, name)).unwrap();
                for (caller, assignment) in [(authored("construct"), false), (authored("replace"), true)] {
                    assert_eq!(caller.get_type().get_return_type(), Some(context.i64_type().into()));
                    let blocks = caller.get_basic_blocks();
                    let calls_to = |callee| blocks.iter().flat_map(|block| std::iter::successors(
                        block.get_first_instruction(), |instruction| instruction.get_next_instruction()))
                        .filter(|&instruction| CallSiteValue::try_from(instruction).ok()
                            .is_some_and(|call| call.get_called_fn_value() == Some(callee))).collect::<Vec<_>>();
                    let [first] = calls_to(authored("_read_from_buffer"))[..] else { panic!("{llvm}") };
                    let [second] = calls_to(authored("_read_next"))[..] else { panic!("{llvm}") };
                    let failure = *blocks.iter().find(|block| block.get_name().to_bytes() == b"ordinary.array_allocation_absent").unwrap();
                    let panic = module.get_function("__wosy_core_system_panic").unwrap();
                    let fail_call = failure.get_first_instruction().unwrap();
                    assert_eq!(CallSiteValue::try_from(fail_call).unwrap().get_called_fn_value(), Some(panic));
                    assert_eq!(fail_call.get_next_instruction(), failure.get_terminator());
                    assert_eq!(failure.get_terminator().unwrap().get_opcode(), InstructionOpcode::Unreachable);
                    fn edges<'ctx>(instruction: InstructionValue<'ctx>) -> Vec<inkwell::basic_block::BasicBlock<'ctx>> {
                        instruction.get_operands()
                            .filter_map(|operand| operand.and_then(|operand| operand.block())).collect()
                    }
                    let first_branch = first.get_parent().unwrap().get_terminator().unwrap();
                    let second_block = second.get_parent().unwrap();
                    assert!(edges(first_branch).contains(&second_block) && edges(first_branch).contains(&failure));
                    assert!(!std::iter::successors(second_block.get_first_instruction(), |i| i.get_next_instruction())
                        .any(|i| i.get_opcode() == InstructionOpcode::Store));
                    let second_successors = edges(second_block.get_terminator().unwrap());
                    assert!(second_successors.contains(&failure));
                    let success = *second_successors.iter().find(|&&block| block != failure).unwrap();
                    let success_instructions = std::iter::successors(success.get_first_instruction(), |i| i.get_next_instruction())
                        .collect::<Vec<_>>();
                    assert_eq!(success_instructions.iter().filter(|i| i.get_opcode() == InstructionOpcode::Store).count(), 2);
                    assert!(success_instructions.into_iter().any(|i|
                        CallSiteValue::try_from(i).ok().and_then(|call| call.get_called_fn_value())
                            .is_some_and(|callee| callee.get_name().to_bytes().starts_with(b"llvm.memmove."))));
                    assert_eq!(calls_to(module.get_function("__wosy_core_alloc").unwrap()).len(), usize::from(assignment));
                    assert_eq!(calls_to(module.get_function("__wosy_core_free").unwrap()).len(), usize::from(assignment));
                }
                if let Some(out) = std::env::var_os("WOSY_ORDINARY_ARRAY_ELEMENTS_IR_DIR") {
                    let target = if layout == ScalarTargetLayout::WASM32 { "wasm32" } else { "native64" };
                    let route = if project { "project" } else { "standalone" };
                    let controlled = llvm.replacen("define i32 @main(", "define i32 @wosy_authored_main(", 1);
                    assert_ne!(controlled, llvm);
                    std::fs::write(std::path::PathBuf::from(out).join(witness).join(format!("{target}-{route}-control.ll")), controlled).unwrap();
                }
            }
        }
    }
}

#[test]
fn guarded_fallible_fixed_array_requires_presence_and_exact_literal_index() {
    use inkwell::{memory_buffer::MemoryBuffer, values::CallSiteValue};
    let text = "%%start
u64(u64) _read_from_buffer = fn(count) { u16[count] scratch; core.array_len(&scratch) };
u64(u64) _read_next = fn(count) { u8[count] scratch; core.array_len(&scratch) };
u64(u64, u64) observed = fn(first, second) {
    u64[2] held =? [_read_from_buffer(first), _read_next(second) ^ 0];
    if (&held == null) { 99 } else { held[0] ^ held[1] }
};
if (observed(7, 9) != 99 || observed(7, 9) != 99 || observed(7, 9) != 99
    || observed(0, 0) != 0 || observed(7, 9) != 14) { core.system_panic(); };
%%end";
    let unavailable = text.replace("if (&held == null)", "held[0]; if (&held == null)");
    let out_of_bounds = text.replace("held[0] ^ held[1]", "held[2] ^ held[1]");
    let different_owner = text.replace("if (&held == null)", "u64[2] other =? [0,0]; if (&other == null)");
    for layout in [ScalarTargetLayout::NATIVE64, ScalarTargetLayout::WASM32] {
        for (case, message) in [
            (unavailable.as_str(), "fallible allocation is not known to be available"),
            (out_of_bounds.as_str(), "fallible allocation access is not a supported"),
            (different_owner.as_str(), "fallible allocation is not known to be available"),
        ] {
            let prepared = prepare_initializer_test(case, layout);
            let checked = analyze_scalar_build(&prepared);
            assert!(checked.diagnostics().iter().any(|diagnostic| diagnostic.message.contains(message)), "{:?}", checked.diagnostics());
        }
        let prepared = prepare_initializer_test(text, layout);
        let checked = analyze_scalar_build(&prepared);
        assert!(checked.diagnostics().is_empty(), "{:?}", checked.diagnostics());
        let source = &prepared.project().modules[0].source;
        for (project, output) in [(false, emit_scalar_build_single_module_llvm(&checked)),
            (true, emit_scalar_build_project_llvm(&checked))]
        {
            let llvm = output.unwrap().to_text();
            let context = Context::create();
            let buffer = MemoryBuffer::create_from_memory_range_copy(llvm.as_bytes(), "guarded fixed array elements");
            let module = context.create_module_from_ir(buffer).unwrap();
            module.verify().unwrap();
            let authored = |name: &str| module.get_function(&self_tail_test_symbol(source, project, name)).unwrap();
            let observed = authored("observed");
            let blocks = observed.get_basic_blocks();
            let calls_to = |callee| blocks.iter().flat_map(|block| std::iter::successors(
                block.get_first_instruction(), |instruction| instruction.get_next_instruction()))
                .filter(|&instruction| CallSiteValue::try_from(instruction).ok()
                    .is_some_and(|call| call.get_called_fn_value() == Some(callee))).collect::<Vec<_>>();
            let [first] = calls_to(authored("_read_from_buffer"))[..] else { panic!("{llvm}") };
            let [second] = calls_to(authored("_read_next"))[..] else { panic!("{llvm}") };
            let [destination] = calls_to(module.get_function("__wosy_core_alloc").unwrap())[..] else { panic!("{llvm}") };
            let [release] = calls_to(module.get_function("__wosy_core_free").unwrap())[..] else { panic!("{llvm}") };
            let absent = *blocks.iter().find(|block| block.get_name().to_bytes() == b"fallible.field_absence").unwrap();
            let enters = |instruction: InstructionValue<'_>, block| instruction.get_operands()
                .any(|operand| operand.and_then(|operand| operand.block()) == Some(block));
            let first_branch = first.get_parent().unwrap().get_terminator().unwrap();
            let second_branch = second.get_parent().unwrap().get_terminator().unwrap();
            assert!(enters(first_branch, second.get_parent().unwrap()) && enters(first_branch, absent));
            assert!(enters(second_branch, destination.get_parent().unwrap()) && enters(second_branch, absent));
            let initialize = *blocks.iter().find(|block| block.get_name().to_bytes() == b"fallible.initialize").unwrap();
            assert_eq!(std::iter::successors(initialize.get_first_instruction(), |i| i.get_next_instruction())
                .filter(|i| i.get_opcode() == InstructionOpcode::Store).count(), 2);
            let continuation = *blocks.iter().find(|block| block.get_name().to_bytes() == b"fallible.continue").unwrap();
            let address = continuation.get_first_instruction().unwrap();
            let phi = inkwell::values::PhiValue::try_from(address).unwrap();
            assert_eq!(phi.count_incoming(), 3);
            assert_eq!(known_field_operand(release, 0).unwrap().as_instruction_value(), Some(address));
            assert!((0..3).filter_map(|index| phi.get_incoming(index)).any(|(value, block)|
                block == absent && value.into_pointer_value().is_null()));
            if let Some(out) = std::env::var_os("WOSY_GUARDED_FIXED_ARRAY_IR_DIR") {
                let target = if layout == ScalarTargetLayout::WASM32 { "wasm32" } else { "native64" };
                let route = if project { "project" } else { "standalone" };
                let controlled = llvm.replacen("define i32 @main(", "define i32 @wosy_authored_main(", 1);
                assert_ne!(controlled, llvm);
                std::fs::write(std::path::PathBuf::from(out).join(format!("{target}-{route}-control.ll")), controlled).unwrap();
            }
        }
    }
}

#[test]
fn guarded_fallible_dynamic_index_preserves_presence_across_join_and_checks_extent() {
    use inkwell::{memory_buffer::MemoryBuffer, values::CallSiteValue};
    let text = "%%start
u64(u64, u64) _choose = fn(size, index) {
    u8[size] scratch;
    core.array_len(&scratch) ^ size ^ index
};
u64(bool, u64) observed = fn(choose, index) {
    u64[2] held =? [41, 42];
    if (&held == null) { 99 } else {
        u64 selected = if (choose) { _choose(3, index) } else { index };
        held[selected]
    }
};
if (observed(true, 1) != 99 || observed(false, 0) != 41 || observed(true, 1) != 42
    || observed(true, 18446744073709551615) != 0) { core.system_panic(); };
%%end";
    let unavailable = text.replace("    if (&held == null)", "    u64 prior = if (&held == null)")
        .replace("        held[selected]\n    }", "        held[selected]\n    };\n    prior ^ held[index]");
    for layout in [ScalarTargetLayout::NATIVE64, ScalarTargetLayout::WASM32] {
        let rejected = prepare_initializer_test(&unavailable, layout);
        let rejected = analyze_scalar_build(&rejected);
        assert!(rejected.diagnostics().iter().any(|diagnostic|
            diagnostic.message.contains("fallible allocation is not known to be available")), "{:?}", rejected.diagnostics());
        let prepared = prepare_initializer_test(text, layout);
        let source = prepared.project().modules[0].source.clone();
        let parsed = parse_source(source.clone(), text.into(), &[]);
        assert!(parsed.diagnostics.is_empty(), "{:?}", parsed.diagnostics);
        let release = crate::prepare_scalar_project_with_access(
            vec![(derive_scalar_declarations_from_cst(parsed.result.canonical_cst()), Vec::new())],
            vec![source.clone()], ScalarPreparationInputs { target_layout: layout },
            Some(wosy_stdlib_source::CheckedAccessComponent::linked()),
        ).unwrap_or_else(|error| panic!("{:?}", error.diagnostics));
        let checked = analyze_scalar_build(&prepared);
        let release_checked = analyze_scalar_build(&release);
        assert!(checked.diagnostics().is_empty(), "{:?}", checked.diagnostics());
        assert!(release_checked.diagnostics().is_empty(), "{:?}", release_checked.diagnostics());
        for (route, project, release_source, output) in [
            ("standalone", false, None, emit_scalar_build_single_module_llvm(&checked)),
            ("project", true, None, emit_scalar_build_project_llvm(&checked)),
            ("release-project", true, release.checked_access_source(), emit_scalar_build_project_llvm(&release_checked)),
        ] {
            let llvm = output.unwrap().to_text();
            let context = Context::create();
            let buffer = MemoryBuffer::create_from_memory_range_copy(llvm.as_bytes(), "guarded dynamic fixed array");
            let module = context.create_module_from_ir(buffer).unwrap();
            module.verify().unwrap();
            let observed = module.get_function(&self_tail_test_symbol(&source, project, "observed")).unwrap();
            let choose = module.get_function(&self_tail_test_symbol(&source, project, "_choose")).unwrap();
            let blocks = observed.get_basic_blocks();
            fn instructions<'ctx>(block: inkwell::basic_block::BasicBlock<'ctx>) -> impl Iterator<Item = InstructionValue<'ctx>> {
                std::iter::successors(block.get_first_instruction(), |instruction| instruction.get_next_instruction())
            }
            let calls_to = |callee| blocks.iter().copied().flat_map(instructions)
                .filter(|&instruction| CallSiteValue::try_from(instruction).ok()
                    .is_some_and(|call| call.get_called_fn_value() == Some(callee))).collect::<Vec<_>>();
            let [selection] = calls_to(choose)[..] else { panic!("{llvm}") };
            assert!(selection.get_parent().unwrap().get_name().to_bytes().starts_with(b"if.then"));
            assert_eq!(calls_to(module.get_function("__wosy_core_alloc").unwrap()).len(), 1);
            assert_eq!(calls_to(module.get_function("__wosy_core_free").unwrap()).len(), 1);
            if let Some(helper_source) = release_source {
                let helper = module.get_function(&project_function_name(helper_source, "check_index_in_extent")).unwrap();
                let [bounds] = calls_to(helper)[..] else { panic!("{llvm}") };
                let extent = known_field_operand(bounds, 1).unwrap().into_int_value();
                assert_eq!(extent.get_zero_extended_constant(), Some(2));
                assert!(std::iter::successors(bounds.get_next_instruction(), |instruction| instruction.get_next_instruction())
                    .any(|instruction| instruction.get_opcode() == InstructionOpcode::GetElementPtr));
            } else {
                let check = blocks.iter().copied().flat_map(instructions).find(|i|
                    i.get_name().is_some_and(|name| name.to_bytes() == b"guarded_index_out_of_bounds")).unwrap();
                assert_eq!(check.get_icmp_predicate(), Some(inkwell::IntPredicate::UGE));
                assert_eq!(known_field_operand(check, 1).unwrap().into_int_value().get_zero_extended_constant(), Some(2));
                let check_block = check.get_parent().unwrap();
                assert!(!instructions(check_block).any(|i| i.get_opcode() == InstructionOpcode::GetElementPtr));
                let safe = *blocks.iter().find(|block| block.get_name().to_bytes() == b"guarded_index_continue").unwrap();
                assert!(instructions(safe).any(|i| i.get_opcode() == InstructionOpcode::GetElementPtr));
            }
            if let Some(out) = std::env::var_os("WOSY_GUARDED_DYNAMIC_INDEX_IR_DIR") {
                let target = if layout == ScalarTargetLayout::WASM32 { "wasm32" } else { "native64" };
                let controlled = llvm.replacen("define i32 @main(", "define i32 @wosy_authored_main(", 1);
                assert_ne!(controlled, llvm);
                std::fs::write(std::path::PathBuf::from(out).join(format!("{target}-{route}-control.ll")), controlled).unwrap();
            }
        }
    }
}

#[test]
fn guarded_fallible_indexed_store_checks_presence_and_orders_rhs_before_place() {
    use inkwell::{memory_buffer::MemoryBuffer, values::CallSiteValue};
    let text = "%%start
u64(u64, u64) _value = fn(size, value) {
    u16[size] scratch;
    core.array_len(&scratch) ^ size ^ value
};
u64(u64, u64) _index = fn(size, index) {
    u8[size] scratch;
    core.array_len(&scratch) ^ size ^ index
};
u64(u64) observed = fn(index) {
    u64[2] held =? [41, 42];
    if (&held == null) { 99 } else {
        held[_index(3, index)] = _value(5, 7);
        held[0] ^ held[1]
    }
};
if (observed(1) != 99 || observed(1) != 46 || observed(4294967297) != 0) { core.system_panic(); };
%%end";
    let unavailable = text.replace("    if (&held == null)", "    held[index] = 7;\n    if (&held == null)");
    let aliased = text.replace("        held[_index", "        *!u64 alias = &!held[0]; *alias;\n        held[_index");
    let whole = text.replace("        held[_index(3, index)] = _value(5, 7);",
        "        u64 selected = _index(3, index); held = [selected, _value(5, 7)];");
    for layout in [ScalarTargetLayout::NATIVE64, ScalarTargetLayout::WASM32] {
        for (source, reason) in [
            (unavailable.as_str(), "fallible allocation is not known to be available"),
            (aliased.as_str(), "fallible allocation access is not a supported"),
            (whole.as_str(), "fallible allocation access is not a supported"),
        ] {
            let prepared = prepare_initializer_test(source, layout);
            let checked = analyze_scalar_build(&prepared);
            assert!(checked.diagnostics().iter().any(|diagnostic| diagnostic.message.contains(reason)), "{:?}", checked.diagnostics());
        }
        let prepared = prepare_initializer_test(text, layout);
        let source = prepared.project().modules[0].source.clone();
        let parsed = parse_source(source.clone(), text.into(), &[]);
        let release = crate::prepare_scalar_project_with_access(
            vec![(derive_scalar_declarations_from_cst(parsed.result.canonical_cst()), Vec::new())],
            vec![source.clone()], ScalarPreparationInputs { target_layout: layout },
            Some(wosy_stdlib_source::CheckedAccessComponent::linked()),
        ).unwrap_or_else(|error| panic!("{:?}", error.diagnostics));
        let fallback = analyze_scalar_build(&prepared);
        let release_checked = analyze_scalar_build(&release);
        assert!(fallback.diagnostics().is_empty(), "{:?}", fallback.diagnostics());
        assert!(release_checked.diagnostics().is_empty(), "{:?}", release_checked.diagnostics());
        for (route, project, release_source, output) in [
            ("standalone", false, None, emit_scalar_build_single_module_llvm(&fallback)),
            ("project", true, None, emit_scalar_build_project_llvm(&fallback)),
            ("release-project", true, release.checked_access_source(), emit_scalar_build_project_llvm(&release_checked)),
        ] {
            let llvm = output.unwrap().to_text();
            let context = Context::create();
            let buffer = MemoryBuffer::create_from_memory_range_copy(llvm.as_bytes(), "guarded indexed store");
            let module = context.create_module_from_ir(buffer).unwrap();
            module.verify().unwrap();
            let authored = |name: &str| module.get_function(&self_tail_test_symbol(&source, project, name)).unwrap();
            let observed = authored("observed");
            let blocks = observed.get_basic_blocks();
            fn instructions<'ctx>(block: inkwell::basic_block::BasicBlock<'ctx>) -> impl Iterator<Item = InstructionValue<'ctx>> {
                std::iter::successors(block.get_first_instruction(), |instruction| instruction.get_next_instruction())
            }
            let calls_to = |callee| blocks.iter().copied().flat_map(instructions)
                .filter(|&instruction| CallSiteValue::try_from(instruction).ok()
                    .is_some_and(|call| call.get_called_fn_value() == Some(callee))).collect::<Vec<_>>();
            let [rhs] = calls_to(authored("_value"))[..] else { panic!("{llvm}") };
            let [selector] = calls_to(authored("_index"))[..] else { panic!("{llvm}") };
            assert!(std::iter::successors(rhs.get_next_instruction(), |instruction| instruction.get_next_instruction())
                .any(|instruction| instruction == selector));
            assert_eq!(calls_to(module.get_function("__wosy_core_alloc").unwrap()).len(), 1);
            assert_eq!(calls_to(module.get_function("__wosy_core_free").unwrap()).len(), 1);
            let is_operand = |instruction, index, expected| known_field_operand(instruction, index)
                .and_then(|value| value.as_instruction_value()) == Some(expected);
            let snapshot = instructions(rhs.get_parent().unwrap()).find(|instruction|
                instruction.get_opcode() == InstructionOpcode::Store && is_operand(*instruction, 0, rhs)).unwrap();
            let temporary = known_field_operand(snapshot, 1).unwrap().as_instruction_value().unwrap();
            assert_eq!(temporary.get_opcode(), InstructionOpcode::Alloca);
            let staged = std::iter::successors(snapshot.get_next_instruction(), |instruction| instruction.get_next_instruction())
                .take_while(|instruction| *instruction != selector)
                .find(|instruction| instruction.get_opcode() == InstructionOpcode::Load
                    && is_operand(*instruction, 0, temporary)).unwrap();
            let install = blocks.iter().copied().flat_map(instructions).find(|instruction|
                instruction.get_opcode() == InstructionOpcode::Store && is_operand(*instruction, 0, staged)).unwrap();
            let element = known_field_operand(install, 1).unwrap().as_instruction_value().unwrap();
            assert_eq!(element.get_opcode(), InstructionOpcode::GetElementPtr);
            assert_eq!(element.get_parent(), install.get_parent());
            if let Some(helper_source) = release_source {
                let helper = module.get_function(&project_function_name(helper_source, "check_index_in_extent")).unwrap();
                let [bounds] = calls_to(helper)[..] else { panic!("{llvm}") };
                assert_eq!(known_field_operand(bounds, 0).unwrap().as_instruction_value(), Some(selector));
                assert!(std::iter::successors(bounds.get_next_instruction(), |instruction| instruction.get_next_instruction())
                    .any(|instruction| instruction == install));
            } else {
                let check = blocks.iter().copied().flat_map(instructions).find(|i|
                    i.get_name().is_some_and(|name| name.to_bytes() == b"guarded_index_out_of_bounds")).unwrap();
                assert_eq!(check.get_icmp_predicate(), Some(inkwell::IntPredicate::UGE));
                assert_eq!(known_field_operand(check, 0).unwrap().as_instruction_value(), Some(selector));
                assert_eq!(known_field_operand(check, 1).unwrap().into_int_value().get_zero_extended_constant(), Some(2));
                assert_eq!(install.get_parent().unwrap().get_name().to_bytes(), b"guarded_index_continue");
            }
            if let Some(out) = std::env::var_os("WOSY_GUARDED_INDEXED_STORE_IR_DIR") {
                let target = if layout == ScalarTargetLayout::WASM32 { "wasm32" } else { "native64" };
                let controlled = llvm.replacen("define i32 @main(", "define i32 @wosy_authored_main(", 1);
                assert_ne!(controlled, llvm);
                std::fs::write(std::path::PathBuf::from(out).join(format!("{target}-{route}-control.ll")), controlled).unwrap();
            }
        }
    }
}

#[test]
fn guarded_fallible_indexed_shared_borrow_checks_presence_extent_and_last_use() {
    use inkwell::{memory_buffer::MemoryBuffer, values::CallSiteValue};
    let text = "%%start
u64(u64, u64) _choose = fn(size, index) {
    u8[size] scratch;
    core.array_len(&scratch) ^ size ^ index
};
u64(u64) observed = fn(index) {
    u64[2] held =? [41, 42];
    if (&held == null) { 99 } else {
        *u64 cell = &held[_choose(3, index)];
        u64 before = *cell;
        held[1] = 7;
        before ^ held[0] ^ held[1]
    }
};
if (observed(1) != 99 || observed(0) != 7 || observed(1) != 4
    || observed(4294967297) != 0) { core.system_panic(); };
%%end";
    let missing_presence = text.replace("    if (&held == null)", "    *u64 early = &held[index];\n    *early;\n    if (&held == null)");
    let active_loan = text.replace("        u64 before = *cell;\n        held[1] = 7;",
        "        held[1] = 7;\n        u64 before = *cell;");
    let alias_guard = text.replace("u64 before = *cell;", "u64 before = if (cell == null) { 0 } else { *cell };");
    for layout in [ScalarTargetLayout::NATIVE64, ScalarTargetLayout::WASM32] {
        for (case, reason) in [
            (missing_presence.as_str(), "fallible allocation is not known to be available"),
            (active_loan.as_str(), "read or move of unavailable place"),
            (alias_guard.as_str(), "fallible allocation access is not a supported"),
        ] {
            let prepared = prepare_initializer_test(case, layout);
            let checked = analyze_scalar_build(&prepared);
            let diagnostic = checked.diagnostics().iter().find(|diagnostic| diagnostic.message.contains(reason))
                .unwrap_or_else(|| panic!("{reason}: {:?}", checked.diagnostics()));
            if case == active_loan.as_str() {
                let borrow = case.find("&held[_choose").unwrap() as u32;
                assert!(diagnostic.labels.iter().any(|label|
                    label.kind == crate::DiagnosticLabelKind::Secondary
                    && label.span.range.start == borrow
                    && label.message == "origin of conflicting place or loan"));
            }
        }
        let prepared = prepare_initializer_test(text, layout);
        let source = prepared.project().modules[0].source.clone();
        let parsed = parse_source(source.clone(), text.into(), &[]);
        let release = crate::prepare_scalar_project_with_access(
            vec![(derive_scalar_declarations_from_cst(parsed.result.canonical_cst()), Vec::new())],
            vec![source.clone()], ScalarPreparationInputs { target_layout: layout },
            Some(wosy_stdlib_source::CheckedAccessComponent::linked()),
        ).unwrap_or_else(|error| panic!("{:?}", error.diagnostics));
        let fallback = analyze_scalar_build(&prepared);
        let release_checked = analyze_scalar_build(&release);
        assert!(fallback.diagnostics().is_empty(), "{:?}", fallback.diagnostics());
        assert!(release_checked.diagnostics().is_empty(), "{:?}", release_checked.diagnostics());
        for (route, project, release_source, output) in [
            ("standalone", false, None, emit_scalar_build_single_module_llvm(&fallback)),
            ("project", true, None, emit_scalar_build_project_llvm(&fallback)),
            ("release-project", true, release.checked_access_source(), emit_scalar_build_project_llvm(&release_checked)),
        ] {
            let llvm = output.unwrap().to_text();
            let context = Context::create();
            let buffer = MemoryBuffer::create_from_memory_range_copy(llvm.as_bytes(), "guarded shared indexed borrow");
            let module = context.create_module_from_ir(buffer).unwrap();
            module.verify().unwrap();
            let authored = |name: &str| module.get_function(&self_tail_test_symbol(&source, project, name)).unwrap();
            let observed = authored("observed");
            let blocks = observed.get_basic_blocks();
            fn instructions<'ctx>(block: inkwell::basic_block::BasicBlock<'ctx>) -> impl Iterator<Item = InstructionValue<'ctx>> {
                std::iter::successors(block.get_first_instruction(), |instruction| instruction.get_next_instruction())
            }
            let calls_to = |callee| blocks.iter().copied().flat_map(instructions)
                .filter(|&instruction| CallSiteValue::try_from(instruction).ok()
                    .is_some_and(|call| call.get_called_fn_value() == Some(callee))).collect::<Vec<_>>();
            let [selector] = calls_to(authored("_choose"))[..] else { panic!("{llvm}") };
            assert_eq!(selector.get_parent().unwrap().get_name().to_bytes(), b"if.else");
            assert_eq!(calls_to(module.get_function("__wosy_core_alloc").unwrap()).len(), 1);
            assert_eq!(calls_to(module.get_function("__wosy_core_free").unwrap()).len(), 1);
            let gep = blocks.iter().copied().flat_map(instructions).find(|instruction| {
                if instruction.get_opcode() != InstructionOpcode::GetElementPtr { return false; }
                let Some(index) = known_field_operand(*instruction, 2).and_then(|v| v.as_instruction_value()) else { return false; };
                index == selector || (index.get_opcode() == InstructionOpcode::Trunc
                    && known_field_operand(index, 0).and_then(|v| v.as_instruction_value()) == Some(selector))
            }).unwrap_or_else(|| panic!("{llvm}"));
            if let Some(helper_source) = release_source {
                let helper = module.get_function(&project_function_name(helper_source, "check_index_in_extent")).unwrap();
                let [bounds] = calls_to(helper)[..] else { panic!("{llvm}") };
                assert_eq!(known_field_operand(bounds, 0).unwrap().as_instruction_value(), Some(selector));
                assert_eq!(known_field_operand(bounds, 1).unwrap().into_int_value().get_zero_extended_constant(), Some(2));
                assert!(std::iter::successors(bounds.get_next_instruction(), |instruction| instruction.get_next_instruction())
                    .any(|instruction| instruction == gep));
            } else {
                let check = blocks.iter().copied().flat_map(instructions).find(|i|
                    i.get_name().is_some_and(|name| name.to_bytes() == b"guarded_index_out_of_bounds")).unwrap();
                assert_eq!(check.get_icmp_predicate(), Some(inkwell::IntPredicate::UGE));
                assert_eq!(known_field_operand(check, 0).unwrap().as_instruction_value(), Some(selector));
                assert_eq!(known_field_operand(check, 1).unwrap().into_int_value().get_zero_extended_constant(), Some(2));
                assert_eq!(gep.get_parent().unwrap().get_name().to_bytes(), b"guarded_index_continue");
            }
            if let Some(out) = std::env::var_os("WOSY_GUARDED_INDEXED_BORROW_IR_DIR") {
                let target = if layout == ScalarTargetLayout::WASM32 { "wasm32" } else { "native64" };
                let controlled = llvm.replacen("define i32 @main(", "define i32 @wosy_authored_main(", 1);
                assert_ne!(controlled, llvm);
                std::fs::write(std::path::PathBuf::from(out).join(format!("{target}-{route}-control.ll")), controlled).unwrap();
            }
        }
    }
}

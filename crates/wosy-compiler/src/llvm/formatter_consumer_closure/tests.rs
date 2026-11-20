use super::*;
use crate::{
    analyze_scalar_build, derive_scalar_declarations_from_cst, parse_source,
    prepare_scalar_project, ScalarNamespaceBinding, ScalarPreparationInputs, ScalarTargetLayout,
};

fn declarations(source: &SourceIdentity, text: &str) -> crate::ScalarSourceDeclarations {
    let parsed = parse_source(source.clone(), text.into(), &[]);
    assert!(parsed.diagnostics.is_empty(), "{:?}", parsed.diagnostics);
    derive_scalar_declarations_from_cst(parsed.result.canonical_cst())
}

fn namespace(binding: &str, target: &SourceIdentity) -> ScalarNamespaceBinding {
    ScalarNamespaceBinding {
        binding: binding.into(),
        target: target.clone(),
        span: ByteSpan::new(0, 0),
    }
}

fn function(module: &crate::ScalarModule, name: &str) -> usize {
    module
        .items
        .iter()
        .position(|item| matches!(item, ScalarItem::Function(fun) if fun.name == name))
        .unwrap()
}

fn final_call<'a>(body: &CallableBody<'a>) -> &'a ScalarExpression {
    body.entry
        .function
        .body
        .final_expressions()
        .next()
        .unwrap()
        .0
}

fn cfg_with(
    cfg: &ScalarReferenceCfg,
    edit: impl FnOnce(&mut [crate::scalar::ReferenceFlowPoint]),
) -> ScalarReferenceCfg {
    let mut points = cfg.points.clone();
    edit(&mut points);
    ScalarReferenceCfg {
        points,
        facts: cfg.facts.clone(),
        initial_points: cfg.initial_points.clone(),
        allocations: cfg.allocations.clone(),
        specialized_policies: cfg.specialized_policies.clone(),
        primary_call_points: cfg.primary_call_points.clone(),
    }
}

fn exercise_u8_nested_cfg_controls(
    ledger: &Ledger<'_>,
    caller: &CallableBody<'_>,
    nested: &nested_caller::U8NestedCallerOpen<'_>,
) {
    let body = |cfg| CallableBody {
        cfg,
        ..ledger.body(caller.entry.id.body, None).unwrap()
    };
    let argument_is_not_output = cfg_with(caller.cfg, |points| {
        let CfgPointKind::Call {
            argument_points, ..
        } = &mut points[nested.consuming.point.0].kind
        else {
            unreachable!()
        };
        argument_points[0] = nested.formatting.actuals[0].point;
    });
    assert!(matches!(
        ledger.u8_nested_caller_open(&body(&argument_is_not_output)),
        Err(Rejected::Contradictory("ordered actual slot"))
    ));
    let foreign_site = cfg_with(caller.cfg, |points| {
        points[nested.formatting.point.0]
            .source_span
            .source
            .revision
            .push_str("-foreign");
    });
    assert!(matches!(
        ledger.u8_nested_caller_open(&body(&foreign_site)),
        Err(Rejected::Contradictory("body call identity"))
    ));
    let wrong_scope = cfg_with(caller.cfg, |points| {
        points[nested.formatting.point.0].scope = ReferenceScopeId(1);
    });
    assert!(matches!(
        ledger.u8_nested_caller_open(&body(&wrong_scope)),
        Err(Rejected::Contradictory("print CFG point"))
    ));
    let reroute = cfg_with(caller.cfg, |points| {
        points[nested.formatting.point.0].successors = vec![nested.markers[1].point];
    });
    assert!(matches!(
        ledger.u8_nested_caller_open(&body(&reroute)),
        Err(Rejected::Contradictory("print CFG order"))
    ));
    let swapped_marker_slots = cfg_with(caller.cfg, |points| {
        let CfgPointKind::Call {
            argument_points, ..
        } = &mut points[nested.markers[1].point.0].kind
        else {
            unreachable!()
        };
        argument_points.swap(2, 3);
    });
    assert!(matches!(
        ledger.u8_nested_caller_open(&body(&swapped_marker_slots)),
        Err(Rejected::Contradictory("print CFG point"))
    ));
    let mut detached = cfg_with(caller.cfg, |_| {});
    let mut event = detached.points[nested.markers[0].point.0].clone();
    event.id = CfgPointId(detached.points.len());
    detached.points.push(event);
    assert!(matches!(
        ledger.u8_nested_caller_open(&body(&detached)),
        Err(Rejected::Contradictory("nested unclassified CFG effect"))
    ));
}

fn exercise_finite_print(
    ledger: &Ledger<'_>,
    print: &CallableBody<'_>,
    open: &print_finite::PrintFiniteOpen,
) {
    use crate::scalar::{ReferencePlaceProjection, ScalarFieldReference};
    let body = |cfg| CallableBody {
        cfg,
        ..ledger.body(print.entry.id.body, None).unwrap()
    };
    let root = &print.entry.function.body;
    let ScalarBlockItem::LocalBinding(vector) = &root.items[6] else {
        unreachable!()
    };
    let ScalarExpression::StructLiteral { fields, .. } = &vector.value else {
        unreachable!()
    };
    let data = &fields[0].value;
    assert!(print.nodes.lexical_member(&data.clone()).is_none());
    let ScalarBlockItem::LocalBinding(complete) = &root.items[2] else {
        unreachable!()
    };
    let ScalarExpression::Binary { left: length, .. } = &complete.value else {
        unreachable!()
    };
    let mut missing_member = ledger.body(print.entry.id.body, None).unwrap();
    assert!(missing_member
        .nodes
        .withhold_lexical_member_occurrence(length));
    assert!(matches!(
        ledger.print_finite_open(&missing_member),
        Err(Rejected::Missing("print lexical member"))
    ));
    let data_point = print.eval(data).unwrap();
    let wrong_member = cfg_with(print.cfg, |points| {
        let CfgPointKind::ReadPlace { candidates, .. } = &mut points[data_point.0].kind else {
            unreachable!()
        };
        let [ReferencePlaceProjection::Field(ScalarFieldReference::Resolved(field))] =
            candidates[0].projections.as_mut_slice()
        else {
            unreachable!()
        };
        field.index += 1;
    });
    assert!(matches!(
        ledger.print_finite_open(&body(&wrong_member)),
        Err(Rejected::Contradictory("print CFG point"))
    ));
    let reordered_branch = cfg_with(print.cfg, |points| {
        points
            .iter_mut()
            .find(|point| matches!(point.kind, CfgPointKind::Branch { .. }))
            .unwrap()
            .successors
            .swap(0, 1);
    });
    assert!(matches!(
        ledger.print_finite_open(&body(&reordered_branch)),
        Err(Rejected::Contradictory("print CFG order"))
    ));
    let wrong_wrapper_slots = cfg_with(print.cfg, |points| {
        let CfgPointKind::Call {
            argument_points, ..
        } = &mut points
            .iter_mut()
            .find(|point| {
                matches!(
                    point.kind,
                    CfgPointKind::Call {
                        target: ResolvedCallTarget::ModuleCallable { .. },
                        ..
                    }
                )
            })
            .unwrap()
            .kind
        else {
            unreachable!()
        };
        argument_points.swap(1, 2);
    });
    assert!(matches!(
        ledger.print_finite_open(&body(&wrong_wrapper_slots)),
        Err(Rejected::Contradictory("print CFG point"))
    ));
    let changed_address = cfg_with(print.cfg, |points| {
        let origins = points
            .iter_mut()
            .filter_map(|point| match &mut point.kind {
                CfgPointKind::Evaluate(expression @ ScalarExpression::RawAddress { .. }) => {
                    Some(expression)
                }
                _ => None,
            })
            .collect::<Vec<_>>();
        let [first, second]: [_; 2] = origins.try_into().unwrap();
        std::mem::swap(first, second);
    });
    assert!(matches!(
        ledger.print_finite_open(&body(&changed_address)),
        Err(Rejected::Contradictory("print CFG point"))
    ));
    let wrong_return = cfg_with(print.cfg, |points| {
        let CfgPointKind::ReturnOutput { output, .. } = &mut points
            .iter_mut()
            .find(|point| matches!(point.kind, CfgPointKind::ReturnOutput { output: 1, .. }))
            .unwrap()
            .kind
        else {
            unreachable!()
        };
        *output = 0;
    });
    assert!(matches!(
        ledger.print_finite_open(&body(&wrong_return)),
        Err(Rejected::Contradictory("print CFG point"))
    ));

    let wrapper = ledger.body(open.wrapper.body, None).unwrap();
    let altered = cfg_with(wrapper.cfg, |points| {
        let CfgPointKind::Call {
            target: ResolvedCallTarget::ExternCallable { source, .. },
            ..
        } = &mut points
            .iter_mut()
            .find(|point| matches!(point.kind, CfgPointKind::Call { .. }))
            .unwrap()
            .kind
        else {
            unreachable!()
        };
        source.revision.push_str("-untrusted");
    });
    let (parameters, _) = wrapper.entry.signature().unwrap();
    let [ScalarType::I32, ScalarType::RawPointer(vec), ScalarType::RawPointer(written)] =
        parameters
    else {
        unreachable!()
    };
    let wrap_body = |cfg| CallableBody {
        cfg,
        ..ledger.body(open.wrapper.body, None).unwrap()
    };
    assert!(matches!(
        ledger.print_wrapper_open(&wrap_body(&altered), vec, written),
        Err(Rejected::Contradictory("print CFG point"))
    ));
    let altered = cfg_with(wrapper.cfg, |points| {
        let CfgPointKind::Call {
            argument_points, ..
        } = &mut points
            .iter_mut()
            .find(|point| matches!(point.kind, CfgPointKind::Call { .. }))
            .unwrap()
            .kind
        else {
            unreachable!()
        };
        argument_points.swap(0, 2);
    });
    assert!(matches!(
        ledger.print_wrapper_open(&wrap_body(&altered), vec, written),
        Err(Rejected::Contradictory("print CFG point"))
    ));
    let mut extra = cfg_with(wrapper.cfg, |_| {});
    let mut discarded = extra.points.last().unwrap().clone();
    discarded.id = CfgPointId(extra.points.len());
    extra.points.push(discarded);
    assert!(matches!(
        ledger.print_wrapper_open(&wrap_body(&extra), vec, written),
        Err(Rejected::Contradictory("print unclassified CFG effect"))
    ));
}

fn rejected_assignment_fact_mutations(
    ledger: &Ledger<'_>,
    print: &CallableBody<'_>,
) -> Vec<&'static str> {
    let ScalarBlockItem::Expression(ScalarExpression::If { then_branch, .. }) =
        &print.entry.function.body.items[10]
    else {
        unreachable!();
    };
    let ScalarBlockItem::Expression(ScalarExpression::Block(block)) = &then_branch.items[0] else {
        unreachable!();
    };
    let [ScalarBlockItem::Assignment(assignment)] = block.items.as_slice() else {
        unreachable!();
    };
    let place = &assignment.targets[0].place;
    assert_eq!(
        print.nodes.checked_place_type(place),
        Some(&ScalarType::I32)
    );
    let mut failures = Vec::new();
    for (cause, ty) in [("missing", None), ("wrong U64", Some(&ScalarType::U64))] {
        let mut changed = ledger.body(print.entry.id.body, None).unwrap();
        changed.nodes.override_checked_place_type(place, ty);
        assert_eq!(changed.nodes.checked_place_type(place), ty);
        if !matches!(
            (cause, ledger.print_finite_open(&changed)),
            (
                "missing",
                Err(Rejected::Missing("print assignment destination type"))
            ) | (
                "wrong U64",
                Err(Rejected::Contradictory("print assignment destination type"))
            )
        ) {
            failures.push(cause);
        }
    }
    failures
}

#[test]
fn formatter_consumer_closure_canonical_stdlib_forwarding_and_distinct_open_regions() {
    let app_text = r#"%%start
std = namespace std "bootstrap.w";
std.utf8(u8) wrap = fn(value) { std.format(value) };
unit(std.utf8) inspect32 = fn(text) { *f32 parsed = std.parse(text); parsed; };
unit(std.utf8) inspect64 = fn(text) { *f64 parsed = std.parse(text); parsed; };
unit(std.utf8) show = fn(text) { std.print(text); };
u64() opaque = fn { core.int_extend<u64>(1) };
u128 tiny = 1;
std.utf8 previous = std.format(tiny);
std.utf8 current = std.format(tiny);
std.print(previous);
std.utf8 interrupted = std.format(tiny);
std.print("noise");
std.print(interrupted);
std.utf8 wrong_target = std.format(tiny);
std.eprint(wrong_target);
observer = extern wasm "fixture_discard" { unit(i32, i32, *?u8, *?u8) checkpoint; };
unit() other_arm = fn {
    observer.checkpoint(0, 0, null, null);
    u16 source = 203;
    std.print(std.format(source));
    observer.checkpoint(1, 0, null, null);
    observer.checkpoint(2, 0, null, null);
};
unit() reused_value = fn {
    observer.checkpoint(0, 0, null, null);
    u8 source = 203;
    std.utf8 text = std.format(source);
    std.print(text);
    observer.checkpoint(1, 0, null, null);
    observer.checkpoint(2, 0, null, null);
};
unit() alternate_print = fn {
    observer.checkpoint(0, 0, null, null);
    u8 source = 203;
    std.eprint(std.format(source));
    observer.checkpoint(1, 0, null, null);
    observer.checkpoint(2, 0, null, null);
};
%%end"#;
    let nested_observer_text = r#"%%start
std = namespace std "bootstrap.w";
observer = extern wasm "fixture_discard" { unit(i32, i32, *?u8, *?u8) checkpoint; };
unit() run = fn {
    observer.checkpoint(0, 0, null, null);
    u8 source = 203;
    std.print(std.format(source));
    observer.checkpoint(1, 0, null, null);
    observer.checkpoint(2, 0, null, null);
};
run();
observer.checkpoint(6, 0, null, null);
%%end
"#;
    let mut accepted_mutations = Vec::new();
    let mut accepted_assignment_mutations = Vec::new();
    for target_layout in [ScalarTargetLayout::NATIVE64, ScalarTargetLayout::WASM32] {
        let source = |pkg: &str, path: &str| {
            SourceIdentity::new("p".into(), pkg.into(), path.into(), "r1".into())
        };
        let std = source("std", "src/bootstrap.w");
        let preview = source("std", "src/wasi/preview1.w");
        let app = source("app", "app.w");
        let fixture = source("app", "fixture.w");
        let nested_fixture = source("production_discard", "src/main.w");
        let prepared = prepare_scalar_project(
            vec![
                (declarations(&std, include_str!("../../../../../stdlib/src/bootstrap.w")), vec![namespace("preview1", &preview)]),
                (declarations(&preview, include_str!("../../../../../stdlib/src/wasi/preview1.w")), Vec::new()),
                (declarations(&app, app_text), vec![namespace("std", &std)]),
                (declarations(&fixture, include_str!("../../../../../tests/fixtures/build/u128_libcall/project/src/format_u128.w")), vec![namespace("std", &std)]),
                (declarations(&nested_fixture, nested_observer_text), vec![namespace("std", &std)]),
            ],
            vec![std.clone(), preview.clone(), app.clone(), fixture.clone(), nested_fixture.clone()],
            ScalarPreparationInputs { target_layout },
        ).unwrap_or_else(|e| panic!("{target_layout:?} prep: {:?}", e.diagnostics));
        let build = analyze_scalar_build(&prepared);
        assert!(
            build.diagnostics().is_empty(),
            "{target_layout:?} build: {:?}",
            build.diagnostics()
        );
        let mut ledger = Ledger::new(&build).unwrap();
        let module = |source: &SourceIdentity| {
            prepared
                .project()
                .modules
                .iter()
                .position(|module| module.source == *source)
                .unwrap()
        };
        let (s, a, f) = (module(&std), module(&app), module(&fixture));
        let std_mod = &prepared.project().modules[s];
        let print_body = ledger.body((s, function(std_mod, "print")), None).unwrap();
        let print_open = ledger.print_finite_open(&print_body).unwrap();
        assert_eq!(print_open.boundary.source.source, preview);
        assert_eq!(print_open.target_layout, target_layout);
        assert_eq!(print_open.boundary.module, "wasi_snapshot_preview1");
        assert_eq!(print_open.boundary.member, "fd_write");
        assert_eq!(print_open.boundary.parameters.len(), 4);
        assert_eq!(print_open.boundary.output, ScalarType::I32);
        exercise_finite_print(&ledger, &print_body, &print_open);
        accepted_assignment_mutations.extend(
            rejected_assignment_fact_mutations(&ledger, &print_body)
                .into_iter()
                .map(|cause| (target_layout, cause)),
        );
        let app_mod = &prepared.project().modules[a];
        let at = |name| (a, function(app_mod, name));
        let n = module(&nested_fixture);
        let run = ledger
            .body((n, function(&prepared.project().modules[n], "run")), None)
            .unwrap();
        let nested = ledger.u8_nested_caller_open(&run).unwrap();
        assert_eq!(nested.caller.source, nested_fixture);
        assert_eq!(nested.print.target_layout, target_layout);
        assert_eq!(
            nested.markers.each_ref().map(|marker| marker.number),
            [0, 1, 2]
        );
        assert_eq!(nested.markers[0].declaration.source.source, nested_fixture);
        assert_eq!(nested.markers[0].declaration.member, "checkpoint");
        assert_eq!(nested.consuming.actuals[0].point, nested.formatting.point);
        assert_eq!(nested.formatting.target, nested.forwarded.entry);
        assert!(matches!(&nested.forwarded.evidence,
            Certificate::Forwarded { target, .. } if target == &nested.producer.entry));
        exercise_u8_nested_cfg_controls(&ledger, &run, &nested);
        for (candidate, cause) in [
            ("other_arm", "nested selected U8 arm"),
            ("reused_value", "nested caller exact root"),
            ("alternate_print", "print selected entry"),
        ] {
            let candidate = ledger.body(at(candidate), None).unwrap();
            assert!(matches!(ledger.u8_nested_caller_open(&candidate),
                Err(Rejected::Contradictory(actual)) if actual == cause));
        }
        let pair = |module: usize, name: &str, distance: usize| {
            let items = &prepared.project().modules.get(module).unwrap().items;
            let (i, binding) = items
                .iter()
                .enumerate()
                .find_map(|(i, item)| match item {
                    ScalarItem::Binding(binding) if binding.name == name => Some((i, binding)),
                    _ => None,
                })
                .unwrap();
            let ScalarItem::Executable(ScalarBlockItem::Expression(consumer)) =
                &items[i + distance]
            else {
                panic!("fixture must contain adjacent direct consumer");
            };
            (binding, consumer)
        };
        let (prior, prior_consumer) = pair(f, "out0", 1);
        let initializer_facts = prepared.initializer_facts(f).unwrap();
        for name in ["out0", "out1", "out2", "out3"] {
            let (binding, consumer) = pair(f, name, 1);
            let open = ledger
                .initializer_local_order_open(f, binding, &binding.value, consumer)
                .unwrap();
            assert_eq!(
                open.receiver,
                SourceSpan::new(fixture.clone(), binding.name_span)
            );
            assert_eq!(open.consumer.arm, None);
            assert_eq!(open.consumer.body, (s, function(std_mod, "print")));
            assert_eq!(open.producer.body, (s, function(std_mod, "format")));
            assert_eq!(open.consumer_parameter, 0);
            assert!(ledger
                .initializer_local_order_open(f, binding, &binding.value, &consumer.clone(),)
                .is_err());
            assert!(initializer_facts
                .binding_output(binding, 1, &binding.value)
                .is_err());
            if !std::ptr::eq(prior, binding) {
                assert!(ledger
                    .initializer_local_order_open(f, prior, &prior.value, consumer,)
                    .is_err());
            }
        }
        assert!(matches!(
            ledger.initializer_local_order_open(a, prior, &prior.value, prior_consumer),
            Err(Rejected::InitializerFact(
                InitializerFactsError::Noncanonical
            ))
        ));
        let app_order = |name, distance| {
            let (binding, consumer) = pair(a, name, distance);
            ledger.initializer_local_order_open(a, binding, &binding.value, consumer)
        };
        assert!(matches!(
            app_order("current", 1),
            Err(Rejected::Contradictory("initializer typed receiver read"))
        ));
        assert!(matches!(
            app_order("interrupted", 2),
            Err(Rejected::InitializerFact(
                InitializerFactsError::Unsupported("initializer next direct call")
            ))
        ));
        assert!(matches!(
            app_order("wrong_target", 1),
            Err(Rejected::Contradictory(
                "initializer selected formatter/print slots"
            ))
        ));
        let print = &std_mod.items[function(std_mod, "print")];
        let ScalarItem::Function(print) = print else {
            unreachable!()
        };
        let print_key = span_key(print.name_span);
        let rows = ledger.declarations.get_mut(&source_key(&std)).unwrap();
        let row = rows.remove(&print_key).unwrap();
        assert!(matches!(
            ledger.initializer_local_order_open(f, prior, &prior.value, prior_consumer),
            Err(Rejected::Missing("declaration"))
        ));
        let rows = ledger.declarations.get_mut(&source_key(&std)).unwrap();
        rows.insert(print_key, IndexEntry::Ambiguous);
        assert!(matches!(
            ledger.initializer_local_order_open(f, prior, &prior.value, prior_consumer),
            Err(Rejected::AmbiguousDeclaration)
        ));
        ledger
            .declarations
            .get_mut(&source_key(&std))
            .unwrap()
            .insert(print_key, row);
        let wrapper = ledger.body(at("wrap"), None).unwrap();
        let call = final_call(&wrapper);
        assert!(matches!(
            ledger.direct_call(&wrapper, &call.clone()),
            Err(Rejected::Missing("body target"))
        ));
        let selected = ledger.direct_call(&wrapper, call).unwrap();
        assert_eq!(selected.target.body, (s, function(std_mod, "format")));
        assert!(selected.target.arm.is_some());
        assert_eq!(selected.source.source, app);
        assert_eq!(selected.actuals[0].ty, ScalarType::U8);

        let format_arm = ledger
            .body(selected.target.body, selected.target.arm)
            .unwrap();
        let inside = ledger
            .direct_call(&format_arm, final_call(&format_arm))
            .unwrap();
        assert_eq!(inside.target.body, (s, function(std_mod, "_format_u8")));
        assert_eq!(inside.target.arm, None);
        let producer = ledger.producer(inside.target.body).unwrap();
        let forwarded = ledger.forward(&format_arm, &producer).unwrap();
        assert_eq!(forwarded.entry, selected.target);
        assert!(
            matches!(forwarded.evidence, Certificate::Forwarded { ref target, site }
            if target == &inside.target && site == inside.point)
        );
        let normal = ledger.forward(&wrapper, &forwarded).unwrap();
        assert_eq!(normal.entry.body, at("wrap"));
        assert_eq!(normal.entry.arm, None);
        assert!(matches!(
            ledger.forward(&wrapper, &producer),
            Err(Rejected::Contradictory("forward target/output"))
        ));
        let replaced_formal = format_arm.nodes.formal(format_arm.facts, 0).unwrap().0;
        let modified_cfg = cfg_with(wrapper.cfg, |points| {
            let CfgPointKind::Read { binding } = &mut points[selected.actuals[0].point.0].kind
            else {
                unreachable!()
            };
            *binding = replaced_formal.clone();
        });
        let modified = CallableBody {
            cfg: &modified_cfg,
            ..ledger.body(at("wrap"), None).unwrap()
        };
        let altered = ledger.forward(&modified, &forwarded).err();
        assert!(
            matches!(
                altered,
                Some(Rejected::Contradictory("forward formal route"))
            ),
            "{target_layout:?} altered formal: {altered:?}"
        );

        let other_build = analyze_scalar_build(&prepared);
        let other_nested_ledger = Ledger::new(&other_build).unwrap();
        assert!(matches!(
            other_nested_ledger.u8_nested_caller_open(&run),
            Err(Rejected::Contradictory("nested caller artifact provenance"))
        ));
        let other_ledger = Ledger::new(&other_build).unwrap();
        let foreign_body = other_ledger.body(at("wrap"), None).unwrap();
        assert!(matches!(
            ledger.direct_call(&foreign_body, final_call(&foreign_body)),
            Err(Rejected::Contradictory("body artifact provenance"))
        ));
        let foreign_producer = other_ledger.producer(inside.target.body).unwrap();
        assert!(matches!(
            ledger.forward(&format_arm, &foreign_producer),
            Err(Rejected::Contradictory("forward certificate provenance"))
        ));

        let modified_cfg = cfg_with(wrapper.cfg, |points| {
            let CfgPointKind::Call {
                argument_points, ..
            } = &mut points[selected.point.0].kind
            else {
                unreachable!()
            };
            argument_points[0] = selected.point;
        });
        let modified = CallableBody {
            cfg: &modified_cfg,
            ..ledger.body(at("wrap"), None).unwrap()
        };
        assert!(matches!(
            ledger.direct_call(&modified, final_call(&modified)),
            Err(Rejected::Contradictory("ordered actual slot"))
        ));
        let modified_cfg = cfg_with(wrapper.cfg, |points| {
            let CfgPointKind::Call { target, .. } = &mut points[selected.point.0].kind else {
                unreachable!()
            };
            *target = ResolvedCallTarget::Core(crate::scalar::CoreOperationId::IntExtend);
        });
        let modified = CallableBody {
            cfg: &modified_cfg,
            ..ledger.body(at("wrap"), None).unwrap()
        };
        assert!(matches!(
            ledger.direct_call(&modified, final_call(&modified)),
            Err(Rejected::Contradictory("body call identity"))
        ));
        let modified_cfg = cfg_with(wrapper.cfg, |points| {
            points
                .iter_mut()
                .find_map(|point| match &mut point.kind {
                    CfgPointKind::ReturnOutput { output, .. } => Some(output),
                    _ => None,
                })
                .map(|output| *output = 1)
                .unwrap();
        });
        let modified = CallableBody {
            cfg: &modified_cfg,
            ..ledger.body(at("wrap"), None).unwrap()
        };
        assert!(matches!(
            ledger.forward(&modified, &forwarded),
            Err(Rejected::Contradictory("forward return identity"))
        ));

        for (label, from, to) in [
            ("detached entry", CfgPointId(0), None),
            (
                "rerouted read successor",
                selected.actuals[0].point,
                Some(CfgPointId(0)),
            ),
        ] {
            let cfg = cfg_with(wrapper.cfg, |points| {
                points[from.0].successors = to.into_iter().collect();
            });
            let changed = CallableBody {
                cfg: &cfg,
                ..ledger.body(at("wrap"), None).unwrap()
            };
            if !matches!(
                ledger.forward(&changed, &forwarded),
                Err(Rejected::Contradictory("forward executable path"))
            ) {
                accepted_mutations.push((target_layout, label));
            }
        }

        let initializer = ledger.initializer(f).unwrap();
        let initializer_call = prepared.project().modules[f]
            .items
            .iter()
            .find_map(|item| {
                let ScalarItem::Binding(binding) = item else {
                    return None;
                };
                matches!(binding.value, ScalarExpression::Call { .. }).then_some(&binding.value)
            })
            .unwrap();
        let init = ledger
            .initializer_target(&initializer, initializer_call)
            .unwrap();
        let foreign_initializer = other_ledger.initializer(f).unwrap();
        assert!(matches!(
            ledger.initializer_target(&foreign_initializer, initializer_call),
            Err(Rejected::Contradictory("initializer artifact provenance"))
        ));
        assert_eq!(init.source.source, fixture);
        assert_eq!(init.target.body, selected.target.body);
        assert_ne!(init.target.arm, selected.target.arm); // u128, not the u8 arm.
        assert!(matches!(
            ledger.direct_call(&wrapper, initializer_call),
            Err(Rejected::Missing("body target"))
        ));
        assert!(matches!(
            ledger.initializer_target(&initializer, call),
            Err(Rejected::Missing("initializer target"))
        ));
        for (name, expected) in [
            ("inspect32", ScalarType::F32),
            ("inspect64", ScalarType::F64),
        ] {
            let body = ledger.body(at(name), None).unwrap();
            let ScalarBlockItem::LocalBinding(binding) = &body.entry.function.body.items[0] else {
                unreachable!()
            };
            let parse = ledger
                .checked_open(&body, &binding.value, &producer)
                .unwrap();
            assert_eq!((parse.descriptor_parameter, parse.checked_output), (0, 0));
            assert_eq!(parse.mode, ScalarReferenceMutability::Shared);
            assert_eq!(parse.referent, expected);
            assert_eq!(parse.call.target.body, (s, function(std_mod, "parse")));
            assert!(parse.call.target.arm.is_some());
            assert_ne!(parse.call.target, init.target);
            assert!(matches!(
                ledger.initializer_target(&initializer, &binding.value),
                Err(Rejected::Missing("initializer target"))
            ));
        }
        let print = ledger.body(at("show"), None).unwrap();
        let ScalarBlockItem::Expression(print_call) = &print.entry.function.body.items[0] else {
            unreachable!()
        };
        let ordinary = ledger.direct_call(&print, print_call).unwrap();
        assert_eq!(ordinary.target.body, (s, function(std_mod, "print")));
        assert_eq!(ordinary.results, [ScalarType::U64, ScalarType::Bool]);
        assert!(matches!(
            ledger.checked_open(&print, print_call, &producer),
            Err(Rejected::Contradictory("checked appointment result"))
        ));
        let opaque = ledger.body(at("opaque"), None).unwrap();
        assert!(matches!(
            ledger.direct_call(&opaque, final_call(&opaque)),
            Err(Rejected::UnsupportedTarget)
        ));

        let arm = ConcreteCallSelection::Overload(ScalarOverloadSelection {
            arm_index: selected.target.arm.unwrap(),
            substitutions: BTreeMap::from([("T".into(), ScalarType::U8)]),
        });
        assert!(matches!(
            ledger.lookup(&std, selected.target.declaration, &arm),
            Err(Rejected::UnsupportedSpecialization)
        ));
        let other_rev = SourceIdentity::new(
            std.project.clone(),
            std.package.clone(),
            std.path.clone(),
            "r2".into(),
        );
        assert!(matches!(
            ledger.lookup(
                &other_rev,
                selected.target.declaration,
                &ConcreteCallSelection::Declaration
            ),
            Err(Rejected::Missing("declaration"))
        ));
        assert!(matches!(
            ledger.lookup(
                &std,
                selected.target.declaration,
                &ConcreteCallSelection::Declaration
            ),
            Err(Rejected::UnsupportedSpecialization)
        ));
        ledger.insert(
            &std_mod.source,
            selected.target.declaration,
            selected.target.body,
        );
        assert!(matches!(
            ledger.direct_call(&wrapper, call),
            Err(Rejected::AmbiguousDeclaration)
        ));
    }
    assert!(
        accepted_mutations.is_empty(),
        "path mutations not rejected: {accepted_mutations:?}"
    );
    assert!(
        accepted_assignment_mutations.is_empty(),
        "print destination mutations not rejected: {accepted_assignment_mutations:?}"
    );
}

#[test]
fn formatter_consumer_closure_index_preserves_full_revision_identity() {
    let first = SourceIdentity::new("p".into(), "pkg".into(), "same.w".into(), "first".into());
    let second = SourceIdentity::new("p".into(), "pkg".into(), "same.w".into(), "second".into());
    let text = "%%start\nu8(u8) identity = fn(value) { value };\n%%end";
    for target_layout in [ScalarTargetLayout::NATIVE64, ScalarTargetLayout::WASM32] {
        let prepared = prepare_scalar_project(
            vec![
                (declarations(&first, text), Vec::new()),
                (declarations(&second, text), Vec::new()),
            ],
            vec![first.clone(), second.clone()],
            ScalarPreparationInputs { target_layout },
        )
        .unwrap_or_else(|err| panic!("{:?}", err.diagnostics));
        let build = analyze_scalar_build(&prepared);
        let ledger = Ledger::new(&build).unwrap();
        let module = &prepared.project().modules[0];
        let ScalarItem::Function(fun) = &module.items[function(module, "identity")] else {
            unreachable!()
        };
        let first_entry = ledger
            .lookup(&first, fun.name_span, &ConcreteCallSelection::Declaration)
            .unwrap();
        let second_entry = ledger
            .lookup(&second, fun.name_span, &ConcreteCallSelection::Declaration)
            .unwrap();
        assert_ne!(first_entry.id, second_entry.id);
        assert_eq!(first_entry.id.source, first);
        assert_eq!(second_entry.id.source, second);
        assert_eq!(first_entry.id.declaration, second_entry.id.declaration);
    }
}

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
%%end"#;
    let mut accepted_mutations = Vec::new();
    for target_layout in [ScalarTargetLayout::NATIVE64, ScalarTargetLayout::WASM32] {
        let source = |pkg: &str, path: &str| {
            SourceIdentity::new("p".into(), pkg.into(), path.into(), "r1".into())
        };
        let std = source("std", "src/bootstrap.w");
        let preview = source("std", "src/wasi/preview1.w");
        let app = source("app", "app.w");
        let fixture = source("app", "fixture.w");
        let prepared = prepare_scalar_project(
            vec![
                (declarations(&std, include_str!("../../../../../stdlib/src/bootstrap.w")), vec![namespace("preview1", &preview)]),
                (declarations(&preview, include_str!("../../../../../stdlib/src/wasi/preview1.w")), Vec::new()),
                (declarations(&app, app_text), vec![namespace("std", &std)]),
                (declarations(&fixture, include_str!("../../../../../tests/fixtures/build/u128_libcall/project/src/format_u128.w")), vec![namespace("std", &std)]),
            ],
            vec![std.clone(), preview.clone(), app.clone(), fixture.clone()],
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
        let app_mod = &prepared.project().modules[a];
        let at = |name| (a, function(app_mod, name));
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

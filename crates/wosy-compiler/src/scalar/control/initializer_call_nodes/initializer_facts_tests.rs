use super::*;
use crate::{
    derive_scalar_declarations_from_cst, parse_source, prepare_scalar_project,
    ScalarNamespaceBinding, ScalarPreparationInputs, ScalarSourceDeclarations, ScalarTargetLayout,
};

fn declarations(source: &SourceIdentity, text: &str) -> ScalarSourceDeclarations {
    let parsed = parse_source(source.clone(), text.into(), &[]);
    assert!(parsed.diagnostics.is_empty(), "{:?}", parsed.diagnostics);
    derive_scalar_declarations_from_cst(parsed.result.canonical_cst())
}

fn namespace(binding: &str, source: &SourceIdentity) -> ScalarNamespaceBinding {
    ScalarNamespaceBinding {
        binding: binding.into(),
        target: source.clone(),
        span: ByteSpan::new(0, 0),
    }
}

#[test]
fn initializer_facts_binding_output_routes_preserve_actual_secondary_occurrences() {
    let text = "%%start\nu8(u8) id = fn(value) { value };\n(u8, u8)(u8) duo = fn(value) { value, value };\nu8 left, u8 right = id(1) + 1, id(2);\nu8 first, u8 second = duo(3);\nid(second);\n%%end";
    for target_layout in [ScalarTargetLayout::NATIVE64, ScalarTargetLayout::WASM32] {
        let source = SourceIdentity::new("p".into(), "app".into(), "routes.w".into(), "r1".into());
        let other = SourceIdentity::new("p".into(), "app".into(), "routes.w".into(), "r2".into());
        let prepared = prepare_scalar_project(
            vec![
                (declarations(&source, text), Vec::new()),
                (declarations(&other, text), Vec::new()),
            ],
            vec![source.clone(), other.clone()],
            ScalarPreparationInputs { target_layout },
        )
        .unwrap_or_else(|error| panic!("{:?}", error.diagnostics));
        let modules = &prepared.project().modules;
        let index = |source: &SourceIdentity| {
            modules
                .iter()
                .position(|module| &module.source == source)
                .unwrap()
        };
        let module = index(&source);
        let items = &modules[module].items;
        let binding = |name: &str| {
            items
                .iter()
                .find_map(|item| match item {
                    ScalarItem::Binding(binding) if binding.name == name => Some(binding),
                    _ => None,
                })
                .unwrap()
        };
        let independent = binding("left");
        let projected = binding("first");
        let ScalarBindingOutput::Evaluate(call) = independent.output_source(1) else {
            panic!("independent initializer");
        };
        let facts = prepared.initializer_facts(module).unwrap();
        let evaluated = facts.binding_output(independent, 1, call).unwrap();
        assert_eq!(evaluated.route, InitializerOutputRoute::EvaluateSecondary);
        assert_eq!(evaluated.ty, &ScalarType::U8);
        assert_eq!(
            evaluated.destination,
            SourceSpan::new(source.clone(), independent.receivers[1].name_span)
        );
        let projection = facts
            .binding_output(projected, 1, &projected.value)
            .unwrap();
        assert_eq!(
            projection.route,
            InitializerOutputRoute::ProjectPrimary { output: 1 }
        );
        assert_eq!(
            projection.destination,
            SourceSpan::new(source.clone(), projected.receivers[1].name_span)
        );
        for (binding, wrong) in [(independent, &independent.value), (projected, call)] {
            assert_eq!(
                facts.binding_output(binding, 1, wrong).unwrap_err(),
                InitializerFactsError::Noncanonical
            );
        }
        assert_eq!(
            prepared
                .initializer_facts(index(&other))
                .unwrap()
                .binding_output(projected, 1, &projected.value)
                .unwrap_err(),
            InitializerFactsError::Noncanonical
        );
        let mut altered = prepared.initialization_region(module).unwrap().clone();
        let ModuleInitializationDetail::Occurrences(topology) = &mut altered.detail else {
            unreachable!()
        };
        let item = facts.root_binding_item(projected).unwrap();
        let rows = &mut topology.structure.blocks[topology.structure.root.0].items;
        let row = rows.iter_mut().find(|row| row.position == item).unwrap();
        let BlockItemKind::Binding(binding_row) = &mut row.kind else {
            unreachable!()
        };
        let BindingInitialization::Outputs(outputs) = &mut binding_row.initialization else {
            unreachable!()
        };
        outputs.secondary[0] = BindingOutput::ProjectPrimary { output: 0 };
        assert_eq!(
            InitializerFacts::bind(&altered, items)
                .unwrap()
                .binding_output(projected, 1, &projected.value)
                .unwrap_err(),
            InitializerFactsError::Malformed("initializer output route")
        );
        let mut missing = prepared.initialization_region(module).unwrap().clone();
        let ModuleInitializationDetail::Occurrences(topology) = &mut missing.detail else {
            unreachable!()
        };
        let rows = &mut topology.structure.blocks[topology.structure.root.0].items;
        let row = rows.iter().position(|row| row.position == item).unwrap();
        let removed = rows.remove(row);
        assert_eq!(
            InitializerFacts::bind(&missing, items)
                .unwrap()
                .binding_output(projected, 1, &projected.value)
                .unwrap_err(),
            InitializerFactsError::Missing("initializer root item")
        );
        let ModuleInitializationDetail::Occurrences(topology) = &mut missing.detail else {
            unreachable!()
        };
        let rows = &mut topology.structure.blocks[topology.structure.root.0].items;
        rows.extend([removed.clone(), removed]);
        assert_eq!(
            InitializerFacts::bind(&missing, items)
                .unwrap()
                .binding_output(projected, 1, &projected.value)
                .unwrap_err(),
            InitializerFactsError::Malformed("initializer root item")
        );
    }
}

#[test]
fn initializer_facts_project_exact_u128_format_and_print_storage() {
    for target_layout in [ScalarTargetLayout::NATIVE64, ScalarTargetLayout::WASM32] {
        let source = |pkg: &str, path: &str| {
            SourceIdentity::new("p".into(), pkg.into(), path.into(), "r".into())
        };
        let app = source("app", "src/format_u128.w");
        let bootstrap = source("std", "src/bootstrap.w");
        let preview = source("std", "src/wasi/preview1.w");
        let prepared = prepare_scalar_project(
            vec![
                (declarations(&app, include_str!("../../../../../../tests/fixtures/build/u128_libcall/project/src/format_u128.w")), vec![namespace("std", &bootstrap)]),
                (declarations(&bootstrap, include_str!("../../../../../../stdlib/src/bootstrap.w")), vec![namespace("preview1", &preview)]),
                (declarations(&preview, include_str!("../../../../../../stdlib/src/wasi/preview1.w")), Vec::new()),
            ],
            vec![app.clone(), bootstrap.clone(), preview],
            ScalarPreparationInputs { target_layout },
        ).unwrap_or_else(|error| panic!("{target_layout:?}: {:?}", error.diagnostics));
        let project = prepared.project();
        let module = project
            .modules
            .iter()
            .position(|module| module.source == app)
            .unwrap();
        let items = &project.modules[module].items;
        let out0 = items
            .iter()
            .find_map(|item| match item {
                ScalarItem::Binding(binding) if binding.name == "out0" => Some(binding),
                _ => None,
            })
            .unwrap();
        let (print, argument) = items
            .iter()
            .find_map(|item| {
                let ScalarItem::Executable(ScalarBlockItem::Expression(
                    call @ ScalarExpression::Call {
                        name, arguments, ..
                    },
                )) = item
                else {
                    return None;
                };
                let [argument @ ScalarExpression::Name { name: actual, .. }] = arguments.as_slice()
                else {
                    return None;
                };
                (name == "print" && actual == "out0").then_some((call, argument))
            })
            .unwrap();
        let facts = prepared.initializer_facts(module).unwrap();
        let format = facts.checked_direct_call(&out0.value).unwrap();
        assert_eq!(format.declaration.source, bootstrap);
        assert!(matches!(
            format.concrete,
            ConcreteCallSelection::Overload(_)
        ));
        assert_eq!(format.parameters, &[ScalarType::U128]);
        let ScalarExpression::Call { arguments, .. } = &out0.value else {
            unreachable!()
        };
        let [zero] = arguments.as_slice() else {
            unreachable!()
        };
        assert_eq!(
            facts.checked_expression_type(zero).unwrap(),
            &ScalarType::U128
        );
        let [output] = format.outputs else {
            unreachable!("one format output");
        };
        assert!(matches!(&output.ty, ScalarType::Struct(_)));
        assert_eq!(
            facts.checked_expression_type(&out0.value).unwrap(),
            &output.ty
        );
        let printed = facts.checked_direct_call(print).unwrap();
        assert_eq!(printed.declaration.source, bootstrap);
        assert!(matches!(
            printed.concrete,
            ConcreteCallSelection::Declaration
        ));
        assert_eq!(printed.parameters, &[output.ty.clone()]);
        assert_eq!(facts.checked_expression_type(argument).unwrap(), &output.ty);
        assert_eq!(
            printed
                .outputs
                .iter()
                .map(|slot| &slot.ty)
                .collect::<Vec<_>>(),
            [&ScalarType::U64, &ScalarType::Bool]
        );
        assert_eq!(
            facts.plain_name(argument).unwrap(),
            QualifiedInitializerStorage::Module(SourceSpan::new(app.clone(), out0.name_span))
        );
        assert_eq!(
            facts.checked_direct_call(&out0.value.clone()).unwrap_err(),
            InitializerFactsError::Noncanonical
        );
        assert_eq!(
            facts.plain_name(&argument.clone()).unwrap_err(),
            InitializerFactsError::Noncanonical
        );

        let region = prepared.initialization_region(module).unwrap();
        let edited = |mutate: &dyn Fn(&mut ModuleInitializationTopology)| {
            let mut copy = region.clone();
            let ModuleInitializationDetail::Occurrences(topology) = &mut copy.detail else {
                unreachable!()
            };
            mutate(topology);
            copy
        };
        let format_id = facts.expression(&out0.value).unwrap();
        let arg_id = facts.expression(argument).unwrap();
        let zero_id = facts.expression(zero).unwrap();
        let removed_type = edited(&|topology| {
            assert!(topology
                .structure
                .checked_types
                .expressions
                .remove(&zero_id)
                .is_some());
        });
        let omitted = InitializerFacts::bind(&removed_type, items).unwrap();
        assert_eq!(
            omitted.checked_expression_type(zero),
            Err(InitializerFactsError::Missing(
                "initializer expression type"
            ))
        );
        assert_eq!(
            omitted.checked_direct_call(&out0.value).unwrap().parameters,
            &[ScalarType::U128]
        );
        let removed_signature = edited(&|topology| {
            assert!(topology
                .checked_call_signatures
                .remove(&format_id)
                .is_some());
        });
        let omitted = InitializerFacts::bind(&removed_signature, items).unwrap();
        assert_eq!(
            omitted.checked_direct_call(&out0.value).unwrap_err(),
            InitializerFactsError::Missing("initializer call signature")
        );
        assert_eq!(
            omitted.checked_expression_type(zero).unwrap(),
            &ScalarType::U128
        );
        let removed_target = edited(&|topology| {
            assert!(topology.call_targets.remove(&format_id).is_some());
        });
        assert_eq!(
            InitializerFacts::bind(&removed_target, items)
                .unwrap()
                .checked_direct_call(&out0.value)
                .unwrap_err(),
            InitializerFactsError::Missing("initializer call target")
        );
        let changed_arity = edited(&|topology| {
            topology
                .checked_call_signatures
                .get_mut(&format_id)
                .unwrap()
                .parameters
                .clear();
        });
        assert_eq!(
            InitializerFacts::bind(&changed_arity, items)
                .unwrap()
                .checked_direct_call(&out0.value)
                .unwrap_err(),
            InitializerFactsError::Malformed("initializer call arity")
        );
        let changed_arm = edited(&|topology| {
            topology
                .checked_call_signatures
                .get_mut(&format_id)
                .unwrap()
                .overload
                .as_mut()
                .unwrap()
                .arm_index += 1;
        });
        assert_eq!(
            InitializerFacts::bind(&changed_arm, items)
                .unwrap()
                .checked_direct_call(&out0.value)
                .unwrap_err(),
            InitializerFactsError::Malformed("initializer call selection")
        );
        let specialized = edited(&|topology| {
            let ModuleInitializationCallTarget::Direct {
                concrete: ConcreteCallSelection::Overload(selection),
                ..
            } = topology.call_targets.get_mut(&format_id).unwrap()
            else {
                unreachable!()
            };
            selection.substitutions.insert("T".into(), ScalarType::U128);
        });
        assert_eq!(
            InitializerFacts::bind(&specialized, items)
                .unwrap()
                .checked_direct_call(&out0.value)
                .unwrap_err(),
            InitializerFactsError::Unsupported("initializer call specialization")
        );
        let opaque = edited(&|topology| {
            *topology.call_targets.get_mut(&format_id).unwrap() =
                ModuleInitializationCallTarget::Core(CoreOperationId::IntExtend);
        });
        assert_eq!(
            InitializerFacts::bind(&opaque, items)
                .unwrap()
                .checked_direct_call(&out0.value)
                .unwrap_err(),
            InitializerFactsError::Unsupported("initializer callable target")
        );
        let removed_name = edited(&|topology| {
            assert!(topology.identities.names.remove(&arg_id).is_some());
        });
        assert_eq!(
            InitializerFacts::bind(&removed_name, items)
                .unwrap()
                .plain_name(argument),
            Err(InitializerFactsError::Missing("initializer plain name"))
        );
        let wrong_kind = edited(&|topology| {
            topology.occurrences[arg_id.0].role = OccurrenceRole::Place;
        });
        assert_eq!(
            InitializerFacts::bind(&wrong_kind, items)
                .unwrap()
                .plain_name(argument),
            Err(InitializerFactsError::Malformed(
                "initializer expression occurrence"
            ))
        );
    }
}

#[test]
fn initializer_facts_keep_shadowed_names_places_destinations_and_revisions_distinct() {
    let first = SourceIdentity::new("p".into(), "app".into(), "same.w".into(), "r1".into());
    let second = SourceIdentity::new("p".into(), "app".into(), "same.w".into(), "r2".into());
    let text = "%%start
u8 value = 3;
u8 observed = value;
u8 shadow = if (true) {
    u8 value = 5;
    value = 7;
    alias = value;
    alias
} else { value };
value = 9;
u8[3] data = [1, 2, 3];
u8 before = data[0];
data[0] = 2;
*?u8 empty = null;
%%end";
    for target_layout in [ScalarTargetLayout::NATIVE64, ScalarTargetLayout::WASM32] {
        let prepared = prepare_scalar_project(
            vec![
                (declarations(&first, text), Vec::new()),
                (declarations(&second, text), Vec::new()),
            ],
            vec![first.clone(), second.clone()],
            ScalarPreparationInputs { target_layout },
        )
        .unwrap_or_else(|error| panic!("{target_layout:?}: {:?}", error.diagnostics));
        let mut observed_across_revisions = Vec::new();
        for source in [&first, &second] {
            let module = prepared
                .project()
                .modules
                .iter()
                .position(|module| &module.source == source)
                .unwrap();
            let items = &prepared.project().modules[module].items;
            let binding = |name| {
                items
                    .iter()
                    .find_map(|item| match item {
                        ScalarItem::Binding(binding) if binding.name == name => Some(binding),
                        _ => None,
                    })
                    .unwrap()
            };
            let callable = prepared.initializer_facts(module).unwrap();
            let other = prepared
                .project()
                .modules
                .iter()
                .position(|module| &module.source != source)
                .unwrap();
            let value = &binding("observed").value;
            let module_name = QualifiedInitializerStorage::Module(SourceSpan::new(
                source.clone(),
                binding("value").name_span,
            ));
            assert_eq!(callable.plain_name(value).unwrap(), module_name);
            assert_eq!(
                prepared.initializer_facts(other).unwrap().plain_name(value),
                Err(InitializerFactsError::Noncanonical)
            );
            observed_across_revisions.push(module_name.clone());
            let ScalarExpression::If {
                then_branch: block, ..
            } = &binding("shadow").value
            else {
                unreachable!()
            };
            let [ScalarBlockItem::LocalBinding(local), ScalarBlockItem::Assignment(local_write), ScalarBlockItem::Assignment(introduce), ScalarBlockItem::Expression(alias)] =
                block.items.as_slice()
            else {
                unreachable!("canonical shadow and initializer assignment occurrences");
            };
            let local_name = QualifiedInitializerStorage::Lexical {
                declaration: SourceSpan::new(source.clone(), local.name_span),
                block_span: block.span,
            };
            assert_eq!(
                callable.plain_name(&introduce.values[0]).unwrap(),
                local_name
            );
            let local_target = &local_write.targets[0].place;
            assert_eq!(
                callable
                    .direct_assignment_destination(local_target)
                    .unwrap(),
                InitializerDestinationFacts::Existing(local_name)
            );
            let alias_name = QualifiedInitializerStorage::Lexical {
                declaration: SourceSpan::new(source.clone(), introduce.targets[0].target_span),
                block_span: block.span,
            };
            assert_eq!(callable.plain_name(alias).unwrap(), alias_name);
            assert_eq!(
                callable
                    .direct_assignment_destination(&introduce.targets[0].place)
                    .unwrap(),
                InitializerDestinationFacts::Introduce(alias_name)
            );
            let mut top_level_assignments = items.iter().filter_map(|item| match item {
                ScalarItem::Executable(ScalarBlockItem::Assignment(assignment)) => Some(assignment),
                _ => None,
            });
            let destination = &top_level_assignments.next().unwrap().targets[0].place;
            let indexed_destination = &top_level_assignments.next().unwrap().targets[0].place;
            assert_eq!(
                callable.direct_assignment_destination(destination).unwrap(),
                InitializerDestinationFacts::Existing(module_name)
            );
            assert_eq!(
                callable.named_place_root(destination),
                Err(InitializerFactsError::Unsupported(
                    "initializer named root shape"
                ))
            );
            let ScalarExpression::IndexedRead { place, .. } = &binding("before").value else {
                unreachable!()
            };
            let ScalarPlace::Index { base, .. } = place else {
                unreachable!()
            };
            assert_eq!(
                callable.named_place_root(base).unwrap(),
                QualifiedInitializerStorage::Module(SourceSpan::new(
                    source.clone(),
                    binding("data").name_span
                ),)
            );
            assert_eq!(
                callable.direct_assignment_destination(base),
                Err(InitializerFactsError::Unsupported(
                    "initializer assignment destination shape"
                ))
            );
            assert_eq!(
                callable.direct_assignment_destination(indexed_destination),
                Err(InitializerFactsError::Unsupported(
                    "initializer assignment destination shape"
                ))
            );
            assert_eq!(
                callable.named_place_root(place),
                Err(InitializerFactsError::Unsupported(
                    "initializer named root shape"
                ))
            );
            assert_eq!(
                callable.plain_name(&binding("empty").value),
                Err(InitializerFactsError::Unsupported("initializer null"))
            );
            assert_eq!(
                callable.named_place_root(&*base.clone()),
                Err(InitializerFactsError::Noncanonical)
            );

            let mut region = prepared.initialization_region(module).unwrap().clone();
            let ModuleInitializationDetail::Occurrences(topology) = &mut region.detail else {
                unreachable!()
            };
            assert!(topology
                .identities
                .roots
                .remove(&callable.place(base).unwrap())
                .is_some());
            assert!(topology
                .identities
                .destinations
                .remove(&callable.place(destination).unwrap())
                .is_some());
            let withheld = InitializerFacts::bind(&region, items).unwrap();
            assert_eq!(
                withheld.named_place_root(base),
                Err(InitializerFactsError::Missing("initializer named root"))
            );
            assert_eq!(
                withheld.direct_assignment_destination(destination),
                Err(InitializerFactsError::Missing(
                    "initializer assignment destination"
                ))
            );
        }
        assert_ne!(observed_across_revisions[0], observed_across_revisions[1]);
    }
}

#[test]
fn initializer_local_scalar_extent_follows_dominated_blocks_and_refuses_address_escape() {
    for layout in [ScalarTargetLayout::NATIVE64, ScalarTargetLayout::WASM32] {
        for (extra, enrolled) in [
            ("", true),
            ("if (true) { if (true) { data[1] = 8; u16 observed = data[1]; }; };", true),
            ("unsafe { *?u16 escaped = &?data[0]; };", false),
        ] {
            let text = format!("%%start\nu8 selected = if (true) {{ u64 count = 4; u16[count] data; data[0] = 7; {extra} 1 }} else {{ 2 }};\n%%end");
            let source = SourceIdentity::new("p".into(), "app".into(), "extent.w".into(), "r1".into());
            let prepared = prepare_scalar_project(
                vec![(declarations(&source, &text), Vec::new())],
                vec![source],
                ScalarPreparationInputs { target_layout: layout },
            ).unwrap_or_else(|error| panic!("{layout:?}: {:?}", error.diagnostics));
            let facts = prepared.initializer_facts(0).unwrap();
            let nodes = InitializerCallNodes::bind(facts.region, facts.items).unwrap();
            let ScalarItem::Binding(selected) = &facts.items[0] else { unreachable!() };
            let ScalarExpression::If { then_branch, .. } = &selected.value else { unreachable!() };
            let ScalarBlockItem::LocalBinding(data) = &then_branch.items[1] else { unreachable!() };
            let ScalarBlockItem::Assignment(write) = &then_branch.items[2] else { unreachable!() };
            let place = &write.targets[0].place;
            let ScalarPlace::Index { base, .. } = place else { unreachable!() };
            assert_eq!(nodes.checked_local_array(data), enrolled);
            assert_eq!(nodes.checked_local_index(place, base), enrolled.then_some(data.receivers[0].name_span));
            assert_eq!(nodes.checked_local_index(place, &base.as_ref().clone()), None);
            if enrolled && !extra.is_empty() {
                let ScalarBlockItem::Expression(ScalarExpression::UnitIf { then_branch: outer, .. }) =
                    &then_branch.items[3] else { unreachable!("outer descendant") };
                let ScalarBlockItem::Expression(ScalarExpression::UnitIf { then_branch: inner, .. }) =
                    &outer.items[0] else { unreachable!("inner descendant") };
                let ScalarBlockItem::Assignment(write) = &inner.items[0] else { unreachable!() };
                let ScalarBlockItem::LocalBinding(observed) = &inner.items[1] else { unreachable!() };
                let ScalarExpression::IndexedRead { place: read, .. } = &observed.value else { unreachable!() };
                for projection in [&write.targets[0].place, read] {
                    let ScalarPlace::Index { base, .. } = projection else { unreachable!() };
                    assert_eq!(nodes.checked_local_index(projection, base), Some(data.receivers[0].name_span));
                }
            }
        }
    }
}

#[test]
fn initializer_indexed_shared_address_needs_only_dominated_scalar_reads() {
    for layout in [ScalarTargetLayout::NATIVE64, ScalarTargetLayout::WASM32] {
        for (statement, enrolled, immediate) in [
            ("u16 observed = *(&data[0]);", true, true),
            ("*u16 held = &data[0]; u16 observed = *held;", true, false),
            ("*u16 held = &data[0]; *u16 other = held; u16 observed = *other;", false, false),
            ("*u16 held = &data[0]; data[1] = 8; u16 observed = *held;", false, false),
        ] {
            let text = format!("%%start\nu8 selected = if (true) {{ u64 count = 4; u16[count] data; data[0] = 7; {statement} 1 }} else {{ 2 }};\n%%end");
            let source = SourceIdentity::new("p".into(), "app".into(), "element-reference.w".into(), "r1".into());
            let prepared = prepare_scalar_project(
                vec![(declarations(&source, &text), Vec::new())],
                vec![source],
                ScalarPreparationInputs { target_layout: layout },
            ).unwrap_or_else(|error| panic!("{layout:?}: {:?}", error.diagnostics));
            let facts = prepared.initializer_facts(0).unwrap();
            let nodes = InitializerCallNodes::bind(facts.region, facts.items).unwrap();
            let ScalarItem::Binding(selected) = &facts.items[0] else { unreachable!() };
            let ScalarExpression::If { then_branch, .. } = &selected.value else { unreachable!() };
            let ScalarBlockItem::LocalBinding(data) = &then_branch.items[1] else { unreachable!() };
            assert_eq!(nodes.checked_local_array(data), enrolled);
            let ScalarBlockItem::LocalBinding(observed) = &then_branch.items[3] else { unreachable!() };
            let address = if immediate {
                let ScalarExpression::Dereference { place: ScalarPlace::Dereference { pointer, .. }, .. } = &observed.value
                    else { unreachable!("exact immediate checked address consumer"); };
                pointer.as_ref()
            } else {
                &observed.value
            };
            let ScalarExpression::CheckedAddress { place, .. } = address else { unreachable!() };
            let ScalarPlace::Index { base, .. } = place else { unreachable!() };
            assert_eq!(nodes.checked_local_index(place, base), enrolled.then_some(data.receivers[0].name_span));
            assert_eq!(nodes.checked_immediate_element_address_type(address), immediate.then_some(&ScalarType::CheckedReference {
                mutability: crate::scalar::ScalarReferenceMutability::Shared,
                inner: Box::new(ScalarType::U16),
            }));
            assert_eq!(nodes.checked_immediate_element_address_type(&address.clone()), None);
        }
    }
}

#[test]
fn initializer_independent_scalar_assignment_keeps_each_indexed_destination() {
    for layout in [ScalarTargetLayout::NATIVE64, ScalarTargetLayout::WASM32] {
        for (assignment, indexed_lanes) in [
            ("index, data[0] = next, value;", &[1][..]),
            ("data[0], data[1] = value, other;", &[0, 1][..]),
        ] {
            let text = format!("%%start\nu8 selected = if (true) {{ u64 count = 4; u16[count] data; u64 index = 0; u64 next = 1; u16 value = 7; u16 other = 8; {assignment} 1 }} else {{ 2 }};\n%%end");
            let source = SourceIdentity::new("p".into(), "app".into(), "scalar-group.w".into(), "r1".into());
            let prepared = prepare_scalar_project(
                vec![(declarations(&source, &text), Vec::new())],
                vec![source],
                ScalarPreparationInputs { target_layout: layout },
            ).unwrap_or_else(|error| panic!("{layout:?}: {:?}", error.diagnostics));
            let facts = prepared.initializer_facts(0).unwrap();
            let nodes = InitializerCallNodes::bind(facts.region, facts.items).unwrap();
            let ScalarItem::Binding(selected) = &facts.items[0] else { unreachable!() };
            let ScalarExpression::If { then_branch, .. } = &selected.value else { unreachable!() };
            let ScalarBlockItem::LocalBinding(data) = &then_branch.items[1] else { unreachable!() };
            let ScalarBlockItem::Assignment(group) = &then_branch.items[6] else { unreachable!() };
            assert!(nodes.checked_local_array(data));
            for &lane in indexed_lanes {
                let place = &group.targets[lane].place;
                let ScalarPlace::Index { base, .. } = place else { unreachable!() };
                assert_eq!(nodes.checked_local_index(place, base), Some(data.receivers[0].name_span));
                assert_eq!(nodes.checked_local_index(&place.clone(), base), None);
            }
            // The group is not an authority if any retained RHS type is missing.
            let mut region = facts.region.clone();
            let ModuleInitializationDetail::Occurrences(topology) = &mut region.detail else { unreachable!() };
            let group = topology.assignments.values().find(|group| group.destinations().len() == 2).unwrap();
            let rhs = group.rhs_evaluations().next().unwrap().expression;
            assert!(topology.structure.checked_types.expressions.remove(&rhs).is_some());
            let incomplete = InitializerCallNodes::bind(&region, facts.items).unwrap();
            assert!(!incomplete.checked_local_array(data));
        }
    }
}

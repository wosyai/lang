use super::*;

fn module_source(ordinal: usize) -> SourceIdentity {
    SourceIdentity::new(
        "project".into(),
        "package".into(),
        format!("module_{ordinal}.w"),
        "r1".into(),
    )
}

fn fixture(
    module_count: usize,
    declarations: usize,
    body: usize,
    target: ScalarTargetLayout,
) -> (ScalarProject, Vec<String>) {
    let mut modules = Vec::new();
    let mut texts = Vec::new();
    for ordinal in 0..module_count {
        let peer = (ordinal + 1) % module_count;
        let mut text = format!("%%start\npeer = namespace app \"module_{peer}.w\";\nenum Mode {{ ");
        for before in 0..ordinal {
            text.push_str(&format!("before_{before}; "));
        }
        text.push_str("ready; }\n");
        for declaration in 0..declarations {
            text.push_str(&format!(
                "struct Record{declaration} {{ i32 field_{ordinal}; }}\n"
            ));
        }
        for declaration in 0..declarations {
            text.push_str(&format!(
                "i32(Record{declaration}) read_{declaration} = fn(record) {{ unsafe {{ "
            ));
            for _ in 0..body {
                text.push_str(&format!("&?record.field_{ordinal}; "));
            }
            text.push_str(&format!("}}; record.field_{ordinal} }};\n"));
        }
        text.push_str("i32(peer.Record0) foreign = fn(record) { unsafe { ");
        for _ in 0..body {
            text.push_str(&format!("&?record.field_{peer}; "));
        }
        text.push_str(&format!(
            "}}; record.field_{peer} }};\nbool() mode = fn {{ Mode.ready == Mode.ready }};\n%%end"
        ));
        let parsed = parse_source(module_source(ordinal), text.clone(), &[]);
        assert!(parsed.diagnostics.is_empty(), "{:?}", parsed.diagnostics);
        let program = derive_scalar_program_with_layout(&parsed.result, target).program;
        let ScalarItem::Namespace(namespace) = &program.items[0] else {
            panic!("namespace")
        };
        let span = namespace.span;
        modules.push(ScalarModule::from_program(
            program,
            vec![ScalarNamespaceBinding {
                binding: "peer".into(),
                target: module_source(peer),
                span,
            }],
        ));
        texts.push(text);
    }
    (
        ScalarProject::new(modules, (0..module_count).map(module_source).collect()),
        texts,
    )
}

#[test]
fn splitter_and_project_place_entry_borrow_ordered_actual_storage_across_independent_axes() {
    for target in [ScalarTargetLayout::WASM32, ScalarTargetLayout::NATIVE64] {
        for module_count in [2, 4, 8] {
            for declarations in [2, 4, 8] {
                for body in [2, 4, 8] {
                    let (project, texts) = fixture(module_count, declarations, body, target);
                    let public = layout::observations::Scope::new();
                    let validation = validate_scalar_project(project);
                    assert!(
                        validation.diagnostics.is_empty(),
                        "{:?}",
                        validation.diagnostics
                    );
                    assert_eq!(public.counts().module_copies, 0);
                    assert!(public.counts().struct_copies > 0);
                    assert_eq!(public.counts().cfg_copies, 0);
                    drop(public);
                    let mut project = validation.project;
                    for (ordinal, module) in project.modules.iter_mut().enumerate() {
                        let peer = (ordinal + 1) % module_count;
                        assert_eq!(module.items.len(), declarations + 3);
                        for (function_index, item) in
                            module.items[1..declarations + 2].iter_mut().enumerate()
                        {
                            let ScalarItem::Function(function) = item else {
                                panic!("function")
                            };
                            let (owner, structure) = if function_index < declarations {
                                (ordinal, function_index)
                            } else {
                                (peer, 0)
                            };
                            let expected_id = ScalarStructFieldId {
                                structure: ScalarStructId {
                                    source: module_source(owner),
                                    index: structure,
                                },
                                index: 0,
                            };
                            assert_eq!(function.parameters, ["record"]);
                            assert_eq!(function.body.items.len(), 2);
                            assert_eq!(function.body.terminated_items, [true, false]);
                            let ScalarBlockItem::Expression(ScalarExpression::Block(block)) =
                                &mut function.body.items[0]
                            else {
                                panic!("unsafe body")
                            };
                            assert!(block.unsafe_context);
                            assert_eq!(block.items.len(), body);
                            for item in &mut block.items {
                                let ScalarBlockItem::Expression(ScalarExpression::RawAddress {
                                    place: ScalarPlace::Field { field, span, .. },
                                    ..
                                }) = item
                                else {
                                    panic!("field address")
                                };
                                assert_eq!(
                                    *field,
                                    ScalarFieldReference::Resolved(expected_id.clone())
                                );
                                let name = format!("field_{owner}");
                                let field_span =
                                    ByteSpan::new(span.end - name.len() as u32, span.end);
                                assert_eq!(
                                    &texts[ordinal]
                                        [field_span.start as usize..field_span.end as usize],
                                    name
                                );
                                *field = ScalarFieldReference::Unresolved {
                                    name,
                                    span: field_span,
                                };
                            }
                        }
                        let ScalarItem::Function(mode) = module.items.last_mut().unwrap() else {
                            panic!("mode")
                        };
                        let ScalarBlockItem::Expression(ScalarExpression::Binary {
                            left,
                            right,
                            ..
                        }) = &mut mode.body.items[0]
                        else {
                            panic!("enum comparison")
                        };
                        for expression in [left, right] {
                            let ScalarExpression::Member {
                                receiver, enum_tag, ..
                            } = expression.as_mut()
                            else {
                                panic!("enum member")
                            };
                            assert_eq!(*enum_tag, Some(ordinal as u32));
                            *receiver = "peer.Mode".into();
                            *enum_tag = None;
                        }
                    }
                    let storage = project
                        .modules
                        .iter()
                        .map(|module| {
                            (
                                &module.source as *const SourceIdentity,
                                &module.members as *const BTreeMap<String, ScalarType>,
                                module.namespace_bindings.as_ptr(),
                                module.structs.as_ptr(),
                                module.enums.as_ptr(),
                                module.items.as_ptr(),
                            )
                        })
                        .collect::<Vec<_>>();
                    let overloads = ScalarOverloadEnvironment::project(&project.modules);
                    let owners = project
                        .modules
                        .iter()
                        .map(|module| ScalarAssignmentOwners::new(&module.items))
                        .collect::<Vec<_>>();
                    let declarations_environment =
                        ScalarDeclarationEnvironment::new(&project.modules);
                    let scope = layout::observations::Scope::new();
                    let (modules, mut items) = split_module_typing(&mut project.modules);
                    assert_eq!(modules.len(), module_count);
                    assert_eq!(items.len(), module_count);
                    for (ordinal, module) in modules.iter().enumerate() {
                        assert_eq!(module.source, &module_source(ordinal));
                        assert_eq!(module.target_layout, target);
                        assert!(std::ptr::eq(module.source, storage[ordinal].0));
                        assert!(std::ptr::eq(module.members, storage[ordinal].1));
                        assert_eq!(module.namespace_bindings.as_ptr(), storage[ordinal].2);
                        assert_eq!(module.structs.as_ptr(), storage[ordinal].3);
                        assert_eq!(module.enums.as_ptr(), storage[ordinal].4);
                        assert_eq!(items[ordinal].as_ptr(), storage[ordinal].5);
                        assert_eq!(module.namespace_bindings.len(), 1);
                        assert_eq!(module.namespace_bindings[0].binding, "peer");
                        assert_eq!(
                            module.namespace_bindings[0].target,
                            module_source((ordinal + 1) % module_count)
                        );
                        assert_eq!(module.structs.len(), declarations);
                        assert_eq!(module.enums.len(), 1);
                        assert_eq!(module.enums[0].id.source, module_source(ordinal));
                        for (index, structure) in module.structs.iter().enumerate() {
                            assert_eq!(structure.name, format!("Record{index}"));
                            assert_eq!(
                                structure.id,
                                ScalarStructId {
                                    source: module_source(ordinal),
                                    index
                                }
                            );
                            assert_eq!(structure.fields.len(), 1);
                            assert_eq!(structure.fields[0].name, format!("field_{ordinal}"));
                        }
                    }
                    for (ordinal, current_items) in items.iter_mut().enumerate() {
                        resolution::resolve_module_places(
                            current_items,
                            &modules[ordinal],
                            &modules,
                            &declarations_environment,
                            ordinal,
                            &overloads.typing(ordinal),
                            &owners[ordinal],
                        );
                    }
                    assert_eq!(scope.counts(), layout::observations::Counts::default());
                    drop(scope);
                    for (ordinal, current_items) in items.iter().enumerate() {
                        let peer = (ordinal + 1) % module_count;
                        for (function_index, item) in
                            current_items[1..declarations + 2].iter().enumerate()
                        {
                            let ScalarItem::Function(function) = item else {
                                panic!("function")
                            };
                            let (owner, structure) = if function_index < declarations {
                                (ordinal, function_index)
                            } else {
                                (peer, 0)
                            };
                            assert_eq!(
                                function.name,
                                if function_index < declarations {
                                    format!("read_{function_index}")
                                } else {
                                    "foreign".into()
                                }
                            );
                            assert!(
                                matches!(&function.signature, ScalarType::Callable { parameters, .. } if parameters == &[ScalarType::Struct(ScalarStructId { source: module_source(owner), index: structure })])
                            );
                            let ScalarBlockItem::Expression(ScalarExpression::Block(block)) =
                                &function.body.items[0]
                            else {
                                panic!("unsafe body")
                            };
                            assert_eq!(block.items.len(), body);
                            for item in &block.items {
                                let ScalarBlockItem::Expression(ScalarExpression::RawAddress {
                                    place: ScalarPlace::Field { field, base, .. },
                                    ..
                                }) = item
                                else {
                                    panic!("field address")
                                };
                                assert!(
                                    matches!(base.as_ref(), ScalarPlace::Name { name, .. } if name == "record")
                                );
                                assert_eq!(
                                    *field,
                                    ScalarFieldReference::Resolved(ScalarStructFieldId {
                                        structure: ScalarStructId {
                                            source: module_source(owner),
                                            index: structure
                                        },
                                        index: 0
                                    })
                                );
                            }
                        }
                        let ScalarItem::Function(mode) = current_items.last().unwrap() else {
                            panic!("mode")
                        };
                        let ScalarBlockItem::Expression(ScalarExpression::Binary {
                            left,
                            right,
                            ..
                        }) = &mode.body.items[0]
                        else {
                            panic!("enum comparison")
                        };
                        for expression in [left, right] {
                            assert!(
                                matches!(expression.as_ref(), ScalarExpression::Member { receiver, enum_tag, .. } if receiver == "peer.Mode" && *enum_tag == Some(peer as u32))
                            );
                        }
                        assert_eq!(
                            resolution::enum_member_in_module(
                                &modules[ordinal],
                                &modules,
                                "peer.Mode",
                                "ready"
                            ),
                            Some((
                                ScalarEnumId {
                                    source: module_source(peer),
                                    index: 0
                                },
                                peer as u32
                            ))
                        );
                    }
                    drop(items);
                    drop(modules);
                    assert_eq!(project.modules.len(), module_count);
                }
            }
        }
    }
}

#[test]
fn project_places_keep_current_ordinal_and_first_complete_import_source_distinct() {
    let mut modules = Vec::new();
    for (path, revision, tag) in [
        ("same.w", "r2", 2),
        ("same.w", "r1", 0),
        ("same.w", "r1", 1),
    ] {
        let source = SourceIdentity::new(
            "project".into(),
            "package".into(),
            path.into(),
            revision.into(),
        );
        let mut text = String::from("%%start\nenum Mode { ");
        for before in 0..tag {
            text.push_str(&format!("before_{before}; "));
        }
        text.push_str("ready; }\nbool() mode = fn { Mode.ready == Mode.ready };\n%%end");
        let parsed = parse_source(source, text, &[]);
        assert!(parsed.diagnostics.is_empty());
        let result = derive_scalar_program(&parsed.result);
        assert!(result.diagnostics.is_empty(), "{:?}", result.diagnostics);
        modules.push(ScalarModule::from_program(result.program, Vec::new()));
    }
    let target = modules[1].source.clone();
    modules[2].namespace_bindings.push(ScalarNamespaceBinding {
        binding: "peer".into(),
        target,
        span: ByteSpan::new(0, 0),
    });
    let ScalarItem::Function(mode) = &mut modules[2].items[0] else {
        panic!("mode")
    };
    let ScalarBlockItem::Expression(ScalarExpression::Binary { left, right, .. }) =
        &mut mode.body.items[0]
    else {
        panic!("enum comparison")
    };
    let ScalarExpression::Member { enum_tag, .. } = left.as_mut() else {
        panic!("local enum")
    };
    *enum_tag = None;
    let ScalarExpression::Member {
        receiver, enum_tag, ..
    } = right.as_mut()
    else {
        panic!("imported enum")
    };
    *receiver = "peer.Mode".into();
    *enum_tag = None;
    let overloads = ScalarOverloadEnvironment::project(&modules);
    let owners = modules
        .iter()
        .map(|module| ScalarAssignmentOwners::new(&module.items))
        .collect::<Vec<_>>();
    let declarations = ScalarDeclarationEnvironment::new(&modules);
    let scope = layout::observations::Scope::new();
    let (modules, mut items) = split_module_typing(&mut modules);
    resolution::resolve_module_places(
        items[2],
        &modules[2],
        &modules,
        &declarations,
        2,
        &overloads.typing(2),
        &owners[2],
    );
    assert_eq!(scope.counts(), layout::observations::Counts::default());
    let ScalarItem::Function(mode) = &items[2][0] else {
        panic!("mode")
    };
    let ScalarBlockItem::Expression(ScalarExpression::Binary { left, right, .. }) =
        &mode.body.items[0]
    else {
        panic!("enum comparison")
    };
    assert!(matches!(
        left.as_ref(),
        ScalarExpression::Member {
            enum_tag: Some(1),
            ..
        }
    ));
    assert!(matches!(
        right.as_ref(),
        ScalarExpression::Member {
            enum_tag: Some(0),
            ..
        }
    ));
    assert_eq!(
        resolution::enum_member_in_module(&modules[2], &modules, "Mode", "ready")
            .unwrap()
            .1,
        1
    );
    assert_eq!(
        resolution::enum_member_in_module(&modules[2], &modules, "peer.Mode", "ready")
            .unwrap()
            .1,
        0
    );
}
